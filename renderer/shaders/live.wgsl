// Real-time viewport passes, run after the 1-spp LIVE path-trace dispatch. The frame is traced, denoised and composed
// at the render resolution (the traced grid: `isz`), and one temporal upscaler takes it to the output grid (`size`), as
// UE's TSR does after Lumen and the vendor upscalers after a denoiser:
//   refl               - the specular reflection resolved from neighbours' rays
//   dtemporal, atrous* - diffuse irradiance: accumulated over its reprojected history, edge-avoiding wavelet filter
//   stemporal, sfilt*  - specular: accumulated over its surface / reflection-hit history, filtered by roughness
//   compose            - albedo x irradiance + specular + what the camera ray saw + deterministic direct light
//   judge, reject      - what of the output-grid colour history this frame's samples contradict
//   upscale            - the composed frame, by its sub-pixel jitter, into the output grid's one colour history
//   dof*, mblur        - depth of field and motion blur on the output grid
//   lum_down, exposure - quarter-resolution luminance: auto exposure's histogram, local exposure's layers, bloom's source
//   bloom_*            - bloom: a sum of Gaussians on a 1/4 .. 1/128 pyramid, the widest summed directly on its top
//   post               - exposure, local exposure, bloom, vignette, grading and tone curve (a 3D table), grain, BGRA8 output
// (The post chain is post.rs's, pass for pass: what each step is and where it comes from is said there.)

struct Cam { pos: vec4<f32>, fwd: vec4<f32>, right: vec4<f32>, up: vec4<f32> }; // pos.w = tan(fov/2), fwd.w = aspect
struct LiveParams {
    cur: Cam,
    prev: Cam,
    size: vec4<u32>,   // width, height, frame, reset history (1)
    misc: vec4<f32>,   // exposure scale, max history, vignette, the bloom pyramid is needed (1) / not (0)
    misc2: vec4<f32>,  // denoise strength, sample clamp (display units at the current exposure), motion blur's length over this frame's motion (0 = none), samples this frame
    mode: vec4<u32>,   // view mode (0 lit, 1 base color, 2 world normal, 3 lit no denoise, 4 raw sample, 5 history), display whites a unit of a reconstruction provider's input is (reconstruction.wgsl `rc_radiance_unit`), the scene's lighting is changing (live/mod.rs `live_lighting`), -
    post: vec4<f32>,   // chromatic aberration, film grain, bloom threshold (exposed luminance; < 0: everything blooms), longest motion blur (output pixels)
    ae: vec4<f32>,     // auto exposure: low / high fraction of the histogram, its log2 luminance min / max
    ae2: vec4<f32>,    // compensation (2^bias; manual: the exposure), min / max average luminance, mode (1 = auto)
    ae3: vec4<f32>,    // speed toward brighter / darker scenes (EV/s), frame time (s), snap to target
    lx: vec4<f32>,     // local exposure: highlight contrast, shadow contrast, blurred-luminance blend, detail strength
    isz: vec4<u32>,    // traced (internal) width, height (= size.xy at 100% render scale); the previous frame's (0 = none)
    bl: vec4<f32>,     // bloom (post.rs `BloomPlan`): weight of pyramid level 0..3
    bl2: vec4<f32>,    // of level 4, 5; the wide pass's reach (texels of the coarsest level; 0 = nothing to sum); weight of the output's own five-pixel kernel
    bw: array<vec4<f32>, 3>, // the wide Gaussians 0..5: a (weight, 1 / (2 sigma^2)) pair each (sigma in texels of the coarsest level)
    lb: vec4<f32>,     // local exposure's wide blur of log luminance: weight of level 0..3
    lb2: vec4<f32>,    // of level 4, 5; its wide Gaussian's weight, 1 / (2 sigma^2)
    dof: vec4<f32>,    // depth of field: CoC radius of infinity (output px; 0 = off), focus distance (m), max radius (px), -
    jit: vec4<f32>,    // this frame's sub-pixel jitter (every traced sample's: x, y in 0 .. 1 of a traced pixel), -, -
    state: vec4<u32>,  // which half of the lasting buffers is this frame's (0 / 1; `this_half`), -, -, -
    lt: vec4<u32>,     // the light tiles (`light_tiles`): this frame's grid offset x, y (traced pixels), -, tiles a row
};
// (`Pixel`, a sample of the trace: records.wgsl, a part of this module. normal.w: `medium_tr`, `medium_share`.)

@group(0) @binding(0) var<uniform> L: LiveParams;
@group(0) @binding(1) var<storage, read> sample_px: array<Pixel>;          // color.w = distance to the surface shown (0 = sky): the camera hit's, or the whole way to what a mirror chain shows

// ---- The live passes' buffers, by what they hold and how long.
//
// The declarations below are the passes' layout: build.rs reads them (with naga) when the renderer is built, and each
// pass binds what its entry point uses, natively and in a browser alike (at most eight storage buffers a pass, a
// browser's: live/layout.rs's test); live/layout.rs says what each binding is bound to, how large a buffer of the
// passes' own is, and the passes' order and grids. A buffer more: its declaration here, a line there.
//
// What lasts from one frame to the next has a buffer of its own, in two halves: this frame's, which the passes write
// (`this_half`), and the previous frame's, which they read (`last_half`); live/frame.rs alternates them (`L.state.x`). A half
// is L.size.x * L.size.y elements, indexed by traced sample (by output pixel: `color_hist`).
//   diffuse_hist  the diffuse irradiance's history: rgb, frames (`dtemporal`; the à-trous passes read its length)
//   diffuse_mom   its luminance moments: mean, mean square, -, 1 where written (`dtemporal`); z: within the frame
//                 the diffuse history's drift factor (`dtemporal`), then the specular history's change chart
//                 (`stemporal` reads the one and writes the other into the half `dtemporal` has written: `spec_chart`)
//   diffuse_fast  the diffuse history's change chart and drift ("The response of the histories"), eight bits each:
//                 x the level of the samples' share against the history (rgb) and the drift, y the pooled residual
//                 average (rgb) and the root of its mean square (`dtemporal`)
//   spec_hist     the specular history: rgb, frames (`stemporal`)
//   gbuffer       octahedral normal, view depth (0 = sky), roughness (`stemporal`; `GB`); once the temporal passes
//                 have read the previous half, `lock_carry` puts its locks' depths into it (`judge` reads it)
//   color_hist    the output grid's colour history: rgb, samples (`upscale`)
//   up_state      the upscaler's state per traced sample (`track`): x, y `Flicker`; z, w `Lock` (its life and the
//                 view depth of what it is for; where in the traced pixel that is, f16 each)
// What lives within a frame shares buffers whose uses do not overlap (the passes run in live/layout.rs `PASSES` order):
//   sample_vel    per traced sample: screen velocity (output pixels this frame), view depth, animated shading
//                 (`compose`; `judge`, `upscale`, depth of field and motion blur read it)
//   judged        `judge`'s findings per traced sample (`Judged`), in up_state's previous half: `track` has read it
//                 by then, and the next frame writes the other (`judged_slot`)
//   fa, fb        the irradiance filter's ping-pong on the traced grid; then fb the composed colour of the traced grid
//                 (`compose`; w: first-hit distance) and fa the image on the output grid (what the post chain reads);
//                 fb depth of field's output when motion blur follows
//   sb            the traced surfaces for the diffuse filter (`refl` writes, the à-trous passes read), then the
//                 upscaled image when depth of field / motion blur follow (w: the circle of confusion)
//   spec_pp       the specular filter's ping-pong (`stemporal` -> `sfilt0..2`: two f16 slots, the result in slot 1),
//                 then in slot 0 `reject`'s verdict
//   lumq          depth of field's tiles (`dof_tiles`, `dof`), then the quarter-resolution log2 luminance
//                 (`lum_down`, `lum_blur`)
//   bloom_a       the bloom pyramid: its levels one after another (`bloom_at`), each the picture's until its `bloom_up`
//                 makes it the sum from there down; then the coarsest eight times more: `bloom_wide`'s sum and
//                 `bloom_wide_rows`' row blurs
//   lock_fwd      per traced sample: the previous frame's lock that lands on it (`lock_carry` writes, `track` reads
//                 and clears it for the next frame)
@group(0) @binding(2) var<storage, read_write> diffuse_hist: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> color_hist: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> gbuffer: array<vec2<u32>>;
@group(0) @binding(5) var<storage, read_write> spec_hist: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> diffuse_mom: array<vec4<f32>>;
@group(0) @binding(7) var<storage, read_write> diffuse_fast: array<vec2<u32>>;
@group(0) @binding(8) var<storage, read_write> up_state: array<vec4<u32>>;
@group(0) @binding(9) var<storage, read_write> sample_vel: array<vec4<f32>>;
@group(0) @binding(10) var<storage, read_write> out_px: array<u32>;
@group(0) @binding(11) var lut: texture_3d<f32>;
@group(0) @binding(12) var lut_samp: sampler;
@group(0) @binding(13) var<storage, read_write> ae_bins: array<atomic<u32>, 128>;  // the luminance histogram: 64 counts, then 64 sums (`lum_down`)
@group(0) @binding(14) var<storage, read_write> ae_state: array<vec4<f32>, 2>;    // exposure, target, mean log2 lum, valid
@group(0) @binding(15) var<storage, read_write> lumq: array<f32>;                 // quarter res log2 luminance (unexposed)
@group(0) @binding(16) var<storage, read_write> lumb: array<f32>;                 // edge-aware blur of lumq
@group(0) @binding(17) var<storage, read_write> fa: array<vec4<f32>>;
@group(0) @binding(18) var<storage, read_write> fb: array<vec4<f32>>;
@group(0) @binding(19) var<storage, read_write> bloom_a: array<vec4<f32>>;    // rgb bloom, w log2 luminance
@group(0) @binding(21) var<storage, read_write> spec_pp: array<vec2<u32>>;
@group(0) @binding(22) var<storage, read_write> sb: array<vec4<f32>>;
// What the trace leaves (the kernel's group 1, live/targets.rs `Targets::kernel`). Reflection resolve: the specular-lobe rays,
// 2 per traced pixel ((radiance back, lobe pdf; 0 = none), (specular albedo E, octahedral direction)), and the
// resolved specular reflection per traced pixel (`refl`).
@group(0) @binding(23) var<storage, read> refl_in: array<vec4<f32>>;
@group(0) @binding(24) var<storage, read_write> refl_res: array<vec4<f32>>;
// Per traced pixel: where its surface point was the previous frame, as a world-space offset (w = 1: a moving object).
@group(0) @binding(25) var<storage, read> motion_in: array<vec4<f32>>;
// Directional lights' direct light (the sun: nearly deterministic) at camera hits per traced pixel: no denoiser
// (`compose` adds it as it is, UE lights directly).
@group(0) @binding(26) var<storage, read> direct_in: array<vec4<f32>>;
@group(0) @binding(27) var<storage, read_write> lock_fwd: array<atomic<u32>>;
// For `light_tiles` alone: the kernel's direct buffer again (w: the local light a traced pixel's camera hit's
// resampled shadow ray found; pathtrace.wgsl "Light tiles"; read only, bound as the lists are: in the merged layout
// both are ranges of one buffer, which a pass uses one way), and the tiles' lists it writes.
@group(0) @binding(28) var<storage, read_write> light_rw: array<vec4<f32>>;
@group(0) @binding(29) var<storage, read_write> light_lists: array<vec4<f32>>;

// Element i of this frame's half of a lasting buffer, and of the previous frame's.
fn this_half(i: u32) -> u32 { return L.state.x * L.size.x * L.size.y + i; }
fn last_half(i: u32) -> u32 { return (1u - L.state.x) * L.size.x * L.size.y + i; }
// Where `judge`'s findings for traced sample q are (see `judged` above).
fn judged_slot(q: u32) -> u32 { return last_half(q); }

fn spec_load(slot: u32, i: u32) -> vec4<f32> {
    let v = spec_pp[slot * L.size.x * L.size.y + i];
    return vec4<f32>(unpack2x16float(v.x), unpack2x16float(v.y));
}

fn spec_store(slot: u32, i: u32, c: vec4<f32>) {
    let v = min(c, vec4<f32>(6.0e4));
    spec_pp[slot * L.size.x * L.size.y + i] = vec2<u32>(pack2x16float(v.rg), pack2x16float(v.ba));
}

fn ray_dir(c: Cam, x: f32, y: f32) -> vec3<f32> {
    let w = f32(L.size.x);
    let h = f32(L.size.y);
    let nx = x / w * 2.0 - 1.0;
    let ny = 1.0 - y / h * 2.0;
    return normalize(c.fwd.xyz + c.right.xyz * (nx * c.pos.w * c.fwd.w) + c.up.xyz * (ny * c.pos.w));
}

fn non_finite(v: vec3<f32>) -> bool {
    let b = bitcast<vec3<u32>>(v) & vec3<u32>(0x7fffffffu);
    return any(b >= vec3<u32>(0x7f800000u));
}

// Exposure and bloom read the whole frame: one bad pixel must not poison them (bloom would spread it everywhere).
fn finite_or_zero(v: vec3<f32>) -> vec3<f32> {
    return select(v, vec3<f32>(0.0), non_finite(v));
}

// YCoCg (Malvar & Sullivan 2003, "YCoCg-R: A Color Space with RGB Reversibility and Low Dynamic Range", the lossy form):
// neighbourhood statistics in a luma / chroma space.
fn ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}
fn ycocg_rgb(y: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(y.x + y.y - y.z, y.x + y.z, y.x - y.y - y.z);
}

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// ------------------------------------------------------------------------------------------------ temporal

// One traced sample's parts (averaged over this frame's samples): diffuse lighting, specular lighting (with the
// resolved reflection, `refl`), and what the camera ray saw directly (emission, sky). Lighting is clamped at
// `sample_clamp()` (an exposure-relative ceiling: fireflies far above white). Each part on its own: a pass binds only
// what the part it takes reads.
fn sample_diffuse(q: u32) -> vec3<f32> {
    var d = sample_px[q].color.rgb / max(L.misc2.w, 1.0);
    if (non_finite(d)) { d = vec3<f32>(0.0); }
    let cl = sample_clamp();
    if (lum(d) > cl) { d *= cl / lum(d); }
    return d;
}

fn sample_specular(q: u32) -> vec3<f32> {
    var s = sample_px[q].spec.rgb / max(L.misc2.w, 1.0) + refl_res[q].rgb;
    if (non_finite(s)) { s = vec3<f32>(0.0); }
    let cl = sample_clamp();
    if (lum(s) > cl) { s *= cl / lum(s); }
    return s;
}

fn sample_emission(q: u32) -> vec3<f32> {
    var e = sample_px[q].emit.rgb / max(L.misc2.w, 1.0);
    if (non_finite(e)) { e = vec3<f32>(0.0); }
    return e;
}

// Roughness below which a surface counts as a near mirror: its reflection hit distance is the resolved one (pass `refl`).
const NEAR_MIRROR: f32 = 0.2;

// ------------------------------------------------------------------------------------------------ reflection resolve

// The specular lobe's distribution and visibility terms, as pathtrace.wgsl has them (`d_ggx`,
// `vis_smith_joint_approx`: GGX of Walter et al. 2007; the height-correlated Smith visibility of Heitz 2014 in Hammon's
// square-root-free form, GDC 2017), for the reflection resolve's weights.
fn d_ggx(a2: f32, noh: f32) -> f32 {
    let d = (noh * a2 - noh) * noh + 1.0;
    return a2 / (3.14159265 * d * d);
}

fn vis_smith(a2: f32, nov: f32, nol: f32) -> f32 {
    return 0.5 / max(mix(2.0 * nol * nov, nol + nov, sqrt(a2)), 1e-7);
}

fn oct_dec(u: u32) -> vec3<f32> {
    return oct_dec_f(unpack2x16snorm(u));
}

fn oct_dec_f(p: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    let t = max(-n.z, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    return normalize(n);
}

fn oct_enc(n: vec3<f32>) -> vec2<f32> {
    let p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if (n.z >= 0.0) { return p; }
    return (1.0 - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
}

// The g-buffer (`gbuffer`), 8 bytes: octahedral normal (snorm16 x 2), view depth
// (0 = sky) and roughness (f16). The surface tests compare depths relatively (2-4 %), far coarser than f16. One fetch
// per tap for the specular filter and the history tests (it was 16 bytes, and roughness a second 16-byte fetch from the
// albedo / emission history): those passes are bandwidth-bound. `moves`: the surface is a moving object's and crossed
// a traced sample this frame (`stemporal`), kept in the roughness's sign (-1 - roughness), so the next frame knows what
// was in front of what it uncovers was moving (`judge`) and that a lock on it moved with it (`lock_carry`).
struct GB { n: vec3<f32>, depth: f32, rough: f32, moves: bool };

// The second word: view depth, roughness and `moves`.
fn gb_dr(depth: f32, rough: f32, moves: bool) -> u32 {
    return pack2x16float(vec2<f32>(min(depth, 6.0e4), select(rough, -1.0 - rough, moves)));
}

fn gb_pack(n: vec3<f32>, depth: f32, rough: f32, moves: bool) -> vec2<u32> {
    return vec2<u32>(pack2x16snorm(oct_enc(n)), gb_dr(depth, rough, moves));
}

fn gb_unpack(g: vec2<u32>) -> GB {
    let dr = unpack2x16float(g.y);
    let moves = dr.y < -0.5;
    return GB(oct_dec(g.x), dr.x, select(dr.y, -1.0 - dr.y, moves), moves);
}

const RESOLVE_N: u32 = 8u;

// Each traced pixel's specular reflection from its own and its neighbours' specular-lobe rays (Stachowiak 2015,
// Lumen's spatial reconstruction): a ray from neighbour j, re-aimed from this surface point at j's hit, counts with
// this pixel's BRDF over j's pdf, and the weighted mean times this pixel's specular albedo E is the estimate. The
// ratio cancels most of the pdf noise that a blur of the same samples keeps. Neighbours on another surface (depth,
// normal) don't count; the radius grows with roughness (mirror-like surfaces keep their own ray).
// w: the reflection's hit distance, weighted the same way (0: none), for the temporal pass's mirror-image
// reprojection. A pixel's own ray is one random direction of the lobe (or none, when it took the diffuse lobe), so
// its distance alone put the mirror image anywhere along the ray.
// ---- The light tiles: each 8 x 8 tile's pixels' reports made its list (pathtrace.wgsl "Light tiles": the lights
// reported, each once: index, reports; ascending; the 64th vec4 holds their number, 63 at most). A workgroup is a
// tile: the grid is the traced one shifted by L.lt.xy, another offset each frame, so tile borders do not stay put.
// (LT_LIST, LT_NONE: records.wgsl. A list is as long as a tile has pixels.)
var<workgroup> lt_keys: array<u32, 64>;
var<workgroup> lt_first: array<u32, 64>;

@compute @workgroup_size(8, 8, 1)
fn light_tiles(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let px = vec2<i32>(gid.xy) - vec2<i32>(L.lt.xy);
    var key = LT_NONE;
    if (px.x >= 0 && px.y >= 0 && px.x < i32(L.isz.x) && px.y < i32(L.isz.y)) {
        key = min(u32(light_rw[u32(px.y) * L.isz.x + u32(px.x)].w), LT_NONE);
    }
    lt_keys[li] = key;
    workgroupBarrier();
    // Whether it is the first of the tile's reports of its light, and how many there are (four reports a step).
    var same = vec4<u32>(0u);
    var before = vec4<u32>(0u);
    let k0 = vec4<u32>(key);
    for (var j = 0u; j < 64u; j += 4u) {
        let eq = vec4<u32>(lt_keys[j], lt_keys[j + 1u], lt_keys[j + 2u], lt_keys[j + 3u]) == k0;
        same += vec4<u32>(eq);
        before += vec4<u32>(eq & (vec4<u32>(j, j + 1u, j + 2u, j + 3u) < vec4<u32>(li)));
    }
    let first = key != LT_NONE && before.x + before.y + before.z + before.w == 0u;
    lt_first[li] = u32(first);
    workgroupBarrier();
    // The first reports are the list: each after those of smaller keys.
    var less = vec4<u32>(0u);
    var all = vec4<u32>(0u);
    for (var j = 0u; j < 64u; j += 4u) {
        let f = vec4<u32>(lt_first[j], lt_first[j + 1u], lt_first[j + 2u], lt_first[j + 3u]);
        less += f * vec4<u32>(vec4<u32>(lt_keys[j], lt_keys[j + 1u], lt_keys[j + 2u], lt_keys[j + 3u]) < k0);
        all += f;
    }
    let list = (wg.y * L.lt.w + wg.x) * LT_LIST;
    let at = less.x + less.y + less.z + less.w;
    if (first && at < LT_LIST - 1u) { light_lists[list + at] = vec4<f32>(f32(key), f32(same.x + same.y + same.z + same.w), 0.0, 0.0); }
    if (li == 0u) { light_lists[list + LT_LIST - 1u] = vec4<f32>(f32(min(all.x + all.y + all.z + all.w, LT_LIST - 1u)), 0.0, 0.0, 0.0); }
}

@compute @workgroup_size(8, 8, 1)
fn refl(@builtin(global_invocation_id) gid: vec3<u32>) {
    let iw = L.isz.x;
    let ih = L.isz.y;
    if (gid.x >= iw || gid.y >= ih) { return; }
    let i = gid.y * iw + gid.x;
    var outv = vec3<f32>(0.0);
    var rt = 0.0;
    let px = sample_px[i];
    let t = px.color.w;
    // (And a compact copy of the sample's surface for the diffuse filter's edge stops, `sb`, free until the upscaler:
    // normal, first-hit distance, negative on animated emitters. Here because this pass reads the sample whole: the
    // 80-byte samples cost the filter twice the bandwidth, and a pass of its own read them again, 0.18 ms.)
    sb[i] = vec4<f32>(normalize(px.normal.xyz + vec3<f32>(1e-6)), select(t, -t, px.albedo.w > 1.5 && t > 0.0));
    if (t > 0.0 && arrayLength(&refl_in) >= 2u * iw * ih) {
        let e = refl_in[i * 2u + 1u].rgb;
        if (any(e > vec3<f32>(0.0))) {
            let n = normalize(px.normal.xyz + vec3<f32>(1e-6));
            let rough = max(px.emit.w, 0.02);
            let a2 = rough * rough * rough * rough;
            let sp = sample_pos(i32(gid.x), i32(gid.y));
            let d = ray_dir(L.cur, sp.x, sp.y);
            let x = L.cur.pos.xyz + d * t;
            let v = -d;
            let nov = max(dot(n, v), 1e-4);
            let radius = clamp(rough * 24.0, 1.0, 10.0);
            let rot = f32((i * 2654435761u + L.size.z * 40503u) >> 8u) / 16777216.0 * 6.2831853;
            var sum = vec3<f32>(0.0);
            var wsum = 0.0;
            var tsum = 0.0;
            var twsum = 0.0;
            var own = vec3<f32>(0.0); // this pixel's specular share when its sample took the diffuse lobe
            for (var k = 0u; k <= RESOLVE_N; k++) {
                var q = vec2<i32>(gid.xy);
                if (k > 0u) {
                    // Vogel disk, rotated per pixel and frame.
                    let r = radius * sqrt((f32(k) - 0.5) / f32(RESOLVE_N));
                    let a = f32(k) * 2.3999632 + rot;
                    q += vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * r));
                }
                if (q.x < 0 || q.y < 0 || q.x >= i32(iw) || q.y >= i32(ih)) { continue; }
                let j = u32(q.y) * iw + u32(q.x);
                let rj = refl_in[j * 2u];
                if (k == 0u && rj.w < 0.0) { own = rj.rgb; }
                if (rj.w <= 0.0) { continue; }
                let pj = sample_px[j];
                let tj = pj.color.w;
                if (k > 0u && (tj <= 0.0 || abs(tj - t) > 0.05 * t + 0.02 || dot(normalize(pj.normal.xyz + vec3<f32>(1e-6)), n) < 0.9)) { continue; }
                var l = oct_dec(bitcast<u32>(refl_in[j * 2u + 1u].w));
                let st = pj.spec.w;
                if (k > 0u && st > 0.0 && st < 1.0e3) {
                    // Re-aim at j's hit from this surface point (contact reflections).
                    let pjp = sample_pos(q.x, q.y);
                    let xj = L.cur.pos.xyz + ray_dir(L.cur, pjp.x, pjp.y) * tj;
                    l = normalize(xj + l * st - x);
                }
                let nol = dot(n, l);
                if (nol <= 0.0) { continue; }
                let h = normalize(v + l);
                let w = d_ggx(a2, max(dot(n, h), 0.0)) * vis_smith(a2, nov, nol) * nol / rj.w;
                if (!(w > 0.0) || non_finite(rj.rgb)) { continue; }
                sum += rj.rgb * w;
                wsum += w;
                if (st > 0.0 && st < 1.0e3) {
                    tsum += st * w;
                    twsum += w;
                }
            }
            if (wsum > 0.0) { outv = e * sum / wsum; } else { outv = own; }
            if (twsum > 0.0) { rt = tsum / twsum; }
        }
    }
    if (non_finite(outv)) { outv = vec3<f32>(0.0); }
    refl_res[i] = vec4<f32>(outv, select(0.0, rt, rt > 0.0 && rt < 1.0e3));
}

// Ceiling for one lighting sample: misc2.y display units at the current (auto) exposure.
fn sample_clamp() -> f32 {
    return L.misc2.y / max(exposure_scale(), 1e-8);
}

// The media between the camera and traced sample q's first hit (pathtrace.wgsl `media_composite`): their
// transmittance, which the sample's diffuse and specular lighting do not carry (it is applied where they are composed,
// after their histories and filters), and how much of the pixel is a medium that changes with time.
fn medium_tr(q: u32) -> f32 {
    return clamp(fract(sample_px[q].normal.w) * 2.0, 0.0, 1.0);
}
fn medium_share(q: u32) -> f32 {
    return floor(sample_px[q].normal.w) / 255.0;
}

fn sample_color(q: u32) -> vec3<f32> {
    return (sample_diffuse(q) + sample_specular(q)) * medium_tr(q) + sample_emission(q);
}

// The traced sample under an output pixel (nearest by the pixels' centres).
fn sample_under(x: u32, y: u32) -> u32 {
    let sx = min(u32((f32(x) + 0.5) * f32(L.isz.x) / f32(L.size.x)), L.isz.x - 1u);
    let sy = min(u32((f32(y) + 0.5) * f32(L.isz.y) / f32(L.size.y)), L.isz.y - 1u);
    return sy * L.isz.x + sx;
}

// Position of traced sample (qx, qy), in output pixels: every sample of a frame has the frame's jitter.
fn sample_pos(qx: i32, qy: i32) -> vec2<f32> {
    return (vec2<f32>(f32(qx), f32(qy)) + L.jit.xy) * vec2<f32>(f32(L.size.x) / f32(L.isz.x), f32(L.size.y) / f32(L.isz.y));
}


// Previous-frame pixel position of world point p (x < 0: behind the previous camera).
fn prev_pixel(p: vec3<f32>) -> vec3<f32> {
    let d = p - L.prev.pos.xyz;
    let z = dot(d, L.prev.fwd.xyz);
    if (z <= 1e-4) { return vec3<f32>(-1.0, -1.0, 0.0); }
    let px = dot(d, L.prev.right.xyz) / (z * L.prev.pos.w * L.prev.fwd.w);
    let py = dot(d, L.prev.up.xyz) / (z * L.prev.pos.w);
    return vec3<f32>((px * 0.5 + 0.5) * f32(L.size.x) - 0.5, (0.5 - py * 0.5) * f32(L.size.y) - 0.5, z);
}

// The same in this frame.
fn cur_pixel(p: vec3<f32>) -> vec3<f32> {
    let d = p - L.cur.pos.xyz;
    let z = dot(d, L.cur.fwd.xyz);
    if (z <= 1e-4) { return vec3<f32>(-1.0, -1.0, 0.0); }
    let px = dot(d, L.cur.right.xyz) / (z * L.cur.pos.w * L.cur.fwd.w);
    let py = dot(d, L.cur.up.xyz) / (z * L.cur.pos.w);
    return vec3<f32>((px * 0.5 + 0.5) * f32(L.size.x) - 0.5, (0.5 - py * 0.5) * f32(L.size.y) - 0.5, z);
}

fn camera_moves() -> bool {
    return any(L.cur.pos != L.prev.pos) || any(L.cur.fwd != L.prev.fwd) || any(L.cur.right != L.prev.right) || any(L.cur.up != L.prev.up);
}

// The world point camera c sees at output position e (pixel edges: sample_pos's) at view depth z (0: the sky, as far
// as compose puts it).
fn view_point(c: Cam, e: vec2<f32>, z: f32) -> vec3<f32> {
    let dir = ray_dir(c, e.x, e.y);
    return c.pos.xyz + dir * select(1.0e6, z / max(dot(dir, c.fwd.xyz), 1e-6), z > 0.0);
}

// ------------------------------------------------------------------------------------------------ diffuse, traced grid
// The diffuse irradiance (diffuse lighting / albedo) is accumulated (`dtemporal`) and filtered (`atrous*`) on the traced
// grid; `compose` remodulates it by each sample's own albedo.

// The previous frame's surface at its traced pixel (tx, ty) is this one (previous-view depth z, normal n; t = 0: sky).
fn same_surface(tx: i32, ty: i32, z: f32, n: vec3<f32>, t: f32) -> bool {
    return same_surface_n(tx, ty, z, n, t, 0.8);
}

// same_surface with the normals' least cosine `ncos`.
fn same_surface_n(tx: i32, ty: i32, z: f32, n: vec3<f32>, t: f32, ncos: f32) -> bool {
    let g = gb_unpack(gbuffer[last_half(u32(ty) * L.isz.z + u32(tx))]);
    if (t > 0.0) { return g.depth > 0.0 && abs(g.depth - z) < 0.04 * z + 0.002 && dot(g.n, n) > ncos; }
    return g.depth == 0.0;
}

// History length (frames) from which `dtemporal` reprojects the pixel's centre instead of its jittered sample.
const HIST_CENTRE: f32 = 16.0;
// Diffuse history length from which the firefly cut stops (dtemporal).
const FIREFLY_FRAMES: f32 = 16.0;

// ---- The response of the histories: how long a pixel's history may be, and how it follows the lighting.
// (Written 2026-11; docs/RUNTIME.md "The temporal response of the live denoiser". Sources are named where a rule
// rests on one, and only texts that were opened for this work: Schied, Peters and Dachsbacher 2018, "Gradient
// Estimation for Real-Time Adaptive Temporal Filtering"; Karis 2014, "High-Quality Temporal Supersampling",
// SIGGRAPH Advances in Real-Time Rendering course slides; Stachowiak 2015, "Stochastic Screen-Space Reflections",
// same course, slides and notes; the NIST/SEMATECH e-Handbook of Statistical Methods, § 6.3.2.4, "EWMA Control
// Charts". The rest is this project's own.)
//
// Within the frame's bound (`L.misc.y`) a history's length is the pixel's own:
// - In motion. A history that moves over the grid is resampled bilinearly every frame, and each resampling blurs it
//   (Karis 2014, slide 47, "reprojection diffusion": reduce the feedback when reprojecting off the pixel). A
//   bilinear tap at a fraction f of a pixel is a two-point kernel of variance f (1 - f) pixels^2, so a history n
//   frames long has been blurred by about n times that; `moving_frames` bounds n so that the blur stays under a
//   variance budget, with f (1 - f) taken at the pixel's speed (its upper envelope beyond half a pixel a frame, so
//   the bound does not come and go with the fraction).
// - On animated shading: ANIM_FRAMES.
// - While the scene's lighting changes (`L.mode.z`), by a change chart per pixel, and a drift:
//   The renderer cannot re-trace last frame's path with last frame's random numbers, so the temporal gradient of
//   Schied et al. 2018 is not to be had; a difference of two independent one-sample estimates carries both their
//   variances (their § 3.2). What is to be had is each frame's sample against the pixel's own history. That is the
//   setting of a control chart: a stream of noisy readings whose mean may shift, watched by an exponentially
//   weighted moving average of the readings, which signals when it leaves a band of k of its own standard
//   deviations (NIST e-Handbook § 6.3.2.4, which credits the chart to Roberts 1959).
//   - The reading is not the sample but its share against the history, s / (s + h') per channel (`chart_share`): 0
//     for a dark sample, about a half for one at the history's level, towards 1 for one far above. A path-traced
//     sample is heavy-tailed (most are dark, a few carry the light), and a chart on the samples themselves reads
//     every run of dark ones as the light having gone. A share is bounded, so one sample moves the chart by a
//     bounded step however bright it is, and the chart counts how many samples changed sides, not by how much.
//   - Its residual is taken from the pixel's own running mean of that share (the level: about a half on a steadily
//     lit pixel, near 0 on one lit by rare bright samples), so every pixel's residual has mean 0 while its lighting
//     holds, whatever its surface, shadow edges or noise are. Residuals of different pixels can then be pooled.
//   - Pooled, because one pixel's few frames prove nothing: each frame the average is read from the pixel where
//     this one was and four at distance sqrt 5 around it (`chart_tap`), averaged, and this frame's residual added
//     with weight CHART_A. That recursion spreads over a few pixels' radius and CHART_A^-1 frames: about 250
//     residuals stand behind a pooled value, where one pixel's own average has 15 (the pooled value's deviation,
//     measured on white residuals: 0.063 of one residual's, CHART_SD; 0.073 without the pixel's own tap). The
//     pooled value is read before this frame's residual is added: the test lags a frame, and is on four times as
//     many samples.
//   - The band: CHART_K deviations of the pooled value, the residuals' own mean square pooled the same way standing
//     in for their variance (a bright sample is in the residual and in the mean square alike), and never less than
//     CHART_FLOOR. (K is 3.5 and not the handbook's usual 3: the residuals are not white. A bright sample raises
//     the history it is measured against, and the samples after it read low until it has faded.)
//   - The response is Schied et al.'s (eq. 15): the blend weight a becomes a + drop (1 - a), `drop` rising from 0 at
//     the band to 1 at 1 + CHART_GAIN^-1 bands (`chart_drop`). The history's length is what that weight leaves, so
//     the frames after it are plain accumulation from the shorter history. The level starts again with the history
//     (its weight is the history's, CHART_LEVEL frames at most), and a history of a frame or two is no level to
//     compare with: its residual counts by CHART_MATURE.
//   - In colour: the chart is kept per channel, and the largest channel's deviation decides. A red light switched
//     off next to a white one moves the red channel's share by far more than the luminance.
//   - The drift. A chart whose level follows the history sees a change and then gets used to it: under a light that
//     falls a little every frame (a dusk) the shares settle at a lower level, the residuals return to 0, and the
//     history trails the light by its whole length for as long as the fall lasts (the model: 2.4 times the light
//     after 170 frames of a fall of 2.6 % a frame under a bound of 24 frames; bench-window's tokyo-dusk under 48:
//     the city's diffuse light 3.5 times the held frame's). What does not get used to it is the plain relative
//     residual (l - h) / h of the luminance: its mean is 0 for a history that is the samples' mean, whatever their
//     distribution, and minus the lag for one that trails. It is pooled as the chart's average is, each pixel's
//     weighted by 1 / (1 + cv^2), cv its samples' own relative deviation (from the luminance moments; a pixel lit
//     by rare bright samples has a large one and counts for little, and its residual is bounded above by
//     DRIFT_CLIP besides, which is where a bounded mean of a skewed sample is biased: on such pixels the drift is
//     blind, and says so). The response is not to shorten the history but to move it: each frame the history (and
//     its moments) is scaled by 1 + DRIFT_GAIN x the pooled residual, less what the pixel's noise allows the
//     pooled value to be by chance (DRIFT_ALLOW of its deviations, DRIFT_ALLOW_UP above, where a sample's tail
//     is). A proportional correction on a pooled error: it takes the lag out and leaves the noise of a full
//     history, where a shorter history took the lag out by letting the noise in. The same factor goes to the
//     specular history of a rough surface (`stemporal`).
//   The constants are from a model of these rules on synthetic samples (tools/chart-model.py: a grid of pixels;
//   steady, exponential, one bright sample in 10 and in 100, a steady half with rare bright ones), then set on the
//   benches (docs/RUNTIME.md has the tables). In the model (frame bound 24), the mean of the histories over the
//   truth 2 / 4 / 8 / 16 frames after the light fell to 1/13: 7.6 / 1.00 / 1.00 / 1.00 on steady samples, 12.0 /
//   0.99 / 0.99 / 1.01 on exponential ones; halved: 1.90 / 1.48 / 1.01 / 1.00 and 1.92 / 1.80 / 1.20 / 1.04; under
//   a fall of 2.6 % a frame, after 170 frames: 1.13 and 1.28 (without the drift 2.41 and 2.21), a steady half with
//   one bright sample in 10: 1.49 (2.41), one bright sample in 10 alone: 2.25 (as without); with nothing changing
//   the histories kept 24.0 of 24 frames and their mean within 0.5 % on every kind but the last (1.8 % low), and
//   histories that started again (a surface uncovered) were 14 to 16 frames long after 16. What was tried and
//   dropped there: letting the chart's average give up the share it was acted on (the response stalled at partial
//   drops), a level that does not follow the history (false signals on exponential samples, 0.2 % of pixels a
//   frame), a level that counts a history's first share (every history that started again read its next frames as
//   the light having fallen: 1.3 frames long after 8), a floor of 0.025 (a steady pixel with rare bright samples
//   signalled on 1.4 % of pixels a frame), shortening the history by the drift instead of moving it (under a fall of
//   0.7 % a frame, steady samples: 1.10 of the light on 14 frames for 1.07 on 48), and a drift not weighted by the pixels' noise (it darkened pixels lit by
//   rare samples by a fifth).
//   The chart's and the drift's state lasts in `diffuse_fast` (per traced sample, eight bits each, rounded at
//   random so that a weight below a step still moves it in the mean: x the level's three channels and the drift; y
//   the residual average's three channels and the root of the mean square) and is kept up whether or not the
//   lighting changes, so that it is warm when it does. With `L.mode.z` 0 nothing responds: the history is the
//   plain mean of its frames.
// The specular history has the same chart on one channel (the mean of the three, `stemporal`), in `diffuse_mom`.z.

// The blur a moving history may have gathered, as a variance (previous traced pixels^2): diffuse irradiance (64
// frames at half a pixel a frame and faster), and the specular history (32 frames).
// The bound trades blur for noise, and which costs more depends on the scene and on how long the motion lasts; what
// a reflection needs in a move of the camera is not this bound but the parallax's (`stemporal`). Measured on the
// integrated tree, 2026-11-24, bench-ghosting --quick (Bistro night; excess dE over the settled frame: mid-turn,
// mid-strafe, half a second after turning back, after strafing back, 0.1 s after a stop), with the parallax bound
// as it is, for budgets of 8 and 8, 16 and 8, 16 and 16: 0.89 / 1.42 / 1.12 / 1.21 / 1.33, 0.87 / 1.39 / 1.13 /
// 1.22 / 1.31, 0.88 / 1.39 / 1.14 / 1.22 / 1.31 (the previous release: 0.85 / 1.60 / 1.13 / 1.25 / 1.34; these
// with rough surfaces still held to their neighbourhood in a move, which was then dropped for its cost: the last
// figure is 1.36 now). And
// bench-window's rooms-turn (768 lamps, a 24-frame turn out of a settled view) in the tree before it had specular
// light, dE to offline at the last moving frame for a diffuse budget of 4 / 8 / none: 2.23 / 2.12 / 1.99: a turn
// that short gathers little blur, and a history cut for it is that much noisier for seconds. (Not 4 for another
// reason too: its 16 frames are FIREFLY_FRAMES and HIST_CENTRE, and a history held at exactly their threshold
// falls to either side of it by rounding.)
const MOVE_BLUR: f32 = 16.0;
const SPEC_MOVE_BLUR: f32 = 8.0;
// Frames of history on animated shading.
const ANIM_FRAMES: f32 = 4.0;
// The change chart. CHART_A: the weight of a frame's residual in the pooled average. CHART_SD: the pooled average's
// standard deviation, in deviations of one residual (see above). CHART_K: the band, in deviations. CHART_FLOOR: the
// least deviation of the pooled share that counts. CHART_GAIN: `drop` per band beyond the band. CHART_MATURE: the
// history length from which a pixel's residual counts in full (a history of a frame or two is no level to compare
// with). CHART_LEVEL: the frames the level averages over at most. CHART_CHANNEL: the share of the history's
// luminance added to each channel's own level in the share's denominator, so that a channel the light has next to
// none of does not read its own noise as change. CHART_DARK: the light (display units at the current exposure)
// below which a change is none.
const CHART_A: f32 = 0.125;
const CHART_SD: f32 = 0.063;
const CHART_K: f32 = 3.5;
const CHART_FLOOR: f32 = 0.04;
const CHART_GAIN: f32 = 2.0;
const CHART_MATURE: f32 = 4.0;
const CHART_LEVEL: f32 = 32.0;
const CHART_CHANNEL: f32 = 0.25;
const CHART_DARK: f32 = 1.0e-3;
// The drift. DRIFT_GAIN: the share of the pooled relative residual taken out of the history a frame. DRIFT_ALLOW,
// DRIFT_ALLOW_UP: what of it is the noise's, in deviations of the pooled value, below and above (a sample's
// distribution has its tail above). DRIFT_CLIP: the most a sample's relative residual counts for.
const DRIFT_GAIN: f32 = 0.25;
const DRIFT_ALLOW: f32 = 2.0;
const DRIFT_ALLOW_UP: f32 = 4.0;
const DRIFT_CLIP: f32 = 7.0;

// The most frames a history moving `speed` pixels a frame over its grid may hold for its blur to stay under `blur`.
fn moving_frames(speed: f32, blur: f32) -> f32 {
    let f = min(speed, 0.5);
    return blur / max(f * (1.0 - f), 1.0e-6);
}

// The four pixels the chart's average is pooled from, around where the pixel was: a quarter turn apart at distance
// sqrt 5, mirrored every other frame (the recursion's reach is then the same along both diagonals).
fn chart_tap(k: i32) -> vec2<i32> {
    var o = vec2<i32>(-2, -1);
    if (k == 1) { o = vec2<i32>(1, -2); }
    if (k == 2) { o = vec2<i32>(2, 1); }
    if (k == 3) { o = vec2<i32>(-1, 2); }
    if ((L.size.z & 1u) == 1u) { o.x = -o.x; }
    return o;
}

// A sample's share against the history h (`level`: the history's own, luminance or mean), per channel.
fn chart_share(s: vec3<f32>, h: vec3<f32>, level: f32) -> vec3<f32> {
    return s / (s + h + vec3<f32>(CHART_CHANNEL * level + CHART_DARK / max(exposure_scale(), 1.0e-8)));
}

// The weight of this frame's share in the level, the history `frames` long with it: the level is the mean of the
// shares since the history started, but for the first (a share against no history is 1 whatever the light: counted,
// every history that started again read its next frames as the light having fallen, and started again: in the
// model 14 to 23 % of such pixels signalled a frame and their histories were 1.3 frames long after 8).
fn chart_level_weight(frames: f32) -> f32 {
    return 1.0 / clamp(frames - 1.0, 1.0, CHART_LEVEL);
}

fn chart_mature(frames: f32) -> f32 {
    return clamp((frames - 1.0) / (CHART_MATURE - 1.0), 0.0, 1.0);
}

// The share of the history a pooled residual average `dev` (its size) takes, `spread` the pooled mean square: none
// within the band, all of it from 1 + 1 / CHART_GAIN bands on.
fn chart_drop(dev: f32, spread: f32) -> f32 {
    return clamp((dev / max(CHART_K * CHART_SD * sqrt(max(spread, 0.0)), CHART_FLOOR) - 1.0) * CHART_GAIN, 0.0, 1.0);
}

// Three numbers in 0 .. 1 for sample q this frame, to round the chart's level at random.
fn chart_dither(q: u32) -> vec3<f32> {
    let k = q * 2654435761u + L.size.z * 40503u;
    return vec3<f32>(f32((k * 3u) >> 8u), f32((k * 5u + 0x9e3779b9u) >> 8u), f32((k * 7u + 0x3c6ef372u) >> 8u)) / 16777216.0;
}

// Diffuse temporal accumulation on the traced grid: the sample's irradiance (diffuse lighting / albedo) blended into
// the previous traced grid's history at its reprojected position (motion vectors for moving objects), with its
// moments (variance for the denoiser, firefly and change tests). Writes the denoiser input (fb, traced grid).
@compute @workgroup_size(8, 8, 1)
fn dtemporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    let iw = L.isz.x;
    let ih = L.isz.y;
    if (gid.x >= iw || gid.y >= ih) { return; }
    let q = gid.y * iw + gid.x;
    let px = sample_px[q];
    let t = px.color.w;
    let n = normalize(px.normal.xyz + vec3<f32>(1e-6));
    let anim = px.albedo.w > 1.5;
    let sp = sample_pos(i32(gid.x), i32(gid.y));
    let dir = ray_dir(L.cur, sp.x, sp.y);
    let p = select(L.cur.pos.xyz + dir * 1.0e6, L.cur.pos.xyz + dir * t, t > 0.0);
    var irr = sample_diffuse(q) / max(px.albedo.rgb, vec3<f32>(0.02));
    var l = lum(irr);
    var hd = vec4<f32>(0.0);
    var hm = vec4<f32>(0.0);
    // (For the response below: how far the pixel's centre moved over the previous traced grid this frame, in its
    // pixels; the history tap that counts most, -1: none; and the previous grid's pixel the history was read around.)
    var speed = 0.0;
    var near_tap = -1;
    var around = vec2<i32>(0);
    let ps = vec2<i32>(L.isz.zw);
    if (L.size.w == 0u && ps.x > 0) {
        let mv = motion_in[q];
        let pp = prev_pixel(p + select(vec3<f32>(0.0), mv.xyz, mv.w > 0.5 && t > 0.0));
        if (pp.z > 0.0) {
            // Where the jittered sample itself was: resampled bilinearly at a random sub-pixel offset every frame, a
            // blur that denoises a young history but grows with it (light bled from lit surfaces into the shadow next
            // to them, and the steep glow of a small lamp on a wall flattened: Bistro's lantern-lit facade 31 m away
            // read 0.46 of the offline radiance; Bistro / Zero Day ΔE rose from 64 to 256 frames). From HIST_CENTRE
            // frames on, the history is fetched where this pixel's centre was (moved by the sample's motion).
            let osz = vec2<f32>(f32(L.size.x), f32(L.size.y));
            var tc = (pp.xy + 0.5) * vec2<f32>(ps) / osz - 0.5;
            let centre = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) * osz / vec2<f32>(L.isz.xy) - 0.5;
            let tcc = (centre + pp.xy - (sp - 0.5) + 0.5) * vec2<f32>(ps) / osz - 0.5;
            let jc = clamp(vec2<i32>(round(tcc)), vec2<i32>(0), ps - 1);
            // Holding still the centre lands on one history pixel exactly, whose stored normal is another jittered
            // sample's: on normal-mapped stones and facade detail it often missed 0.8 and the history restarted
            // (Bistro's street and facades at 256 frames held 8-30). The centre fetch takes 0.5 (depth unchanged).
            let centred = diffuse_hist[last_half(u32(jc.y * ps.x + jc.x))].w >= HIST_CENTRE;
            if (centred) { tc = tcc; }
            // (Nothing moved and the grid is the same one: exactly none, not the reprojection's rounding.)
            if (camera_moves() || (mv.w > 0.5 && t > 0.0) || any(ps != vec2<i32>(L.isz.xy))) { speed = length(tcc - vec2<f32>(f32(gid.x), f32(gid.y))); }
            around = vec2<i32>(round(tc));
            let x0 = i32(floor(tc.x));
            let y0 = i32(floor(tc.y));
            let f = tc - floor(tc);
            var wsum = 0.0;
            var most = 0.0;
            for (var k = 0; k < 4; k++) {
                let tx = x0 + (k & 1);
                let ty = y0 + (k >> 1u);
                if (tx < 0 || ty < 0 || tx >= ps.x || ty >= ps.y || !same_surface_n(tx, ty, pp.z, n, t, select(0.8, 0.5, centred))) { continue; }
                let bw = select(1.0 - f.x, f.x, (k & 1) == 1) * select(1.0 - f.y, f.y, (k >> 1u) == 1);
                let j = u32(ty * ps.x + tx);
                hd += diffuse_hist[last_half(j)] * bw;
                hm += diffuse_mom[last_half(j)] * bw;
                wsum += bw;
                if (bw > most) {
                    most = bw;
                    near_tap = i32(j);
                }
            }
            if (wsum > 1e-3) {
                hd /= wsum;
                hm /= wsum;
            } else {
                hd = vec4<f32>(0.0);
                hm = vec4<f32>(0.0);
            }
            if (non_finite(hd.rgb) || non_finite(hm.xyz)) {
                hd = vec4<f32>(0.0);
                hm = vec4<f32>(0.0);
            }
            if (hd.w <= 0.0) { near_tap = -1; }
        }
    }
    // ---- How long this pixel's history may be, and how it follows a change of the lighting ("The response of the
    // histories" above).
    // The change chart's state where this pixel was: its level at the history tap that counts most, the pooled
    // residual and its mean square from that tap and four pixels around (`chart_tap`).
    var level = vec3<f32>(0.0);
    var pooled = vec3<f32>(0.0);
    var spread = 0.0;
    var drift = 0.0;
    if (near_tap >= 0) {
        let own = diffuse_fast[last_half(u32(near_tap))];
        let l0 = unpack4x8snorm(own.x);
        level = l0.xyz * (127.0 / 255.0) + 0.5;
        drift = l0.w;
        let e0 = unpack4x8snorm(own.y);
        pooled = e0.xyz;
        spread = e0.w * e0.w;
        var taps = 1.0;
        for (var k = 0; k < 4; k++) {
            let c = around + chart_tap(k);
            if (c.x < 0 || c.y < 0 || c.x >= ps.x || c.y >= ps.y) { continue; }
            let tap = diffuse_fast[last_half(u32(c.y * ps.x + c.x))];
            let e = unpack4x8snorm(tap.y);
            pooled += e.xyz;
            spread += e.w * e.w;
            drift += unpack4x8snorm(tap.x).w;
            taps += 1.0;
        }
        pooled /= max(taps, 1.0);
        spread /= max(taps, 1.0);
        drift *= 0.5 / max(taps, 1.0);
    }
    // The drift ("The response of the histories"): what the pooled relative residual says the history is off by,
    // less what the pixel's own noise allows it to be, taken out of the history at DRIFT_GAIN a frame. The
    // luminance moments go with it.
    let cv2 = max(hm.y / max(hm.x * hm.x, 1.0e-12) - 1.0, 0.0);
    let trust = 1.0 / (1.0 + cv2);
    var drifted = 1.0;
    if (L.mode.z != 0u && !anim) {
        let allow = select(DRIFT_ALLOW, DRIFT_ALLOW_UP, drift > 0.0) * CHART_SD * trust * sqrt(cv2);
        drifted = 1.0 + DRIFT_GAIN * sign(drift) * max(abs(drift) - allow, 0.0) / max(trust, 0.5);
        hd = vec4<f32>(hd.rgb * drifted, hd.w);
        hm = vec4<f32>(hm.x * drifted, hm.y * drifted * drifted, hm.z, hm.w);
    }
    let cap = min(select(L.misc.y, min(L.misc.y, ANIM_FRAMES), anim), moving_frames(speed, MOVE_BLUR));
    var m1 = hm.x;
    var m2 = hm.y;
    let sd = sqrt(max(m2 - m1 * m1, 0.0));
    if (hd.w >= 4.0 && hd.w < FIREFLY_FRAMES) {
        // Firefly, while the history is young (after a disocclusion or a lighting change): a sample moves the pixel by
        // (l - mean) / frames; cut only what would show as a sparkle (more than half the mean above it). Not from
        // FIREFLY_FRAMES on: histories are capped while the view moves or the lighting changes, so a cut tied to the frame count
        // kept clipping the rare bright samples of small strong lights for good. Bistro crowd at 10 s, held: diffuse
        // 0.70 of our offline render with it, 0.82 without; moving ΔE 15.3 -> 12.9.
        let hi = m1 + 4.0 * sd + 0.5 * m1 * hd.w + 1e-4;
        if (l > hi) { irr *= hi / l; l = hi; }
    }
    // This frame's sample against the history, per channel, as a share (`chart_share`), and its residual from the
    // level: by how much more or less of the light the sample holds than this pixel's samples usually do.
    let hist = max(hd.rgb, vec3<f32>(0.0));
    let hist_l = lum(hist);
    let share = chart_share(max(irr, vec3<f32>(0.0)), hist, hist_l);
    let res = (share - level) * chart_mature(hd.w);
    // (The relative residual, for the drift: bounded above, weighted by how far the pixel's samples can be trusted.)
    let rel = clamp((l - hist_l) / (hist_l + CHART_DARK / max(exposure_scale(), 1.0e-8)), -1.0, DRIFT_CLIP) * chart_mature(hd.w) * trust;
    // The response: only while the scene's lighting changes, and not on animated shading (a few frames already).
    var drop = 0.0;
    if (L.mode.z != 0u && !anim) { drop = chart_drop(max(max(abs(pooled.x), abs(pooled.y)), abs(pooled.z)), spread); }
    // (Schied et al. 2018, eq. 15: the blend weight is the plain one, moved towards 1 by the share of the history the
    // change takes.)
    let plain = 1.0 / max(min(hd.w + 1.0, cap), 1.0);
    let a = plain + drop * (1.0 - plain);
    let count = 1.0 / a;
    // (z: the drift's factor this frame, for `stemporal`, which then puts its own chart there.)
    let mom = vec4<f32>(mix(m1, l, a), mix(m2, l * l, a), drifted, 1.0);
    let d = mix(hd.rgb, irr, a);
    diffuse_hist[this_half(q)] = vec4<f32>(d, count);
    diffuse_mom[this_half(q)] = mom;
    // The chart's state: the level follows the history (it starts again with it); the pooled averages take this
    // frame's residual. (None on animated shading or the sky.)
    let level_now = mix(level, share, chart_level_weight(count));
    var e_now = vec4<f32>(0.0);
    var drift_now = 0.0;
    if (!anim && t > 0.0) {
        e_now = vec4<f32>(mix(pooled, res, CHART_A), sqrt(mix(spread, max(max(res.x * res.x, res.y * res.y), res.z * res.z), CHART_A)));
        // (Less what a signal of the chart has just acted on: the history it leaves is of this frame, and a drift
        // read before it would be taken out again, an overshoot of a few per cent after a lamp came on.)
        drift_now = clamp(mix(drift, rel, CHART_A), -0.5, 0.5) * (1.0 - drop);
    }
    // (Eight bits each, rounded at random: the level about a half, the drift doubled.)
    let dither = (vec4<f32>(chart_dither(q), fract(chart_dither(q).x * 7.0 + 0.37)) - 0.5) / 127.0;
    diffuse_fast[this_half(q)] = vec2<u32>(pack4x8snorm(vec4<f32>((level_now - 0.5) * (255.0 / 127.0), 2.0 * drift_now) + dither), pack4x8snorm(e_now));
    // Denoiser input; w = variance of the accumulated mean luminance (its noise estimate).
    fb[q] = vec4<f32>(max(d, vec3<f32>(0.0)), max(mom.y - mom.x * mom.x, 0.0) / max(count, 1.0));
}

// A traced sample's surface (`dtemporal` copies it to sb): normal, first-hit distance (0 = sky).
fn traced_surface(q: u32) -> vec4<f32> {
    let v = sb[q];
    return vec4<f32>(v.xyz, abs(v.w));
}

// ------------------------------------------------------------------------------------------------ denoise

// Diffuse: edge-avoiding a-trous on demodulated irradiance, luminance edge-stop scaled by the pixel's temporal
// standard deviation (SVGF). Stage 0 reads the temporal output; odd stages read fa / write fb, even stages read fb /
// write fa. The last stage adds the denoised specular and the directly seen emission back (result in fa).
fn read_stage(stage: u32, q: u32) -> vec4<f32> {
    if (stage == 0u) { return fb[q]; }
    if ((stage & 1u) == 1u) { return fa[q]; }
    return fb[q];
}

fn write_stage(stage: u32, i: u32, v: vec4<f32>) {
    if ((stage & 1u) == 1u) { fb[i] = v; } else { fa[i] = v; }
}


// `max_noise`: a stage runs where the accumulated mean is still noisier than that (relative); wide stages need fresh
// history (disocclusions, lighting that just changed), the job Lumen's radiance cache does for under-sampled pixels.
// On the traced grid, on irradiance (no albedo to divide out), edge stops from each sample's own surface.
fn atrous_gated(gid: vec3<u32>, stage: u32, step: i32, max_noise: f32) {
    let w = L.isz.x;
    let h = L.isz.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let i = gid.y * w + gid.x;
    let c = read_stage(stage, i);
    let count = diffuse_hist[this_half(i)].w;
    let strength = L.misc2.x / sqrt(max(count, 1.0));
    // Animated emitters (signs, screens) show their emission, which bypasses the denoiser: skipping them keeps whole
    // 8x8 groups on the cheap path once the rest has converged.
    let anim = sb[i].w < 0.0;
    let irr = c.rgb;
    let li = lum(irr);
    // A stage runs while the accumulated mean is still visibly noisy (relative deviation of the mean above
    // `max_noise`), not by history length: long histories of noisy pixels keep a light filter. Fresh history always.
    // The wide stages (2..4) also keep round 3's history limits: on fresh history only (cost: they would otherwise run on
    // every noisy night pixel, +1.9 ms on Kabukicho at 60 %).
    let limits = array<f32, 5>(256.0, 128.0, 32.0, 8.0, 8.0);
    var noisy = (count < 4.0 || sqrt(max(c.w, 0.0)) > max_noise * max(li, 1e-4)) && (stage < 2u || count < limits[stage]);
    if (strength < 0.02 || !noisy || anim) { write_stage(stage, i, c); return; }
    let g = traced_surface(i);
    // Noise of the accumulated mean: temporal deviation / sqrt(frames). Fresh history has no reliable moments: assume
    // 1-spp noise of the order of the local mean. Each stage halves the remaining noise.
    var lmean = 0.0;
    var vmean = 0.0;
    var nmean = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let qx = i32(gid.x) + dx * step;
            let qy = i32(gid.y) + dy * step;
            if (qx < 0 || qy < 0 || qx >= i32(w) || qy >= i32(h)) { continue; }
            let q = u32(qy) * w + u32(qx);
            let cq = read_stage(stage, q);
            lmean += lum(cq.rgb);
            vmean += max(cq.w, 0.0);
            nmean += 1.0;
        }
    }
    lmean = max(lmean / max(nmean, 1.0), 0.0);
    vmean /= max(nmean, 1.0);
    // c.w carries the variance estimate through the stages (each stage roughly halves it). The width takes the
    // neighbourhood's mean variance when larger (SVGF filters it): a pixel among fireflies that drew none (low mean, low
    // variance) rejected its neighbours and stayed a dark hole.
    var sd = sqrt(max(max(c.w, 0.0), vmean));
    if (count < 4.0) { sd = max(sd, lmean); }
    let sigma_l = 4.0 * sd + 0.1 * lmean + 1e-4;
    var kern = array<f32, 3>(0.375, 0.25, 0.0625);
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let qx = i32(gid.x) + dx * step;
            let qy = i32(gid.y) + dy * step;
            if (qx < 0 || qy < 0 || qx >= i32(w) || qy >= i32(h)) { continue; }
            let q = u32(qy) * w + u32(qx);
            let gq = traced_surface(q);
            let iq = read_stage(stage, q).rgb;
            var wt = kern[abs(dx)] * kern[abs(dy)];
            wt *= pow(max(dot(g.xyz, gq.xyz), 0.0), 32.0);
            if ((g.w == 0.0) != (gq.w == 0.0)) { wt = 0.0; }
            wt *= exp(-abs(gq.w - g.w) / (0.02 * g.w + 0.01));
            // Fresh history arrives as raw 1-spp samples here: a luminance stop would reject their bright tail and darken
            // (sun-room k = 1 5 % too dark); the first stage averages it by geometry alone.
            if (!(stage == 0u && count < 4.0)) { wt *= exp(-abs(lum(iq) - li) / sigma_l); }
            sum += iq * wt;
            wsum += wt;
        }
    }
    var outv = select(irr, sum / wsum, wsum > 1e-6);
    if (non_finite(outv)) { outv = max(c.rgb, vec3<f32>(0.0)); }
    write_stage(stage, i, vec4<f32>(outv, c.w * 0.5));
}

// Each stage halves the variance it passes on; wider stages need more remaining noise (and fresh history) to run, so
// they work on disoccluded / relit history (the job Lumen's radiance cache does) and converged areas stop at a light 3x3.
@compute @workgroup_size(8, 8, 1) fn atrous0(@builtin(global_invocation_id) gid: vec3<u32>) { atrous_gated(gid, 0u, 1, 0.004); }
@compute @workgroup_size(8, 8, 1) fn atrous1(@builtin(global_invocation_id) gid: vec3<u32>) { atrous_gated(gid, 1u, 2, 0.008); }
@compute @workgroup_size(8, 8, 1) fn atrous2(@builtin(global_invocation_id) gid: vec3<u32>) { atrous_gated(gid, 2u, 4, 0.02); }
@compute @workgroup_size(8, 8, 1) fn atrous3(@builtin(global_invocation_id) gid: vec3<u32>) { atrous_gated(gid, 3u, 8, 0.05); }
@compute @workgroup_size(8, 8, 1) fn atrous4(@builtin(global_invocation_id) gid: vec3<u32>) { atrous_gated(gid, 4u, 16, 0.05); }

// ------------------------------------------------------------------------------------------------ render resolution
// Everything noisy is accumulated and filtered on the traced grid (the render resolution), composed there into one
// colour, and one temporal upscaler takes that to the output grid (`upscale`): the structure of UE's TSR after Lumen
// and of the vendor upscalers after a denoiser. `gbuffer` is the traced grid's g-buffer.

// Specular history at previous-frame position pp (prev_pixel: output pixels, view depth) on the previous traced grid:
// bilinear over the taps that show this surface; `virt`: through a reflection (the reflecting surface's normal and
// roughness must match, not its depth).
fn spec_history(pp: vec3<f32>, n: vec3<f32>, t: f32, rough: f32, virt: bool) -> vec4<f32> {
    let ps = vec2<i32>(L.isz.zw);
    let tc = (pp.xy + 0.5) * vec2<f32>(ps) / vec2<f32>(f32(L.size.x), f32(L.size.y)) - 0.5;
    let x0 = i32(floor(tc.x));
    let y0 = i32(floor(tc.y));
    let f = tc - floor(tc);
    var h = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var k = 0; k < 4; k++) {
        let tx = x0 + (k & 1);
        let ty = y0 + (k >> 1u);
        if (tx < 0 || ty < 0 || tx >= ps.x || ty >= ps.y) { continue; }
        let j = u32(ty * ps.x + tx);
        let g = gb_unpack(gbuffer[last_half(j)]);
        var ok = false;
        if (virt) {
            ok = g.depth > 0.0 && dot(g.n, n) > 0.95 && abs(g.rough - rough) < 0.1;
        } else if (t > 0.0) {
            ok = g.depth > 0.0 && abs(g.depth - pp.z) < 0.04 * pp.z + 0.002 && dot(g.n, n) > 0.8;
        } else {
            ok = g.depth == 0.0;
        }
        if (!ok) { continue; }
        let bw = select(1.0 - f.x, f.x, (k & 1) == 1) * select(1.0 - f.y, f.y, (k >> 1u) == 1);
        h += spec_hist[last_half(j)] * bw;
        wsum += bw;
    }
    if (wsum <= 1e-3) { return vec4<f32>(0.0); }
    h /= wsum;
    if (non_finite(h.rgb) || non_finite(vec3<f32>(h.w))) { return vec4<f32>(0.0); }
    return h;
}

// ---- The specular history's rules (`stemporal`; the lengths and the chart as "The response of the histories").
// Set on the integrated tree, 2026-11-24 (bench-ghosting, bench-object-ghosting, bench-temporal's water and edit,
// bench-window; docs/RUNTIME.md has the tables).
// A reflection's history is tried through the reflection too when the two places lie this far apart (previous traced
// pixels): nearer, they are one fetch.
const SPEC_PAR_MIN: f32 = 0.25;
// The roughness from which the reflection's own place is not tried (no second fetch), and from which the surface's
// history starts to lose against the reflection's below it.
const SPEC_VIRT_ROUGH: f32 = 0.6;
const SPEC_VIRT_FROM: f32 = 0.3;
// What the surface's history counts for on a mirror, where it is right only if nothing in the reflection differs.
const SPEC_SURFACE_MIRROR: f32 = 0.1;
// The neighbourhood's band, in its deviations, and as a share of the history's level.
const SPEC_GAMMA: f32 = 1.25;
const SPEC_REL: f32 = 0.1;
// The chart level below which a pixel counts as lit by rare samples (a dark neighbourhood is then no evidence against
// its history), and from which it counts as steadily lit.
const SPEC_RARE: f32 = 0.3;
const SPEC_STEADY: f32 = 0.45;
// How far a history may have slid against what it shows (previous traced pixels, summed over its frames) on a
// surface of roughness 0, and how much further by roughness squared (the lobe's width: a blurred image hides a
// slide); the share of the parallax that counts on a mirror, where the reflection's own place is known; and the
// share of the bound lifted where the reflection's history is found there and agrees with this frame.
const SPEC_SLIDE: f32 = 4.0;
const SPEC_SLIDE_ROUGH: f32 = 32.0;
const SPEC_PAR_MIRROR: f32 = 0.25;
const SPEC_FOUND: f32 = 0.9;
// On animated shading: the share of the band's allowance for rare samples that is kept. bench-temporal --shot
// water (a canal's waves under two lamps and a sign; dE to the converged frame of the same moment, the water's
// reflections, time held / running), for 1 (no clamp there) / 0.5 / 0.25 / 0: 1.16 / 3.22 / 3.67 / 3.77 and 20.6 /
// 12.5 / 10.8 / 9.0 (the previous release: 3.08 and 13.4); with a bound of 2 frames and no clamp 5.51 and 12.2.
const SPEC_ANIM_RARE: f32 = 0.5;
// The most frames while the scene's lighting changes: on a mirror, at the roughness where a reflection is still
// sharp but the filter already reaches (SPEC_GLOSS), and from SPEC_VIRT_ROUGH on (there the frame's bound alone).
const SPEC_LIVE_MIRROR: f32 = 24.0;
const SPEC_LIVE_GLOSS: f32 = 12.0;
const SPEC_GLOSS: f32 = 0.3;
// A traced sample's specular lighting, as `sample_specular`, the ceiling `cl` (`sample_clamp`) read once.
fn spec_sample(q: u32, cl: f32) -> vec3<f32> {
    var s = sample_px[q].spec.rgb / max(L.misc2.w, 1.0) + refl_res[q].rgb;
    if (non_finite(s)) { s = vec3<f32>(0.0); }
    let l = lum(s);
    if (l > cl) { s *= cl / l; }
    return max(s, vec3<f32>(0.0));
}

// The specular chart's state, a whole number below 2^24 in `diffuse_mom`.z (a float holds it exactly; 0, a cleared
// buffer, is no state): the level (8 bits, rounded at random), the residual average (8 bits, signed) and the root of
// the residuals' mean square (8 bits).
fn spec_chart(v: f32) -> vec3<f32> {
    let u = u32(clamp(v, 0.0, 16777215.0));
    return vec3<f32>(f32(u & 255u) / 255.0, f32(i32(((u >> 8u) & 255u) ^ 128u) - 128) / 127.0, f32((u >> 16u) & 255u) / 255.0);
}

fn spec_chart_pack(level: f32, e: f32, rms: f32, dither: f32) -> f32 {
    let a = u32(clamp(level * 255.0 + dither, 0.0, 255.0));
    let b = u32(i32(round(clamp(e, -1.0, 1.0) * 127.0))) & 255u;
    let c = u32(clamp(round(rms * 255.0), 0.0, 255.0));
    return f32(a | (b << 8u) | (c << 16u));
}

// Specular temporal accumulation on the traced grid: this frame's specular sample (with the resolved reflection,
// `refl`) blended into its history. Writes `spec_hist` (rgb, frames), the specular filter's input (`spec_store(0u, ..)`),
// the specular chart's state (`diffuse_mom`.z of this frame's half, which `dtemporal` has written by then) and the
// traced grid's g-buffer.
// - Where the history is. A reflection's image does not move with the surface that shows it: "reflected objects
//   move according to their depth, not the depth of the surface which reflects them", and reprojected by the
//   surface they smear (Stachowiak 2015, notes to the slides on temporal reprojection, who reproject by the average
//   depth of what is reflected). Here: seen in a plane mirror, a thing at distance d behind the surface point at
//   distance t lies at t + d along the view ray. Both places are read when they differ: where the surface point was
//   (`prev_pixel`, moving objects by their motion), and where that point behind the surface was (for a surface
//   that moved: carried by twice its move along its normal, the law of reflection for an image of what stands
//   still). Which to believe is this frame's samples' to say: each history is held against the mean and deviation
//   of the nine samples around, in YCoCg (a box along luma, Karis 2014, slide 33; a statistical one in place of his
//   minimum and maximum, as Schied et al. 2018 § 2 report of Patney et al. 2016), and counts by exp(-e^2), e its
//   distance in bands, times what the roughness says before any sample does (the reflection's place on a mirror,
//   the surface's from SPEC_VIRT_ROUGH).
// - How long it may be: the frame's bound; the bound of a history resampled in motion (`moving_frames`); and a bound
//   by the parallax, the distance between the two places a frame: a history n frames long has slid n times that
//   against one of them, and may by SPEC_SLIDE pixels and more the wider the lobe. The parallax is none in a turn
//   of the camera, when nothing bounds, and in a move it is what a reflection needs: where the sample has no hit
//   distance the parallax of a far reflection is taken (the most it can be), and rough surfaces are bound too (a
//   lamp's highlight on wet stones slides with the view as a mirror's image does, only blurred: without that, the
//   street 0.1 s after a stop read an excess dE of 1.78 for 1.33). On a mirror only a share of it counts, and
//   little of it where the reflection's history was found in its place and agrees with this frame. While the
//   lighting changes, a bound by roughness: shortest where a reflection is sharp enough to show a lag and the
//   filter already reaches (SPEC_GLOSS), longer on a mirror, which has the temporal mean alone against its noise,
//   and the frame's bound alone on rough surfaces, whose reflection is too blurred to show one. Their histories
//   take the diffuse history's drift instead (above).
// - What it follows, while the lighting changes or the reflection moves against its surface, and always on animated
//   shading: the chart (above), on the mean of the three channels, pooled around where the surface point was; and
//   the neighbourhood's band: the history is clamped into mean +- (SPEC_GAMMA deviations + a share of its own
//   level), Karis's neighbourhood clamp (slides 30 to 34) on those statistics, with one more term. One sample a
//   pixel is heavy-tailed: where the light comes by rare bright samples, nine dark ones around say nothing against
//   a history that holds their mean, and clipping it to them every frame darkens the reflection for good. The
//   chart's level knows which pixels those are (the mean share of their samples is far below a half): there the
//   band is as wide as the history's level (it cannot be clamped down, only up, when all nine are brighter), and
//   tight only where the samples were steady (a mirror's image of a lamp, which nine dark samples do contradict).
//   A reflection that passes is steady while it is there: when the clamp raises a history (all nine samples above
//   it at once) the level is set to a steady one, and a pixel takes the highest level of the pixels its chart
//   pools from, so that the rim of a passing reflection is treated as its middle. (Without the two, a light thrown
//   over a polished floor left its reflection along its path, hollowed out: bench-object-ghosting's orb 0.1 s
//   after it lands, dE / trail 4.88 / 280 px for 3.63 / 96.) What the clamp takes shortens the history by the same
//   share.
@compute @workgroup_size(8, 8, 1)
fn stemporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    let iw = L.isz.x;
    let ih = L.isz.y;
    if (gid.x >= iw || gid.y >= ih) { return; }
    let q = gid.y * iw + gid.x;
    let px = sample_px[q];
    let t = px.color.w;
    let n = normalize(px.normal.xyz + vec3<f32>(1e-6));
    let rough = px.emit.w;
    // The history is the traced pixel's, whatever sub-pixel point each frame's jitter sampled it at: it is fetched
    // where the pixel's centre was (the sample's surface, through the centre). Fetched where the jittered sample
    // itself was, it was resampled at a new sub-pixel offset every frame, each time a little blurrier: the wet
    // street's reflections converged to blobs (Kabukichō, bench-ghosting's converged frames).
    let sp = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) * vec2<f32>(f32(L.size.x) / f32(iw), f32(L.size.y) / f32(ih));
    let dir = ray_dir(L.cur, sp.x, sp.y);
    var p = L.cur.pos.xyz + dir * 1.0e6;
    var depth = 0.0;
    if (t > 0.0) {
        p = L.cur.pos.xyz + dir * t;
        depth = dot(p - L.cur.pos.xyz, L.cur.fwd.xyz);
    }
    let cl = sample_clamp();
    let cur = spec_sample(q, cl);
    let mv = motion_in[q];
    let live = L.mode.z != 0u;
    // The diffuse history's drift factor at this pixel this frame (`dtemporal` left it in this frame's half).
    let drifted = diffuse_mom[this_half(q)].z;
    let ps = vec2<i32>(L.isz.zw);
    var h = vec4<f32>(0.0);  // the surface's history
    var hv = vec4<f32>(0.0); // the reflection's
    var speed = 0.0;
    var par = 0.0;
    var chart = vec3<f32>(0.0); // the level where the surface point was, the pooled residual average and mean square
    var around_level = 0.0;     // the highest level among the pooled pixels
    if (L.size.w == 0u && ps.x > 0) {
        let object_moves = mv.w > 0.5 && t > 0.0;
        let moved = select(vec3<f32>(0.0), mv.xyz, object_moves);
        let pps = prev_pixel(p + moved);
        if (pps.z > 0.0) {
            let k = vec2<f32>(ps) / vec2<f32>(f32(L.size.x), f32(L.size.y));
            let in_motion = camera_moves() || object_moves || any(ps != vec2<i32>(L.isz.xy));
            if (in_motion) { speed = length((pps.xy - (sp - 0.5)) * k); }
            h = spec_history(pps, n, t, rough, false);
            // The reflection's hit distance: this sample's ray's, or the resolved one (`refl`) on a near mirror and
            // where the sample has none (it took the diffuse lobe). With neither, the frame knows nothing of the
            // reflection's place: the surface's history, and for the bound the parallax of a reflection far away,
            // which is the most it can be (none in a turn of the camera, the surface's own motion against the
            // distance in a move).
            var dist = px.spec.w;
            if (rough < NEAR_MIRROR || !(dist > 0.0 && dist < 1.0e3)) { dist = refl_res[q].w; }
            let known = dist > 0.0 && dist < 1.0e3;
            if (t > 0.0 && in_motion) {
                // (Where the reflection's image was: with the camera's move alone for a still surface. A surface
                // that moved carried the image of what stands still by twice its move along its normal, the law of
                // reflection at a plane; not by its move, which slides the mirror under an image that stays.)
                let ppv = prev_pixel(p + dir * select(1.0e4, dist, known) + 2.0 * dot(moved, n) * n);
                if (ppv.z > 0.0) {
                    par = length((ppv.xy - pps.xy) * k);
                    if (known && par >= SPEC_PAR_MIN && rough < SPEC_VIRT_ROUGH) { hv = spec_history(ppv, n, t, rough, true); }
                }
            }
            // The chart's state: the level at the pixel the surface point was in, the averages from it and four
            // around it.
            let c0 = vec2<i32>(round((pps.xy + 0.5) * k - 0.5));
            if (t > 0.0 && c0.x >= 0 && c0.y >= 0 && c0.x < ps.x && c0.y < ps.y) {
                chart = spec_chart(diffuse_mom[last_half(u32(c0.y * ps.x + c0.x))].z);
                around_level = chart.x;
                chart.z *= chart.z;
                var taps = 1.0;
                for (var j = 0; j < 4; j++) {
                    let c = c0 + chart_tap(j);
                    if (c.x < 0 || c.y < 0 || c.x >= ps.x || c.y >= ps.y) { continue; }
                    let e = spec_chart(diffuse_mom[last_half(u32(c.y * ps.x + c.x))].z);
                    around_level = max(around_level, e.x);
                    chart.y += e.y;
                    chart.z += e.z * e.z;
                    taps += 1.0;
                }
                chart.y /= max(taps, 1.0);
                chart.z /= max(taps, 1.0);
            }
        }
    }
    // This frame's samples around: needed to choose between two histories, and to hold one to them while the lighting
    // changes or the reflection moves against its surface. (A settled scene seen from a still camera, or in a turn,
    // reads none: its history is the plain mean of its frames.)
    // The light that drifts for the diffuse history drifts for a rough reflection of it too: the same factor, from
    // SPEC_GLOSS on (a sharp reflection is of one thing, and has its own short bound).
    if (live && drifted > 0.0) {
        let share = smoothstep(SPEC_GLOSS, SPEC_VIRT_ROUGH, rough);
        h = vec4<f32>(h.rgb * mix(1.0, drifted, share), h.w);
        hv = vec4<f32>(hv.rgb * mix(1.0, drifted, share), hv.w);
    }
    // Animated shading (waves, ripples): the surface holds still and its normal does not, so what it reflects is
    // another thing each frame. Its history is held to this frame's samples whether or not the lighting changes,
    // with SPEC_ANIM_RARE of the allowance for rare samples (a glint that has moved on is not one; a glint under a
    // stopped clock is). A bound in frames did worse both ways: short, the stopped water is noise; long, the
    // running water is the blur of its waves.
    let anim = px.albedo.w > 1.5 && rough < SPEC_GLOSS;
    let two = hv.w > 0.0;
    // (In a move of the camera: below SPEC_VIRT_ROUGH. A rough surface has the parallax bound alone: held to the
    // nine samples too, the street 0.1 s after a stop read an excess dE of 1.31 for 1.36, for nine more taps on
    // every rough pixel in every move.)
    let held = (live || anim || (par >= SPEC_PAR_MIN && rough < SPEC_VIRT_ROUGH)) && (h.w > 0.0 || two);
    var clamped = 0.0; // the share of the history the clamp took
    var found = 0.0;   // how far the reflection's history was found, and agrees with this frame
    var risen = 0.0;   // the share by which the clamp raised the history
    if (two || held) {
        var s1 = vec3<f32>(0.0);
        var s2 = vec3<f32>(0.0);
        var sn = 0.0;
        for (var j = 0; j < 9; j++) {
            let qx = i32(gid.x) + j % 3 - 1;
            let qy = i32(gid.y) + j / 3 - 1;
            if (qx < 0 || qy < 0 || qx >= i32(iw) || qy >= i32(ih)) { continue; }
            let y = ycocg(select(spec_sample(u32(qy) * iw + u32(qx), cl), cur, j == 4));
            s1 += y;
            s2 += y * y;
            sn += 1.0;
        }
        let mean = s1 / sn;
        let dev = sqrt(max(s2 / sn - mean * mean, vec3<f32>(0.0)));
        let dark = CHART_DARK / max(exposure_scale(), 1.0e-8);
        if (two) {
            // Each history's distance from the samples' mean, in bands, and what the roughness says of each.
            let band = SPEC_GAMMA * dev + SPEC_REL * mean.x + dark;
            let ds = abs(ycocg(h.rgb) - mean) / band;
            let dv = abs(ycocg(hv.rgb) - mean) / band;
            let es = min(max(max(ds.x, ds.y), ds.z), 3.0);
            let ev = min(max(max(dv.x, dv.y), dv.z), 3.0);
            let of_surface = smoothstep(SPEC_VIRT_FROM, SPEC_VIRT_ROUGH, rough);
            let ws = select(0.0, mix(SPEC_SURFACE_MIRROR, 1.0, smoothstep(0.0, SPEC_VIRT_FROM, rough)) * exp(-es * es), h.w > 0.0);
            let wv = (1.0 - of_surface) * exp(-ev * ev);
            h = (h * ws + hv * wv) / max(ws + wv, 1.0e-6);
            found = wv / max(ws + wv, 1.0e-6) * exp(-ev * ev);
        }
        if (held && h.w > 0.0) {
            let hy = ycocg(h.rgb);
            // (Steadily lit: by the level, and by a history long enough for its level to mean it. While the lighting
            // changes, by the level here or of a pooled pixel beside it, the rim of a passing reflection being lit
            // as its middle is, and from a history half as long. Not so in a move of the camera under a settled
            // light, where the reflections slide but nothing comes or goes: bench-ghosting's strafe read an excess
            // dE of 1.77 with the wider rule for 1.39.)
            let steady = smoothstep(SPEC_RARE, SPEC_STEADY, select(chart.x, around_level, live)) * chart_mature(select(0.5, 1.0, live) * h.w);
            let rare = (1.0 - steady) * select(1.0, SPEC_ANIM_RARE, anim);
            // (Chroma to twice the luma: a saturated colour's Co is.)
            let band = SPEC_GAMMA * dev + (SPEC_REL + rare) * max(hy.x, 0.0) * vec3<f32>(1.0, 2.0, 2.0) + dark;
            let hc = clamp(hy, mean - band, mean + band);
            let took = abs(hc - hy);
            clamped = clamp(max(max(took.x, took.y), took.z) / (max(hy.x, mean.x) + dark), 0.0, 1.0);
            // (Raised: all nine samples are above the history at once. That is no rare sample but a light that
            // has come, a reflection passing: the level is told so below, or when it has passed, the nine dark
            // samples would count for nothing against it: a thrown light's reflection stayed along its path.)
            if (live && hc.x > hy.x) { risen = clamped; }
            h = vec4<f32>(max(ycocg_rgb(hc), vec3<f32>(0.0)), h.w);
        }
    }
    // The chart: this frame's share against the history (one channel: the mean of the three), its residual from the
    // level, and, while the lighting changes, what the pooled average takes of the history.
    let third = vec3<f32>(1.0 / 3.0);
    let level = dot(h.rgb, third);
    let share = chart_share(vec3<f32>(dot(cur, third)), vec3<f32>(level), level).x;
    let res = (share - chart.x) * chart_mature(h.w);
    var drop = 0.0;
    if (live) { drop = chart_drop(abs(chart.y), chart.z); }
    // The history's length.
    var cap = min(L.misc.y, moving_frames(speed, SPEC_MOVE_BLUR));
    if (par > 0.0) {
        // (The share of the parallax that counts: SPEC_PAR_MIRROR on a mirror, all of it from NEAR_MIRROR, none
        // from SPEC_VIRT_ROUGH, where the reflection stays with its surface.)
        // Less where the reflection's history was found where it should be and agrees with this frame's samples:
        // the bound is for a place that is not known.
        let counts = mix(SPEC_PAR_MIRROR, 1.0, smoothstep(0.0, NEAR_MIRROR, rough)) * (1.0 - SPEC_FOUND * found);
        cap = min(cap, 1.0 + (SPEC_SLIDE + SPEC_SLIDE_ROUGH * rough * rough) / max(par * counts, 1.0e-6));
    }
    if (live) {
        let sharp = mix(SPEC_LIVE_MIRROR, SPEC_LIVE_GLOSS, smoothstep(0.0, SPEC_GLOSS, rough));
        cap = min(cap, mix(sharp, max(L.misc.y, sharp), smoothstep(SPEC_GLOSS, SPEC_VIRT_ROUGH, rough)));
    }
    // What the clamp took shortens the history by the same share, as the chart's signal does (the response's rule),
    // but not below CHART_MATURE frames, and the chart's level does not start again for it as it does with a history
    // the chart itself cut. The clamp leaves the history at the band's edge, still off (by several times the light
    // where the samples are noisy), and the chart is what follows the rest: a history too short to count in it, or
    // a level learnt against a history still off, and the chart went quiet. (A synthetic signal through this pass,
    // the lamp's pool 8 frames after it went out, over the settled frame: 1.58 so, 1.23 as it is; and not
    // shortened at all, the pool 8 frames after the lamp came on: 0.88 for 0.94.)
    let plain = 1.0 / max(min(h.w + 1.0, cap), 1.0);
    let a = max(plain + drop * (1.0 - plain), min(plain + clamped * (1.0 - plain), max(plain, 1.0 / CHART_MATURE)));
    let count_s = 1.0 / a;
    let count_chart = 1.0 / (plain + drop * (1.0 - plain));
    let s_out = max(mix(h.rgb, cur, a), vec3<f32>(0.0));
    spec_hist[this_half(q)] = vec4<f32>(s_out, count_s);
    spec_store(0u, q, vec4<f32>(s_out, count_s));
    var state = 0.0;
    if (t > 0.0) {
        state = spec_chart_pack(mix(mix(chart.x, share, chart_level_weight(count_chart)), 1.0 / (2.0 + CHART_CHANNEL), risen), mix(chart.y, res, CHART_A), sqrt(mix(chart.z, res * res, CHART_A)), chart_dither(q).x);
    }
    diffuse_mom[this_half(q)].z = state;
    // `moves`: a moving object's surface that crossed a traced sample this frame. One that moved less (a leaf in the
    // wind, a walker far away) has not left its place among the nine: what it shows and hides is the jitter's, as at
    // a still edge, and the rules for what a moving object uncovers made every such edge start again each frame.
    var swept = false;
    if (mv.w > 0.5 && t > 0.0) {
        let now = prev_pixel(p + mv.xyz);
        let still = prev_pixel(p);
        swept = now.z <= 0.0 || still.z <= 0.0 || length(now.xy - still.xy) >= f32(L.size.x) / f32(max(L.isz.z, 1u));
    }
    gbuffer[this_half(q)] = gb_pack(n, depth, rough, swept);
}

// The specular filter's reach (traced pixels) a unit of roughness squared. bench-window's rooms-turn (a floor of
// roughness 0.3 under 768 lamps, turning), p99 dE to offline at the last moving frame and 30 frames after the
// stop: 3.88 and 2.74 at 25 (the floor got the first pass alone), 3.32 and 2.43 from 64 on (all three; the previous
// release: 3.44 and 2.50).
const SPEC_REACH: f32 = 64.0;

// The specular filter (`spec_filter`'s kernel) on the traced grid: three passes over `spec_pp`, slot 0 -> 1 -> 0 -> 1.
fn spec_filter_traced(gid: vec3<u32>, step: i32, from_a: bool) {
    let w = L.isz.x;
    let h = L.isz.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let i = gid.y * w + gid.x;
    let src = select(1u, 0u, from_a);
    let c = spec_load(src, i);
    let g = gb_unpack(gbuffer[this_half(i)]);
    let rough = g.rough;
    let fresh = c.w < 4.0;
    // How far the filter reaches at this roughness (traced pixels; a pass of `step` runs when that is within it): by
    // the lobe's width, which goes with roughness squared (the alpha of this renderer's microfacet lobe). None on a
    // mirror, whose image has nothing to average with; the first pass from roughness 0.125, the second from 0.18,
    // all three from 0.25.
    let reach = SPEC_REACH * rough * rough;
    let npow = select(64.0, 8.0, fresh);
    let strength = (0.25 + smoothstep(0.08, 0.5, rough)) / sqrt(max(c.w, 1.0)) * 4.0;
    var outv = c.rgb;
    if (strength >= 0.02 && f32(step) <= reach) {
        var li = 0.0;
        var nl = 0.0;
        for (var k = 0; k < 9; k++) {
            if ((k & 1) == 0 && k != 4) { continue; } // centre + 4-neighbourhood
            let qx = i32(gid.x) + (k % 3 - 1) * step;
            let qy = i32(gid.y) + (k / 3 - 1) * step;
            if (qx < 0 || qy < 0 || qx >= i32(w) || qy >= i32(h)) { continue; }
            li += lum(spec_load(src, u32(qy) * w + u32(qx)).rgb);
            nl += 1.0;
        }
        li /= max(nl, 1.0);
        var sum = vec3<f32>(0.0);
        var wsum = 0.0;
        var kern = array<f32, 2>(0.5, 0.25);
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let qx = i32(gid.x) + dx * step;
                let qy = i32(gid.y) + dy * step;
                if (qx < 0 || qy < 0 || qx >= i32(w) || qy >= i32(h)) { continue; }
                let q = u32(qy) * w + u32(qx);
                let gq = gb_unpack(gbuffer[this_half(q)]);
                let sq = spec_load(src, q).rgb;
                var wt = kern[abs(dx)] * kern[abs(dy)];
                wt *= pow(max(dot(g.n, gq.n), 0.0), npow);
                if ((g.depth == 0.0) != (gq.depth == 0.0)) { wt = 0.0; }
                wt *= exp(-abs(gq.depth - g.depth) / (0.02 * g.depth + 0.01));
                wt *= exp(-abs(gq.rough - rough) * 10.0);
                wt *= exp(-abs(lum(sq) - li) / (strength * 8.0 * (li + 1e-3)));
                sum += sq * wt;
                wsum += wt;
            }
        }
        if (wsum > 1e-6) { outv = sum / wsum; }
        if (non_finite(outv)) { outv = c.rgb; }
    }
    spec_store(1u - src, i, vec4<f32>(outv, c.w));
}

@compute @workgroup_size(8, 8, 1) fn sfilt0(@builtin(global_invocation_id) gid: vec3<u32>) { spec_filter_traced(gid, 1, true); }
@compute @workgroup_size(8, 8, 1) fn sfilt1(@builtin(global_invocation_id) gid: vec3<u32>) { spec_filter_traced(gid, 2, false); }
@compute @workgroup_size(8, 8, 1) fn sfilt2(@builtin(global_invocation_id) gid: vec3<u32>) { spec_filter_traced(gid, 4, true); }

// The frame at render resolution: each traced sample's colour (its own albedo times the denoised irradiance, the
// denoised specular, both through the media before the surface, what the camera ray saw directly and the deterministic
// direct light) into fb (w: first-hit distance), and its motion into `sample_vel`: screen velocity (output pixels this
// frame; camera and object), view depth, and the upscaler's reactive mask, as FSR 2 and UE's responsive AA take one
// (what changes there has no motion vector): 1 where the surface's shading is animated (signs, screens), and up to 1
// with the share of the pixel that is a medium changing with time (fire, simulated smoke).
@compute @workgroup_size(8, 8, 1)
fn compose(@builtin(global_invocation_id) gid: vec3<u32>) {
    let iw = L.isz.x;
    let ih = L.isz.y;
    if (gid.x >= iw || gid.y >= ih) { return; }
    let q = gid.y * iw + gid.x;
    let px = sample_px[q];
    let t = px.color.w;
    var lighting: vec3<f32>;
    if (L.mode.w == 1u) {
        // The trace kept the whole specular path, including deferred-cache lighting. No irradiance
        // demodulation or temporal/spatial filter here. Keep the pre-reconstruction signals for snapshots.
        var spec = finite_or_zero(px.spec.rgb / L.misc2.w);
        let ceiling = sample_clamp();
        spec *= min(1.0, ceiling / max(lum(spec), 1e-8));
        spec_store(1u, q, vec4<f32>(spec, 1.0));
        lighting = sample_diffuse(q) + spec;
    } else {
        lighting = fa[q].rgb * max(px.albedo.rgb, vec3<f32>(0.02)) + spec_load(1u, q).rgb;
    }
    let c = lighting * medium_tr(q) + sample_emission(q) + direct_in[q].rgb;
    let sp = sample_pos(i32(gid.x), i32(gid.y));
    let dir = ray_dir(L.cur, sp.x, sp.y);
    var p = L.cur.pos.xyz + dir * 1.0e6;
    var depth = 0.0;
    if (t > 0.0) {
        p = L.cur.pos.xyz + dir * t;
        depth = dot(p - L.cur.pos.xyz, L.cur.fwd.xyz);
    }
    // (A static surface under a camera that holds still has no velocity: exactly none, not the reprojection's rounding,
    // so the upscaler reads its history where it wrote it.)
    var vel = vec2<f32>(0.0);
    let mv = motion_in[q];
    let object_moves = mv.w > 0.5 && t > 0.0;
    if (L.size.w == 0u && (camera_moves() || object_moves)) {
        let pv = prev_pixel(p + select(vec3<f32>(0.0), mv.xyz, object_moves));
        if (pv.z > 0.0) { vel = clamp(sp - 0.5 - pv.xy, vec2<f32>(-2048.0), vec2<f32>(2048.0)); }
    }
    let shown = finite_or_zero(max(c, vec3<f32>(0.0)));
    fb[q] = vec4<f32>(shown, t);
    // (None on animated shading: it shows this frame's picture, and its history is not kept for it to stand by.)
    let reactive = max(select(0.0, 1.0, px.albedo.w > 1.5), clamp(medium_share(q) / UP_MEDIUM, 0.0, 1.0));
    sample_vel[q] = vec4<f32>(vel, depth, reactive);
}

// The upscaler's state of each traced sample, after `compose`: how much it flickers (`Flicker`) and the thin feature's
// lock (`Lock`), into this frame's half of `up_state`.
@compute @workgroup_size(8, 8, 1)
fn track(@builtin(global_invocation_id) gid: vec3<u32>) {
    let iw = L.isz.x;
    let ih = L.isz.y;
    if (gid.x >= iw || gid.y >= ih) { return; }
    let q = gid.y * iw + gid.x;
    let sp = sample_pos(i32(gid.x), i32(gid.y));
    let sv = sample_vel[q];
    let vel = sv.xy;
    let depth = sv.z;
    let reactive = sv.w;
    // How much this sample flickers (`Flicker`), from where its surface was in the previous frame's traced grid.
    let now = min(lum(fb[q].rgb), 6.0e4);
    var f = Flicker(vec2<f32>(0.0), 0.0, 0.0);
    let ps = vec2<i32>(L.isz.zw);
    if (L.size.w == 0u && ps.x > 0) {
        let pc = vec2<i32>(floor((sp - vel) * vec2<f32>(ps) / vec2<f32>(f32(L.size.x), f32(L.size.y))));
        if (pc.x >= 0 && pc.y >= 0 && pc.x < ps.x && pc.y < ps.y) {
            f = flicker_at(u32(pc.y * ps.x + pc.x));
        }
    }
    let still = 1.0 - clamp(length(vel) / UP_SPEED, 0.0, 1.0);
    // The light here changed (a shadow's edge passed, a lamp's pool moved, a light switched): the composed sample
    // jumped by more than LIGHT_CHANGE of itself since the previous frame at the same surface point, and by far more
    // than it flickers under the jitter (`Flicker`). Its history is of the light before: `upscale` holds it to this
    // frame's samples. Without it, a shadow swept over a still floor (a character running in the sun) left a
    // line of its edge a frame, fading over UP_SAMPLES frames: the floor stands still, so nothing clamped it, and a
    // line a pixel wide widens its own range. While the lighting is live (`mode.z`: something moved, as anti-lag).
    // The flicker's frame count starts again there (`n`): until it is LIGHT_SETTLE again, `upscale` holds the history
    // around to this frame's samples (the line of an edge outlives the frame that edge passed).
    var changed = false;
    if (L.mode.z != 0u && f.n >= 2.0) {
        let jump = abs(now - f.last.x) / max(max(now, f.last.x), 1.0e-4 / exposure_scale());
        changed = jump > LIGHT_CHANGE && jump > LIGHT_FLICKER * f.sd;
    }
    var sd = select(1.0, f.sd, f.n >= 1.0);
    if (f.n >= 2.0) {
        let bend = min(abs(now - 2.0 * f.last.x + f.last.y) / max(max(now, max(f.last.x, f.last.y)), 1e-9), UP_ONSET * max(f.sd, 0.01));
        sd = sqrt(mix(f.sd * f.sd, bend * bend / 6.0, still / min(f.n - 1.0, 8.0)));
    }
    // The thin feature's lock here (`Lock`): new where this sample's emission is one, else the one `lock_carry` brought
    // here, a frame older.
    let floor_l = 0.01 / exposure_scale();
    var e9: array<f32, 9>;
    var like = 0u;
    for (var k = 0; k < 9; k++) {
        let j = u32(clamp(i32(gid.y) + k / 3 - 1, 0, i32(ih) - 1)) * iw + u32(clamp(i32(gid.x) + k % 3 - 1, 0, i32(iw) - 1));
        e9[k] = lum(sample_px[j].emit.rgb) / max(L.misc2.w, 1.0) + floor_l;
    }
    for (var k = 0; k < 9; k++) {
        if (max(e9[k], e9[4]) < THIN_LIKE * min(e9[k], e9[4])) { like |= 1u << u32(k); }
    }
    var across = false;
    for (var k = 0; k < 4; k++) {
        let a = e9[k];
        let b = e9[8 - k];
        across = across || (e9[4] > THIN_LIKE * max(a, b)) || (THIN_LIKE * e9[4] < min(a, b));
    }
    let thin = across && (like & 27u) != 27u && (like & 54u) != 54u && (like & 216u) != 216u && (like & 432u) != 432u;
    let fw = atomicLoad(&lock_fwd[q]);
    atomicStore(&lock_fwd[q], 0u);
    var was = Carried(0.0, 0.0, 0.0, vec2<f32>(0.0), false);
    // (Not onto a moving object's surface: the thing locked there a frame ago has moved, likely with the object that
    // started moving under it.)
    if (fw != 0u && L.size.w == 0u && ps.x > 0 && !gb_unpack(gbuffer[this_half(q)]).moves) { was = carry((fw & 0x1ffffffu) - 1u); }
    var life = max(was.life - 1.0 / jitter_phases(), 0.0);
    var lz = was.depth;
    var at = clamp(was.at - vec2<f32>(f32(gid.x), f32(gid.y)), vec2<f32>(0.0), vec2<f32>(0.999));
    if (thin) {
        life = 1.0;
        lz = depth;
        at = L.jit.xy;
    }
    life *= 1.0 - reactive;
    up_state[this_half(q)] = vec4<u32>(pack2x16float(vec2<f32>(now, select(now, f.last.x, f.n >= 1.0))), pack2x16float(vec2<f32>(sd * still, select(min(max(f.n + still, 1.0), 9.0), 1.0, changed))), pack2x16float(vec2<f32>(life, min(lz, 6.0e4))), pack2x16float(at));
}

// The upscaler works on linear radiance and its means are plain ones. (Weighted by 1 / (1 + luminance x exposure), so
// that a bright sample does not outweigh its neighbours, the blend with the history lost light around lamps: Bistro
// read 0.974 of offline's luminance for 1.015, with no less flicker and no shorter a trail in motion to show for it.)

// Samples of colour history an output pixel holds at most (a frame adds its nearest traced sample's weight): holding
// still, and moving UP_SPEED output pixels a frame or faster.
const UP_SAMPLES: f32 = 16.0;
const UP_MOVING: f32 = 4.0;
const UP_SPEED: f32 = 1.0;
// The reconstruction kernel exp(-UP_SHARP d^2), d in output pixels from the pixel's centre to a traced sample.
const UP_SHARP: f32 = 3.5;
// A history that this frame's samples lie outside of by this share of their luminance is dropped entirely (`reject`).
const UP_REJECT: f32 = 0.25;
// The same on animated shading (`compose`'s reactive mask): it shows this frame's picture, not a mean of the last ones.
const UP_REACTIVE: f32 = 4.0;
// The share of a pixel a medium that changes with time takes (`medium_share`) from which the pixel is as reactive.
const UP_MEDIUM: f32 = 0.25;
// How much a composed sample's luminance changes from the previous frame, as a share of it, and in deviations of its
// own flicker, before the light there counts as changed (`track`), and the flicker frame count from which it counts
// as settled again (`upscale`).
const LIGHT_CHANGE: f32 = 0.35;
const LIGHT_FLICKER: f32 = 2.5;
const LIGHT_SETTLE: f32 = 6.0;
// How far outside the range of the history's pixels around it a point sample may lie (`judge`): this many deviations
// of the traced sample's own flicker (`Flicker`). Of 1, 2, 3 (measured while the deviation was still an amount of
// light): Bistro held still changes 0.49, 0.42, 0.38 from frame to frame, and Tokyo's skyline at dusk is furthest from
// its held frame at 3.
const UP_FLICKER: f32 = 2.0;
// The most a frame's change counts for in the flicker, in deviations of what it was (or of 1 % of the level).
const UP_ONSET: f32 = 4.0;
// A surface counts as gone from in front when what is nearest now lies behind what was nearest by this share of its
// depth (`judge`), and the history as stale there when it lies outside this frame's range by UP_STALE of its luminance.
const UP_GONE: f32 = 0.05;
const UP_STALE: f32 = 0.05;

// History rejection at the render resolution (where UE's TSR judges its history too), in two passes. What a frame
// can say about the history is its low frequencies. Clipping each output pixel's history into the traced colours'
// range (a full-resolution TAA's rule) cut the detail finer than a traced pixel every frame; here the test is turned
// round: each traced sample is held against the range of the history around the very point it was taken at (where
// that point was a frame ago). A still, converged image explains every sample, however fine its detail and whichever
// side of an edge the jitter put the sample on; what the history cannot explain (a disocclusion, a light switched) is
// how far the sample lies outside that range.

// The one case that test cannot see is the rim of something that went away: output pixels that were part object, part
// background, whose colour the background's samples next to them explain. That is a disocclusion, and it is found the
// way every upscaler finds one, by depth: what is nearest the camera among the nine samples now lies behind what was
// nearest among the nine of the previous frame there (both dilated, so a still edge under the jitter mostly agrees
// with itself), confirmed by colour (the history there lies outside this frame's range), so the still edges that the
// jitter does catch keep their history. Not where a thin feature is locked (`Lock`): a lamp finer than a pixel comes
// and goes among the nine samples with the jitter alone, in front of whatever is behind it, its light in the history
// and not in this frame; every lamp against the sky was something gone each frame the samples missed it, and a field
// of them, held still, kept a third of its light. (Thin things that do not emit, a wire, a lattice by day, are still
// taken so. Asking that the two depths move against each other on screen was tried and is wrong here: a wall blown
// out uncovers what is behind it with nothing moving.)

// A sample is a point and the history's pixels are means over a pixel: of a feature finer than a pixel (a row of
// windows, a grating) a sample lies outside their range in a still picture too, a little further on some jitters than
// on others, and correcting for that flickers. How far is not the history's to say. (It was: half the range of the
// history's nine pixels. A pixel standing off the truth then widened the range of the samples next to it, which were
// allowed to stand off by half of that, and so outward: under a sky that dimmed every frame a silhouette kept the sky
// of seconds ago in a glow ten pixels wide, with nothing but the blend to bound it.) It is the samples': how much the
// traced sample at this place changes from one frame to the next under the jitter, which is what a temporal
// upscaler's flicker history is for (UE's TSR keeps the input's luminance over time for the same decision).

// `Flicker`, per traced sample, in `up_state`'s x and y (f16; `track` keeps it): the composed sample's luminance of
// the last frame and of the one before, the deviation of its frame-to-frame change as a share of its level, and of how
// many frames that is (0: none; the deviation is then 1: unknown, as much as the level), all read where the sample's
// surface was a frame ago (in motion the place on screen would read the motion as flicker).
// - The deviation is of the second difference (this frame's, less twice the last, plus the one before: of
//   independent samples its mean square is six variances), which a steady rise or fall does not have: a sky dimming
//   4 % a frame is no flicker. A running mean over the first eight frames, an exponential one after.
// - A share of the level (the largest of the three frames), not an amount of light: what a lit floor's samples
//   scattered by says nothing of the same floor with the lamp off.
// - Tolerance is earned slowly: a frame's second difference counts for UP_ONSET deviations at most, so a light
//   switched (two large ones) raises it by a fifth and a pixel that really flickers reaches its level in a second.
// - Flicker is the jitter's with all else still: a pixel in motion changes by its motion (a wet street's reflections
//   every frame), so the deviation is added to by how still the pixel is and kept by the same, none from UP_SPEED on
//   (TSR fades its flicker rule out with velocity too): a pixel that moved has none and earns it again at rest,
//   while its history, cut to UP_MOVING samples by the motion, grows back.
// `Lock`, per traced sample, in `up_state`'s z (f16; the other half free): how much longer the picture here counts as a
// thin feature, 1 when a sample showed one, a jitter sequence later 0 (FSR 2's pixel locks). A thin feature is a sample
// no 2 x 2 block of the nine around it is like (within THIN_LIKE of its luminance: FSR 2's rule) and that is above both
// or below both of some pair of opposite neighbours: a lamp or a lit window finer than a traced pixel, which most
// frames' samples miss, and the dark sample between two of them, which is this frame's miss. (FSR 2 asks for a sample
// brighter or darker than all those unlike it, a single line's rule: of lamps strung a pixel apart, in several colours,
// most have a brighter neighbour, and Tokyo Tower's legs went unlocked. A slope is above one neighbour and below the
// other in every direction and stays out.) The rules that hold the history to this frame's samples stand back for a
// locked pixel: at the locked thing its history is not clamped into the nine samples in motion (`upscale`, LOCK_REACH:
// the lamp no sample hit this frame is still there; Tokyo Tower in a turn lost a quarter of its light and its legs), and its
// sample's distance from the history's range is no finding (`judge`: a hit lies above the range, a miss within it, and
// the corrections went one way); nor is a lamp the samples miss this frame something gone from in front (`judge`'s
// depth test). UE's TSR loosens its clamp the same way where it detects thin geometry.
// A lock moves with what it is for, not with the surface the sample behind it hit: it keeps that thing's view depth
// (in z's other half: the lamp's, the sample's that hit it) and where in its traced pixel it was (w: the sample's
// jitter then), and the next frame `lock_carry` takes that point to where it is now, by the two cameras alone (a lamp
// that moves itself is found again when a sample next hits it), onto the traced sample whose pixel it falls in. And
// it moves the history there: in `judge` a locked lamp is a candidate for the nearest surface among the nine, with the
// motion of its own depth. (Before, a lamp no sample hit this frame went with what was behind it: its lock drifted
// over the background in a parallax, and its history was read where the background had been, a second copy of the
// lamp where something else had been one; the clamp in motion, spared only in part over the nine samples around a
// lock, bounded that copy.)
// The test runs on what the camera ray saw directly (the sample's emission: lamps, windows, signs), which has no noise:
// FSR 2's test is made for a rasterised frame, and on the composed sample of a traced one (short histories in motion,
// reflections without a spatial filter: more than 5 % from sample to sample) it called most of Kabukichō thin; the
// clamp was then off everywhere and the reflections on the wet street trailed 78 pixels in a strafe for 53.
const THIN_LIKE: f32 = 1.05;
// How far from the locked thing (output pixels, from where in its traced pixel it is: `Lock`) the lock spares the
// history from the clamp in motion, in full; half a pixel further, not at all. The reconstruction's own reach (a
// sample's weight a pixel away is 3 %, UP_SHARP). Nearer, the lamp's own light is cut; further, the halo that
// resampling the history spreads around it every frame is kept. Tokyo Tower in a quick orbit of the play window (1.146
// degrees a frame about a point 50 m ahead) against offline, the last moving frame, ΔE / luminance: 0.5 pixel
// 4.31 / 0.73, 1 pixel 3.90 / 0.97, 1.5 pixels 4.17 / 1.17; before (every output pixel of the nine traced samples
// around a lock spared by 0.75) 4.30 / 0.99. In a turn (0.25 degrees a frame) 2.29 / 0.69, 2.01 / 0.92, 2.13 / 1.06;
// before 2.12 / 0.84. (Spared in full over the nine, the tower was too bright even in the turn, where nothing moves
// against what is behind it: 1.10.)
const LOCK_REACH: f32 = 1.0;
// The frames of the jitter sequence (live/frame.rs: 8 at full resolution, as many more as an output pixel has fewer samples).
fn jitter_phases() -> f32 {
    return clamp(round(8.0 * f32(L.size.x) * f32(L.size.y) / (f32(L.isz.x) * f32(L.isz.y))), 8.0, 512.0);
}
// (In full for the first half of its life, fading over the second.)
fn locked(q: u32) -> f32 { return clamp(2.0 * unpack2x16float(up_state[this_half(q)].z).x, 0.0, 1.0); }

// The previous frame's lock at its traced sample p (`Lock`): its life, the view depth of what it is for in the previous
// frame's view (`was`: the previous g-buffer's) and in this frame's (`depth`; 0 both: the sky), and where it is now,
// in this frame's traced coordinates (sample q sits at q + L.jit.xy); `ahead`: in front of the camera.
struct Carried { life: f32, was: f32, depth: f32, at: vec2<f32>, ahead: bool };
fn carry(p: u32) -> Carried {
    let ps = vec2<f32>(L.isz.zw);
    let v = up_state[last_half(p)];
    let lz = unpack2x16float(v.z);
    let was = (vec2<f32>(f32(p % L.isz.z), f32(p / L.isz.z)) + unpack2x16float(v.w)) * vec2<f32>(f32(L.size.x), f32(L.size.y)) / ps;
    let c = cur_pixel(view_point(L.prev, was, lz.y));
    return Carried(lz.x, lz.y, select(0.0, c.z, lz.y > 0.0), (c.xy + 0.5) * vec2<f32>(f32(L.isz.x), f32(L.isz.y)) / vec2<f32>(f32(L.size.x), f32(L.size.y)), c.z > 0.0);
}

// Each lock of the previous frame onto the traced sample its point falls in now (`Lock`), the longest-lived where two
// fall in one (the later sample where they tie): an atomic max of (life in 7 bits, the sample + 1 in 25: an 8K grid),
// which `track` reads. And the previous frame's g-buffer learns what was locked there: the nearest thing in that
// traced pixel was the lamp, which its sample most frames missed (`judge`'s test for something gone from in front
// reads it: the sky a lamp has passed over keeps the lamp in its history otherwise, which no sample there contradicts,
// since the history's pixels around a sparse lamp are dark too). Over the previous frame's traced grid (live/frame.rs).
@compute @workgroup_size(8, 8, 1)
fn lock_carry(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (L.size.w != 0u || gid.x >= L.isz.z || gid.y >= L.isz.w) { return; }
    let p = gid.y * L.isz.z + gid.x;
    let c = carry(p);
    let g = gb_unpack(gbuffer[last_half(p)]);
    // (A lock on a moving object's surface went with it: the two cameras cannot take it there, and left where it was
    // it kept the object's light in the history over whatever it uncovered, a copy of its screen's text a frame. It
    // is found again where a sample next shows the thing.)
    if (g.moves) { return; }
    if (c.life > 0.0 && c.was > 0.0 && (g.depth <= 0.0 || c.was < g.depth)) {
        gbuffer[last_half(p)].y = gb_dr(c.was, g.rough, g.moves);
    }
    if (c.life * jitter_phases() <= 1.0 || !c.ahead) { return; }
    let q = vec2<i32>(floor(c.at));
    if (q.x < 0 || q.y < 0 || q.x >= i32(L.isz.x) || q.y >= i32(L.isz.y)) { return; }
    atomicMax(&lock_fwd[u32(q.y) * L.isz.x + u32(q.x)], ((u32(c.life * 126.0) + 1u) << 25u) | (p + 1u));
}

// The motion of the lock at traced sample j (`Lock`) by its own depth: from where its point was the previous frame
// to where it is (output pixels, as `compose`'s).
fn lock_vel(j: u32) -> vec2<f32> {
    let v = up_state[this_half(j)];
    let up = vec2<f32>(f32(L.size.x), f32(L.size.y)) / vec2<f32>(f32(L.isz.x), f32(L.isz.y));
    let e = (vec2<f32>(f32(j % L.isz.x), f32(j / L.isz.x)) + unpack2x16float(v.w)) * up;
    let pv = prev_pixel(view_point(L.cur, e, unpack2x16float(v.z).y));
    if (pv.z <= 0.0) { return vec2<f32>(0.0); }
    return clamp(e - 0.5 - pv.xy, vec2<f32>(-2048.0), vec2<f32>(2048.0));
}

struct Flicker { last: vec2<f32>, sd: f32, n: f32 };
fn flicker_at(q: u32) -> Flicker {
    let v = up_state[last_half(q)];
    let b = unpack2x16float(v.y);
    return Flicker(unpack2x16float(v.x), b.x, b.y);
}

// `judge`: per traced sample, into `judged_slot` (f16): that distance (linear RGB), the sample's luminance (< 0: no
// history there), the motion its output pixels reproject by: the surface nearest the camera among the nine samples
// around (the sky the farthest), since an edge moves with what is in front of it; and whether its history is of
// something that is gone.
struct Judged { d: vec3<f32>, lum: f32, vel: vec2<f32>, gone: bool, moved: bool };
fn judged_at(q: u32) -> Judged {
    let v = up_state[judged_slot(q)];
    let b = unpack2x16float(v.y);
    return Judged(vec3<f32>(unpack2x16float(v.x), b.x), b.y, unpack2x16float(v.z), v.w != 0u, v.w == 2u);
}

@compute @workgroup_size(8, 8, 1)
fn judge(@builtin(global_invocation_id) gid: vec3<u32>) {
    let iw = i32(L.isz.x);
    let ih = i32(L.isz.y);
    if (i32(gid.x) >= iw || i32(gid.y) >= ih) { return; }
    let q = gid.y * L.isz.x + gid.x;
    var near = 1.0e30;
    var vel = vec2<f32>(0.0);
    // (A lock's depth among them, for the motion only: a lamp the samples missed moves the history by its own.)
    var vnear = 1.0e30;
    let parallax = L.size.w == 0u && camera_moves();
    for (var k = 0; k < 9; k++) {
        let qx = i32(gid.x) + k % 3 - 1;
        let qy = i32(gid.y) + k / 3 - 1;
        if (qx < 0 || qy < 0 || qx >= iw || qy >= ih) { continue; }
        let j = u32(qy * iw + qx);
        let mv = sample_vel[j];
        let z = select(1.0e29, mv.z, mv.z > 0.0);
        near = min(near, z);
        if (z < vnear) {
            vnear = z;
            vel = mv.xy;
        }
        let lz = unpack2x16float(up_state[this_half(j)].z);
        // (Not the lamp this sample hit: its own depth, to f16's rounding, and its own motion, the lamp's if it moves.)
        if (parallax && lz.x > 0.0 && lz.y > 0.0 && lz.y < vnear && lz.y < 0.98 * z) {
            vnear = lz.y;
            vel = lock_vel(j);
        }
    }
    var d = vec3<f32>(0.0);
    var l = -1.0;
    var gone = false;
    var gone_moving = false;
    if (L.size.w == 0u) {
        let up = vec2<f32>(f32(L.size.x), f32(L.size.y)) / vec2<f32>(f32(iw), f32(ih));
        // Where this sample's point was a frame ago (output pixels; a pixel's centre at + 0.5).
        let was = (vec2<f32>(f32(gid.x), f32(gid.y)) + L.jit.xy) * up - vel;
        let pp = vec2<i32>(floor(was));
        var lo = vec3<f32>(1.0e30);
        var hi = vec3<f32>(-1.0e30);
        var hsum = vec3<f32>(0.0);
        var hn = 0.0;
        for (var k = 0; k < 9; k++) {
            let x = pp.x + k % 3 - 1;
            let y = pp.y + k / 3 - 1;
            if (x < 0 || y < 0 || x >= i32(L.size.x) || y >= i32(L.size.y)) { continue; }
            let hv = color_hist[last_half(u32(y) * L.size.x + u32(x))];
            if (hv.w <= 0.0 || non_finite(hv.rgb)) { continue; }
            lo = min(lo, hv.rgb);
            hi = max(hi, hv.rgb);
            hsum += hv.rgb;
            hn += 1.0;
        }
        if (hn > 0.0) {
            // (The sample is a point and the history's pixels are means: UP_FLICKER deviations of its own flicker
            // outside their range are the jitter's.)
            let c = fb[q].rgb;
            let slack = UP_FLICKER * unpack2x16float(up_state[this_half(q)].y).x * c;
            d = clamp(c - clamp(c, lo - slack, hi + slack), vec3<f32>(-6.0e4), vec3<f32>(6.0e4));
            l = min(lum(c), 6.0e4);
            // Gone from in front: the previous frame's nearest depth around where this point was (its traced grid's
            // g-buffer) against this frame's nearest, taken to the previous view by this sample's own depths.
            let ps = vec2<i32>(L.isz.zw);
            if (ps.x > 0) {
                var zr = 1.0;
                let t = fb[q].w;
                let z = sample_vel[q].z;
                if (t > 0.0 && z > 0.0) {
                    let sp = sample_pos(i32(gid.x), i32(gid.y));
                    let mv = motion_in[q];
                    let pv = prev_pixel(L.cur.pos.xyz + ray_dir(L.cur, sp.x, sp.y) * t + select(vec3<f32>(0.0), mv.xyz, mv.w > 0.5));
                    if (pv.z > 0.0) { zr = pv.z / z; }
                }
                let pc = vec2<i32>(floor(was * vec2<f32>(ps) / vec2<f32>(f32(L.size.x), f32(L.size.y))));
                var was_near = 1.0e30;
                var was_moving = false;
                for (var k = 0; k < 9; k++) {
                    let x = pc.x + k % 3 - 1;
                    let y = pc.y + k / 3 - 1;
                    if (x < 0 || y < 0 || x >= ps.x || y >= ps.y) { continue; }
                    let gq = gb_unpack(gbuffer[last_half(u32(y * ps.x + x))]);
                    let zq = select(1.0e29, gq.depth, gq.depth > 0.0);
                    if (zq < was_near) {
                        was_near = zq;
                        was_moving = gq.moves;
                    }
                }
                let now_near = select(near * zr, 1.0e29, near >= 1.0e29);
                // Uncovered by something moving: this sample's own surface (not the nearest of the nine) stands still
                // and lies behind the moving object's surface that was nearest here. The band behind a moving edge
                // takes the edge's motion (the nearest of the nine is still the object), so its history is the
                // object's, and the clamp in motion keeps it (the object's colour is among the nine): each frame
                // left a line of it where the edge had been.
                let own = select(1.0e29, sample_vel[q].z * zr, sample_vel[q].z > 0.0);
                let uncovered = was_moving && motion_in[q].w <= 0.5 && was_near < 1.0e29 && own > was_near * (1.0 + UP_GONE) + 0.01;
                if (was_near < 1.0e29 && (uncovered || now_near > was_near * (1.0 + UP_GONE) + 0.01)) {
                    var clo = vec3<f32>(1.0e30);
                    var chi = vec3<f32>(-1.0e30);
                    // (And no thin feature's lock alive among the nine: a lamp the samples miss this frame is not gone.
                    // The lock is where the lamp is now, `lock_carry`; one that has gone on leaves its last place.)
                    var alive = 0.0;
                    for (var k = 0; k < 9; k++) {
                        let qx = i32(gid.x) + k % 3 - 1;
                        let qy = i32(gid.y) + k / 3 - 1;
                        if (qx < 0 || qy < 0 || qx >= iw || qy >= ih) { continue; }
                        let j = u32(qy * iw + qx);
                        let cq = fb[j].rgb;
                        clo = min(clo, cq);
                        chi = max(chi, cq);
                        alive = max(alive, unpack2x16float(up_state[this_half(j)].z).x);
                    }
                    let hm = hsum / hn;
                    // (What was in front and is gone was a moving object's surface: it went, the jitter did not take it
                    // out of the nine. No colour asked: a rim half the object's light, half what it uncovers, lies
                    // within the noisy range of the samples of what it uncovers, and stayed a line where the object's
                    // edge was, one a frame, fading over UP_SAMPLES frames.)
                    gone = alive <= 0.0 && (was_moving || lum(abs(hm - clamp(hm, clo, chi))) > UP_STALE * lum(hm) + 0.01 / exposure_scale());
                    gone_moving = was_moving;
                }
            }
        }
    }
    // A locked sample's distance from the history's range is no finding (a lamp finer than a pixel lies above it
    // whenever a sample hits it).
    d *= 1.0 - locked(q);
    // (w: 1 gone, 2 gone with what was in front moving.)
    up_state[judged_slot(q)] = vec4<u32>(pack2x16float(d.xy), pack2x16float(vec2<f32>(d.z, l)), pack2x16float(vel), select(u32(gone), 2u, gone && gone_moving));
}

// `reject`: per traced pixel, into `spec_pp`'s slot 0 (free once the specular filter is done; f16): the correction
// `upscale` adds to every output pixel's history here (linear RGB), and the share of that history to drop. The
// correction is the mean of the nine samples' distances less what their scatter explains (noise, which the blend
// averages); a correction that is a large share of the colour drops the history too, as do samples that found none
// and one whose history is of something gone.
@compute @workgroup_size(8, 8, 1)
fn reject(@builtin(global_invocation_id) gid: vec3<u32>) {
    let iw = i32(L.isz.x);
    let ih = i32(L.isz.y);
    if (i32(gid.x) >= iw || i32(gid.y) >= ih) { return; }
    let q = gid.y * L.isz.x + gid.x;
    let e = exposure_scale();
    var kern = array<f32, 2>(0.5, 0.25);
    var d1 = vec3<f32>(0.0);
    var x1 = 0.0;
    var x2 = 0.0;
    var level = 0.0;
    var wsum = 0.0;
    var wall = 0.0;
    for (var k = 0; k < 9; k++) {
        let dx = k % 3 - 1;
        let dy = k / 3 - 1;
        let qx = i32(gid.x) + dx;
        let qy = i32(gid.y) + dy;
        if (qx < 0 || qy < 0 || qx >= iw || qy >= ih) { continue; }
        let wt = kern[abs(dx)] * kern[abs(dy)];
        wall += wt;
        let j = judged_at(u32(qy * iw + qx));
        if (j.lum < 0.0) { continue; }
        let hw = 1.0 / (1.0 + j.lum * e);
        let x = lum(j.d) * hw;
        d1 += j.d * wt;
        x1 += x * wt;
        x2 += x * x * wt;
        level += j.lum * hw * wt;
        wsum += wt;
    }
    var delta = vec3<f32>(0.0);
    var drop = 1.0;
    if (wsum > 0.0) {
        // The scatter of the distances about their mean (in luminances compressed by the exposure, so a bright
        // sample's noise counts as a dark one's): a mean of nine carries about a third of it.
        let mean = x1 / wsum;
        let sd = sqrt(max(x2 / wsum - mean * mean, 0.0));
        let keep = clamp(1.0 - 0.5 * sd / max(abs(mean), 1e-12), 0.0, 1.0);
        delta = d1 / wsum * keep;
        let found = wsum / wall;
        drop = clamp(abs(mean) * keep / (UP_REJECT * level / wsum + 0.01 / e), 0.0, 1.0) * found + (1.0 - found);
    }
    // (Its history is of something that is gone from in front of this: all of it goes; and where what went was moving,
    // in every output pixel this sample's verdict reaches, `upscale`: drop 2 says so. Not for a still scene's gone: the
    // jitter alone finds one at an edge now and then, and dropped whole every time, a held Bistro flickered half as
    // much again, Kabukichō twice.)
    let jq = judged_at(q);
    if (jq.gone) {
        delta = vec3<f32>(0.0);
        drop = select(1.0, 2.0, jq.moved);
    }
    spec_store(0u, q, vec4<f32>(clamp(delta, vec3<f32>(-6.0e4), vec3<f32>(6.0e4)), drop));
}

// Colour history (linear, w = samples) at previous-frame output position (fx, fy): Catmull-Rom over 4 x 4 pixels
// (bilinear resampling every frame blurs while the view moves), held within the range of the four pixels around the
// position (the filter's negative lobes ring at strong edges).
fn up_history(fx: f32, fy: f32) -> vec4<f32> {
    let x1 = floor(fx);
    let y1 = floor(fy);
    let t = vec2<f32>(fx - x1, fy - y1);
    let w0 = t * (-0.5 + t * (1.0 - 0.5 * t));
    let w1 = 1.0 + t * t * (-2.5 + 1.5 * t);
    let w2 = t * (0.5 + t * (2.0 - 1.5 * t));
    let w3 = t * t * (-0.5 + 0.5 * t);
    let wx = array<f32, 4>(w0.x, w1.x, w2.x, w3.x);
    let wy = array<f32, 4>(w0.y, w1.y, w2.y, w3.y);
    var h = vec4<f32>(0.0);
    var lo = vec4<f32>(1.0e30);
    var hi = vec4<f32>(-1.0e30);
    for (var j = 0; j < 4; j++) {
        let qy = u32(clamp(i32(y1) + j - 1, 0, i32(L.size.y) - 1));
        for (var i = 0; i < 4; i++) {
            let qx = u32(clamp(i32(x1) + i - 1, 0, i32(L.size.x) - 1));
            let v = color_hist[last_half(qy * L.size.x + qx)];
            h += v * (wx[i] * wy[j]);
            if ((i == 1 || i == 2) && (j == 1 || j == 2)) {
                lo = min(lo, v);
                hi = max(hi, v);
            }
        }
    }
    return clamp(h, lo, hi);
}

// The temporal upscaler: the composed frame of the traced grid (fb, `compose`) to the output grid, one colour history. Each output pixel takes this frame's traced samples around it by their distance (the frame's jitter moves
// them), reprojects its history by the motion `judge` chose there, corrects and discounts that history by `reject`'s
// verdict, and blends by how near a sample came.
@compute @workgroup_size(8, 8, 1)
fn upscale(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = L.size.x;
    let h = L.size.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let i = gid.y * w + gid.x;
    let iw = i32(L.isz.x);
    let ih = i32(L.isz.y);
    let up = vec2<f32>(f32(w), f32(h)) / vec2<f32>(f32(iw), f32(ih)); // output pixels a traced pixel
    // In traced-sample units: sample (qx, qy) sits at (qx, qy), this pixel's centre at rc.
    let rc = vec2<f32>(f32(gid.x) + 0.5, f32(gid.y) + 0.5) / up - L.jit.xy;
    let bc = clamp(vec2<i32>(round(rc)), vec2<i32>(0), vec2<i32>(iw - 1, ih - 1));
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    var xlo = vec3<f32>(1.0e30);
    var xhi = vec3<f32>(-1.0e30);
    // (A lock spares the output pixels at the thing it is for: where in its traced pixel that is, `Lock`; full within
    // LOCK_REACH output pixels, none half a pixel further.)
    var lock = 0.0;
    var shaded = 0.0;
    let centre = vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5;
    for (var k = 0; k < 9; k++) {
        let qx = bc.x + k % 3 - 1;
        let qy = bc.y + k / 3 - 1;
        if (qx < 0 || qy < 0 || qx >= iw || qy >= ih) { continue; }
        let d = (vec2<f32>(f32(qx), f32(qy)) - rc) * up;
        let wt = exp(-UP_SHARP * dot(d, d));
        let c = fb[u32(qy * iw + qx)].rgb;
        sum += c * wt;
        wsum += wt;
        xlo = min(xlo, c);
        xhi = max(xhi, c);
        let j = u32(qy * iw + qx);
        let at = (vec2<f32>(f32(qx), f32(qy)) + unpack2x16float(up_state[this_half(j)].w)) * up;
        lock = max(lock, locked(j) * clamp(LOCK_REACH + 0.5 - length(at - centre), 0.0, 1.0));
        // (A sample whose light changed lately, `track`, near enough to count: fully the frame it changed, less as its
        // flicker's count grows back to LIGHT_SETTLE.)
        if (L.mode.z != 0u && wt > 0.02) {
            shaded = max(shaded, clamp((LIGHT_SETTLE - unpack2x16float(up_state[this_half(j)].y).y) / (LIGHT_SETTLE - 1.0), 0.0, 1.0));
        }
    }
    let cur = select(fb[u32(bc.y * iw + bc.x)].rgb, sum / wsum, wsum > 1e-9);
    let conf = min(wsum, 1.0);
    // `reject`'s verdict, bilinear over the four traced pixels around; the motion `judge` chose at the nearest.
    let f0 = vec2<i32>(floor(rc));
    let ft = rc - floor(rc);
    var verdict = vec4<f32>(0.0);
    var vsum = 0.0;
    var gone_near = false;
    for (var k = 0; k < 4; k++) {
        let qx = f0.x + (k & 1);
        let qy = f0.y + (k >> 1u);
        if (qx < 0 || qy < 0 || qx >= iw || qy >= ih) { continue; }
        let bw = select(1.0 - ft.x, ft.x, (k & 1) == 1) * select(1.0 - ft.y, ft.y, (k >> 1u) == 1);
        let v = spec_load(0u, u32(qy * iw + qx));
        // (A history of something gone, `reject`'s drop 2, goes wherever its verdict reaches: blended with its
        // neighbours' it left part of a moving object's edge where the edge had been, a dashed line a frame.)
        gone_near = gone_near || (v.w > 1.5 && bw > 0.0);
        verdict += vec4<f32>(v.xyz, min(v.w, 1.0)) * bw;
        vsum += bw;
    }
    verdict = select(vec4<f32>(0.0, 0.0, 0.0, 1.0), verdict / vsum, vsum > 1e-4);
    if (gone_near) { verdict = vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    let vel = judged_at(u32(bc.y * iw + bc.x)).vel;
    var hist = vec4<f32>(0.0);
    if (L.size.w == 0u) {
        let hp = vec2<f32>(f32(gid.x), f32(gid.y)) - vel;
        if (hp.x > -0.5 && hp.y > -0.5 && hp.x < f32(w) - 0.5 && hp.y < f32(h) - 0.5) {
            if (vel.x == 0.0 && vel.y == 0.0) { hist = color_hist[last_half(i)]; } else { hist = up_history(hp.x, hp.y); }
        }
        if (non_finite(hist.rgb) || non_finite(vec3<f32>(hist.w))) { hist = vec4<f32>(0.0); }
    }
    var outc = cur;
    var count = conf;
    // In motion the history is resampled every frame, each time a little blurrier, and what it holds of another
    // surface (the band behind a moving edge reprojects by the edge's motion) nothing else removes: so, as the pixel's
    // speed nears UP_SPEED, it holds fewer samples (UE TSR's r.TSR.Velocity.WeightClampingSampleCount / PixelSpeed) and
    // is clamped into the range of this frame's nine samples (every temporal AA's rule; FSR 2's rectification), but for
    // a thin feature (`Lock`). Holding still it keeps both its length and the detail finer than a traced pixel, which
    // the clamp would cut. The orbit of Thames at 9 pixels a frame against offline (detail / trail px): neither
    // 0.48 / 49, the clamp 0.45 / 36, 4 samples 0.57 / 37, both 0.53 / 31.
    let moving = clamp(length(vel) / UP_SPEED, 0.0, 1.0);
    let kept = hist.w * (1.0 - verdict.w);
    if (kept > 0.0) {
        var hc = max(hist.rgb + verdict.xyz, vec3<f32>(0.0));
        // (And where the light changed among the samples around: the history is of the light before.)
        hc = mix(hc, clamp(hc, xlo, xhi), max(moving, shaded) * (1.0 - lock));
        let most = min(mix(UP_SAMPLES, UP_MOVING, moving), mix(UP_SAMPLES, UP_REACTIVE, clamp(sample_vel[u32(bc.y * iw + bc.x)].w, 0.0, 1.0)));
        count = min(kept + conf, most);
        let a = clamp(conf / max(count, 1e-4), 0.0, 1.0);
        outc = mix(hc, cur, a);
    }
    let lin = finite_or_zero(max(outc, vec3<f32>(0.0)));
    color_hist[this_half(i)] = vec4<f32>(lin, count);
    // The image: into fa, or into sb for depth of field / motion blur to read (w: the CoC of the nearest sample's depth).
    let img = vec4<f32>(lin, dof_coc(sample_vel[u32(bc.y * iw + bc.x)].z));
    if (L.misc2.z > 0.0 || L.dof.x > 0.0) { sb[i] = img; } else { fa[i] = img; }
}

// ------------------------------------------------------------------------------------------------ exposure

// Exposure applied to scene radiance: manual (ae2.x) or this frame's auto exposure.
fn exposure_scale() -> f32 {
    if (L.ae2.w > 0.5) { return ae_state[0].x; }
    return L.misc.x;
}

fn qw() -> u32 { return (L.size.x + 3u) / 4u; }
fn qh() -> u32 { return (L.size.y + 3u) / 4u; }

// The exposure the frame's metering and thresholds go by: the manual one, or auto exposure's as the last frame left
// it (its first frame: the exposure the view starts with).
fn metering_exposure() -> f32 {
    if (L.ae2.w > 0.5 && ae_state[0].w > 0.5) { return ae_state[0].x; }
    return L.misc.x;
}

const AE_BINS: u32 = 64u;
// What a sample's position in the histogram's range (0 .. 1) is multiplied by in a bin's sum: as fine as 32 bits hold
// for every quarter-resolution texel in one bin.
fn ae_fix() -> f32 { return min(65536.0, floor(4.0e9 / f32(qw() * qh()))); }

// How much of a pixel blooms (post.rs `bloom_share`): all of it under a negative threshold, else from the threshold
// (exposed luminance) to one unit above it.
fn bloom_share(exposed: f32) -> f32 {
    if (L.post.z < 0.0) { return 1.0; }
    return clamp(exposed - L.post.z, 0.0, 1.0);
}

// The quarter-resolution view of the image (`fa`, unexposed), one read of it for three users. Per 4 x 4 block: the
// log2 of its mean luminance into `lumq` (`lum_blur` and local exposure read it; not under 12 stops below middle grey
// at the frame's exposure, post.rs `log_floor`); the same into auto exposure's histogram (`ae_bins`: a count and the
// sum of the positions in the range per bin; luminances outside the range are not counted, post.rs `exposure_scale`);
// and the mean of what blooms with that log2 luminance as the bloom pyramid's level 0 (post.rs `Pyramid`).
@compute @workgroup_size(8, 8, 1)
fn lum_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= qw() || gid.y >= qh()) { return; }
    let e = metering_exposure();
    var glow = vec3<f32>(0.0);
    var sum = 0.0;
    var n = 0.0;
    for (var y = gid.y * 4u; y < min(gid.y * 4u + 4u, L.size.y); y++) {
        for (var x = gid.x * 4u; x < min(gid.x * 4u + 4u, L.size.x); x++) {
            let c = finite_or_zero(max(fa[y * L.size.x + x].rgb, vec3<f32>(0.0)));
            let l = lum(c);
            glow += c * bloom_share(l * e);
            sum += l;
            n += 1.0;
        }
    }
    let mean = sum / max(n, 1.0);
    let floor_l = log2(0.18 / 4096.0 / max(e, 1e-20));
    var l2 = floor_l;
    if (mean > 0.0) {
        l2 = log2(mean);
        if (L.ae2.w > 0.5) {
            let t = (l2 - L.ae.z) / (L.ae.w - L.ae.z);
            if (t >= 0.0 && t <= 1.0) {
                let b = min(u32(t * f32(AE_BINS)), AE_BINS - 1u);
                atomicAdd(&ae_bins[b], 1u);
                atomicAdd(&ae_bins[AE_BINS + b], u32(t * ae_fix() + 0.5));
            }
        }
    }
    let q = gid.y * qw() + gid.x;
    lumq[q] = max(l2, floor_l);
    if (L.misc.w > 0.5) { bloom_a[q] = vec4<f32>(glow / max(n, 1.0), max(l2, floor_l)); }
}

// Beyond this distance from its target (stops) auto exposure moves at its speed; within it, it closes in
// exponentially (no stop at the speed's last step). Unreal's documentation describes the same two regimes and this
// default distance.
const AE_EASE: f32 = 1.5;

// Auto exposure (L.ae2.w > 0.5) from the histogram, into `ae_state[0]` = (exposure scale, its target, log2 of the
// metered average luminance, 1 = valid): post.rs `metered_average` (the average luminance between the low and the
// high fraction of the histogram) and `auto_target` (middle grey x compensation over it, the average within its
// limits), then the exposure moved toward the target at the speed for a scene that got brighter / darker (L.ae3;
// snapped on a reset). One invocation does it and clears the histogram for the next frame.
@compute @workgroup_size(64, 1, 1)
fn exposure(@builtin(local_invocation_id) lid: vec3<u32>) {
    if (lid.x != 0u || L.ae2.w < 0.5) { return; }
    let prev = ae_state[0];
    var total = 0.0;
    for (var b = 0u; b < AE_BINS; b++) { total += f32(atomicLoad(&ae_bins[b])); }
    var goal = select(L.misc.x, prev.y, prev.w > 0.5);
    var avg_log = prev.z;
    if (total > 0.0) {
        let lo = clamp(L.ae.x, 0.0, 1.0) * total;
        let hi = max(clamp(L.ae.y, 0.0, 1.0), clamp(L.ae.x, 0.0, 1.0)) * total;
        let fix = ae_fix();
        var seen = 0.0;
        var sum = 0.0;
        var n = 0.0;
        var at_low = -1.0;
        for (var b = 0u; b < AE_BINS; b++) {
            let c = f32(atomicLoad(&ae_bins[b]));
            if (c > 0.0) {
                let l = exp2(L.ae.z + f32(atomicLoad(&ae_bins[AE_BINS + b])) / (c * fix) * (L.ae.w - L.ae.z));
                let part = min(seen + c, hi) - max(seen, lo);
                if (part > 0.0) {
                    sum += part * l;
                    n += part;
                }
                // (A window narrower than one sample: the sample at the low bound.)
                if (at_low < 0.0 && seen + c >= lo) { at_low = l; }
            }
            seen += c;
        }
        var avg = at_low;
        if (n > 0.0) { avg = sum / n; }
        if (avg > 0.0) {
            goal = 0.18 * L.ae2.x / clamp(avg, L.ae2.y, L.ae2.z);
            avg_log = log2(avg);
        }
    }
    var e = goal;
    if (prev.w > 0.5 && prev.x > 0.0 && L.ae3.w < 0.5) {
        // Stops still to go: positive when the scene got brighter (the exposure has to come down).
        let d = log2(prev.x / goal);
        let speed = select(L.ae3.y, L.ae3.x, d > 0.0) * L.ae3.z;
        let far = max(abs(d) - AE_EASE, 0.0);
        let linear = min(far, speed);
        // (What of the frame's time the linear part did not use closes in exponentially.)
        var rest = abs(d) - linear;
        if (far <= speed) { rest *= exp(-(speed - linear) / AE_EASE); }
        e = goal * exp2(sign(d) * rest);
    }
    ae_state[0] = vec4<f32>(e, goal, avg_log, 1.0);
    for (var b = 0u; b < 2u * AE_BINS; b++) { atomicStore(&ae_bins[b], 0u); }
}

// Edge-aware blur of the quarter-res log luminance (local exposure's base layer): range weight on log2 lum.
@compute @workgroup_size(8, 8, 1)
fn lum_blur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let W = qw();
    let H = qh();
    if (gid.x >= W || gid.y >= H) { return; }
    let c = lumq[gid.y * W + gid.x];
    var acc = 0.0;
    var ws = 0.0;
    for (var j = -4; j <= 4; j++) {
        for (var i = -4; i <= 4; i++) {
            let x = clamp(i32(gid.x) + i * 2, 0, i32(W) - 1);
            let y = clamp(i32(gid.y) + j * 2, 0, i32(H) - 1);
            let v = lumq[u32(y) * W + u32(x)];
            let d = v - c;
            let wgt = exp(-f32(i * i + j * j) / 18.0 - d * d * 0.5);
            acc += v * wgt;
            ws += wgt;
        }
    }
    lumb[gid.y * W + gid.x] = acc / ws;
}

// Local exposure's factor (post.rs `local_exposure_factor`: a base / detail decomposition of log luminance) for exposed
// log2 luminance `l` at output pixel (x, y), exposure scale `e`: the base is `lumb` (the edge-aware blur of `lumq`)
// blended with `wide` (the wide plain blur, the bloom pyramid's fourth channel) by L.lx.z; its distance from the pivot
// (middle grey, times auto exposure's compensation) is scaled by L.lx.x above and L.lx.y below, the detail by L.lx.w.
fn local_exposure(l: f32, x: f32, y: f32, e: f32, wide: f32) -> f32 {
    if (L.lx.x == 1.0 && L.lx.y == 1.0 && L.lx.w == 1.0) { return 1.0; }
    let p = (vec2<f32>(x, y) + 0.5) / 4.0 - 0.5;
    let p0 = vec2<i32>(floor(p));
    let f = p - floor(p);
    let hi = vec2<i32>(i32(qw()) - 1, i32(qh()) - 1);
    let a = clamp(p0, vec2<i32>(0), hi);
    let b = clamp(p0 + 1, vec2<i32>(0), hi);
    let W = i32(qw());
    let edge = mix(mix(lumb[a.y * W + a.x], lumb[a.y * W + b.x], f.x), mix(lumb[b.y * W + a.x], lumb[b.y * W + b.x], f.x), f.y);
    let le = log2(max(e, 1e-20));
    let base = mix(edge, wide, L.lx.z) + le;
    let pivot = log2(0.18 * select(1.0, L.ae2.x, L.ae2.w > 0.5));
    let o = pivot + (base - pivot) * select(L.lx.y, L.lx.x, base > pivot) + (l - base) * L.lx.w;
    return exp2(clamp(o - l, -8.0, 8.0));
}

// ------------------------------------------------------------------------------------------------ bloom

// Bloom (post.rs `BloomPlan`, `Pyramid`: the same arithmetic on the CPU): a sum of Gaussians of the picture, one a
// stage, as a weighted sum of the levels of an image pyramid (Burt & Adelson 1983, "The Laplacian Pyramid as a
// Compact Image Code": each level the one above under a small binomial kernel, at half the resolution) with the
// Gaussians wider than its coarsest level summed directly on that level's few texels. Level 0 is `lum_down`'s
// (a quarter of the output each way), `bloom_down1..5` make the rest, `bloom_wide_rows` and `bloom_wide` the coarsest
// level's part of the sum, `bloom_up4..0` add each level by its weight to the sum from below, bilinearly; `bloom_at`
// reads level 0. The fourth channel carries log2 luminance through the same passes with weights of its own: the wide
// blur local exposure blends its base with. 12 passes after `lum_down`, of 1/64 of the output's pixels and less. A stage
// narrower than level 0 can be (under about 2 output pixels: the first stages of a small output) is blended with a
// five-pixel kernel of the output's own pixels in `post`.
// (The offline chain takes each stage's Gaussian to its own shape, post.rs `gaussian`: next to a light hundreds of
// times brighter than its surroundings the pyramid's compact kernels end sooner than a Gaussian's tail.)
const BLOOM_TOP: u32 = 5u;

fn bloom_size(j: u32) -> vec2<u32> {
    var d = vec2<u32>(qw(), qh());
    for (var k = 0u; k < j; k++) { d = (d + 1u) / 2u; }
    return max(d, vec2<u32>(1u));
}

// Where level j starts in `bloom_a` (BLOOM_TOP + 1: `bloom_wide`'s sum; after it, `bloom_wide_rows`' copies).
fn bloom_base(j: u32) -> u32 {
    var d = max(vec2<u32>(qw(), qh()), vec2<u32>(1u));
    var o = 0u;
    for (var k = 0u; k < j; k++) {
        o += d.x * d.y;
        if (k < BLOOM_TOP) { d = max((d + 1u) / 2u, vec2<u32>(1u)); }
    }
    return o;
}

fn bloom_texel(base: u32, d: vec2<u32>, x: i32, y: i32) -> vec4<f32> {
    return bloom_a[base + u32(clamp(y, 0, i32(d.y) - 1)) * d.x + u32(clamp(x, 0, i32(d.x) - 1))];
}

fn bloom_bilinear(base: u32, d: vec2<u32>, p: vec2<f32>) -> vec4<f32> {
    let p0 = vec2<i32>(floor(p));
    let f = p - floor(p);
    return mix(mix(bloom_texel(base, d, p0.x, p0.y), bloom_texel(base, d, p0.x + 1, p0.y), f.x), mix(bloom_texel(base, d, p0.x, p0.y + 1), bloom_texel(base, d, p0.x + 1, p0.y + 1), f.x), f.y);
}

// Level j from level j - 1: the binomial [1 3 3 1] / 8 each way, centred between its texels.
fn bloom_down(gid: vec3<u32>, j: u32) {
    let d = bloom_size(j);
    if (L.misc.w < 0.5 || gid.x >= d.x || gid.y >= d.y) { return; }
    let sd = bloom_size(j - 1u);
    let so = bloom_base(j - 1u);
    var s = vec4<f32>(0.0);
    for (var dy = 0; dy < 4; dy++) {
        for (var dx = 0; dx < 4; dx++) {
            let k = select(0.375, 0.125, dx == 0 || dx == 3) * select(0.375, 0.125, dy == 0 || dy == 3);
            s += bloom_texel(so, sd, i32(gid.x) * 2 + dx - 1, i32(gid.y) * 2 + dy - 1) * k;
        }
    }
    bloom_a[bloom_base(j) + gid.y * d.x + gid.x] = s;
}

@compute @workgroup_size(8, 8, 1)
fn bloom_down1(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_down(gid, 1u); }
@compute @workgroup_size(8, 8, 1)
fn bloom_down2(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_down(gid, 2u); }
@compute @workgroup_size(8, 8, 1)
fn bloom_down3(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_down(gid, 3u); }
@compute @workgroup_size(8, 8, 1)
fn bloom_down4(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_down(gid, 4u); }
@compute @workgroup_size(8, 8, 1)
fn bloom_down5(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_down(gid, 5u); }

// The Gaussians wider than the pyramid, summed on the coarsest level's texels within L.bl2.z of each (the edge texels
// repeat outward), each way in turn: a Gaussian is the product of its two axes, so one pass blurs the rows once per
// Gaussian (its own copy of the level behind the pyramid in `bloom_a`: BLOOM_TOP + 2 + k; the last for the fourth
// channel's) and the next the columns, adding them by their weights (L.bw; L.lb2.zw for the fourth channel) to the
// level's own share: the coarsest level's part of the sum (BLOOM_TOP + 1). Each kernel is divided by its own sum.
const BLOOM_WIDE: u32 = 6u;

fn bloom_wide_pair(k: u32) -> vec2<f32> {
    if (k == BLOOM_WIDE) { return L.lb2.zw; }
    let pair = L.bw[k / 2u];
    if ((k & 1u) == 0u) { return pair.xy; }
    return pair.zw;
}

@compute @workgroup_size(8, 8, 1)
fn bloom_wide_rows(@builtin(global_invocation_id) gid: vec3<u32>) {
    let d = bloom_size(BLOOM_TOP);
    let r = i32(L.bl2.z);
    if (L.misc.w < 0.5 || r <= 0 || gid.x >= d.x || gid.y >= d.y) { return; }
    let o = bloom_base(BLOOM_TOP);
    let n = d.x * d.y;
    for (var k = 0u; k <= BLOOM_WIDE; k++) {
        let pair = bloom_wide_pair(k);
        if (pair.x <= 0.0) { continue; }
        var s = vec4<f32>(0.0);
        var ws = 0.0;
        for (var dx = -r; dx <= r; dx++) {
            let g = exp(-f32(dx * dx) * pair.y);
            s += bloom_texel(o, d, i32(gid.x) + dx, i32(gid.y)) * g;
            ws += g;
        }
        bloom_a[o + (2u + k) * n + gid.y * d.x + gid.x] = s / ws;
    }
}

@compute @workgroup_size(8, 8, 1)
fn bloom_wide(@builtin(global_invocation_id) gid: vec3<u32>) {
    let d = bloom_size(BLOOM_TOP);
    if (L.misc.w < 0.5 || gid.x >= d.x || gid.y >= d.y) { return; }
    let o = bloom_base(BLOOM_TOP);
    let n = d.x * d.y;
    let own = bloom_a[o + gid.y * d.x + gid.x];
    var sum = vec4<f32>(own.rgb * L.bl2.y, own.w * L.lb2.y);
    let r = i32(L.bl2.z);
    for (var k = 0u; k <= BLOOM_WIDE; k++) {
        let pair = bloom_wide_pair(k);
        if (r <= 0 || pair.x <= 0.0) { continue; }
        var s = vec4<f32>(0.0);
        var ws = 0.0;
        for (var dy = -r; dy <= r; dy++) {
            let g = exp(-f32(dy * dy) * pair.y);
            s += bloom_texel(o + (2u + k) * n, d, i32(gid.x), i32(gid.y) + dy) * g;
            ws += g;
        }
        s *= pair.x / ws;
        if (k == BLOOM_WIDE) { sum.w += s.w; } else { sum += vec4<f32>(s.rgb, 0.0); }
    }
    bloom_a[o + n + gid.y * d.x + gid.x] = sum;
}

// Level j becomes the sum from it down: itself by its weights, plus the level below's sum, bilinear.
fn bloom_up(gid: vec3<u32>, j: u32) {
    let d = bloom_size(j);
    if (L.misc.w < 0.5 || gid.x >= d.x || gid.y >= d.y) { return; }
    let o = bloom_base(j) + gid.y * d.x + gid.x;
    var src = j + 1u;
    if (src == BLOOM_TOP) { src = BLOOM_TOP + 1u; }
    let below = bloom_bilinear(bloom_base(src), bloom_size(j + 1u), (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) / 2.0 - 0.5);
    let own = bloom_a[o];
    var wc = L.bl2[j & 3u];
    var wb = L.lb2[j & 3u];
    if (j < 4u) {
        wc = L.bl[j];
        wb = L.lb[j];
    }
    bloom_a[o] = vec4<f32>(own.rgb * wc + below.rgb, own.w * wb + below.w);
}

@compute @workgroup_size(8, 8, 1)
fn bloom_up4(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_up(gid, 4u); }
@compute @workgroup_size(8, 8, 1)
fn bloom_up3(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_up(gid, 3u); }
@compute @workgroup_size(8, 8, 1)
fn bloom_up2(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_up(gid, 2u); }
@compute @workgroup_size(8, 8, 1)
fn bloom_up1(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_up(gid, 1u); }
@compute @workgroup_size(8, 8, 1)
fn bloom_up0(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_up(gid, 0u); }

// The pyramid's sum at output pixel (x, y): rgb the bloom to add (unexposed), w the wide blur of log2 luminance.
fn bloom_at(x: f32, y: f32) -> vec4<f32> {
    return bloom_bilinear(0u, vec2<u32>(qw(), qh()), (vec2<f32>(x, y) + 0.5) / 4.0 - 0.5);
}

// ------------------------------------------------------------------------------------------------ depth of field

// The signed circle-of-confusion radius (output px) at view depth z (0 = sky: as far as can be), negative in front
// of the focus plane: a thin lens' (live/mod.rs `dof_params`: L.dof.x the radius of a point at infinity, L.dof.y the
// focus distance), r_inf (1 - s / z), held to the largest radius L.dof.z.
fn dof_coc(z: f32) -> f32 {
    if (L.dof.x <= 0.0) { return 0.0; }
    if (z <= 0.0) { return min(L.dof.x, L.dof.z); }
    return clamp(L.dof.x * (1.0 - L.dof.y / z), -L.dof.z, L.dof.z);
}

const DOF_TILE: u32 = 16u;
const DOF_SAMPLES: i32 = 48;

// Largest CoC per 16 x 16 tile (into lumq, free until `lum_down`): bounds each pixel's gather.
@compute @workgroup_size(8, 8, 1)
fn dof_tiles(@builtin(global_invocation_id) gid: vec3<u32>) {
    let tw = (L.size.x + DOF_TILE - 1u) / DOF_TILE;
    let th = (L.size.y + DOF_TILE - 1u) / DOF_TILE;
    if (gid.x >= tw || gid.y >= th || L.dof.x <= 0.0) { return; }
    var m = 0.0;
    for (var y = gid.y * DOF_TILE; y < min((gid.y + 1u) * DOF_TILE, L.size.y); y++) {
        for (var x = gid.x * DOF_TILE; x < min((gid.x + 1u) * DOF_TILE, L.size.x); x++) {
            m = max(m, abs(sb[y * L.size.x + x].w));
        }
    }
    lumq[gid.y * tw + gid.x] = m;
}

// Depth of field on the upscaled HDR image (sb -> fa, or fb when motion blur follows), before motion blur as in UE.
// Scatter as gather over the largest CoC around: each pixel spreads its colour over its own CoC disk. Samples in front
// of this pixel form a foreground layer over the rest, its coverage from their spread; samples behind count only as
// far as this pixel's own CoC reaches, so blurred background doesn't cover a sharp pixel.
@compute @workgroup_size(8, 8, 1)
fn dof(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = L.size.x;
    let h = L.size.y;
    if (gid.x >= w || gid.y >= h || L.dof.x <= 0.0) { return; }
    let i = gid.y * w + gid.x;
    let center = sb[i];
    let ci = abs(center.w);
    let tw = (w + DOF_TILE - 1u) / DOF_TILE;
    let th = (h + DOF_TILE - 1u) / DOF_TILE;
    let tx = i32(gid.x / DOF_TILE);
    let ty = i32(gid.y / DOF_TILE);
    var r = ci;
    for (var k = 0; k < 9; k++) {
        let x = tx + k % 3 - 1;
        let y = ty + k / 3 - 1;
        if (x < 0 || y < 0 || x >= i32(tw) || y >= i32(th)) { continue; }
        r = max(r, lumq[u32(y) * tw + u32(x)]);
    }
    var outv = center.rgb;
    if (r >= 0.5) {
        let pi = 3.14159265;
        let zi = view_depth(i);
        let area = pi * r * r / f32(DOF_SAMPLES); // pixels each sample stands for
        let rot = 6.2831853 * fract(52.9829189 * fract(0.06711056 * f32(gid.x) + 0.00583715 * f32(gid.y)));
        var bw = 1.0 / (pi * max(ci * ci, 0.25));
        var bg = center.rgb * bw;
        var fg = vec3<f32>(0.0);
        var fw = 0.0;
        let pc = vec2<f32>(f32(gid.x), f32(gid.y));
        for (var k = 0; k < DOF_SAMPLES; k++) {
            let a = f32(k) * 2.39996323 + rot; // Vogel disk
            let q = vec2<i32>(round(pc + r * sqrt((f32(k) + 0.5) / f32(DOF_SAMPLES)) * vec2<f32>(cos(a), sin(a))));
            if (q.x < 0 || q.y < 0 || q.x >= i32(w) || q.y >= i32(h)) { continue; }
            let j = u32(q.y) * w + u32(q.x);
            let sj = sb[j];
            let cj = abs(sj.w);
            let d = length(vec2<f32>(q) - pc);
            let front = clamp((zi - view_depth(j)) / (0.02 * zi + 0.05), 0.0, 1.0);
            let wf = front * clamp(cj - d + 0.5, 0.0, 1.0) * area / (pi * max(cj * cj, 0.25));
            let rb = min(cj, ci);
            let wb = (1.0 - front) * clamp(rb - d + 0.5, 0.0, 1.0) * area / (pi * max(rb * rb, 0.25));
            fg += sj.rgb * wf;
            fw += wf;
            bg += sj.rgb * wb;
            bw += wb;
        }
        outv = bg / bw;
        if (fw > 1e-6) { outv = mix(outv, fg / fw, clamp(fw, 0.0, 1.0)); }
        if (non_finite(outv)) { outv = center.rgb; }
    }
    if (L.misc2.z > 0.0) { fb[i] = vec4<f32>(outv, 0.0); } else { fa[i] = vec4<f32>(outv, 0.0); }
}

// ------------------------------------------------------------------------------------------------ motion blur

// The image motion blur reads: depth of field's output, or the composed image.
fn mb_src(q: u32) -> vec3<f32> {
    if (L.dof.x > 0.0) { return fb[q].rgb; }
    return sb[q].rgb;
}

// Output pixel q's motion and depth: those of the traced sample under it (`compose`).
fn motion_under(q: u32) -> vec4<f32> {
    return sample_vel[sample_under(q % L.size.x, q / L.size.x)];
}

fn velocity(q: u32) -> vec2<f32> {
    return motion_under(q).xy * L.misc2.z;
}

fn view_depth(q: u32) -> f32 {
    let z = motion_under(q).z;
    return select(z, 1.0e6, z <= 0.0);
}

const MB_SAMPLES: i32 = 12;

// UE-style motion blur on the HDR image before bloom (MotionBlurAmount / Max / TargetFPS: misc2.z scales this frame's
// velocity to the blur length, post.w caps it). A reconstruction filter after McGuire et al. 2012 without tiles:
// samples along the pixel's own blur; a sample counts where it is in front and its own blur reaches this pixel, or
// behind and this pixel's blur reaches it, so moving objects smear over a static background but a static foreground
// stays sharp. sb (`upscale` writes there while it runs: misc2.z > 0, set only when the view or an object moved), or
// fb after depth of field -> fa.
@compute @workgroup_size(8, 8, 1)
fn mblur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = L.size.x;
    let h = L.size.y;
    if (gid.x >= w || gid.y >= h || L.misc2.z <= 0.0) { return; }
    let i = gid.y * w + gid.x;
    var vc = velocity(i);
    let len_c = length(vc);
    let c = mb_src(i);
    if (len_c < 0.5) {
        fa[i] = vec4<f32>(c, 0.0);
        return;
    }
    if (len_c > L.post.w) { vc *= L.post.w / len_c; }
    let lc = min(len_c, L.post.w);
    let zc = view_depth(i);
    let jit = f32(((gid.x * 73856093u) ^ (gid.y * 19349663u) ^ (L.size.z * 83492791u)) & 1023u) / 1024.0 - 0.5;
    var sum = c / lc;
    var wsum = 1.0 / lc;
    for (var k = 0; k < MB_SAMPLES; k++) {
        let t = (f32(k) + 0.5 + jit) / f32(MB_SAMPLES) - 0.5;
        let q = vec2<i32>(round(vec2<f32>(f32(gid.x), f32(gid.y)) + vc * t));
        if (q.x < 0 || q.y < 0 || q.x >= i32(w) || q.y >= i32(h)) { continue; }
        let j = u32(q.y) * w + u32(q.x);
        let lj = min(length(velocity(j)), L.post.w);
        let zj = view_depth(j);
        let d = abs(t) * lc;
        let front = clamp(1.0 - (zj - zc) / (0.02 * zc + 0.05), 0.0, 1.0); // j at or in front of this pixel
        let back = clamp(1.0 - (zc - zj) / (0.02 * zc + 0.05), 0.0, 1.0);  // j at or behind it
        let cone_j = clamp(1.0 - d / max(lj, 1e-3), 0.0, 1.0) / max(lj, 1.0);
        let cone_c = clamp(1.0 - d / lc, 0.0, 1.0) / lc;
        let both = (1.0 - smoothstep(0.95 * lj, 1.05 * lj, d)) * (1.0 - smoothstep(0.95 * lc, 1.05 * lc, d)) / max(max(lj, lc), 1.0);
        let wt = front * cone_j + back * cone_c + 2.0 * both;
        sum += mb_src(j) * wt;
        wsum += wt;
    }
    var outv = sum / wsum;
    if (non_finite(outv)) { outv = c; }
    fa[i] = vec4<f32>(outv, 0.0);
}

// ------------------------------------------------------------------------------------------------ view modes

// Unreal-style view modes (`L.mode.x`), passes of their own before `post` that run only in them (live/layout.rs
// `PASSES`), so that `post` binds only what the picture needs. `view_raw`: base colour (1), world normal (2) and the
// diffuse history's length (5) of the traced sample under each output pixel, as they are, into the output (`post`
// leaves it).
@compute @workgroup_size(8, 8, 1)
fn view_raw(@builtin(global_invocation_id) gid: vec3<u32>) {
    let mode = L.mode.x;
    if (gid.x >= L.size.x || gid.y >= L.size.y || (mode != 1u && mode != 2u && mode != 5u)) { return; }
    let tq = sample_under(gid.x, gid.y);
    var d = vec3<f32>(0.0);
    if (mode == 1u) {
        let a = clamp(sample_px[tq].albedo.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        d = select(pow((a + 0.055) / 1.055, vec3<f32>(1.0 / 2.4)) * 1.055 - 0.055, a * 12.92, a <= vec3<f32>(0.0031308));
    } else if (mode == 2u) {
        let g = gb_unpack(gbuffer[this_half(tq)]);
        d = select(g.n * 0.5 + 0.5, vec3<f32>(0.0), g.depth == 0.0);
    } else if (mode == 5u) {
        let t = clamp(log2(max(diffuse_hist[this_half(tq)].w, 1.0)) / 12.0, 0.0, 1.0); // 1 .. 4096 frames (diffuse)
        d = clamp(vec3<f32>(1.5 - abs(4.0 * t - 3.0), 1.5 - abs(4.0 * t - 2.0), 1.5 - abs(4.0 * t - 1.0)), vec3<f32>(0.0), vec3<f32>(1.0));
    } else {
        return;
    }
    let q = vec3<u32>(clamp(d, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + 0.5);
    out_px[gid.y * L.size.x + gid.x] = q.z | (q.y << 8u) | (q.x << 16u) | (255u << 24u);
}

// `view_hdr`: in place of the image `post` exposes and tonemaps (`fa`), the traced sample under the pixel: its
// accumulations without the spatial filters (3), and the denoised signals apart: diffuse (6: the composed colour less
// the rest), specular (7), deterministic direct light (8). (6 does not hold with depth of field and motion blur both
// on: the composed colour's buffer is theirs then.)
@compute @workgroup_size(8, 8, 1)
fn view_hdr(@builtin(global_invocation_id) gid: vec3<u32>) {
    let mode = L.mode.x;
    if (gid.x >= L.size.x || gid.y >= L.size.y || (mode != 3u && mode != 6u && mode != 7u && mode != 8u)) { return; }
    let i = gid.y * L.size.x + gid.x;
    let tq = sample_under(gid.x, gid.y);
    var src = fa[i].rgb;
    if (mode == 3u) { src = (diffuse_hist[this_half(tq)].rgb * max(sample_px[tq].albedo.rgb, vec3<f32>(0.02)) + spec_hist[this_half(tq)].rgb) * medium_tr(tq) + sample_emission(tq) + direct_in[tq].rgb; }
    if (mode == 6u) { src = max(fb[tq].rgb - spec_load(1u, tq).rgb * medium_tr(tq) - sample_emission(tq) - direct_in[tq].rgb, vec3<f32>(0.0)); }
    if (mode == 8u) { src = direct_in[tq].rgb; }
    if (mode == 7u) { src = spec_load(1u, tq).rgb * medium_tr(tq); }
    fa[i] = vec4<f32>(src, fa[i].w);
}

// `view_sample`: the same with the raw sample under the pixel (4).
@compute @workgroup_size(8, 8, 1)
fn view_sample(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= L.size.x || gid.y >= L.size.y || L.mode.x != 4u) { return; }
    let i = gid.y * L.size.x + gid.x;
    fa[i] = vec4<f32>(sample_color(sample_under(gid.x, gid.y)), fa[i].w);
}

// ------------------------------------------------------------------------------------------------ post

// The colour table's axis (post.rs `lut_coord`): log2 of the exposed linear value over LUT_LOG_MIN .. LUT_LOG_MAX.
const LUT_LOG_MIN: f32 = -10.5;
const LUT_LOG_MAX: f32 = 6.5;
// post.rs `VIGNETTE_FIELD`, `FRINGE`, `GRAIN`.
const VIGNETTE_FIELD: f32 = 1.425;
const FRINGE: f32 = 0.002;
const GRAIN: f32 = 0.085;

// A 32-bit integer hash (Chris Wellons' "lowbias32", public domain: https://nullprogram.com/blog/2018/07/31/).
fn hash32(v: u32) -> u32 {
    var x = v;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

// The image (`fa`) at p (pixels, centres at integers), bilinear.
fn image_bilinear(p: vec2<f32>) -> vec3<f32> {
    let hi = vec2<i32>(i32(L.size.x) - 1, i32(L.size.y) - 1);
    let p0 = vec2<i32>(floor(p));
    let f = p - floor(p);
    let a = clamp(p0, vec2<i32>(0), hi);
    let b = clamp(p0 + 1, vec2<i32>(0), hi);
    let W = i32(L.size.x);
    return mix(mix(fa[a.y * W + a.x].rgb, fa[a.y * W + b.x].rgb, f.x), mix(fa[b.y * W + a.x].rgb, fa[b.y * W + b.x].rgb, f.x), f.y);
}

// The final pass: post.rs `finish`, step for step. Chromatic aberration (red and blue read from where the lens puts
// them: magnified / reduced about the centre), exposure, local exposure, bloom added, the cos^4 vignette, colour
// grading and the tone curve (the `lut` texture, live/targets.rs `tonemap_lut`: sRGB-encoded display values by the
// log2 of the exposed linear colour, trilinear), film grain, 8-bit output (BGRA in memory) into `out_px`.
@compute @workgroup_size(8, 8, 1)
fn post(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = L.size.x;
    let h = L.size.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let i = gid.y * w + gid.x;
    let mode = L.mode.x;
    // (The view modes that show a buffer as it is are `view_raw`'s picture.)
    if (mode == 1u || mode == 2u || mode == 5u) { return; }
    let e = exposure_scale();
    let centre = vec2<f32>(f32(w), f32(h)) * 0.5;
    let at = vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5 - centre;
    var c = fa[i].rgb;
    if (L.post.x > 0.0) {
        let k = L.post.x * FRINGE;
        c = vec3<f32>(image_bilinear(centre + at * (1.0 - k) - 0.5).r, c.g, image_bilinear(centre + at * (1.0 + k) - 0.5).b);
    }
    c = finite_or_zero(max(c, vec3<f32>(0.0))) * e;
    if (L.misc.w > 0.5) {
        let glow = bloom_at(f32(gid.x), f32(gid.y));
        let floor_l = log2(0.18 / 4096.0);
        c *= local_exposure(max(log2(max(lum(c), 1e-30)), floor_l), f32(gid.x), f32(gid.y), e, glow.w);
        c += glow.rgb * e;
    }
    if (L.bl2.w > 0.0) {
        // The bloom stages narrower than the pyramid's first level (post.rs `BloomPlan::fine`; at output sizes where
        // there are any): the pixel and its four neighbours, a fifth each (a kernel of variance 0.4 each way).
        var s = vec3<f32>(0.0);
        for (var k = 0; k < 5; k++) {
            let x = u32(clamp(i32(gid.x) + select(0, k * 2 - 1, k < 2), 0, i32(w) - 1));
            let y = u32(clamp(i32(gid.y) + select(0, k * 2 - 5, k > 1 && k < 4), 0, i32(h) - 1));
            let t = finite_or_zero(max(fa[y * w + x].rgb, vec3<f32>(0.0)));
            s += t * bloom_share(lum(t) * e);
        }
        c += s * (0.2 * L.bl2.w * e);
    }
    let t = VIGNETTE_FIELD * L.misc.z * length(at) / length(centre);
    c /= (1.0 + t * t) * (1.0 + t * t);
    let n = f32(textureDimensions(lut).x);
    let u = clamp((log2(max(c, vec3<f32>(1e-9))) - LUT_LOG_MIN) / (LUT_LOG_MAX - LUT_LOG_MIN), vec3<f32>(0.0), vec3<f32>(1.0));
    var d = textureSampleLevel(lut, lut_samp, (u * (n - 1.0) + 0.5) / n, 0.0).rgb;
    if (L.post.y > 0.0) {
        // Film grain (post.rs `grain`): zero-mean, triangular, the same on the three channels, most in the mid-tones.
        let r = hash32(gid.x ^ hash32(gid.y ^ hash32(L.size.z)));
        let v = lum(d);
        d += min(L.post.y, 1.0) * GRAIN * (f32(r & 0xffffu) / 65536.0 + f32(r >> 16u) / 65536.0 - 1.0) * 4.0 * v * (1.0 - v);
    }
    let q = vec3<u32>(clamp(d, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + 0.5);
    out_px[i] = q.z | (q.y << 8u) | (q.x << 16u) | (255u << 24u); // BGRA in memory
}

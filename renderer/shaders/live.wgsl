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
//   bloom_*            - UE Standard bloom: six Gaussian stages on a 1/4 .. 1/64 downsample chain, summed from the widest up
//   post               - exposure, bloom, UE cos^4 vignette, UE filmic tonemap via a log-encoded 3D LUT, BGRA8 output

struct Cam { pos: vec4<f32>, fwd: vec4<f32>, right: vec4<f32>, up: vec4<f32> }; // pos.w = tan(fov/2), fwd.w = aspect
struct LiveParams {
    cur: Cam,
    prev: Cam,
    size: vec4<u32>,   // width, height, frame, reset history (1)
    misc: vec4<f32>,   // exposure scale, max history, vignette intensity, bloom on (1) / off (0)
    misc2: vec4<f32>,  // denoise strength, sample clamp (display units at the current exposure), -, samples this frame
    mode: vec4<u32>,   // view mode (0 lit, 1 base color, 2 world normal, 3 lit no denoise, 4 raw sample, 5 history), frames since the view translated, the scene's lighting is changing (live.rs `live_lighting`), experiment flags
    post: vec4<f32>,   // chromatic aberration (UE scene fringe), film grain, bloom threshold (<= 0 off), -
    ae: vec4<f32>,     // auto exposure: low / high fraction, histogram log2 min / max
    ae2: vec4<f32>,    // compensation (2^bias; manual: the exposure), min / max average luminance, mode (1 = auto)
    ae3: vec4<f32>,    // speed up / down (EV/s), frame time (s), snap to target
    lx: vec4<f32>,     // local exposure: highlight contrast, shadow contrast, blurred-luminance blend, detail strength
    isz: vec4<u32>,    // traced (internal) width, height (= size.xy at 100% render scale); the previous frame's (0 = none)
    br: vec4<f32>,     // bloom stage 0..3 Gaussian radius, texels of the stage's level
    br2: vec4<f32>,    // bloom stage 4, 5 radius, -, -
    bt: vec4<f32>,     // bloom stage 0..3 tint (BloomN Tint * intensity / 6)
    bt2: vec4<f32>,    // bloom stage 4, 5 tint, -, -
    dof: vec4<f32>,    // depth of field: CoC radius of infinity (output px; 0 = off), focus distance (m), max radius (px), -
    jit: vec4<f32>,    // this frame's sub-pixel jitter (every traced sample's: x, y in 0 .. 1 of a traced pixel), -, -
};
// color = diffuse lighting + first-hit distance, emit = directly seen emission / sky + roughness, spec = specular lighting
// + reflection hit distance, normal.w = the media before the first hit (`medium_tr`, `medium_share`; see pathtrace.wgsl).
struct Pixel { color: vec4<f32>, albedo: vec4<f32>, normal: vec4<f32>, emit: vec4<f32>, spec: vec4<f32> };

@group(0) @binding(0) var<uniform> L: LiveParams;
@group(0) @binding(1) var<storage, read> sample_px: array<Pixel>;          // color.w = first-hit distance (0 = sky)
// Diffuse irradiance history (rgb, frames) on the traced grid (`dtemporal`; `mom_in` / `mom_out` hold its moments),
// then the output grid's colour history (rgb, samples; `out_hist`, `upscale`).
@group(0) @binding(2) var<storage, read> hist_in: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> hist_out: array<vec4<f32>>;
// The traced grid's g-buffer (`stemporal`): octahedral normal, view depth (0 = sky), roughness (`GB`).
@group(0) @binding(4) var<storage, read> gb_in: array<vec2<u32>>;
@group(0) @binding(5) var<storage, read_write> gb_out: array<vec2<u32>>;
// fa: the irradiance filter's ping-pong on the traced grid, then the image on the output grid (what the post chain
// reads). fb: its other half, then the composed colour of the traced grid (`compose`; w: first-hit distance), then
// depth of field's output when motion blur follows.
@group(0) @binding(6) var<storage, read_write> fa: array<vec4<f32>>;
@group(0) @binding(7) var<storage, read_write> fb: array<vec4<f32>>;
@group(0) @binding(8) var<storage, read_write> bloom_a: array<vec4<f32>>;     // bloom pyramid levels, then stage sums
@group(0) @binding(9) var<storage, read_write> bloom_b: array<vec4<f32>>;     // horizontal blur scratch
@group(0) @binding(10) var<storage, read_write> out_px: array<u32>;
@group(0) @binding(11) var lut: texture_3d<f32>;
@group(0) @binding(12) var lut_samp: sampler;
@group(0) @binding(13) var<storage, read_write> ae_bins: array<atomic<u32>, 64>;   // fixed point (x 256)
@group(0) @binding(14) var<storage, read_write> ae_state: array<vec4<f32>, 2>;    // exposure, target, mean log2 lum, valid
@group(0) @binding(15) var<storage, read_write> lumq: array<f32>;                 // quarter res log2 luminance (unexposed)
@group(0) @binding(16) var<storage, read_write> lumb: array<f32>;                 // edge-aware blur of lumq
// Per traced sample, what `judge` finds of the output grid's colour history (`Judged`, f16). (`ae_in`: its ping-pong
// partner, not read.)
@group(0) @binding(17) var<storage, read> ae_in: array<vec4<u32>>;
@group(0) @binding(18) var<storage, read_write> ae_out: array<vec4<u32>>;
// Specular history on the traced grid (rgb + history length), diffuse luminance moments (mean, mean square, fast mean, -).
@group(0) @binding(19) var<storage, read> shist_in: array<vec4<f32>>;
@group(0) @binding(20) var<storage, read_write> shist_out: array<vec4<f32>>;
@group(0) @binding(21) var<storage, read> mom_in: array<vec4<f32>>;
@group(0) @binding(22) var<storage, read_write> mom_out: array<vec4<f32>>;
// The specular denoiser's ping-pong on the traced grid (`stemporal` -> `sfilt0` -> `sfilt1` -> `sfilt2`): (rgb, history
// length) as f16, two slots (the passes are bandwidth-bound: 8 bytes per pixel instead of 16). The result is slot 1;
// slot 0 then holds `reject`'s verdict.
@group(0) @binding(23) var<storage, read_write> spec_pp: array<vec2<u32>>;
// Scratch: the traced surfaces for the diffuse denoiser (`dtemporal`), then the upscaled image when depth of field /
// motion blur follow (they read it; w: the circle of confusion).
@group(0) @binding(24) var<storage, read_write> sb: array<vec4<f32>>;
// Reflection resolve: the trace kernel's specular-lobe rays, 2 per traced pixel ((radiance back, lobe pdf; 0 = none),
// (specular albedo E, octahedral direction)), and the resolved specular reflection per traced pixel.
@group(0) @binding(25) var<storage, read> refl_in: array<vec4<f32>>;
@group(0) @binding(26) var<storage, read_write> refl_res: array<vec4<f32>>;
// Per traced pixel: where its surface point was the previous frame, as a world-space offset (w = 1: a moving object).
@group(0) @binding(27) var<storage, read> motion_in: array<vec4<f32>>;
// Directional lights' direct light (the sun: nearly deterministic) at camera hits per traced pixel: no denoiser
// (`compose` adds it as it is, UE lights directly). `xhist_out`: per traced sample, its screen velocity (output pixels
// this frame), view depth and whether its shading is animated (`compose`). (`xhist_in`: its ping-pong partner, not read.)
@group(0) @binding(28) var<storage, read> direct_in: array<vec4<f32>>;
@group(0) @binding(29) var<storage, read> xhist_in: array<vec4<f32>>;
@group(0) @binding(30) var<storage, read_write> xhist_out: array<vec4<f32>>;
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

// YCoCg (UE LumenRGBToYCoCg): neighbourhood clamps in a luma / chroma space.
fn ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}
fn ycocg_rgb(y: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(y.x + y.y - y.z, y.x + y.z, y.x - y.y - y.z);
}

// Experiment switches (THREERT_TEMPORAL_FLAGS): 1 = a-trous gated by history length (round 3), 2 = specular spatial
// filter on mirror-like surfaces too (round 3), 4 = specular clamp also while the view holds still (UE's literal
// setting; see the specular block in `temporal`).
fn xflag(b: u32) -> bool { return (L.mode.w & b) != 0u; }

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// ------------------------------------------------------------------------------------------------ temporal

// One traced sample's parts (averaged over this frame's samples): diffuse lighting, specular lighting, and what the
// camera ray saw directly (emission, sky). Lighting is clamped at `sample_clamp()` (an exposure-relative ceiling:
// fireflies far above white). Returns (diffuse, specular, emission).
fn sample_parts(q: u32) -> array<vec3<f32>, 3> {
    let n = max(L.misc2.w, 1.0);
    var d = sample_px[q].color.rgb / n;
    var s = sample_px[q].spec.rgb / n + refl_res[q].rgb;
    var e = sample_px[q].emit.rgb / n;
    if (non_finite(d)) { d = vec3<f32>(0.0); }
    if (non_finite(s)) { s = vec3<f32>(0.0); }
    if (non_finite(e)) { e = vec3<f32>(0.0); }
    let cl = sample_clamp();
    if (lum(d) > cl) { d *= cl / lum(d); }
    if (lum(s) > cl) { s *= cl / lum(s); }
    return array<vec3<f32>, 3>(d, s, e);
}

// Roughness below which a surface counts as a near mirror: its reflection hit distance is the resolved one (pass `refl`).
const NEAR_MIRROR: f32 = 0.2;

// ------------------------------------------------------------------------------------------------ reflection resolve

fn d_ggx(a2: f32, noh: f32) -> f32 {
    let d = (noh * a2 - noh) * noh + 1.0;
    return a2 / (3.14159265 * d * d);
}

fn vis_smith(a2: f32, nov: f32, nol: f32) -> f32 {
    let a = sqrt(a2);
    return 0.5 / max(nol * (nov * (1.0 - a) + a) + nov * (nol * (1.0 - a) + a), 1e-7);
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

// The output-grid g-buffer (`gb_out`, next frame's `gb_in`), 8 bytes: octahedral normal (snorm16 x 2), view depth
// (0 = sky) and roughness (f16). The surface tests compare depths relatively (2-4 %), far coarser than f16. One fetch
// per tap for the specular filter and the history tests (it was 16 bytes, and roughness a second 16-byte fetch from the
// albedo / emission history): those passes are bandwidth-bound.
struct GB { n: vec3<f32>, depth: f32, rough: f32 };

fn gb_pack(n: vec3<f32>, depth: f32, rough: f32) -> vec2<u32> {
    return vec2<u32>(pack2x16snorm(oct_enc(n)), pack2x16float(vec2<f32>(min(depth, 6.0e4), rough)));
}

fn gb_unpack(g: vec2<u32>) -> GB {
    let dr = unpack2x16float(g.y);
    return GB(oct_dec(g.x), dr.x, dr.y);
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
    let s = sample_parts(q);
    return (s[0] + s[1]) * medium_tr(q) + s[2];
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

// The output grid's colour history follows the traced grid's diffuse history in hist_in / hist_out (sized for both).
fn out_hist(i: u32) -> u32 { return L.size.x * L.size.y + i; }

// Previous-frame pixel position of world point p (x < 0: behind the previous camera).
fn prev_pixel(p: vec3<f32>) -> vec3<f32> {
    let d = p - L.prev.pos.xyz;
    let z = dot(d, L.prev.fwd.xyz);
    if (z <= 1e-4) { return vec3<f32>(-1.0, -1.0, 0.0); }
    let px = dot(d, L.prev.right.xyz) / (z * L.prev.pos.w * L.prev.fwd.w);
    let py = dot(d, L.prev.up.xyz) / (z * L.prev.pos.w);
    return vec3<f32>((px * 0.5 + 0.5) * f32(L.size.x) - 0.5, (0.5 - py * 0.5) * f32(L.size.y) - 0.5, z);
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
    let g = gb_unpack(gb_in[u32(ty) * L.isz.z + u32(tx)]);
    if (t > 0.0) { return g.depth > 0.0 && abs(g.depth - z) < 0.04 * z + 0.002 && dot(g.n, n) > ncos; }
    return g.depth == 0.0;
}

// History length (frames) from which `dtemporal` reprojects the pixel's centre instead of its jittered sample.
const HIST_CENTRE: f32 = 16.0;
// The most frames the irradiance history holds where the pixel moves a pixel a frame or more (it is resampled every
// frame, each time a little blurrier), and the relative difference of its fast mean from its slow one, beyond their
// noise, at which anti-lag leaves it two frames (`dtemporal`).
const HIST_MOVING: f32 = 48.0;
const ANTILAG: f32 = 0.15;
// Diffuse history length from which the firefly cut stops (dtemporal).
const FIREFLY_FRAMES: f32 = 16.0;

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
    // A compact copy of the sample's surface for the denoiser's and the upsample's edge stops (`sb` is free until
    // `spec0`; the 80-byte samples cost twice the bandwidth): normal, first-hit distance, negative on animated emitters.
    sb[q] = vec4<f32>(n, select(t, -t, anim && t > 0.0));
    let sp = sample_pos(i32(gid.x), i32(gid.y));
    let dir = ray_dir(L.cur, sp.x, sp.y);
    let p = select(L.cur.pos.xyz + dir * 1.0e6, L.cur.pos.xyz + dir * t, t > 0.0);
    var irr = sample_parts(q)[0] / max(px.albedo.rgb, vec3<f32>(0.02));
    var l = lum(irr);
    var hd = vec4<f32>(0.0);
    var hm = vec4<f32>(0.0);
    var around = vec2<f32>(-1.0);
    var speed = 0.0; // output pixels this frame
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
            let centred = hist_in[u32(jc.y * ps.x + jc.x)].w >= HIST_CENTRE;
            if (centred) { tc = tcc; }
            let x0 = i32(floor(tc.x));
            let y0 = i32(floor(tc.y));
            let f = tc - floor(tc);
            var wsum = 0.0;
            for (var k = 0; k < 4; k++) {
                let tx = x0 + (k & 1);
                let ty = y0 + (k >> 1u);
                if (tx < 0 || ty < 0 || tx >= ps.x || ty >= ps.y || !same_surface_n(tx, ty, pp.z, n, t, select(0.8, 0.5, centred))) { continue; }
                let bw = select(1.0 - f.x, f.x, (k & 1) == 1) * select(1.0 - f.y, f.y, (k >> 1u) == 1);
                let j = u32(ty * ps.x + tx);
                hd += hist_in[j] * bw;
                hm += mom_in[j] * bw;
                wsum += bw;
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
            speed = length(sp - 0.5 - pp.xy);
            // Slow / fast means over the 3x3 around it on the same surface (anti-lag compares them).
            if (t > 0.0 && L.mode.z != 0u) {
                var acc = vec2<f32>(0.0);
                var na = 0.0;
                for (var k = 0; k < 9; k++) {
                    let tx = i32(round(tc.x)) + k % 3 - 1;
                    let ty = i32(round(tc.y)) + k / 3 - 1;
                    if (tx < 0 || ty < 0 || tx >= ps.x || ty >= ps.y || !same_surface(tx, ty, pp.z, n, t)) { continue; }
                    let m = mom_in[u32(ty * ps.x + tx)];
                    if (m.w <= 0.0 || non_finite(m.xyz)) { continue; }
                    acc += vec2<f32>(m.x, m.z);
                    na += 1.0;
                }
                if (na > 0.0) { around = acc / na; }
            }
        }
    }
    // How long the history may be at most is the frame's (`misc.y`: live.rs HISTORY_LIVE while the scene's lighting
    // changes, far longer once it has settled); within it, each pixel's own: less where it moves (until 2026-11-12 the
    // whole frame's was held to 48 while the camera moved), a few frames on animated shading, and anti-lag below.
    let cap = mix(select(L.misc.y, min(L.misc.y, 4.0), anim), min(L.misc.y, select(HIST_MOVING, 4.0, anim)), clamp(speed, 0.0, 1.0));
    var m1 = hm.x;
    var m2 = hm.y;
    let sd = sqrt(max(m2 - m1 * m1, 0.0));
    if (hd.w >= 4.0 && hd.w < FIREFLY_FRAMES) {
        // Firefly, while the history is young (after a disocclusion or a lighting change): a sample moves the pixel by
        // (l - mean) / frames; cut only what would show as a sparkle (more than half the mean above it). Not from
        // FIREFLY_FRAMES on: histories are capped (48 frames moving, 32 while the scene's lighting changes), so a cut tied to the frame count
        // kept clipping the rare bright samples of small strong lights for good. Bistro crowd at 10 s, held: diffuse
        // 0.70 of our offline render with it, 0.82 without; moving ΔE 15.3 -> 12.9.
        let hi = m1 + 4.0 * sd + 0.5 * m1 * hd.w + 1e-4;
        if (l > hi) { irr *= hi / l; l = hi; }
    }
    let fast = select(l, mix(hm.z, l, 0.25), hd.w > 0.0);
    var count = min(hd.w + 1.0, cap);
    // Anti-lag (as ReBLUR's): around this point the fast average left the slow one by more than its
    // (neighbourhood-averaged) noise allows: the lighting changed here (a sign switched off, a door opened, a wall
    // gone, a spark passing). The history is cut by how far, down to two frames at ANTILAG of the mean: a slow change
    // shortens it a little every frame until it follows, a sudden one restarts it. While the scene's lighting changes
    // (`mode.z`): once it has settled there is nothing to find, and what the test then finds is noise (a still Bistro
    // read dE 2.43 against offline with it always on, 2.23 without).
    // (The specular has none: at one sample a pixel its fast mean is noise, and cutting by it cost more than the lag:
    // the Bistro slice at 5 s read dE 2.98 with it for 2.82.)
    if (L.mode.z != 0u && hd.w >= 6.0 && around.x >= 0.0) {
        let over = abs(around.y - around.x) - 3.0 * 0.38 * sd / 3.0 - 1e-4;
        let keep = 1.0 - clamp(over / (ANTILAG * max(max(around.x, around.y), 1e-6)), 0.0, 1.0);
        if (keep < 1.0) {
            count = max(min(count, hd.w * keep + 1.0), 2.0);
            m1 = mix(around.y, m1, keep);
            m2 = m1 * m1 + sd * sd;
        }
    }
    let a = 1.0 / max(count, 1.0);
    let mom = vec4<f32>(mix(m1, l, a), mix(m2, l * l, a), fast, 1.0);
    let d = mix(hd.rgb, irr, a);
    hist_out[q] = vec4<f32>(d, count);
    mom_out[q] = mom;
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
    let count = hist_out[i].w;
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
    if (xflag(1u)) { noisy = count < limits[stage]; }
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
// and of the vendor upscalers after a denoiser. `gb_out` is the traced grid's g-buffer.

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
        let g = gb_unpack(gb_in[j]);
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
        h += shist_in[j] * bw;
        wsum += bw;
    }
    if (wsum <= 1e-3) { return vec4<f32>(0.0); }
    h /= wsum;
    if (non_finite(h.rgb) || non_finite(vec3<f32>(h.w))) { return vec4<f32>(0.0); }
    return h;
}

// Specular temporal accumulation on the traced grid (UE 5.8 Lumen reflections, LumenReflectionDenoiserTemporal.usf): the
// surface history and the reflection-hit one (reprojected through the mirror image of what it shows); the one closer to
// this frame's neighbourhood mean wins. Writes the specular filter's input and the traced grid's g-buffer.
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
    let cur = sample_parts(q)[1];
    // This frame's neighbourhood mean (same surface), in YCoCg.
    var m1 = vec3<f32>(0.0);
    var mn = 0.0;
    for (var k = 0; k < 9; k++) {
        let qx = i32(gid.x) + k % 3 - 1;
        let qy = i32(gid.y) + k / 3 - 1;
        if (qx < 0 || qy < 0 || qx >= i32(iw) || qy >= i32(ih)) { continue; }
        let j = u32(qy) * iw + u32(qx);
        let s = sample_px[j];
        var ok = false;
        if (t > 0.0) {
            ok = s.color.w > 0.0 && abs(s.color.w - t) < 0.05 * t + 0.01 && dot(normalize(s.normal.xyz + vec3<f32>(1e-6)), n) > 0.8;
        } else {
            ok = s.color.w == 0.0;
        }
        if (!ok) { continue; }
        m1 += ycocg(sample_parts(j)[1]);
        mn += 1.0;
    }
    let smean = m1 / max(mn, 1.0);
    // Reflection hit distance for the mirror-image reprojection: on near mirrors the resolved one (pass `refl`).
    let st = select(px.spec.w, refl_res[q].w, refl_res[q].w > 0.0 && rough < NEAR_MIRROR);
    let mv = motion_in[q];
    var hs = vec4<f32>(0.0);
    var hv = vec4<f32>(0.0);
    if (L.size.w == 0u && L.isz.z > 0u) {
        let pp = prev_pixel(p + select(vec3<f32>(0.0), mv.xyz, mv.w > 0.5 && t > 0.0));
        if (pp.z > 0.0) { hs = spec_history(pp, n, t, rough, false); }
        // (Not on moving objects: the camera-only reprojection ignores a moving reflector's own motion.)
        if (t > 0.0 && st > 0.0 && mv.w <= 0.5) {
            let pv = prev_pixel(L.cur.pos.xyz + dir * (t + min(st, 1.0e3)));
            if (pv.z > 0.0) { hv = spec_history(pv, n, t, rough, true); }
        }
    }
    let cap = select(L.misc.y, min(L.misc.y, 4.0), px.albedo.w > 1.5);
    let moving = any(L.cur.pos.xyz != L.prev.pos.xyz) || any(L.cur.fwd.xyz != L.prev.fwd.xyz);
    let translated = L.mode.y < 8u && !xflag(16384u);
    let ys = ycocg(hs.rgb);
    let yv = ycocg(hv.rgb);
    var surf = select(0.0, 0.8, length(smean - yv) > length(smean - ys) + 0.01 / exposure_scale());
    surf = select(surf, 1.0, hv.w <= 0.0);
    surf = select(surf, 0.0, hs.w <= 0.0 && hv.w > 0.0);
    let hy = mix(yv, ys, surf);
    var max_s = select(cap, mix(2.0, 12.0, clamp(rough / 0.05, 0.0, 1.0)), moving);
    if (translated) { max_s = min(max_s, 4.0); }
    let frames_s = select(hv.w, hs.w, surf > 0.5);
    var count_s = 1.0;
    var s_out = cur;
    if (hs.w > 0.0 || hv.w > 0.0) {
        count_s = min(min(frames_s + 1.0, max_s), cap);
        s_out = mix(max(ycocg_rgb(hy), vec3<f32>(0.0)), cur, clamp(1.0 / max(count_s, 1e-4), 0.0, 1.0));
    }
    s_out = select(max(s_out, vec3<f32>(0.0)), vec3<f32>(0.0), non_finite(s_out));
    shist_out[q] = vec4<f32>(s_out, count_s);
    spec_store(0u, q, vec4<f32>(s_out, count_s));
    gb_out[q] = gb_pack(n, depth, rough);
}

// The specular filter (`spec_filter`'s kernel) on the traced grid: three passes over `spec_pp`, slot 0 -> 1 -> 0 -> 1.
fn spec_filter_traced(gid: vec3<u32>, step: i32, from_a: bool) {
    let w = L.isz.x;
    let h = L.isz.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let i = gid.y * w + gid.x;
    let src = select(1u, 0u, from_a);
    let c = spec_load(src, i);
    let g = gb_unpack(gb_out[i]);
    let rough = g.rough;
    let fresh = c.w < 4.0;
    let reach = select(4.0 * clamp(rough * 8.0, 0.0, 1.0), 4.0, xflag(2u) || fresh);
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
                let gq = gb_unpack(gb_out[q]);
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
// direct light) into fb (w: first-hit distance), and its motion into `xhist_out`: screen velocity (output pixels this
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
    let c = (fa[q].rgb * max(px.albedo.rgb, vec3<f32>(0.02)) + spec_load(1u, q).rgb) * medium_tr(q) + sample_parts(q)[2] + direct_in[q].rgb;
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
    let camera_moves = any(L.cur.pos != L.prev.pos) || any(L.cur.fwd != L.prev.fwd) || any(L.cur.right != L.prev.right) || any(L.cur.up != L.prev.up);
    if (L.size.w == 0u && (camera_moves || object_moves)) {
        let pv = prev_pixel(p + select(vec3<f32>(0.0), mv.xyz, object_moves));
        if (pv.z > 0.0) { vel = clamp(sp - 0.5 - pv.xy, vec2<f32>(-2048.0), vec2<f32>(2048.0)); }
    }
    fb[q] = vec4<f32>(finite_or_zero(max(c, vec3<f32>(0.0))), t);
    let reactive = max(select(0.0, 1.0, px.albedo.w > 1.5), clamp(medium_share(q) / UP_MEDIUM, 0.0, 1.0));
    xhist_out[q] = vec4<f32>(vel, depth, reactive);
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
// How far outside the range of the history's pixels around it a point sample may lie, in that range's widths (`judge`).
const UP_POINT: f32 = 0.5;
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
// jitter does catch keep their history.

// `judge`: per traced sample, into `ae_out` (f16): that distance (linear RGB), the sample's luminance (< 0: no
// history there), the motion its output pixels reproject by: the surface nearest the camera among the nine samples
// around (the sky the farthest), since an edge moves with what is in front of it; and whether its history is of
// something that is gone.
struct Judged { d: vec3<f32>, lum: f32, vel: vec2<f32>, gone: bool };
fn judged_at(q: u32) -> Judged {
    let v = ae_out[q];
    let b = unpack2x16float(v.y);
    return Judged(vec3<f32>(unpack2x16float(v.x), b.x), b.y, unpack2x16float(v.z), v.w != 0u);
}

@compute @workgroup_size(8, 8, 1)
fn judge(@builtin(global_invocation_id) gid: vec3<u32>) {
    let iw = i32(L.isz.x);
    let ih = i32(L.isz.y);
    if (i32(gid.x) >= iw || i32(gid.y) >= ih) { return; }
    let q = gid.y * L.isz.x + gid.x;
    var near = 1.0e30;
    var vel = vec2<f32>(0.0);
    for (var k = 0; k < 9; k++) {
        let qx = i32(gid.x) + k % 3 - 1;
        let qy = i32(gid.y) + k / 3 - 1;
        if (qx < 0 || qy < 0 || qx >= iw || qy >= ih) { continue; }
        let mv = xhist_out[u32(qy * iw + qx)];
        let z = select(1.0e29, mv.z, mv.z > 0.0);
        if (z < near) {
            near = z;
            vel = mv.xy;
        }
    }
    var d = vec3<f32>(0.0);
    var l = -1.0;
    var gone = false;
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
            let hv = hist_in[out_hist(u32(y) * L.size.x + u32(x))];
            if (hv.w <= 0.0 || non_finite(hv.rgb)) { continue; }
            lo = min(lo, hv.rgb);
            hi = max(hi, hv.rgb);
            hsum += hv.rgb;
            hn += 1.0;
        }
        if (hn > 0.0) {
            // The history's pixels are means over a pixel and the sample is a point: of a feature finer than a pixel
            // it can lie outside their range by as much as they differ among themselves (UP_POINT of it allowed).
            let slack = UP_POINT * (hi - lo);
            let c = fb[q].rgb;
            d = clamp(c - clamp(c, lo - slack, hi + slack), vec3<f32>(-6.0e4), vec3<f32>(6.0e4));
            l = min(lum(c), 6.0e4);
            // Gone from in front: the previous frame's nearest depth around where this point was (its traced grid's
            // g-buffer) against this frame's nearest, taken to the previous view by this sample's own depths.
            let ps = vec2<i32>(L.isz.zw);
            if (ps.x > 0) {
                var zr = 1.0;
                let t = fb[q].w;
                let z = xhist_out[q].z;
                if (t > 0.0 && z > 0.0) {
                    let sp = sample_pos(i32(gid.x), i32(gid.y));
                    let mv = motion_in[q];
                    let pv = prev_pixel(L.cur.pos.xyz + ray_dir(L.cur, sp.x, sp.y) * t + select(vec3<f32>(0.0), mv.xyz, mv.w > 0.5));
                    if (pv.z > 0.0) { zr = pv.z / z; }
                }
                let pc = vec2<i32>(floor(was * vec2<f32>(ps) / vec2<f32>(f32(L.size.x), f32(L.size.y))));
                var was_near = 1.0e30;
                for (var k = 0; k < 9; k++) {
                    let x = pc.x + k % 3 - 1;
                    let y = pc.y + k / 3 - 1;
                    if (x < 0 || y < 0 || x >= ps.x || y >= ps.y) { continue; }
                    let zq = gb_unpack(gb_in[u32(y * ps.x + x)]).depth;
                    was_near = min(was_near, select(1.0e29, zq, zq > 0.0));
                }
                let now_near = select(near * zr, 1.0e29, near >= 1.0e29);
                if (was_near < 1.0e29 && now_near > was_near * (1.0 + UP_GONE) + 0.01) {
                    var clo = vec3<f32>(1.0e30);
                    var chi = vec3<f32>(-1.0e30);
                    for (var k = 0; k < 9; k++) {
                        let qx = i32(gid.x) + k % 3 - 1;
                        let qy = i32(gid.y) + k / 3 - 1;
                        if (qx < 0 || qy < 0 || qx >= iw || qy >= ih) { continue; }
                        let cq = fb[u32(qy * iw + qx)].rgb;
                        clo = min(clo, cq);
                        chi = max(chi, cq);
                    }
                    let hm = hsum / hn;
                    gone = lum(abs(hm - clamp(hm, clo, chi))) > UP_STALE * lum(hm) + 0.01 / exposure_scale();
                }
            }
        }
    }
    ae_out[q] = vec4<u32>(pack2x16float(d.xy), pack2x16float(vec2<f32>(d.z, l)), pack2x16float(vel), u32(gone));
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
    // (Its history is of something that is gone from in front of this: all of it goes.)
    if (judged_at(q).gone) {
        delta = vec3<f32>(0.0);
        drop = 1.0;
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
            let v = hist_in[out_hist(qy * L.size.x + qx)];
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
    }
    let cur = select(fb[u32(bc.y * iw + bc.x)].rgb, sum / wsum, wsum > 1e-9);
    let conf = min(wsum, 1.0);
    // `reject`'s verdict, bilinear over the four traced pixels around; the motion `judge` chose at the nearest.
    let f0 = vec2<i32>(floor(rc));
    let ft = rc - floor(rc);
    var verdict = vec4<f32>(0.0);
    var vsum = 0.0;
    for (var k = 0; k < 4; k++) {
        let qx = f0.x + (k & 1);
        let qy = f0.y + (k >> 1u);
        if (qx < 0 || qy < 0 || qx >= iw || qy >= ih) { continue; }
        let bw = select(1.0 - ft.x, ft.x, (k & 1) == 1) * select(1.0 - ft.y, ft.y, (k >> 1u) == 1);
        verdict += spec_load(0u, u32(qy * iw + qx)) * bw;
        vsum += bw;
    }
    verdict = select(vec4<f32>(0.0, 0.0, 0.0, 1.0), verdict / vsum, vsum > 1e-4);
    let vel = judged_at(u32(bc.y * iw + bc.x)).vel;
    var hist = vec4<f32>(0.0);
    if (L.size.w == 0u) {
        let hp = vec2<f32>(f32(gid.x), f32(gid.y)) - vel;
        if (hp.x > -0.5 && hp.y > -0.5 && hp.x < f32(w) - 0.5 && hp.y < f32(h) - 0.5) {
            if (vel.x == 0.0 && vel.y == 0.0) { hist = hist_in[out_hist(i)]; } else { hist = up_history(hp.x, hp.y); }
        }
        if (non_finite(hist.rgb) || non_finite(vec3<f32>(hist.w))) { hist = vec4<f32>(0.0); }
    }
    var outc = cur;
    var count = conf;
    // In motion the history is resampled every frame, each time a little blurrier, and what it holds of another
    // surface (the band behind a moving edge reprojects by the edge's motion) nothing else removes: so, as the pixel's
    // speed nears UP_SPEED, it holds fewer samples (UE TSR's r.TSR.Velocity.WeightClampingSampleCount / PixelSpeed) and
    // is clamped into the range of this frame's nine samples (every temporal AA's rule; FSR 2's rectification). Holding
    // still it keeps both its length and the detail finer than a traced pixel, which the clamp would cut. The orbit of
    // Thames at 9 pixels a frame against offline (detail / trail px): neither 0.48 / 49, the clamp 0.45 / 36, 4 samples
    // 0.57 / 37, both 0.53 / 31.
    let moving = clamp(length(vel) / UP_SPEED, 0.0, 1.0);
    let kept = hist.w * (1.0 - verdict.w);
    if (kept > 0.0) {
        var hc = max(hist.rgb + verdict.xyz, vec3<f32>(0.0));
        hc = mix(hc, clamp(hc, xlo, xhi), moving);
        let most = min(mix(UP_SAMPLES, UP_MOVING, moving), mix(UP_SAMPLES, UP_REACTIVE, clamp(xhist_out[u32(bc.y * iw + bc.x)].w, 0.0, 1.0)));
        count = min(kept + conf, most);
        let a = clamp(conf / max(count, 1e-4), 0.0, 1.0);
        outc = mix(hc, cur, a);
    }
    let lin = finite_or_zero(max(outc, vec3<f32>(0.0)));
    hist_out[out_hist(i)] = vec4<f32>(lin, count);
    // The image: into fa, or into sb for depth of field / motion blur to read (w: the CoC of the nearest sample's depth).
    let img = vec4<f32>(lin, dof_coc(xhist_out[u32(bc.y * iw + bc.x)].z));
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

// Quarter-resolution log2 luminance (UE eye adaptation: uniform RGB weights) and the UE histogram (64 buckets over
// [log min, log max], each sample split between its two buckets).
@compute @workgroup_size(8, 8, 1)
fn lum_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= qw() || gid.y >= qh()) { return; }
    var acc = vec3<f32>(0.0);
    var n = 0.0;
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let sx = gid.x * 4u + x;
            let sy = gid.y * 4u + y;
            if (sx < L.size.x && sy < L.size.y) {
                acc += finite_or_zero(fa[sy * L.size.x + sx].rgb);
                n += 1.0;
            }
        }
    }
    let c = acc / max(n, 1.0);
    let ll = log2(max((c.r + c.g + c.b) / 3.0, 1e-6));
    lumq[gid.y * qw() + gid.x] = ll;
    if (L.ae2.w > 0.5) {
        let fb = clamp((ll - L.ae.z) / (L.ae.w - L.ae.z), 0.0, 1.0) * 63.0;
        let b0 = min(u32(floor(fb)), 63u);
        let w1 = fb - f32(b0);
        atomicAdd(&ae_bins[b0], u32((1.0 - w1) * 256.0 + 0.5));
        atomicAdd(&ae_bins[min(b0 + 1u, 63u)], u32(w1 * 256.0 + 0.5));
    }
}

// UE ComputeAverageLuminanceWithoutOutlier + EyeAdaptationCommon: average log2 luminance between the low / high
// percentiles, target exposure = 2^bias * 0.18 / clamp(average, min, max), adapted at the up / down speed.
@compute @workgroup_size(64, 1, 1)
fn exposure(@builtin(local_invocation_id) lid: vec3<u32>) {
    if (lid.x != 0u) { return; }
    var h: array<f32, 64>;
    var total = 0.0;
    var mean_sum = 0.0;
    for (var i = 0u; i < 64u; i++) {
        h[i] = f32(atomicLoad(&ae_bins[i])) / 256.0;
        atomicStore(&ae_bins[i], 0u);
        total += h[i];
        mean_sum += h[i] * (L.ae.z + f32(i) / 63.0 * (L.ae.w - L.ae.z));
    }
    if (L.ae2.w < 0.5 || total <= 0.0) { return; }
    var min_f = min(L.ae.x, L.ae.y) * total;
    var max_f = L.ae.y * total;
    var sum = 0.0;
    var cnt = 0.0;
    for (var i = 0u; i < 64u; i++) {
        var v = h[i];
        let sub = min(v, min_f);
        v -= sub;
        min_f -= sub;
        max_f -= sub;
        v = min(v, max_f);
        max_f -= v;
        sum += v * (L.ae.z + f32(i) / 63.0 * (L.ae.w - L.ae.z));
        cnt += v;
    }
    let avg = select(L.ae2.y, exp2(sum / cnt), cnt > 0.0);
    let target_ = L.ae2.x * 0.18 / clamp(avg, L.ae2.y, L.ae2.z);
    var e = target_;
    let prev = ae_state[0];
    if (prev.w > 0.5 && L.ae3.w < 0.5) {
        let d = log2(target_) - log2(prev.x);
        let speed = select(L.ae3.y, L.ae3.x, d > 0.0);
        let step = speed * L.ae3.z;
        e = exp2(log2(prev.x) + clamp(d, -step, step));
    }
    ae_state[0] = vec4<f32>(e, target_, mean_sum / total, 1.0);
}

// Edge-aware blur of the quarter-res log luminance (stand-in for UE's bilateral grid): range weight on log2 lum.
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

// UE local exposure (CalculateLogLocalExposure, zero thresholds) for exposed log2 luminance `l`.
fn local_exposure(l: f32, x: f32, y: f32, e: f32) -> f32 {
    if (L.lx.x == 1.0 && L.lx.y == 1.0 && L.lx.w == 1.0) { return 1.0; }
    let qx = clamp(u32(x / 4.0), 0u, qw() - 1u);
    let qy = clamp(u32(y / 4.0), 0u, qh() - 1u);
    let bil = lumb[qy * qw() + qx] + log2(e);
    let wide = select(bil, ae_state[0].z + log2(e), L.ae2.w > 0.5);
    let base = mix(bil, wide, L.lx.z);
    let mid = log2(0.18 * L.ae2.x);
    let contrast = select(L.lx.y, L.lx.x, base > mid);
    return exp2((base - mid) * (contrast - 1.0) + (l - base) * (L.lx.w - 1.0));
}

// ------------------------------------------------------------------------------------------------ bloom

// Pyramid level j (0..4) is 1/4 .. 1/64 resolution; bloom stage k (0..5) blurs level max(k - 1, 0) (UE stage k runs at
// 1/2^(k+1); stage 0 at 1/4 here). bloom_a holds the five levels, then the six stage sums.
fn bw() -> u32 { return (L.size.x + 3u) / 4u; }
fn bh() -> u32 { return (L.size.y + 3u) / 4u; }
fn lvl_dim(j: u32) -> vec2<u32> { return vec2<u32>((bw() + (1u << j) - 1u) >> j, (bh() + (1u << j) - 1u) >> j); }
fn lvl_off(j: u32) -> u32 {
    var o = 0u;
    for (var i = 0u; i < j; i++) { let d = lvl_dim(i); o += d.x * d.y; }
    return o;
}
fn stage_lvl(k: u32) -> u32 { return max(k, 1u) - 1u; }
fn stage_off(k: u32) -> u32 {
    var o = lvl_off(5u);
    for (var i = 0u; i < k; i++) { let d = lvl_dim(stage_lvl(i)); o += d.x * d.y; }
    return o;
}
fn stage_radius(k: u32) -> f32 { return select(L.br2[(k - 4u) & 1u], L.br[k & 3u], k < 4u); }
fn stage_tint(k: u32) -> f32 { return select(L.bt2[(k - 4u) & 1u], L.bt[k & 3u], k < 4u); }

@compute @workgroup_size(8, 8, 1)
fn bloom_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= bw() || gid.y >= bh()) { return; }
    var acc = vec3<f32>(0.0);
    var n = 0.0;
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let sx = gid.x * 4u + x;
            let sy = gid.y * 4u + y;
            if (sx < L.size.x && sy < L.size.y) {
                acc += finite_or_zero(fa[sy * L.size.x + sx].rgb);
                n += 1.0;
            }
        }
    }
    var c = acc / max(n, 1.0) * exposure_scale();
    if (L.post.z > 0.0) {
        // UE bloom threshold: Color * saturate((Luminance - Threshold) * 0.5), Luminance = dot(c, (0.3, 0.59, 0.11))
        c *= clamp((dot(c, vec3<f32>(0.3, 0.59, 0.11)) - L.post.z) * 0.5, 0.0, 1.0);
    }
    bloom_a[gid.y * bw() + gid.x] = vec4<f32>(c, 1.0);
}

// Level j from level j - 1 (2x2 box, edge clamped).
fn bloom_downsample(gid: vec3<u32>, j: u32) {
    let d = lvl_dim(j);
    if (gid.x >= d.x || gid.y >= d.y) { return; }
    let s = lvl_dim(j - 1u);
    let so = lvl_off(j - 1u);
    var acc = vec3<f32>(0.0);
    for (var k = 0u; k < 4u; k++) {
        let x = min(gid.x * 2u + (k & 1u), s.x - 1u);
        let y = min(gid.y * 2u + (k >> 1u), s.y - 1u);
        acc += bloom_a[so + y * s.x + x].rgb;
    }
    bloom_a[lvl_off(j) + gid.y * d.x + gid.x] = vec4<f32>(acc * 0.25, 1.0);
}

// UE Compute1DGaussianFilterKernel: weights exp(-16.7 (x/R)^2), R clamped to 31 texels.
fn bloom_blur(gid: vec3<u32>, k: u32, horizontal: bool) {
    let j = stage_lvl(k);
    let d = lvl_dim(j);
    if (gid.x >= d.x || gid.y >= d.y) { return; }
    let rf = clamp(stage_radius(k), 1e-4, 31.0);
    let r = i32(ceil(rf));
    let src = select(0u, lvl_off(j), horizontal);
    var acc = vec3<f32>(0.0);
    var ws = 0.0;
    for (var t = -r; t <= r; t++) {
        let x = select(i32(gid.x), clamp(i32(gid.x) + t, 0, i32(d.x) - 1), horizontal);
        let y = select(clamp(i32(gid.y) + t, 0, i32(d.y) - 1), i32(gid.y), horizontal);
        let wt = exp(-16.7 * (f32(t) / rf) * (f32(t) / rf));
        let q = u32(y) * d.x + u32(x);
        acc += select(bloom_b[q].rgb, bloom_a[src + q].rgb, horizontal) * wt;
        ws += wt;
    }
    let i = gid.y * d.x + gid.x;
    if (horizontal) {
        bloom_b[i] = vec4<f32>(acc / ws, 1.0);
        return;
    }
    // Stage sum: this stage's blur * tint + the next (wider, coarser) stage's sum, bilinearly upsampled.
    var c = acc / ws * stage_tint(k);
    if (k < 5u) {
        let dn = lvl_dim(stage_lvl(k + 1u));
        let f = vec2<f32>(dn) / vec2<f32>(d);
        let fx = clamp((f32(gid.x) + 0.5) * f.x - 0.5, 0.0, f32(dn.x - 1u));
        let fy = clamp((f32(gid.y) + 0.5) * f.y - 0.5, 0.0, f32(dn.y - 1u));
        c += sample_level(stage_off(k + 1u), dn, fx, fy);
    }
    bloom_a[stage_off(k) + i] = vec4<f32>(c, 1.0);
}

fn sample_level(off: u32, d: vec2<u32>, fx: f32, fy: f32) -> vec3<f32> {
    let x0 = u32(floor(fx));
    let y0 = u32(floor(fy));
    let x1 = min(x0 + 1u, d.x - 1u);
    let y1 = min(y0 + 1u, d.y - 1u);
    let ax = fx - f32(x0);
    let ay = fy - f32(y0);
    let a = mix(bloom_a[off + y0 * d.x + x0].rgb, bloom_a[off + y0 * d.x + x1].rgb, ax);
    let b = mix(bloom_a[off + y1 * d.x + x0].rgb, bloom_a[off + y1 * d.x + x1].rgb, ax);
    return mix(a, b, ay);
}

@compute @workgroup_size(8, 8, 1) fn bloom_dn1(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_downsample(gid, 1u); }
@compute @workgroup_size(8, 8, 1) fn bloom_dn2(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_downsample(gid, 2u); }
@compute @workgroup_size(8, 8, 1) fn bloom_dn3(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_downsample(gid, 3u); }
@compute @workgroup_size(8, 8, 1) fn bloom_dn4(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_downsample(gid, 4u); }
@compute @workgroup_size(8, 8, 1) fn bloom_h5(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 5u, true); }
@compute @workgroup_size(8, 8, 1) fn bloom_v5(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 5u, false); }
@compute @workgroup_size(8, 8, 1) fn bloom_h4(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 4u, true); }
@compute @workgroup_size(8, 8, 1) fn bloom_v4(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 4u, false); }
@compute @workgroup_size(8, 8, 1) fn bloom_h3(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 3u, true); }
@compute @workgroup_size(8, 8, 1) fn bloom_v3(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 3u, false); }
@compute @workgroup_size(8, 8, 1) fn bloom_h2(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 2u, true); }
@compute @workgroup_size(8, 8, 1) fn bloom_v2(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 2u, false); }
@compute @workgroup_size(8, 8, 1) fn bloom_h1(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 1u, true); }
@compute @workgroup_size(8, 8, 1) fn bloom_v1(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 1u, false); }
@compute @workgroup_size(8, 8, 1) fn bloom_h0(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 0u, true); }
@compute @workgroup_size(8, 8, 1) fn bloom_v0(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_blur(gid, 0u, false); }

fn bloom_at(x: f32, y: f32) -> vec3<f32> {
    let d = lvl_dim(0u);
    return sample_level(stage_off(0u), d, clamp(x / 4.0 - 0.5, 0.0, f32(d.x - 1u)), clamp(y / 4.0 - 0.5, 0.0, f32(d.y - 1u)));
}

// ------------------------------------------------------------------------------------------------ depth of field

// Signed circle-of-confusion radius (output px) at view depth z (0 = sky), negative in front of the focus plane: UE's
// physical model (DiaphragmDOFUtils.cpp, FPhysicalCocModel): (z - focus) / z x the CoC of infinity, clamped to
// r.DOF.Kernel.MaxForeground / MaxBackgroundRadius (2.5 % of the width).
fn dof_coc(z: f32) -> f32 {
    let zz = select(1.0e9, z, z > 0.0);
    return clamp((zz - L.dof.y) / zz * L.dof.x, -L.dof.z, L.dof.z);
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
    return xhist_out[sample_under(q % L.size.x, q / L.size.x)];
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

// ------------------------------------------------------------------------------------------------ post

// UE LinToLog (PostProcessCombineLUTs): LUT input encoding for the 32^3 tonemap LUT.
fn lin_to_log(c: vec3<f32>) -> vec3<f32> {
    return clamp(log2(c) / 14.0 - log2(0.18) / 14.0 + 444.0 / 1023.0, vec3<f32>(0.0), vec3<f32>(1.0));
}

@compute @workgroup_size(8, 8, 1)
fn post(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = L.size.x;
    let h = L.size.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let i = gid.y * w + gid.x;
    let x = f32(gid.x) + 0.5;
    let y = f32(gid.y) + 0.5;
    let mode = L.mode.x;
    // Unreal-style view modes. Base color / normal / history bypass exposure + tonemapping.
    if (mode == 1u || mode == 2u || mode == 5u) {
        var d = vec3<f32>(0.0);
        if (mode == 1u) {
            let a = clamp(sample_px[sample_under(gid.x, gid.y)].albedo.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
            d = select(pow((a + 0.055) / 1.055, vec3<f32>(1.0 / 2.4)) * 1.055 - 0.055, a * 12.92, a <= vec3<f32>(0.0031308));
        } else if (mode == 2u) {
            let g = gb_unpack(gb_out[sample_under(gid.x, gid.y)]);
            d = select(g.n * 0.5 + 0.5, vec3<f32>(0.0), g.depth == 0.0);
        } else {
            let t = clamp(log2(max(hist_out[sample_under(gid.x, gid.y)].w, 1.0)) / 12.0, 0.0, 1.0); // 1 .. 4096 frames (diffuse)
            d = clamp(vec3<f32>(1.5 - abs(4.0 * t - 3.0), 1.5 - abs(4.0 * t - 2.0), 1.5 - abs(4.0 * t - 1.0)), vec3<f32>(0.0), vec3<f32>(1.0));
        }
        let q = vec3<u32>(clamp(d, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + 0.5);
        out_px[i] = q.z | (q.y << 8u) | (q.x << 16u) | (255u << 24u);
        return;
    }
    var src = fa[i].rgb;
    // The traced sample under the pixel: its accumulations without the spatial filters (3), the raw sample (4), and the
    // denoised signals apart: diffuse (6: the composed colour less the rest), specular (7), deterministic direct light (8).
    // (6 does not hold with depth of field and motion blur both on: the composed colour's buffer is theirs then.)
    let tq = sample_under(gid.x, gid.y);
    if (mode == 3u) { src = (hist_out[tq].rgb * max(sample_px[tq].albedo.rgb, vec3<f32>(0.02)) + shist_out[tq].rgb) * medium_tr(tq) + sample_parts(tq)[2] + direct_in[tq].rgb; }
    if (mode == 4u) { src = sample_color(tq); }
    if (mode == 6u) { src = max(fb[tq].rgb - spec_load(1u, tq).rgb * medium_tr(tq) - sample_parts(tq)[2] - direct_in[tq].rgb, vec3<f32>(0.0)); }
    if (mode == 8u) { src = direct_in[tq].rgb; }
    if (mode == 7u) { src = spec_load(1u, tq).rgb * medium_tr(tq); }
    if (L.post.x > 0.0 && mode == 0u) {
        // Scene fringe: red sampled radially outward, blue inward (nearest texel).
        let k = L.post.x * 0.0025;
        let d = vec2<f32>(x / f32(w) - 0.5, y / f32(h) - 0.5);
        let pr = vec2<i32>((vec2<f32>(0.5) + d * (1.0 + k)) * vec2<f32>(f32(w), f32(h)));
        let pb = vec2<i32>((vec2<f32>(0.5) + d * (1.0 - k)) * vec2<f32>(f32(w), f32(h)));
        let cr = clamp(pr, vec2<i32>(0), vec2<i32>(i32(w) - 1, i32(h) - 1));
        let cb = clamp(pb, vec2<i32>(0), vec2<i32>(i32(w) - 1, i32(h) - 1));
        src = vec3<f32>(fa[u32(cr.y) * w + u32(cr.x)].r, src.g, fa[u32(cb.y) * w + u32(cb.x)].b);
    }
    let e = exposure_scale();
    let ll = log2(max((src.r + src.g + src.b) / 3.0 * e, 1e-9));
    var c = src * e * local_exposure(ll, x, y, e) + bloom_at(x, y) * L.misc.w;
    if (L.misc.z > 0.0) {
        let aspect = f32(h) / f32(w);
        let scale = sqrt(2.0) / sqrt(1.0 + aspect * aspect);
        let pos = vec2<f32>(x / f32(w) * 2.0 - 1.0, 1.0 - y / f32(h) * 2.0) * vec2<f32>(1.0, aspect) * scale * L.misc.z;
        let t2 = dot(pos, pos);
        let v = 1.0 / (t2 + 1.0);
        c *= v * v;
    }
    let log_c = lin_to_log(max(c, vec3<f32>(0.0)) + 0.0026883);
    let uvw = log_c * (31.0 / 32.0) + 0.5 / 32.0;
    var srgb = textureSampleLevel(lut, lut_samp, uvw, 0.0).rgb;
    if (L.post.y > 0.0) {
        // Film grain: mid-tone weighted multiplicative noise, new every frame.
        var hsh = (i ^ (L.size.z * 0x9e3779b9u)) * 747796405u + 2891336453u;
        hsh = ((hsh >> ((hsh >> 28u) + 4u)) ^ hsh) * 277803737u;
        hsh = (hsh >> 22u) ^ hsh;
        let n = f32(hsh >> 8u) / 16777216.0 - 0.5;
        let l = clamp(lum(srgb), 0.0, 1.0);
        srgb *= 1.0 + L.post.y * n * 0.6 * max(1.0 - (2.0 * l - 1.0) * (2.0 * l - 1.0), 0.15);
    }
    let q = vec3<u32>(clamp(srgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + 0.5);
    out_px[i] = q.z | (q.y << 8u) | (q.x << 16u) | (255u << 24u); // BGRA in memory
}

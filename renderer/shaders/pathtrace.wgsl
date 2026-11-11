// ThreeRT native path tracer. One sample per pixel per dispatch, accumulated in `accum`.
// Shading model mirrors UE5 DefaultLit: Lambert diffuse + GGX / Smith-joint-approx / Schlick(F90 = saturate(50 F0.g)),
// with UE's GGX multi-scattering energy conservation and diffuse energy preservation.
// Material / atmosphere extensions (docs/PARITY.md): texture pages with ray-cone LOD, alpha cutout, world mapping,
// wetness / puddles / ripples / grime, procedural facade windows, emissive animation, RIS many-light sampling, height
// fog with volumetric single scattering. Each is compiled out when the scene does not use it (FEAT).

const PI: f32 = 3.14159265358979;
const INF: f32 = 3.0e38;
const TRI_NO_SHADOW: u32 = 1u;
const TRACE_CLOSEST: u32 = 0u;
const TRACE_SHADOW: u32 = 1u;
const TRACE_ANY: u32 = 2u;
// Any hit on any surface, alpha-cutout triangles taken as opaque (live emitter shadow rays: `occluded_emitter`).
const TRACE_ANY_OPAQUE: u32 = 3u;
const TRI_MASKED: u32 = 2u;
const TRI_DYNAMIC: u32 = 4u;
// Highest texture-page mip (texpack::MIPS - 1) and the ray-cone LOD bias (negative = sharper; accumulation antialiases).
const TEX_MAX_LEVEL: f32 = 4.0;
const LOD_BIAS: f32 = 0.5;

// Pixel footprint for texture LOD: the output resolution when tracing below it (upsampled like UE TSR's mip bias).
fn lod_height() -> f32 {
    return f32(select(P.size.y, P.counts2.z, P.counts2.z > 0u));
}
// Local lights: up to RIS_EXACT are evaluated exhaustively, more are resampled from RIS_M candidates (one shadow ray).
const RIS_EXACT: u32 = 8u;
const RIS_M: u32 = 8u;
const LIGHT_GROUP: u32 = 16u; // lib.rs LIGHT_GROUP
// Emitter candidates per camera-ray hit (sample_emitter_ris).
const EMIT_RIS_M: u32 = 6u;
// Volumetric fog runs on 1 / (VOL_RATE_MASK + 1) of the pixel tiles per sample (0 = every pixel).
const VOL_RATE_MASK: u32 = 1u;
const MAT_DOUBLE_SIDED: i32 = 1;
const MAT_EMITTER_CARD: i32 = 2;
// Live (real-time) mode: write this frame's 1-spp sample + first-hit distance instead of accumulating.
override LIVE: bool = false;
// Features the scene uses (gpu::F_*): the rest is compiled out, so plain scenes pay nothing for them.
override FEAT: u32 = 0xffffffffu;
const F_MASKED: u32 = 1u;
const F_FOG: u32 = 64u;
const F_VOLUME: u32 = 128u;
const F_TEX: u32 = 256u;
const F_FUNC: u32 = 512u;
const F_RESTIR: u32 = 1024u;
const F_GLASS: u32 = 2048u;
const F_COAT: u32 = 4096u;
const F_HUGE: u32 = 8192u;
const F_INST: u32 = 16384u;
const F_MEDIA: u32 = 32768u;
const F_PEMIT: u32 = 65536u; // opaque particles: their emitter handling (nominal areas, not light-sampled ones)
const F_HMEDIA: u32 = 131072u; // homogeneous media (water): live attenuation of paths and shadow rays (media_homogeneous)
const F_AERIAL: u32 = 16777216u; // the sky's aerial perspective (aerial_tr) on segments ending on a surface
const F_PROC: u32 = 262144u; // procedural geometry (ProceduralMesh): AABB candidates in every query, `surface_proc`
fn has(f: u32) -> bool { return (FEAT & f) != 0u; }

struct Params {
    cam_pos: vec4<f32>,   // w = tan(fov/2)
    cam_fwd: vec4<f32>,   // w = aspect
    cam_right: vec4<f32>, // w = lens radius
    cam_up: vec4<f32>,    // w = focus distance
    size: vec4<u32>,      // width, height, sample index, seed
    counts: vec4<u32>,    // lights, emitters, max bounces, rr depth
    env: vec4<f32>,       // has env map, env width, env height, sun light index (-1 none)
    misc: vec4<f32>,      // clamp (absolute radiance), emitter total weight, env total weight, _
    sky_color: vec4<f32>, // constant sky radiance when no env map
    counts2: vec4<u32>,   // directional lights (first in `lights`), environment NEE on, output height for texture LOD (0 = traced height), first shadowed local light
    time: vec4<f32>,      // scene time (s), wetness, rain, -
    fog: vec4<f32>,       // density at fog height (1/m), height falloff (1/m), fog height, start distance (m)
    fog_color: vec4<f32>, // ambient inscattering radiance, max opacity
    vol: vec4<f32>,       // volumetric scattering albedo (0 = off), HG anisotropy, extinction scale in range, -
    vol2: vec4<f32>,      // volumetric albedo tint rgb, range (m, 0 = unlimited)
    raster: vec4<f32>,    // live primary visibility from the raster pass (1 = on), this frame's sub-pixel jitter x, y; ray tracing off (1)
    cache: vec4<f32>,     // live radiance cache (1 = on), history cap in samples, history cap in frames, -
    motion: vec4<f32>,    // live motion vectors: 1 = on, -, -, - (first dynamic triangle: group_first[1].x)
    prev_pos: vec4<f32>,  // live: the previous frame's camera (emitter reservoir reprojection), as cam_*
    prev_fwd: vec4<f32>,
    prev_right: vec4<f32>,
    prev_up: vec4<f32>,
    restir: vec4<f32>,    // live emitter reservoirs: 0 off, 1 on without history, 2 on; previous traced width, height; this frame's half (0 / 1)
    group_first: array<vec4<u32>, 2>, // first global triangle of each traversal group ([1].x = the first dynamic one)
    light_lod: vec4<f32>, // live light LOD (lib.rs emitter_lod): reference position, d0 (0 = off)
    emit_power: vec4<f32>, // x: the emitter CDF's total without light LOD (= misc.y when it is off); y: emitter cull radius (0 = off); z: its reach in the view cone
    lod_view: vec4<f32>,   // emitter cull's view cone (lib.rs EmitterCull): forward direction, cos of the half-angle
    ltree: vec4<f32>,      // light tree (ltree.rs): first node in `emitters` (bits), its entry count (bits: the moving emitters' first), P(pick a moving one), candidates at camera hits (0 = off)
    procs: vec4<u32>,      // procedural geometry (F_PROC): its data's first word in env_cdf, object count, first record in `instances`, -
    aerial: vec4<f32>,     // the sky's aerial perspective (F_AERIAL): density multiple, Rayleigh scale, Mie scale, -
};

struct TriAttr { n0: vec4<f32>, n1: vec4<f32>, n2: vec4<f32>, uv: vec4<f32>, t: vec4<f32>, x: vec4<f32> };
struct Node { bmin: vec3<f32>, left_or_first: u32, bmax: vec3<f32>, count: u32 };
// See scene::GpuMaterial for the field layout.
struct Material {
    base_color: vec4<f32>, emissive: vec4<f32>, params: vec4<f32>, tex0: vec4<i32>, tex1: vec4<i32>, uv: vec4<f32>,
    uv2: vec4<f32>, layer: vec4<f32>, mp: array<vec4<f32>, 8>, mt: array<i32, 8>,
};
// See scene::GpuLight (x = volumetric scattering scale, specular scale, rect barn-door cosine, -).
struct Light { pos: vec4<f32>, dir: vec4<f32>, color: vec4<f32>, ext: vec4<f32>, x: vec4<f32> };
// Live: color = diffuse lighting at the first hit (+ first-hit distance), emit = what the camera ray sees directly
// (emission, sky; kept out of the denoiser) + first-hit roughness, spec = specular lighting at the first hit +
// distance to what the first specular bounce hit (reprojection of reflections). Offline: color / albedo / normal sums.
struct Pixel { color: vec4<f32>, albedo: vec4<f32>, normal: vec4<f32>, emit: vec4<f32>, spec: vec4<f32> };

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> tri_verts: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> attrs: array<TriAttr>;
@group(0) @binding(3) var<storage, read> nodes: array<Node>;
@group(0) @binding(4) var<storage, read> materials: array<Material>;
@group(0) @binding(5) var<storage, read> lights: array<Light>;
// emitters[i] = (triangle index bits, cumulative weight (normalized), weight, area) for i < counts.y, then the guide
// table: EMITTER_GUIDE + 1 entry indices (bits), four per vec4 (lib.rs emitter_cdf).
@group(0) @binding(6) var<storage, read> emitters: array<vec4<f32>>;
@group(0) @binding(7) var textures: texture_2d_array<f32>;
@group(0) @binding(8) var samp: sampler;
@group(0) @binding(9) var env_tex: texture_2d<f32>;
// [0, H): marginal row CDF, then H rows of W conditional CDF entries; then the procedural data from P.procs.x (`gdata`).
@group(0) @binding(10) var<storage, read> env_cdf: array<f32>;
@group(0) @binding(11) var<storage, read_write> accum: array<Pixel>;
// GGX energy LUT (E.x, E.y) for 32 roughness x 32 NoV, packed 2 per vec4
@group(0) @binding(12) var<uniform> energy_lut: array<vec4<f32>, 512>;
// Per texture: (u0, v0, du, dv) of its rect in the page, (layer, wrap, width, height). See texpack.rs.
@group(0) @binding(14) var<storage, read> tex_info: array<vec4<f32>>;
// Per object (scene node): pivot (world position), world bounds size (UE ObjectPositionWS / ObjectBounds).
@group(0) @binding(15) var<storage, read> objects: array<vec4<f32>>;
// The dynamic triangles' vertices (kernel layout, from triangle P.group_first[1].x on) as they were the previous frame.
@group(0) @binding(16) var<storage, read> prev_verts: array<vec4<f32>>;
// Model instances (scene::GpuInstance): world matrix rows, normal matrix rows, (first triangle, object, LOD offset bits, -).
struct Instance { m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, n0: vec4<f32>, n1: vec4<f32>, n2: vec4<f32>, info: vec4<u32>, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32> };
@group(0) @binding(17) var<storage, read> instances: array<Instance>;
// Bounded participating media (`VolumeMesh`, gpu.rs MediumGpu; F_MEDIA), P.time.w of them: world -> local affine rows,
// (volume function, shape: 0 box / 1 sphere, -, -), (majorant 1/m, HG anisotropy, -, -), customData, parameters.
struct Medium { w0: vec4<f32>, w1: vec4<f32>, w2: vec4<f32>, a: vec4<u32>, b: vec4<f32>, c: vec4<f32>, p: array<vec4<f32>, 8> };
@group(0) @binding(18) var<storage, read> media: array<Medium>;


fn inst_vec(r: Instance, v: vec3<f32>) -> vec3<f32> { return vec3<f32>(dot(r.m0.xyz, v), dot(r.m1.xyz, v), dot(r.m2.xyz, v)); }
fn inst_normal(r: Instance, n: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(normalize(vec3<f32>(dot(r.n0.xyz, n.xyz), dot(r.n1.xyz, n.xyz), dot(r.n2.xyz, n.xyz))), n.w);
}
// A model-space triangle's shading attributes placed by instance r (scene::instance_attr).
fn inst_attr(r: Instance, a: TriAttr) -> TriAttr {
    var b = a;
    b.n0 = inst_normal(r, a.n0);
    b.n1 = inst_normal(r, a.n1);
    b.n2 = inst_normal(r, a.n2);
    b.t = vec4<f32>(normalize(inst_vec(r, a.t.xyz)), a.t.w);
    b.x = vec4<f32>(a.x.x + bitcast<f32>(r.info.z), bitcast<f32>(r.info.y), a.x.z, a.x.w);
    return b;
}

// ------------------------------------------------------------------------------------------------
// RNG

var<private> rng_state: u32;
// Path vertex index of the current shading point (cheaper procedural detail / fewer candidates past the first hit).
var<private> cur_depth: u32;
// Live "ray tracing off" (P.raster.w, LiveCore::rays): set once the camera's own ray has found its surface. From then
// on `trace` reports a miss, so nothing is shadowed, a bounce sees the sky and a mirror reflects the sky: what a
// rasterizer without shadow maps draws, on any GPU, from the same scene and the same shading. The switch is a uniform:
// no kernel is rebuilt to flip it.
var<private> rays_off: bool;
// The path so far is camera ray + mirror-like bounces (puddles, glass): surface detail is still visible.
var<private> mirror_path: bool;

// Shading points the camera sees, directly or in a mirror-like reflection (wet streets, glass): they get the exact
// light loop and resampled emitters, so reflections are as clean as what they reflect.
fn seen_directly() -> bool {
    return cur_depth == 0u || (cur_depth == 1u && mirror_path);
}
// Specular share of the last BSDF evaluation (per channel), the specular part of the direct light being summed, and
// whether the last BSDF sample took the specular lobe: the live denoiser filters diffuse and specular apart.
var<private> split_frac: vec3<f32>;
// Where the last vertex sampled emitters (its point and normal, 0 = none: fog): a BSDF ray's emitter hit weighs its MIS
// by the light tree's pmf there (emit_mis).
var<private> mis_x: vec3<f32>;
var<private> mis_n: vec3<f32>;
var<private> dl_spec: vec3<f32>;
// At camera hits, the direct light from directional lights (the sun: a tiny disk, so nearly deterministic) and its
// specular part: the live viewport keeps it in a signal of its own (no denoiser; short history while things change).
// Local lights stay in the denoised signals: their soft shadows are noisy, and on Kabukicho moving them here cost
// motion and static error (excess 1.71 / 1.86 / 2.20 -> 1.92 / 1.95 / 2.33, k = 1 SSIM 0.640 -> 0.620).
var<private> dl_det: vec3<f32>;
var<private> dl_det_spec: vec3<f32>;
var<private> sampled_spec: bool;
// The last BSDF sample went through the surface (glass: refraction, a delta lobe).
var<private> sampled_trans: bool;

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn rnd() -> f32 {
    rng_state = pcg(rng_state);
    return f32(rng_state >> 8u) * (1.0 / 16777216.0);
}

// Buckets of the emitter CDF's guide table (lib.rs EMITTER_GUIDE).
const EMITTER_GUIDE: u32 = 65536u;

fn emitter_guide(k: u32) -> u32 {
    return bitcast<u32>(emitters[P.counts.y + k / 4u][k % 4u]);
}

// One emitter CDF entry: the first whose cumulative weight is not below u. The guide table bounds the binary search to
// u's bucket (u * EMITTER_GUIDE is exact for the 24-bit u), which holds the same entry a search over the whole CDF
// finds. Requires counts.y > 0.
fn emitter_pick() -> vec4<f32> {
    return emitter_pick_u(rnd());
}

fn emitter_pick_u(u: f32) -> vec4<f32> {
    let k = min(u32(u * f32(EMITTER_GUIDE)), EMITTER_GUIDE - 1u);
    var lo = emitter_guide(k);
    var hi = emitter_guide(k + 1u);
    while (lo < hi) {
        let mid = (lo + hi) / 2u;
        if (emitters[mid].y < u) { lo = mid + 1u; } else { hi = mid; }
    }
    return emitters[min(lo, P.counts.y - 1u)];
}

// Light tree (ltree.rs): nodes after the CDF in `emitters`, one vec4 each: pack2x16float(centre x, y),
// pack2x16float(centre z, radius), power, child word (the left child, the right one follows it; or LT_LEAF |
// (count - 1) << 29 | its first entry). The tree's emitters come first in the CDF in leaf order, the moving ones after
// them (from entry P.ltree.y).
const LT_LEAF: u32 = 0x80000000u;
const LT_PATH_BITS: u32 = 26u;
// Live: tree nodes wholly farther than this (m) from the shading point are not proposed (never within one Bistro; in
// the 3 x 3 district its far copies: 2.0 -> 0.5 ms).
const LT_FAR: f32 = 120.0;

fn lt_on() -> bool { return P.ltree.w > 0.5; }

fn lt_node(i: u32) -> vec4<f32> { return emitters[bitcast<u32>(P.ltree.x) + i]; }

// Importance of a tree node seen from x on a surface with normal n (n = 0: no hemisphere, a fog point): power x the
// best cosine any point of its sphere can have at x / max(distance², radius²).
fn lt_imp(nd: vec4<f32>, x: vec3<f32>, n: vec3<f32>) -> f32 {
    let cxy = unpack2x16float(bitcast<u32>(nd.x));
    let czr = unpack2x16float(bitcast<u32>(nd.y));
    let dv = vec3<f32>(cxy, czr.x) - x;
    let d2 = dot(dv, dv);
    let r2 = czr.y * czr.y;
    var cb = 1.0;
    if (d2 > r2 && dot(n, n) > 0.0) {
        let inv = inverseSqrt(d2);
        let cos_t = dot(n, dv) * inv;
        let sin_b = czr.y * inv;
        let cos_b = sqrt(max(1.0 - sin_b * sin_b, 0.0));
        if (cos_t < cos_b) {
            cb = max(cos_t * cos_b + sqrt(max(1.0 - cos_t * cos_t, 0.0)) * sin_b, 0.0);
        }
    }
    // Live, nodes wholly farther than LT_FAR from x are not proposed (their light comes by BSDF rays, emit_mis weighs
    // them 1): shadow rays across a district cost more than the light they find.
    if (LIVE && d2 > (LT_FAR + czr.y) * (LT_FAR + czr.y)) { return 0.0; }
    return nd.z * cb / max(d2, r2);
}

// A CDF entry by power within entries [lo, hi) (cumulative p0 .. p1): one guide-table lookup.
fn emitter_pick_range(lo: u32, hi: u32) -> vec4<f32> {
    let p0 = select(0.0, emitters[max(lo, 1u) - 1u].y, lo > 0u);
    let p1 = emitters[hi - 1u].y;
    let u = min(p0 + rnd() * (p1 - p0), p1);
    let k = min(u32(u * f32(EMITTER_GUIDE)), EMITTER_GUIDE - 1u);
    var l = emitter_guide(k);
    var h = emitter_guide(k + 1u);
    while (l < h) {
        let mid = (l + h) / 2u;
        if (emitters[mid].y < u) { l = mid + 1u; } else { h = mid; }
    }
    return emitters[clamp(l, lo, hi - 1u)];
}

// An emitter entry for a point x (normal n), its selection probability `p` and its weight `w` (area x luminance x
// power, as the CDF weighs). With the tree: the moving emitters by power with probability P.ltree.z, else the tree
// descended by importance to a leaf, and one of its few entries by weight. Without it: the CDF entry (p = weight /
// total).
struct EPick { e: vec4<f32>, p: f32, w: f32 };

fn emitter_draw(x: vec3<f32>, n: vec3<f32>) -> EPick {
    if (!lt_on()) {
        let e = emitter_pick();
        return EPick(e, e.z / P.misc.y, e.z);
    }
    if (rnd() < P.ltree.z) {
        let e = emitter_pick_range(bitcast<u32>(P.ltree.y), P.counts.y);
        return EPick(e, e.z / P.misc.y, e.z);
    }
    var p = 1.0 - P.ltree.z;
    var w = bitcast<u32>(lt_node(0u).w);
    for (var k = 0u; k <= LT_PATH_BITS && (w & LT_LEAF) == 0u; k++) {
        let l = lt_node(w);
        let r = lt_node(w + 1u);
        let il = lt_imp(l, x, n);
        let ir = lt_imp(r, x, n);
        if (!(il + ir > 0.0)) { return EPick(vec4<f32>(0.0), 0.0, 0.0); }
        let pl = il / (il + ir);
        if (rnd() < pl) { w = bitcast<u32>(l.w); p *= pl; } else { w = bitcast<u32>(r.w); p *= 1.0 - pl; }
    }
    if ((w & LT_LEAF) == 0u) { return EPick(vec4<f32>(0.0), 0.0, 0.0); }
    let lo = w & 0x1fffffffu;
    let hi = lo + ((w >> 29u) & 3u) + 1u;
    var wsum = 0.0;
    for (var j = lo; j < hi; j++) { wsum += emitters[j].z; }
    var t = rnd() * wsum;
    var e = emitters[hi - 1u];
    for (var j = lo; j < hi; j++) {
        let c = emitters[j];
        if (t < c.z) { e = c; break; }
        t -= c.z;
    }
    return EPick(e, p * e.z / wsum, e.z);
}

// With the tree: the probability that emitter_draw(x, n) picks emitter triangle ti (attrs x.w: its path code, 0 = a
// moving one), the same choices down its path; 0 for triangles not light-sampled.
fn emitter_sel(ti: u32, x: vec3<f32>, n: vec3<f32>) -> f32 {
    let a = attrs[ti];
    let code = bitcast<u32>(a.x.w);
    if (code == 0u) {
        if (P.misc.y <= 0.0 || (has(F_PEMIT) && a.x.z < 0.0)) { return 0.0; }
        var area = 0.5 * length(cross(tri_verts[ti * 3u + 1u].xyz, tri_verts[ti * 3u + 2u].xyz));
        if (has(F_PEMIT) && a.x.z > 0.0) { area = a.x.z; }
        return luminance(materials[bitcast<u32>(a.uv.w)].emissive.rgb) * obj_power(a) * area / P.misc.y;
    }
    let depth = (code >> LT_PATH_BITS) - 1u;
    var p = 1.0 - P.ltree.z;
    var w = bitcast<u32>(lt_node(0u).w);
    for (var k = 0u; k < depth; k++) {
        let l = lt_node(w);
        let r = lt_node(w + 1u);
        let il = lt_imp(l, x, n);
        let ir = lt_imp(r, x, n);
        if (!(il + ir > 0.0)) { return 0.0; }
        let right = (code >> k) & 1u;
        p *= select(il, ir, right == 1u) / (il + ir);
        w = bitcast<u32>(select(l.w, r.w, right == 1u));
    }
    let lo = w & 0x1fffffffu;
    let hi = lo + ((w >> 29u) & 3u) + 1u;
    var wsum = 0.0;
    var we = 0.0;
    for (var j = lo; j < hi; j++) {
        let c = emitters[j];
        wsum += c.z;
        if (bitcast<u32>(c.x) == ti) { we = c.z; }
    }
    return p * we / wsum;
}

// ------------------------------------------------------------------------------------------------
// Intersection

// `inst`: the model instance whose model-space triangle `tri` is (F_INST), NO_INST for world-space triangles.
struct Hit { t: f32, tri: u32, u: f32, v: f32, inst: u32 };
const NO_INST: u32 = 0xffffffffu;

// ------------------------------------------------------------------------------------------------
// Procedural geometry (F_PROC; scene::ProcObject): a game's boxes and its geometry function (`gf_<name>`), which
// intersects the object-space ray with what one box holds; the hardware (or the software loop below) finds the boxes.
// A procedural object's record in `instances` follows the model instances': world rows, normal rows, info = (header
// word in proc_data, object, geometry function, casts shadows), the previous frame's world rows. Its hits are
// Hit(t, PROC_BIT | material << 23 | box, the object-space normal octahedral in (u, v), record).
const PROC_BIT: u32 = 0x80000000u;

// The geometry function's input: the ray in the object's space (d is not normalised: t is the world ray's parameter)
// and the interval a hit must lie in, the box (its index, min, max), where the object's data starts and its length in
// words (read it with `gdata` / `gdataf`), the object's customData and the scene time.
struct GIn { o: vec3<f32>, d: vec3<f32>, tmin: f32, tmax: f32, prim: u32, lo: vec3<f32>, hi: vec3<f32>, data: u32, count: u32, custom: vec4<f32>, time: f32 };
// Its output: t (none: outside (tmin, tmax), e.g. the default -1), the object-space normal (any length, facing either
// way), the material (an index into the mesh's materials; past the last: the last).
struct GOut { t: f32, normal: vec3<f32>, material: u32, attr: vec2<f32> };

// Procedural data (scene::proc_data): per object its header (`proc_vec`), materials, boxes (min xyz, max xyz) and the
// game's own data, 32-bit words after the environment's CDF in env_cdf (the kernel has no binding to spare).
// `pword`: the renderer's own reads (headers, boxes, materials). `gdata`: the geometry function's, of its object's data
// only, word i.data .. i.data + i.count - 1; anything else reads 0. The kernel is compiled without bounds checks (its
// indices come from the scene build) and the function is game code: it must not reach past its data.
fn pword(k: u32) -> u32 { return bitcast<u32>(env_cdf[P.procs.x + k]); }
fn pwordf(k: u32) -> f32 { return env_cdf[P.procs.x + k]; }
var<private> gdata_first: u32;
var<private> gdata_count: u32;
fn gdata(k: u32) -> u32 {
    if (k - gdata_first >= gdata_count) { return 0u; }
    return pword(k);
}
fn gdataf(k: u32) -> f32 { return bitcast<f32>(gdata(k)); }

// @geometry-functions-begin
fn geometry_function(f: u32, i: GIn) -> GOut { return GOut(-1.0, vec3<f32>(0.0, 1.0, 0.0), 0u, vec2<f32>(0.0)); }
// @geometry-functions-end

fn is_proc(h: Hit) -> bool { return has(F_PROC) && h.tri != 0xffffffffu && (h.tri & PROC_BIT) != 0u; }

// Header words of a procedural object (scene::PROC_HEADER): box count, first box word, first data word, material count,
// bounds min xyz, first material word, bounds max xyz, data length.
fn proc_vec(w: u32) -> vec3<f32> { return vec3<f32>(pwordf(w), pwordf(w + 1u), pwordf(w + 2u)); }

// Octahedral unit vector in two floats (a hit's u, v).
fn proc_oct(v: vec3<f32>) -> vec2<f32> {
    let n = v / max(abs(v.x) + abs(v.y) + abs(v.z), 1e-30);
    let w = (1.0 - abs(n.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
    return select(w, n.xy, n.z >= 0.0);
}
fn proc_unoct(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    let t = max(-n.z, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    return normalize(n);
}

// World -> object space of a record: its world rows' inverse (the normal rows are the inverse transpose of its 3 x 3).
fn proc_dir(r: Instance, d: vec3<f32>) -> vec3<f32> { return r.n0.xyz * d.x + r.n1.xyz * d.y + r.n2.xyz * d.z; }
fn proc_point(r: Instance, p: vec3<f32>) -> vec3<f32> { return proc_dir(r, p - vec3<f32>(r.m0.w, r.m1.w, r.m2.w)); }

// Box `prim` of record `rec` against the object-space ray: the geometry function's hit in (tmin, tmax), else t = -1.
fn proc_box(rec: u32, prim: u32, o: vec3<f32>, d: vec3<f32>, tmin: f32, tmax: f32) -> GOut {
    let info = instances[rec].info;
    let b = pword(info.x + 1u) + prim * 6u;
    gdata_first = pword(info.x + 2u);
    gdata_count = pword(info.x + 11u);
    let g = geometry_function(info.z, GIn(o, d, tmin, tmax, prim, proc_vec(b), proc_vec(b + 3u), gdata_first, gdata_count, objects[info.y * OBJECT_STRIDE + 2u], P.time.x));
    if (!(g.t > tmin && g.t < tmax)) { return GOut(-1.0, g.normal, 0u, g.attr); }
    return g;
}

// (u, v) carry 30 bits each as an ordinary float: exponent bits 01xxxxxx (64..127: never NaN, infinite or denormal,
// whose bits WGSL does not promise to keep through a bitcast), the payload in the sign bit and bits 28..0. u: the
// octahedral normal, 15 bits per component; v: the attribute, two half floats rounded to 15 bits (9-bit mantissa).
fn proc_word(p: u32) -> f32 { return bitcast<f32>(((p >> 29u) << 31u) | 0x20000000u | (p & 0x1fffffffu)); }
fn proc_bits(f: f32) -> u32 { let w = bitcast<u32>(f); return ((w >> 31u) << 29u) | (w & 0x1fffffffu); }

fn proc_hit(t: f32, g: GOut, prim: u32, rec: u32) -> Hit {
    let q = vec2<u32>(round(clamp(proc_oct(g.normal) * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0)) * 32767.0));
    let a = pack2x16float(g.attr);
    // Each half rounded to its upper 15 bits (a carry into the exponent is the right rounding; 0x7fff stays put).
    let ha = min(((a & 0xffffu) + 1u) >> 1u, 0x7fffu);
    let hb = min(((a >> 16u) + 1u) >> 1u, 0x7fffu);
    return Hit(t, PROC_BIT | (min(g.material, 255u) << 23u) | prim, proc_word(q.x | (q.y << 15u)), proc_word(ha | (hb << 15u)), rec);
}

// A procedural hit's attribute (GOut.attr): its material function's `i.uv`.
fn proc_attr(h: Hit) -> vec2<f32> {
    let p = proc_bits(h.v);
    return unpack2x16float(((p & 0x7fffu) << 1u) | (((p >> 15u) & 0x7fffu) << 17u));
}

// A procedural hit's world normal (geometric = shading) through its record's normal rows.
fn proc_normal(h: Hit) -> vec3<f32> {
    let r = instances[h.inst];
    let p = proc_bits(h.u);
    let n = proc_unoct(vec2<f32>(f32(p & 0x7fffu), f32(p >> 15u)) / 32767.0 * 2.0 - 1.0);
    return normalize(vec3<f32>(dot(r.n0.xyz, n), dot(r.n1.xyz, n), dot(r.n2.xyz, n)));
}

// Live primary visibility rasterized by the viewport (triangle index + 1, 0 = sky) at this frame's jitter.
@group(1) @binding(0) var vis_tex: texture_2d<u32>;

// The camera ray's hit on a known triangle (two-sided, no t range): the raster pass already resolved visibility.
fn tri_hit(ti: u32, o: vec3<f32>, d: vec3<f32>) -> Hit {
    let a = tri_verts[ti * 3u].xyz;
    let e1 = tri_verts[ti * 3u + 1u].xyz;
    let e2 = tri_verts[ti * 3u + 2u].xyz;
    let p = cross(d, e2);
    let det = dot(e1, p);
    let inv = 1.0 / select(det, 1e-20, abs(det) < 1e-20);
    let s = o - a;
    let u = dot(s, p) * inv;
    let q = cross(s, e1);
    return Hit(dot(e2, q) * inv, ti, clamp(u, 0.0, 1.0), clamp(dot(d, q) * inv, 0.0, 1.0), NO_INST);
}

// @trace-begin (software BVH; replaced by a ray-query implementation when hardware RT is available)

fn ray_aabb(o: vec3<f32>, inv_d: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>, tmax: f32) -> f32 {
    let t0 = (bmin - o) * inv_d;
    let t1 = (bmax - o) * inv_d;
    let tmin_v = min(t0, t1);
    let tmax_v = max(t0, t1);
    let tn = max(max(tmin_v.x, tmin_v.y), max(tmin_v.z, 0.0));
    let tf = min(min(tmax_v.x, tmax_v.y), min(tmax_v.z, tmax));
    return select(INF, tn, tn <= tf);
}

fn intersect_tri(o: vec3<f32>, d: vec3<f32>, i: u32, tmax: f32) -> vec3<f32> {
    let v0 = tri_verts[i * 3u].xyz;
    let e1 = tri_verts[i * 3u + 1u].xyz;
    let e2 = tri_verts[i * 3u + 2u].xyz;
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    if (abs(det) < 1e-12) { return vec3<f32>(INF, 0.0, 0.0); }
    let inv = 1.0 / det;
    let tv = o - v0;
    let u = dot(tv, pv) * inv;
    if (u < 0.0 || u > 1.0) { return vec3<f32>(INF, 0.0, 0.0); }
    let qv = cross(tv, e1);
    let v = dot(d, qv) * inv;
    if (v < 0.0 || u + v > 1.0) { return vec3<f32>(INF, 0.0, 0.0); }
    let t = dot(e2, qv) * inv;
    if (t <= 0.0 || t >= tmax) { return vec3<f32>(INF, 0.0, 0.0); }
    return vec3<f32>(t, u, v);
}

// mode: TRACE_CLOSEST, TRACE_SHADOW (any hit among shadow casters), TRACE_ANY (any hit on any surface).
fn trace(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, mode: u32) -> Hit {
    var hit = Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST);
    if (rays_off) { return hit; }
    let inv_d = 1.0 / select(d, vec3<f32>(1e-20), abs(d) < vec3<f32>(1e-20));
    var stack: array<u32, 48>;
    var sp = 0u;
    var node = 0u;
    loop {
        let n = nodes[node];
        if (n.count > 0u) {
            for (var k = 0u; k < n.count; k++) {
                let ti = n.left_or_first + k;
                let flags = bitcast<u32>(tri_verts[ti * 3u].w);
                if (mode == TRACE_SHADOW && (flags & TRI_NO_SHADOW) != 0u) { continue; }
                let r = intersect_tri(o, d, ti, hit.t);
                if (r.x < hit.t) {
                    if (has(F_MASKED) && (flags & TRI_MASKED) != 0u && !cutouts_opaque() && mode != TRACE_ANY_OPAQUE && !alpha_pass(ti, NO_INST, r.y, r.z, r.x)) { continue; }
                    hit = Hit(r.x, ti, r.y, r.z, NO_INST);
                    if (mode != TRACE_CLOSEST) { return hit; }
                }
            }
            if (sp == 0u) { break; }
            sp--;
            node = stack[sp];
            continue;
        }
        let l = n.left_or_first;
        let r = l + 1u;
        let nl = nodes[l];
        let nr = nodes[r];
        var tl = ray_aabb(o, inv_d, nl.bmin, nl.bmax, hit.t);
        var tr = ray_aabb(o, inv_d, nr.bmin, nr.bmax, hit.t);
        if (tl == INF && tr == INF) {
            if (sp == 0u) { break; }
            sp--;
            node = stack[sp];
            continue;
        }
        var near = l;
        var far = r;
        if (tr < tl) {
            near = r;
            far = l;
            let tmp = tl; tl = tr; tr = tmp;
        }
        node = near;
        if (tr != INF && sp < 48u) {
            stack[sp] = far;
            sp++;
        }
    }
    if (has(F_PROC)) { return trace_procs(o, d, hit, mode); }
    return hit;
}

// Procedural geometry, software traversal (the reference the hardware one is checked against): each object's bounds,
// then its boxes through the object's BVH (scene::ProcLayout with `sw`: nodes as bvh::GpuNode words, and the box of
// each leaf slot, so the geometry function gets the box's own index).
fn trace_procs(o: vec3<f32>, d: vec3<f32>, hit_in: Hit, mode: u32) -> Hit {
    var hit = hit_in;
    let first = P.procs.z;
    for (var k = 0u; k < P.procs.y; k++) {
        let rec = first + k;
        let r = instances[rec];
        if (mode == TRACE_SHADOW && r.info.w == 0u) { continue; }
        let lo = proc_point(r, o);
        let ld = proc_dir(r, d);
        let inv = 1.0 / select(ld, vec3<f32>(1e-20), abs(ld) < vec3<f32>(1e-20));
        let h = r.info.x;
        if (ray_aabb(lo, inv, proc_vec(h + 4u), proc_vec(h + 8u), hit.t) == INF) { continue; }
        let bw = pword(h + 1u);
        let nb = pword(h + 12u);
        let ob = pword(h + 13u);
        var stack: array<u32, 48>;
        var sp = 0u;
        var node = 0u;
        loop {
            let w = nb + node * 8u;
            let first_or_left = pword(w + 3u);
            let count = pword(w + 7u);
            if (count > 0u) {
                for (var q = 0u; q < count; q++) {
                    let p = pword(ob + first_or_left + q);
                    if (ray_aabb(lo, inv, proc_vec(bw + p * 6u), proc_vec(bw + p * 6u + 3u), hit.t) == INF) { continue; }
                    let g = proc_box(rec, p, lo, ld, 0.0, hit.t);
                    if (g.t >= 0.0) {
                        hit = proc_hit(g.t, g, p, rec);
                        if (mode != TRACE_CLOSEST) { return hit; }
                    }
                }
                if (sp == 0u) { break; }
                sp--;
                node = stack[sp];
                continue;
            }
            let wl = nb + first_or_left * 8u;
            var tl = ray_aabb(lo, inv, proc_vec(wl), proc_vec(wl + 4u), hit.t);
            var tr = ray_aabb(lo, inv, proc_vec(wl + 8u), proc_vec(wl + 12u), hit.t);
            if (tl == INF && tr == INF) {
                if (sp == 0u) { break; }
                sp--;
                node = stack[sp];
                continue;
            }
            var near = first_or_left;
            var far = first_or_left + 1u;
            if (tr < tl) {
                near = far;
                far = first_or_left;
                let tmp = tl; tl = tr; tr = tmp;
            }
            node = near;
            if (tr != INF && sp < 48u) {
                stack[sp] = far;
                sp++;
            }
        }
    }
    return hit;
}

// @trace-end

// Shadow rays of the analytic lights: objects that cast no shadow (castShadow = false, glass) let them through.
fn occluded(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> bool {
    shadow_tint = vec3<f32>(1.0);
    if (trace(o, d, tmax, TRACE_SHADOW).tri != 0xffffffffu) { return true; }
    if (!LIVE && has(F_GLASS) && glass_blocks(o, d, tmax, false)) { return true; }
    // Through media: blocked with probability 1 - transmittance (an unbiased estimate of it, ratio tracking).
    if (media_shadows()) { return rnd() >= media_tr(o, d, tmax); }
    // Live: homogeneous media (water) analytically.
    return LIVE && has(F_HMEDIA) && rnd() >= media_homogeneous(o, d, tmax, false).tr;
}

// Offline: what the glass a shadow ray crosses lets through (it has passed every opaque caster). At each glass surface
// the share of the BSDF's transmission lobe: transmission x (1 - the specular directional albedo) at the crossing
// angle. The ray goes straight (no refraction, no caustics, no tint: UE's path tracer), so a light seen from under
// water at more than the critical angle still lights through the surface, as its refracted light does. `sky`: an odd
// number of glass surfaces lets nothing through (occluded_sky). Unattenuated, light under a water surface came both
// through it and from the paths it reflects back (a white furnace through water read 1.25).
fn glass_tr(o_in: vec3<f32>, d: vec3<f32>, tmax: f32, sky: bool) -> vec3<f32> {
    var o = o_in;
    var left = tmax;
    var tr = vec3<f32>(1.0);
    var odd = false;
    var t0 = 0.0;
    for (var k = 0u; k < 8u; k++) {
        let h = trace(o, d, left, TRACE_CLOSEST);
        if (h.tri == 0xffffffffu) { break; }
        t0 += h.t;
        let g = glass_hit(h, d, t0);
        if (g.share > 0.0) {
            tr *= g.share * g.tint;
            odd = !odd;
        }
        o = offset_origin(o + d * h.t, select(g.ng, -g.ng, dot(g.ng, d) < 0.0));
        left -= h.t;
        if (left <= 0.0 || all(tr <= vec3<f32>(0.0))) { break; }
    }
    if (sky && odd) { return vec3<f32>(0.0); }
    return tr;
}

// The colour a shadow ray's light keeps past glass (glass_tr), drawn as a single decision: blocked with probability
// 1 - p, p its largest channel; passing, `shadow_tint` = the colour / p (unbiased; light_vis hands it to the caller).
var<private> shadow_tint: vec3<f32> = vec3<f32>(1.0);
fn glass_blocks(o: vec3<f32>, d: vec3<f32>, tmax: f32, sky: bool) -> bool {
    let t = glass_tr(o, d, tmax, sky);
    let p = max(t.r, max(t.g, t.b));
    if (rnd() >= p) { return true; }
    shadow_tint = t / p;
    return false;
}

// A shadow ray's glass tint where there can be one (offline, scenes with glass); elsewhere a constant 1 the compiler
// folds away, so their code (and images) stay as they were.
fn gtint(v: vec3<f32>) -> vec3<f32> {
    if (!LIVE && has(F_GLASS)) { return v; }
    return vec3<f32>(1.0);
}

// What reaches a lit point from a light along a shadow ray: 0 when occluded, else 1 or the tint of the glass it passed
// (offline). `shadows` false (a light that casts none): 1, no ray.
fn light_vis(shadows: bool, o: vec3<f32>, d: vec3<f32>, tmax: f32) -> vec3<f32> {
    if (!shadows) { return vec3<f32>(1.0); }
    if (occluded(o, d, tmax)) { return vec3<f32>(0.0); }
    return shadow_tint;
}

// Glass at a hit along d, `t` from the ray's start (texture LOD): its geometric normal, the share of the ray its
// transmission lobe takes at that angle (transmission (x its texture) x (1 - E_spec), as lobes(); 0: not glass), its
// tint (base colour x texture: what the transmission lobe passes), and whether a ray from that side is totally
// reflected.
struct GlassAt { ng: vec3<f32>, share: f32, tir: bool, tint: vec3<f32> };

// `glass_at` of any hit: procedural surfaces are opaque (`surface_proc`). (Kept out of glass_at itself: there, even
// compiled out, it changed how Metal compiled the glass path, and glass scenes rendered 0.08 % of values differently.)
fn glass_hit(h: Hit, d: vec3<f32>, t: f32) -> GlassAt {
    if (is_proc(h)) { return GlassAt(proc_normal(h), 0.0, false, vec3<f32>(1.0)); }
    return glass_at(h, d, t);
}

fn glass_at(h: Hit, d: vec3<f32>, t: f32) -> GlassAt {
    var a = attrs[h.tri];
    var e1 = tri_verts[h.tri * 3u + 1u].xyz;
    var e2 = tri_verts[h.tri * 3u + 2u].xyz;
    if (has(F_INST) && h.inst != NO_INST) {
        let r = instances[h.inst];
        e1 = inst_vec(r, e1);
        e2 = inst_vec(r, e2);
        a = inst_attr(r, a);
    }
    let m = materials[bitcast<u32>(a.uv.w)];
    var g = GlassAt(normalize(cross(e1, e2)), 0.0, false, vec3<f32>(1.0));
    var tr = clamp(m.layer.x, 0.0, 1.0) * (1.0 - clamp(m.params.x, 0.0, 1.0));
    if (tr <= 0.0) { return g; }
    g.tint = m.base_color.rgb;
    if (has(F_TEX) && (m.tex0.x >= 0 || m.uv.z > 0.5)) {
        let uv = mesh_uv(a, m, h.u, h.v);
        var lod = mesh_lod(a, m) + log2(max(t * 2.0 * P.cam_pos.w / lod_height(), 1e-7)) - LOD_BIAS;
        if (has(F_INST) && h.inst != NO_INST) { lod += bitcast<f32>(instances[h.inst].info.z); }
        if (m.tex0.x >= 0) { g.tint *= srgb_to_linear(tex(m.tex0.x, uv, lod).rgb); }
        if (m.uv.z > 0.5) { tr *= tex(i32(m.uv.z) - 1, uv, lod).r; }
    }
    let c = abs(dot(g.ng, d));
    let ior = max(m.layer.y, 1.0001);
    let eta = select(1.0 / ior, ior, dot(g.ng, d) > 0.0);
    g.tir = eta * eta * (1.0 - c * c) >= 1.0;
    let r = (ior - 1.0) / (ior + 1.0);
    let f0 = r * r;
    let e = energy_lookup(clamp(m.params.y, 0.02, 1.0), c);
    let w = 1.0 + P.misc.z * f0 * ((1.0 - e.x) / e.x);
    g.share = tr * (1.0 - w * (e.x * f0 + e.y * (clamp(50.0 * f0, 0.0, 1.0) - f0)));
    return g;
}

// Shadow rays toward the environment: as `occluded`, but glass that bends the ray (an odd number of glass surfaces:
// a water surface, the shading point inside a glass solid) blocks it. The sky there is the refracted paths' (the
// path loop counts it in full past such glass), which find it where it is (Snell's window from under water).
fn occluded_sky(o: vec3<f32>, d: vec3<f32>) -> bool {
    shadow_tint = vec3<f32>(1.0);
    if (trace(o, d, INF, TRACE_SHADOW).tri != 0xffffffffu) { return true; }
    if (!LIVE && has(F_GLASS) && glass_blocks(o, d, INF, true)) { return true; }
    if (media_shadows()) { return rnd() >= media_tr(o, d, INF); }
    return LIVE && has(F_HMEDIA) && rnd() >= media_homogeneous(o, d, INF, false).tr;
}

// Live homogeneous media (VolumeMesh `homogeneous`: constant coefficients, Coeff = albedo rgb + extinction, Emission rgb,
// in the function's first parameters) along o + d t, t < tmax, without the volume function: transmittance, and with
// `light` the emission and the in-scattering from the volume voxels at each crossing's middle. The live megakernel's
// only media code: water and other constant media attenuate reflection, refraction and shadow rays; other media are
// the camera rays' alone (media_composite).
fn media_homogeneous(o: vec3<f32>, d: vec3<f32>, tmax: f32, light: bool) -> March {
    var r = March(1.0, vec3<f32>(0.0));
    let n = min(u32(P.time.w), 32u);
    for (var k = 0u; k < n; k++) {
        let m = media[k];
        if (m.a.z == 0u) { continue; }
        let sp = medium_span(m, o, d);
        let a = max(sp.x, 0.0);
        let b = min(sp.y, tmax);
        if (a >= b) { continue; }
        let st = m.p[0].w;
        let tr = exp(-st * (b - a));
        if (light) {
            // In-scattering at two points stratified by transmittance (half of what scatters toward o happens before
            // the first, ...): one lookup in the middle lit a deep crossing with its dimmer depth.
            var lin = vec3<f32>(0.0);
            if (cache_on()) {
                for (var j = 0u; j < 2u; j++) {
                    let q = (f32(j) + rnd()) * 0.5 * (1.0 - tr);
                    let t = select(-log(1.0 - q) / st, (f32(j) + 0.5) * 0.5 * (b - a), st <= 1e-6);
                    lin += 0.5 * cache_point(o + d * (a + t)).rgb;
                }
            }
            let w = select(b - a, (1.0 - tr) / st, st > 1e-6);
            r.light += r.tr * (m.p[0].rgb * st * lin + m.p[1].rgb) * w;
        }
        r.tr *= tr;
    }
    return r;
}

// Media in shadow rays: offline, and in the live media voxel training (media_train sets `media_voxels`). The live
// megakernel has no media code at all: inlined at every shadow ray, the volume functions cost Kabukichō 0.4 ms with
// a smoke column out of view (its media are composited in a pass of their own, media_composite).
var<private> media_voxels: bool = false;
fn media_shadows() -> bool { return (!LIVE || media_voxels) && has(F_MEDIA) && P.time.w > 0.5; }

// Shadow rays toward emissive surfaces: every surface blocks them, as it blocks the BSDF rays that find the same
// emitters (their MIS partner; a pass-through here counted light that no path can reach, and more of it the larger
// the emitter's sampling share). UE's path tracer, too, reaches emissive surfaces only through rays that every mesh
// blocks.
// Shadow ray to an emitter sample (any surface blocks). Live, alpha-cutout triangles are opaque to it at camera hits too
// (deeper vertices take them as opaque anyway, `cutouts_opaque`): the leaves of a tree block as whole cards. A bias,
// accepted by the user on 2026-11-09 to bring the Bistro crowd within its 60 fps budget: the alpha tests of these rays
// were ~0.9 ms of the frame (crowd GPU p50 15.06 -> 14.16 ms, frame start p95 17.14 -> 16.67 ms), and cost the live
// image under foliage ~1.7 % of its light (static Bistro at 25 s along its path; layer B Bistro ΔE 3.23 -> 3.30).
// Removing the bias without the cost (an opacity micromap, or these rays compacted into a pass of their own) is a
// follow-up item (ROADMAP § 0). Offline rendering always tests.
fn occluded_emitter(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> bool {
    if (trace(o, d, tmax, select(TRACE_ANY, TRACE_ANY_OPAQUE, LIVE)).tri != 0xffffffffu) { return true; }
    if (media_shadows()) { return rnd() >= media_tr(o, d, tmax); }
    return LIVE && has(F_HMEDIA) && rnd() >= media_homogeneous(o, d, tmax, false).tr;
}

fn occluded_any(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> bool {
    if (trace(o, d, tmax, TRACE_ANY).tri != 0xffffffffu) { return true; }
    if (media_shadows()) { return rnd() >= media_tr(o, d, tmax); }
    return LIVE && has(F_HMEDIA) && rnd() >= media_homogeneous(o, d, tmax, false).tr;
}

// ------------------------------------------------------------------------------------------------
// Surface

struct Surface {
    p: vec3<f32>,
    ng: vec3<f32>,
    n: vec3<f32>,
    base: vec3<f32>,
    metallic: f32,
    roughness: f32,
    specular: f32,
    emissive: vec3<f32>,
    front: bool,
    card: bool,
    // Shading changes over time (animated emission, rain ripples): the live viewport keeps a short history here.
    anim: bool,
    // Glass (transmission share, index of refraction) and clearcoat (amount, perceptual roughness).
    transmission: f32,
    ior: f32,
    coat: f32,
    coat_rough: f32,
};

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// Texture pages: `lod` is log2 of the ray footprint in texture-UV units; the mip level adds the texture's resolution.
fn tex_level(id: i32, uv: vec2<f32>, level: f32) -> vec4<f32> {
    let a = tex_info[u32(id) * 2u];
    let b = tex_info[u32(id) * 2u + 1u];
    let f = select(clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), fract(uv), b.y > 0.5);
    return textureSampleLevel(textures, samp, a.xy + f * a.zw, i32(b.x), clamp(level, 0.0, TEX_MAX_LEVEL));
}

fn tex(id: i32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let b = tex_info[u32(id) * 2u + 1u];
    return tex_level(id, uv, lod + 0.5 * log2(b.z * b.w));
}

fn mesh_uv(a: TriAttr, m: Material, u: f32, v: f32) -> vec2<f32> {
    let w = 1.0 - u - v;
    return (vec2<f32>(a.n0.w, a.n1.w) * w + vec2<f32>(a.n2.w, a.uv.x) * u + a.uv.yz * v) * m.uv.xy + m.uv2.xy;
}

// log2(texture-UV units per metre) of a triangle's mesh UVs (incl. material repeat).
fn mesh_lod(a: TriAttr, m: Material) -> f32 {
    return a.x.x + 0.5 * log2(max(abs(m.uv.x * m.uv.y), 1e-12));
}

// ------------------------------------------------------------------------------------------------
// Material functions: scene-supplied WGSL (like a UE Custom-HLSL material), docs/MATERIAL_FUNCTIONS.md.
// Inputs in world space (metres, Y up); helpers convert to UE space (cm, Z up) for ported UE code.

struct MatIn {
    wp: vec3<f32>,   // world position, m
    n: vec3<f32>,    // vertex normal (facing the viewer)
    ng: vec3<f32>,   // geometric normal (facing the viewer)
    t: vec3<f32>,    // tangent (+u)
    b: vec3<f32>,    // bitangent (image up = -v)
    uv: vec2<f32>,   // mesh UV0 (image convention: v down)
    uvd: f32,        // log2(mesh UV units per metre): msample(..., exp2(i.uvd)) for mesh-UV lookups
    view: vec3<f32>, // unit vector toward the viewer / ray origin
    op: vec3<f32>,   // object pivot, m
    ob: vec3<f32>,   // object world bounds size, m
    time: f32,
    wetness: f32,
    rain: f32,
    lod: f32,        // log2 of the ray footprint, m
    depth: u32,      // path vertex (0 = camera hit)
    power: f32,      // instance power (UE "Power"): scale the emission by it
    near: bool,      // seen directly or in mirror-like reflections (puddles, glass): where fine detail shows
    custom: vec4<f32>, // per-instance data (UE PerInstanceCustomData): the object's `customData`, or its particle's
};

struct MatOut {
    base: vec3<f32>,
    roughness: f32,
    metallic: f32,
    specular: f32,
    emissive: vec3<f32>,
    normal: vec3<f32>, // world-space shading normal
    opacity: f32,      // alpha-tested against the material's alphaTest
    animated: bool,    // the shading here moves with time (live: short history, as for animated emission)
};

// Texture slot `slot` of the material function at `uv`, mip from the ray footprint and `uv_per_m` (uv units / m).
fn msample(m: Material, i: MatIn, slot: u32, uv: vec2<f32>, uv_per_m: f32) -> vec4<f32> {
    let id = m.mt[slot];
    if (id < 0) { return vec4<f32>(1.0); }
    return tex(id, uv, i.lod + log2(max(uv_per_m, 1e-9)) - LOD_BIAS);
}

// Colour texture (sRGB-encoded) -> linear rgb, alpha as stored.
fn msample_color(m: Material, i: MatIn, slot: u32, uv: vec2<f32>, uv_per_m: f32) -> vec4<f32> {
    let c = msample(m, i, slot, uv, uv_per_m);
    return vec4<f32>(srgb_to_linear(c.rgb), c.a);
}

// Tangent-space normal (x along +u, y image-up, z out) -> world.
fn tangent_normal(i: MatIn, tn: vec3<f32>) -> vec3<f32> {
    let w = i.t * tn.x + i.b * tn.y + i.n * tn.z;
    if (dot(w, w) < 1e-12) { return i.n; }
    let nn = normalize(w);
    return select(i.n, nn, dot(nn, i.ng) > 0.0);
}

// Unreal space: (X, Y, Z) cm, Z up = ours (x, z, y) * 100.
fn ue_pos(p: vec3<f32>) -> vec3<f32> { return vec3<f32>(p.x, p.z, p.y) * 100.0; }
fn ue_vec(v: vec3<f32>) -> vec3<f32> { return vec3<f32>(v.x, v.z, v.y); }
fn from_ue_vec(v: vec3<f32>) -> vec3<f32> { return vec3<f32>(v.x, v.z, v.y); }

// @material-functions-begin
fn material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut { return base; }
fn translucent_material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut { return base; }
fn alpha_material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut { return base; }
fn emissive_material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut { return base; }
// @material-functions-end

fn object_of(a: TriAttr) -> u32 { return bitcast<u32>(a.x.y); }
// vec4s per object (scene.rs OBJECT_STRIDE): (pivot, power), (bounds size, time of the last power edit), per-instance
// data (`MatIn.custom`).
const OBJECT_STRIDE: u32 = 3u;
// Instance power (UE material "Power", scripts animate it): scales the emission of everything the node draws.
fn obj_power(a: TriAttr) -> f32 { return objects[object_of(a) * OBJECT_STRIDE].w; }

// Inputs at a point of triangle ti (barycentrics u, v) for material functions outside full shading (alpha, NEE).
fn mat_in_at(ti: u32, u: f32, v: f32, view: vec3<f32>, lod: f32) -> MatIn {
    let a = attrs[ti];
    let m = materials[bitcast<u32>(a.uv.w)];
    let w = 1.0 - u - v;
    let p = tri_verts[ti * 3u].xyz + tri_verts[ti * 3u + 1u].xyz * u + tri_verts[ti * 3u + 2u].xyz * v;
    let ng = normalize(cross(tri_verts[ti * 3u + 1u].xyz, tri_verts[ti * 3u + 2u].xyz));
    var n = normalize(a.n0.xyz * w + a.n1.xyz * u + a.n2.xyz * v);
    let t = normalize(a.t.xyz - n * dot(n, a.t.xyz) + vec3<f32>(1e-9));
    let oi = object_of(a);
    return MatIn(p, n, ng, t, cross(n, t) * a.t.w, mesh_uv(a, m, u, v), mesh_lod(a, m), view, objects[oi * OBJECT_STRIDE].xyz, objects[oi * OBJECT_STRIDE + 1u].xyz,
                 P.time.x, P.time.y, P.time.z, lod, cur_depth, objects[oi * OBJECT_STRIDE].w, cur_depth == 0u || mirror_path,
                 objects[oi * OBJECT_STRIDE + 2u]);
}

fn mat_base(m: Material) -> MatOut {
    return MatOut(m.base_color.rgb, m.params.y, m.params.x, m.params.z, m.emissive.rgb, vec3<f32>(0.0, 1.0, 0.0), m.base_color.a, false);
}

// Live rays past the camera ray and its own light samples, off mirror-like paths (whose image is still seen), take
// alpha-cutout triangles as opaque: no alpha test, no stepping through the leaves behind. The leaves of a tree are then
// solid cards to indirect light and to the light samples of later bounces. Through foliage that was most of the trace:
// Bistro along its camera path, frame GPU 16.1 / 19.9 ms -> 14.2 / 16.0 at its first pose / 15 s in; against the
// unbiased offline render the live image 0.3 % darker, ΔE +0.02–0.04 (the clamps' darkening is 9–13 %). Offline
// rendering always tests.
fn cutouts_opaque() -> bool {
    return LIVE && cur_depth >= 1u && !mirror_path;
}

// Alpha-cutout test for candidate hits on masked triangles (mesh UVs, footprint of a camera ray at distance t). `inst`:
// the model instance a pool triangle belongs to (its scale shifts the texel density, as in `inst_attr`), or NO_INST.
fn alpha_pass(ti: u32, inst: u32, u: f32, v: f32, t: f32) -> bool {
    let a = attrs[ti];
    let m = materials[bitcast<u32>(a.uv.w)];
    var alpha = m.base_color.a;
    var lod = mesh_lod(a, m) + log2(max(t * 2.0 * P.cam_pos.w / lod_height(), 1e-7)) - LOD_BIAS;
    if (has(F_INST) && inst != NO_INST) { lod += bitcast<f32>(instances[inst].info.z); }
    if (has(F_TEX) && m.tex0.x >= 0) {
        alpha *= tex(m.tex0.x, mesh_uv(a, m, u, v), lod).a;
    }
    if (has(F_FUNC) && m.tex1.w > 0) {
        let i = mat_in_at(ti, u, v, vec3<f32>(0.0, 1.0, 0.0), log2(max(t * 2.0 * P.cam_pos.w / lod_height(), 1e-7)));
        var b = mat_base(m);
        b.opacity = alpha;
        alpha = alpha_material_function(m.tex1.w - 1, i, m, b).opacity;
    }
    return alpha >= m.uv2.w;
}

// Emitted radiance at `uv`: factor x emissive map.
fn emission(m: Material, uv: vec2<f32>, lod: f32) -> vec3<f32> {
    var em = m.emissive.rgb;
    if (all(em <= vec3<f32>(0.0))) { return em; }
    if (has(F_TEX) && m.tex1.x >= 0) { em *= srgb_to_linear(tex(m.tex1.x, uv, lod).rgb); }
    return em;
}

// `width` = ray-cone footprint (m) at the hit, for texture LOD.
fn surface(o: vec3<f32>, d: vec3<f32>, h: Hit, width: f32) -> Surface {
    if (is_proc(h)) { return surface_proc(o, d, h, width); }
    let i = h.tri;
    var e1 = tri_verts[i * 3u + 1u].xyz;
    var e2 = tri_verts[i * 3u + 2u].xyz;
    var a = attrs[i];
    if (has(F_INST) && h.inst != NO_INST) {
        let r = instances[h.inst];
        e1 = inst_vec(r, e1);
        e2 = inst_vec(r, e2);
        a = inst_attr(r, a);
    }
    let w = 1.0 - h.u - h.v;
    var ng = normalize(cross(e1, e2));
    var n = normalize(a.n0.xyz * w + a.n1.xyz * h.u + a.n2.xyz * h.v);
    let mi = bitcast<u32>(a.uv.w);
    let m = materials[mi];
    let front = dot(ng, d) < 0.0;
    if (!front) { ng = -ng; }
    if (dot(n, ng) < 0.0) { n = -n; }
    let p = o + d * h.t;

    let uv = mesh_uv(a, m, h.u, h.v);
    let lod = mesh_lod(a, m) + log2(max(width, 1e-7)) - 0.5 * log2(max(abs(dot(ng, d)), 1e-3)) - LOD_BIAS;

    var s: Surface;
    s.p = p;
    s.front = front;
    s.card = m.tex1.z == MAT_EMITTER_CARD;
    var base = m.base_color.rgb;
    var rough = m.params.y;
    var metal = m.params.x;
    var spec = m.params.z;
    if (has(F_TEX)) {
        if (m.tex0.x >= 0) { base *= srgb_to_linear(tex(m.tex0.x, uv, lod).rgb); }
        if (m.tex0.z >= 0) { rough *= tex(m.tex0.z, uv, lod).g; }
        if (m.tex0.w >= 0) { metal *= tex(m.tex0.w, uv, lod).b; }
    }
    s.emissive = emission(m, uv, lod) * obj_power(a);
    // Power just changed by a script / gameplay (objects[1].w = scene time of the edit): short history.
    s.anim = P.time.x - objects[object_of(a) * OBJECT_STRIDE + 1u].w < 0.5;
    // Detail that only matters where it is seen directly or in mirror-like reflections (puddles, glass, chrome).
    let near = cur_depth == 0u || mirror_path;
    if (has(F_TEX) && m.tex0.y >= 0 && near) {
        let tn = tex(m.tex0.y, uv, lod).xyz * 2.0 - 1.0;
        let tp = a.t.xyz - n * dot(n, a.t.xyz);
        if (dot(tp, tp) > 1e-12) {
            let t = normalize(tp);
            let b = cross(n, t) * a.t.w;
            let pn = t * (tn.x * m.params.w) + b * (tn.y * m.params.w) + n * tn.z;
            if (dot(pn, pn) > 1e-12 && dot(pn, ng) > 0.0) { n = normalize(pn); }
        }
    }
    if (has(F_FUNC) && m.tex1.w > 0) {
        let oi = object_of(a);
        var t = normalize(a.t.xyz - n * dot(n, a.t.xyz) + vec3<f32>(1e-9));
        let nv = normalize(a.n0.xyz * w + a.n1.xyz * h.u + a.n2.xyz * h.v) * select(-1.0, 1.0, front || dot(a.n0.xyz, ng) > 0.0);
        let i = MatIn(p, select(-nv, nv, dot(nv, ng) >= 0.0), ng, t, cross(n, t) * a.t.w, mesh_uv(a, m, h.u, h.v), mesh_lod(a, m), -d,
                      objects[oi * OBJECT_STRIDE].xyz, objects[oi * OBJECT_STRIDE + 1u].xyz, P.time.x, P.time.y, P.time.z,
                      log2(max(width, 1e-7)) - 0.5 * log2(max(abs(dot(ng, d)), 1e-3)), cur_depth, objects[oi * OBJECT_STRIDE].w, near,
                      objects[oi * OBJECT_STRIDE + 2u]);
        let r = material_function(m.tex1.w - 1, i, m, MatOut(base, rough, metal, spec, s.emissive, n, m.base_color.a, false));
        base = r.base;
        rough = r.roughness;
        metal = r.metallic;
        spec = r.specular;
        s.emissive = r.emissive;
        n = r.normal;
        // Shading the function says moves with time (wave normals) is as animated as emission is: short history.
        s.anim = s.anim || any(r.emissive > vec3<f32>(0.0)) || r.animated;
    }
    s.base = base;
    s.roughness = clamp(rough, 0.02, 1.0);
    s.metallic = clamp(metal, 0.0, 1.0);
    s.specular = spec;
    s.ng = ng;
    s.n = n;
    s.ior = 1.5;
    if (has(F_GLASS)) {
        s.transmission = clamp(m.layer.x, 0.0, 1.0);
        // glTF transmissionTexture (R), its index + 1 in uv.z.
        if (has(F_TEX) && m.uv.z > 0.5) { s.transmission *= tex(i32(m.uv.z) - 1, uv, lod).r; }
        s.ior = max(m.layer.y, 1.0001);
    }
    if (has(F_COAT)) {
        s.coat = clamp(m.layer.z, 0.0, 1.0);
        s.coat_rough = clamp(m.layer.w, 0.02, 1.0);
    }
    return s;
}

// A procedural hit's surface: the material its geometry function picked, the normal it gave (geometric and shading,
// two-sided), the object's power and customData. No textures (no uv: material functions get uv 0 and can work from
// `i.wp`), no transmission.
fn surface_proc(o: vec3<f32>, d: vec3<f32>, h: Hit, width: f32) -> Surface {
    let r = instances[h.inst];
    let hd = r.info.x;
    let mi = pword(pword(hd + 7u) + min((h.tri >> 23u) & 0xffu, pword(hd + 3u) - 1u));
    let m = materials[mi];
    let oi = r.info.y;
    var ng = proc_normal(h);
    let front = dot(ng, d) < 0.0;
    if (!front) { ng = -ng; }
    var n = ng;
    let p = o + d * h.t;
    var s: Surface;
    s.p = p;
    s.front = front;
    s.card = false;
    var base = m.base_color.rgb;
    var rough = m.params.y;
    var metal = m.params.x;
    var spec = m.params.z;
    s.emissive = m.emissive.rgb * objects[oi * OBJECT_STRIDE].w;
    s.anim = P.time.x - objects[oi * OBJECT_STRIDE + 1u].w < 0.5;
    if (has(F_FUNC) && m.tex1.w > 0) {
        let near = cur_depth == 0u || mirror_path;
        let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
        let t = normalize(cross(a, n));
        let i = MatIn(p, n, ng, t, cross(n, t), proc_attr(h), 0.0, -d, objects[oi * OBJECT_STRIDE].xyz, objects[oi * OBJECT_STRIDE + 1u].xyz,
                      P.time.x, P.time.y, P.time.z, log2(max(width, 1e-7)) - 0.5 * log2(max(abs(dot(ng, d)), 1e-3)), cur_depth,
                      objects[oi * OBJECT_STRIDE].w, near, objects[oi * OBJECT_STRIDE + 2u]);
        let f = material_function(m.tex1.w - 1, i, m, MatOut(base, rough, metal, spec, s.emissive, n, m.base_color.a, false));
        base = f.base;
        rough = f.roughness;
        metal = f.metallic;
        spec = f.specular;
        s.emissive = f.emissive;
        n = f.normal;
        s.anim = s.anim || any(f.emissive > vec3<f32>(0.0)) || f.animated;
    }
    s.base = base;
    s.roughness = clamp(rough, 0.02, 1.0);
    s.metallic = clamp(metal, 0.0, 1.0);
    s.specular = spec;
    s.ng = ng;
    s.n = n;
    s.ior = 1.5;
    if (has(F_COAT)) {
        s.coat = clamp(m.layer.z, 0.0, 1.0);
        s.coat_rough = clamp(m.layer.w, 0.02, 1.0);
    }
    return s;
}

// ------------------------------------------------------------------------------------------------
// BSDF (UE DefaultLit)

fn energy_lookup(roughness: f32, nov: f32) -> vec2<f32> {
    let x = clamp(nov * 31.0, 0.0, 31.0);
    let y = clamp(roughness * 31.0, 0.0, 31.0);
    let x0 = u32(floor(x)); let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, 31u); let y1 = min(y0 + 1u, 31u);
    let fx = x - f32(x0); let fy = y - f32(y0);
    let a = mix(lut(x0, y0), lut(x1, y0), fx);
    let b = mix(lut(x0, y1), lut(x1, y1), fx);
    return mix(a, b, fy);
}

fn lut(x: u32, y: u32) -> vec2<f32> {
    let i = y * 32u + x;
    let v = energy_lut[i / 2u];
    return select(v.xy, v.zw, (i & 1u) == 1u);
}

// Base layer: diffuse + GGX specular (p_spec: the specular share of the base's samples). Over it a clearcoat (GGX, F0
// 0.04; the base gets what the coat's directional albedo e_coat lets through), and for glass a transmitted share
// `tint` (delta lobe, sampled with p_trans) in place of the diffuse. p_coat / p_trans: shares of all samples.
struct Lobes {
    diffuse: vec3<f32>, f0: vec3<f32>, f90: vec3<f32>, w: vec3<f32>, e_spec: vec3<f32>, a2: f32, p_spec: f32,
    coat: f32, coat_alpha: f32, e_coat: f32, p_coat: f32, tint: vec3<f32>, p_trans: f32,
};

fn lobes(s: Surface, nov: f32) -> Lobes {
    var l: Lobes;
    let t = s.transmission * (1.0 - s.metallic);
    l.diffuse = s.base * (1.0 - s.metallic) * (1.0 - t);
    var f0 = 0.08 * s.specular;
    if (t > 0.0) {
        // Glass: F0 from the index of refraction.
        let r = (s.ior - 1.0) / (s.ior + 1.0);
        f0 = r * r;
    }
    l.f0 = mix(vec3<f32>(f0), s.base, s.metallic);
    l.f90 = vec3<f32>(clamp(50.0 * l.f0.g, 0.0, 1.0));
    let e = energy_lookup(s.roughness, nov);
    // misc.z = multiple-scattering compensation amount (1 = energy conserving, matches the white furnace).
    l.w = 1.0 + P.misc.z * l.f0 * ((1.0 - e.x) / e.x);
    l.e_spec = l.w * (e.x * l.f0 + e.y * (l.f90 - l.f0));
    let r = s.roughness;
    l.a2 = r * r * r * r;
    let ws = dot(l.e_spec, vec3<f32>(0.3, 0.6, 0.1));
    let wd = dot(l.diffuse * (1.0 - l.e_spec), vec3<f32>(0.3, 0.6, 0.1));
    l.p_spec = clamp(ws / max(ws + wd, 1e-6), 0.05, 1.0);
    if (wd <= 0.0) { l.p_spec = 1.0; }
    l.coat_alpha = 1.0;
    if (has(F_COAT) && s.coat > 0.0) {
        let ec = energy_lookup(s.coat_rough, nov);
        l.coat = s.coat;
        l.e_coat = s.coat * (ec.x * 0.04 + ec.y * 0.96);
        l.coat_alpha = s.coat_rough * s.coat_rough;
    }
    let under = 1.0 - l.e_coat;
    l.tint = s.base * t * (1.0 - l.e_spec) * under;
    let wt = dot(l.tint, vec3<f32>(0.3, 0.6, 0.1));
    let total = (ws + wd) * under + l.e_coat + wt;
    l.p_coat = l.e_coat / max(total, 1e-6);
    l.p_trans = wt / max(total, 1e-6);
    return l;
}

fn d_ggx(a2: f32, noh: f32) -> f32 {
    let d = (noh * a2 - noh) * noh + 1.0;
    return a2 / (PI * d * d);
}

fn vis_smith_joint_approx(a2: f32, nov: f32, nol: f32) -> f32 {
    let a = sqrt(a2);
    let v = nol * (nov * (1.0 - a) + a);
    let l = nov * (nol * (1.0 - a) + a);
    return 0.5 / max(v + l, 1e-7);
}

fn f_schlick(f0: vec3<f32>, f90: vec3<f32>, voh: f32) -> vec3<f32> {
    let fc = pow(1.0 - voh, 5.0);
    return f0 + (f90 - f0) * fc;
}

// BSDF * NoL with the specular lobe scaled (per-light specular scale)
fn eval_bsdf_k(s: Surface, lb: Lobes, v: vec3<f32>, l: vec3<f32>, k: f32) -> vec3<f32> {
    let nol = dot(s.n, l);
    let nov = max(dot(s.n, v), 1e-4);
    if (nol <= 0.0 || dot(s.ng, l) <= 0.0) { return vec3<f32>(0.0); }
    let h = normalize(v + l);
    let noh = max(dot(s.n, h), 0.0);
    let voh = max(dot(v, h), 0.0);
    var spec = d_ggx(lb.a2, noh) * vis_smith_joint_approx(lb.a2, nov, nol) * f_schlick(lb.f0, lb.f90, voh) * lb.w * k;
    var diff = lb.diffuse * (1.0 / PI) * (1.0 - lb.e_spec);
    if (has(F_COAT) && lb.coat > 0.0) {
        let ca2 = lb.coat_alpha * lb.coat_alpha;
        let coat = d_ggx(ca2, noh) * vis_smith_joint_approx(ca2, nov, nol) * f_schlick(vec3<f32>(0.04), vec3<f32>(1.0), voh).x * lb.coat * k;
        diff *= 1.0 - lb.e_coat;
        spec = spec * (1.0 - lb.e_coat) + vec3<f32>(coat);
    }
    split_frac = spec / max(diff + spec, vec3<f32>(1e-12));
    return (diff + spec) * nol;
}

// BSDF * NoL
fn eval_bsdf(s: Surface, lb: Lobes, v: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    return eval_bsdf_k(s, lb, v, l, 1.0);
}

fn g1(a2: f32, nov: f32) -> f32 {
    return 2.0 * nov / (nov + sqrt(a2 + (1.0 - a2) * nov * nov));
}

fn pdf_bsdf(s: Surface, lb: Lobes, v: vec3<f32>, l: vec3<f32>) -> f32 {
    let nol = dot(s.n, l);
    if (nol <= 0.0) { return 0.0; }
    let nov = max(dot(s.n, v), 1e-4);
    let h = normalize(v + l);
    let noh = max(dot(s.n, h), 0.0);
    let voh = max(dot(v, h), 1e-6);
    let dv = g1(lb.a2, nov) * voh * d_ggx(lb.a2, noh) / nov;
    let ps = dv / (4.0 * voh);
    let pd = nol / PI;
    var pc = 0.0;
    if (has(F_COAT) && lb.coat > 0.0) {
        let ca2 = lb.coat_alpha * lb.coat_alpha;
        pc = g1(ca2, nov) * d_ggx(ca2, noh) / (4.0 * nov);
    }
    // The delta transmission lobe has no density here: its share of samples only lowers the others'.
    return lb.p_coat * pc + (1.0 - lb.p_coat - lb.p_trans) * (lb.p_spec * ps + (1.0 - lb.p_spec) * pd);
}

fn basis(n: vec3<f32>) -> mat3x3<f32> {
    let sgn = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (sgn + n.z);
    let b = n.x * n.y * a;
    let t = vec3<f32>(1.0 + sgn * n.x * n.x * a, sgn * b, -sgn * n.x);
    let bt = vec3<f32>(b, sgn + n.y * n.y * a, -n.y);
    return mat3x3<f32>(t, bt, n);
}

// Heitz 2018 VNDF sampling (isotropic), returns world-space half vector.
fn sample_vndf(v: vec3<f32>, tbn: mat3x3<f32>, alpha: f32, u1: f32, u2: f32) -> vec3<f32> {
    let vl = transpose(tbn) * v;
    let vh = normalize(vec3<f32>(alpha * vl.x, alpha * vl.y, vl.z));
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1 = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-vh.y, vh.x, 0.0) / sqrt(lensq), lensq > 0.0);
    let t2 = cross(vh, t1);
    let r = sqrt(u1);
    let phi = 2.0 * PI * u2;
    let p1 = r * cos(phi);
    var p2 = r * sin(phi);
    let s = 0.5 * (1.0 + vh.z);
    p2 = (1.0 - s) * sqrt(1.0 - p1 * p1) + s * p2;
    let nh = p1 * t1 + p2 * t2 + sqrt(max(0.0, 1.0 - p1 * p1 - p2 * p2)) * vh;
    let hl = normalize(vec3<f32>(alpha * nh.x, alpha * nh.y, max(0.0, nh.z)));
    return tbn * hl;
}

fn sample_bsdf(s: Surface, lb: Lobes, v: vec3<f32>) -> vec3<f32> {
    let tbn = basis(s.n);
    sampled_trans = false;
    if (has(F_COAT) || has(F_GLASS)) {
        let u = rnd();
        if (u < lb.p_coat) {
            sampled_spec = true;
            return reflect(-v, sample_vndf(v, tbn, lb.coat_alpha, rnd(), rnd()));
        }
        if (u < lb.p_coat + lb.p_trans) {
            // Refraction through a microfacet normal (Walter et al. 2007; smooth glass: the shading normal).
            sampled_trans = true;
            sampled_spec = false;
            var h = s.n;
            if (s.roughness > 0.05) { h = sample_vndf(v, tbn, s.roughness * s.roughness, rnd(), rnd()); }
            let t = refract(-v, h, select(s.ior, 1.0 / s.ior, s.front));
            // Total internal reflection: the ray stays inside.
            return select(t, reflect(-v, h), all(t == vec3<f32>(0.0)));
        }
    }
    sampled_spec = rnd() < lb.p_spec;
    if (sampled_spec) {
        let h = sample_vndf(v, tbn, s.roughness * s.roughness, rnd(), rnd());
        return reflect(-v, h);
    }
    let r = sqrt(rnd());
    let phi = 2.0 * PI * rnd();
    return tbn * vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(0.0, 1.0 - r * r)));
}

// ------------------------------------------------------------------------------------------------
// Environment

fn env_uv(d: vec3<f32>) -> vec2<f32> {
    let u = atan2(d.x, -d.z) / (2.0 * PI) + 0.5;
    let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
    return vec2<f32>(u, v);
}

fn env_radiance(d: vec3<f32>) -> vec3<f32> {
    if (P.env.x < 0.5) { return P.sky_color.rgb; }
    return textureSampleLevel(env_tex, samp, env_uv(d), 0.0).rgb;
}

fn env_pdf(d: vec3<f32>) -> f32 {
    if (P.env.x < 0.5) { return 1.0 / (4.0 * PI); }
    let w = u32(P.env.y); let h = u32(P.env.z);
    let uv = env_uv(d);
    let x = min(u32(uv.x * f32(w)), w - 1u);
    let y = min(u32(uv.y * f32(h)), h - 1u);
    let row_p = env_cdf[y] - select(0.0, env_cdf[y - 1u], y > 0u);
    let base = h + y * w;
    let col_p = env_cdf[base + x] - select(0.0, env_cdf[base + x - 1u], x > 0u);
    let sin_t = sin((f32(y) + 0.5) / f32(h) * PI);
    return row_p * col_p * f32(w) * f32(h) / (2.0 * PI * PI * max(sin_t, 1e-4));
}

fn lower_bound(start: u32, count: u32, x: f32) -> u32 {
    var lo = 0u;
    var hi = count;
    while (lo < hi) {
        let mid = (lo + hi) / 2u;
        if (env_cdf[start + mid] < x) { lo = mid + 1u; } else { hi = mid; }
    }
    return min(lo, count - 1u);
}

fn sample_env() -> vec3<f32> {
    if (P.env.x < 0.5) {
        let z = 1.0 - 2.0 * rnd();
        let r = sqrt(max(0.0, 1.0 - z * z));
        let phi = 2.0 * PI * rnd();
        return vec3<f32>(r * cos(phi), z, r * sin(phi));
    }
    let w = u32(P.env.y); let h = u32(P.env.z);
    let y = lower_bound(0u, h, rnd());
    let x = lower_bound(h + y * w, w, rnd());
    let u = (f32(x) + rnd()) / f32(w);
    let v = (f32(y) + rnd()) / f32(h);
    let theta = v * PI;
    let phi = (u - 0.5) * 2.0 * PI;
    return vec3<f32>(sin(theta) * sin(phi), cos(theta), -sin(theta) * cos(phi));
}

fn power_heuristic(a: f32, b: f32) -> f32 {
    let a2 = a * a;
    return a2 / max(a2 + b * b, 1e-20);
}

fn non_finite(v: vec3<f32>) -> bool {
    let b = bitcast<vec3<u32>>(v) & vec3<u32>(0x7fffffffu);
    return any(b >= vec3<u32>(0x7f800000u));
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn offset_origin(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return p + n * (1e-4 * max(1.0, max(abs(p.x), max(abs(p.y), abs(p.z)))));
}

// What a BSDF-sampled ray (solid-angle pdf `pdf`; 0 = the camera ray, no MIS) that left the scene brings back: the
// environment and the directional lights' disks, MIS-weighted against their sampling in `direct_light`.
fn sky_mis(d: vec3<f32>, pdf: f32) -> vec3<f32> {
    var wgt = 1.0;
    if (pdf > 0.0 && P.counts2.y != 0u) {
        wgt = power_heuristic(pdf, env_pdf(d));
    }
    var c = env_radiance(d) * wgt;
    if (has(F_AERIAL) && pdf <= 0.0) { c = aerial_light(d); }
    // BSDF rays that fall inside a directional light's cone see its disk (MIS vs cone sampling).
    if (pdf > 0.0) {
        for (var i = 0u; i < P.counts2.x; i++) {
            let li = lights[i];
            if (dot(d, -li.dir.xyz) >= li.dir.w) {
                let omega = 2.0 * PI * (1.0 - li.dir.w);
                c += (li.color.rgb / omega) * power_heuristic(pdf, 1.0 / omega);
            }
        }
    }
    return c;
}

// MIS weight of emission that a BSDF-sampled ray (pdf `pdf`) hit, against emitter sampling in `direct_light`.
// Without sampled emitters (only material functions emit, e.g. facade windows) the BSDF ray is the only estimator.
// It weighs by emitted power alone, as emitter sampling's MIS does with or without light LOD (sample_emitter_ris).
fn emit_mis(hit: Hit, d: vec3<f32>, pdf: f32) -> f32 {
    // Procedural surfaces are not light-sampled (no emitter entries): the BSDF ray is the only estimator.
    if (is_proc(hit)) { return 1.0; }
    if (lt_on()) {
        // The light tree's pmf where the ray left (the vertex that sampled emitters), per area of the hit triangle.
        if (P.counts.y == 0u) { return 1.0; }
        let sel = emitter_sel(hit.tri, mis_x, mis_n);
        if (!(sel > 0.0)) { return 1.0; }
        let crt = cross(tri_verts[hit.tri * 3u + 1u].xyz, tri_verts[hit.tri * 3u + 2u].xyz);
        let at = 0.5 * length(crt);
        return power_heuristic(pdf, sel / max(at, 1e-12) * hit.t * hit.t / max(abs(dot(normalize(crt), d)), 1e-6));
    }
    if (P.counts.y == 0u || P.misc.y <= 0.0 || emitter_culled(hit.tri)) { return 1.0; }
    let cr = cross(tri_verts[hit.tri * 3u + 1u].xyz, tri_verts[hit.tri * 3u + 2u].xyz);
    let area = 0.5 * length(cr);
    let cos_l = abs(dot(normalize(cr), d));
    let ha = attrs[hit.tri];
    let mat = materials[bitcast<u32>(ha.uv.w)];
    let pdf_a = luminance(mat.emissive.rgb) * obj_power(ha) / P.emit_power.x;
    let pdf_w = pdf_a * hit.t * hit.t / max(cos_l, 1e-6);
    if (!has(F_PEMIT) || ha.x.z == 0.0) { return power_heuristic(pdf, pdf_w); }
    // Particles (x.z != 0): a nominal area (> 0) is what the CDF weighed them by, the point is uniform over the actual
    // area; without one (< 0) they are not light-sampled and the BSDF ray is the only estimator.
    if (ha.x.z < 0.0) { return 1.0; }
    return power_heuristic(pdf, pdf_w * ha.x.z / max(area, 1e-12));
}

// Light LOD factor of emitter triangle ti (lib.rs emitter_lod): the emitter CDF's weights carry it, so every pdf of an
// emitter point does too. Moving triangles have none (lib.rs EmitterTri::moves).
fn emitter_lod(ti: u32) -> f32 {
    let v0 = tri_verts[ti * 3u];
    if (P.light_lod.w <= 0.0 || (bitcast<u32>(v0.w) & TRI_DYNAMIC) != 0u) { return 1.0; }
    let c = v0.xyz + (tri_verts[ti * 3u + 1u].xyz + tri_verts[ti * 3u + 2u].xyz) / 3.0;
    let k = P.light_lod.w / max(distance(c, P.light_lod.xyz), P.light_lod.w);
    return k * k;
}

// Live: emitters farther than P.emit_power.y from the light LOD's reference are never proposed (lib.rs emitter_lod),
// so BSDF rays that hit them keep their whole light (emit_mis), unless they lie in the view cone (P.lod_view) within
// P.emit_power.z: those light what the camera sees. The same centroid and test as lib.rs EmitterCull::culls.
fn emitter_culled(ti: u32) -> bool {
    if (P.emit_power.y <= 0.0 || P.light_lod.w <= 0.0) { return false; }
    let v0 = tri_verts[ti * 3u];
    if ((bitcast<u32>(v0.w) & TRI_DYNAMIC) != 0u) { return false; }
    let c = v0.xyz + (tri_verts[ti * 3u + 1u].xyz + tri_verts[ti * 3u + 2u].xyz) / 3.0;
    let d = distance(c, P.light_lod.xyz);
    if (d <= P.emit_power.y) { return false; }
    return !(d <= P.emit_power.z && dot(c - P.light_lod.xyz, P.lod_view.xyz) >= P.lod_view.w * d);
}

fn emitter_pdf_area(tri: u32, emissive: vec3<f32>) -> f32 {
    // Selection probability (area * luminance / total) times 1/area => luminance / total.
    if (P.counts.y == 0u || P.misc.y <= 0.0) { return 0.0; }
    return luminance(emissive) / P.misc.y;
}

// ------------------------------------------------------------------------------------------------
// Direct lighting (NEE)

struct LS { c: vec3<f32>, l: vec3<f32>, sl: vec3<f32>, dist: f32 };

// Point / spot light seen from x: punctual intensity x attenuation (unshadowed), direction to the light centre `l`,
// and a shadow ray toward a point sampled on the light's sphere (soft shadows).
fn local_light(li: Light, x: vec3<f32>) -> LS {
    if (u32(li.pos.w) == 3u) {
        // Rect (UE RectLight): uniform point on the rectangle; luminance x area x cos_light / d^2 (Lambertian),
        // cut beyond the barn-door angle.
        let n = li.dir.xyz;
        let ux = li.ext.xyz;
        let uy = normalize(cross(n, ux)) * li.dir.w;
        let lp = li.pos.xyz + ux * (2.0 * rnd() - 1.0) + uy * (2.0 * rnd() - 1.0);
        let dl = lp - x;
        let d2 = max(dot(dl, dl), 1e-8);
        let dist = sqrt(d2);
        let l = dl / dist;
        let cos_l = dot(n, -l);
        var k = 0.0;
        if (cos_l > 0.0) {
            let area = 4.0 * length(ux) * li.dir.w;
            k = area * cos_l / d2 * smoothstep(li.x.z - 0.04, li.x.z + 0.04, cos_l);
            let range = li.color.w;
            if (range > 0.0) {
                let q = d2 / (range * range);
                let win = clamp(1.0 - q * q, 0.0, 1.0);
                k *= win * win;
            }
        }
        return LS(li.color.rgb * k, l, l, dist);
    }
    var lp = li.pos.xyz;
    let radius = li.dir.w;
    if (radius > 0.0) {
        let z = 1.0 - 2.0 * rnd();
        let r = sqrt(max(0.0, 1.0 - z * z));
        let phi = 2.0 * PI * rnd();
        lp += radius * vec3<f32>(r * cos(phi), r * sin(phi), z);
    }
    let dl = li.pos.xyz - x;
    let d2 = max(dot(dl, dl), 1e-8);
    let l = dl * inverseSqrt(d2);
    var atten = 1.0 / d2;
    let range = li.color.w;
    if (range > 0.0) {
        let q = d2 / (range * range);
        let win = clamp(1.0 - q * q, 0.0, 1.0);
        atten *= win * win;
    }
    if (u32(li.pos.w) == 2u) {
        let m = clamp((dot(-l, li.dir.xyz) - li.ext.x) * li.ext.y, 0.0, 1.0);
        atten *= m * m;
    }
    let ls = lp - x;
    let dist = max(length(ls), 1e-6);
    return LS(li.color.rgb * atten, l, ls / dist, dist);
}

// Local lights (after the directional ones; unshadowed before shadowed, P.counts2.w = first shadowed). At camera-ray
// hits every unshadowed light is evaluated exactly, as UE's deferred lights are (no sampling noise), and the shadowed
// ones too when there are few (one shadow ray each). Otherwise: resampled importance sampling (Talbot 2005) of RIS_M
// uniform candidates by unshadowed contribution, then one shadow ray. `vol` = in-scattering in the fog (phase
// function) instead of the surface BSDF. Surface results add their specular part to `dl_spec`.
fn local_lights(s: Surface, lb: Lobes, v: vec3<f32>, o: vec3<f32>, x: vec3<f32>, vol: bool, d: vec3<f32>) -> vec3<f32> {
    let nd = P.counts2.x;
    let nl = P.counts.x - nd;
    if (nl == 0u) { return vec3<f32>(0.0); }
    var sum = vec3<f32>(0.0);
    var first = nd;
    var count = nl;
    if (!vol && (seen_directly() || nl <= RIS_EXACT)) {
        let ns = P.counts2.w;
        let n_sh = P.counts.x - ns;
        let end = select(ns, P.counts.x, n_sh <= RIS_EXACT || nl <= RIS_EXACT);
        // In groups of LIGHT_GROUP (spatially sorted; after the lights, a sphere holding all their light per group:
        // lib.rs lights_with_groups): a group x is outside of is skipped whole.
        var i = nd;
        loop {
            if (i >= end) { break; }
            let gs = lights[P.counts.x + (i - nd) / LIGHT_GROUP].pos;
            let gend = min(i + LIGHT_GROUP, end);
            if (distance(x, gs.xyz) > gs.w) {
                i = gend;
                continue;
            }
            for (; i < gend; i++) {
                let li = lights[i];
                // Cheap rejects before sampling the light: beyond its attenuation range (the window is zero there), or
                // entirely behind the surface.
                let dc = li.pos.xyz - x;
                let reach = li.color.w + select(li.dir.w, length(li.ext.xyz) + li.dir.w, u32(li.pos.w) == 3u);
                if (li.color.w > 0.0 && dot(dc, dc) > reach * reach) { continue; }
                if (u32(li.pos.w) != 3u && dot(dc, s.ng) < -li.dir.w) { continue; } // eval_bsdf rejects it anyway
                let ls = local_light(li, x);
                if (all(ls.c <= vec3<f32>(0.0))) { continue; }
                let f = eval_bsdf_k(s, lb, v, ls.l, li.x.y);
                if (any(f > vec3<f32>(0.0))) {
                    let vis = light_vis(li.ext.w >= 0.5, o, ls.sl, ls.dist * 0.999);
                    if (any(vis > vec3<f32>(0.0))) {
                        sum += ls.c * f * gtint(vis);
                        dl_spec += ls.c * f * gtint(vis) * split_frac;
                    }
                }
            }
        }
        if (end == P.counts.x) { return sum; }
        first = ns;
        count = n_sh;
    }
    // Target: unshadowed intensity x cosine (cheap; the full BSDF / phase is applied to the chosen light only).
    let m_cand = select(3u, RIS_M, seen_directly());
    var wsum = 0.0;
    var pick = 0u;
    var pick_ph = 0.0;
    var pick_ls: LS;
    for (var k = 0u; k < m_cand; k++) {
        let idx = first + min(u32(rnd() * f32(count)), count - 1u);
        let ls = local_light(lights[idx], x);
        var ph = luminance(ls.c);
        if (vol) { ph *= lights[idx].x.x; } else { ph *= max(dot(s.n, ls.l), 0.0) + 0.05 * max(dot(s.ng, ls.l), 0.0); }
        if (ph <= 0.0) { continue; }
        wsum += ph;
        if (rnd() * wsum < ph) {
            pick = idx;
            pick_ph = ph;
            pick_ls = ls;
        }
    }
    if (pick_ph > 0.0) {
        var c: vec3<f32>;
        let lk = lights[pick].x;
        if (vol) { c = pick_ls.c * hg_phase(P.vol.y, dot(pick_ls.l, d)) * lk.x; } else { c = pick_ls.c * eval_bsdf_k(s, lb, v, pick_ls.l, lk.y); }
        if (any(c > vec3<f32>(0.0))) {
            let vis = light_vis(lights[pick].ext.w >= 0.5, o, pick_ls.sl, pick_ls.dist * 0.999);
            if (any(vis > vec3<f32>(0.0))) {
                let r = c * gtint(vis) * (wsum * f32(count) / (f32(m_cand) * pick_ph));
                sum += r;
                if (!vol) { dl_spec += r * split_frac; }
            }
        }
    }
    return sum;
}

// An emitter sample: radiance, direction, distance, its source pdf (solid angle; MIS weights) and its contribution
// weight `w` (1 / pdf, or the RIS weight).
struct ES { le: vec3<f32>, l: vec3<f32>, dist: f32, pdf_w: f32, w: f32 };

// Emitted radiance of triangle ti at (b1, b2) toward -l (emissive map, animation, material function, node power).
fn emitter_radiance(ti: u32, b1: f32, b2: f32, l: vec3<f32>) -> vec3<f32> {
    let a = attrs[ti];
    let mi = bitcast<u32>(a.uv.w);
    let m = materials[mi];
    // 5 cm footprint: a blurred texel is plenty for lighting and keeps NEE cheap.
    let pw = obj_power(a);
    var le = emission(m, mesh_uv(a, m, b1, b2), mesh_lod(a, m) + log2(0.05)) * pw;
    if (has(F_FUNC) && m.tex1.w > 0) {
        var mb = mat_base(m);
        mb.emissive *= pw;
        le = emissive_material_function(m.tex1.w - 1, mat_in_at(ti, b1, b2, -l, log2(0.05)), m, mb).emissive;
    }
    return le;
}

// Camera-ray hits: RIS over `m` candidates from the emitter CDF, resampled by emitted luminance x geometry term at x
// (with m > 1 the receiving side's cosine is the surface's BSDF x cosine toward the candidate, `eval_bsdf`: a glossy
// surface then picks the lights in its specular lobe, not only the bright and near ones. The string lights on the
// crowd's moving van roofs were sparse sparkles; live specular in motion 0.88 -> 0.91 of our offline render, layer B
// Bistro 3.47 -> 3.34. The target is free to choose: the estimate stays unbiased, tools/unbiased-check.ts within 0.1 %)
// (the CDF knows emitted power, and with light LOD the distance to the camera; without it, it mostly proposes signs
// far down the street). The target leaves the light LOD out, so the light LOD only changes which candidates are
// proposed. MIS against the BSDF rays weighs by power alone, the CDF's pdf without light LOD (emit_mis): the weights
// m_nee + m_bsdf still sum to one, so the combination stays unbiased, and which of the two carries which light is the
// same with or without light LOD. With light LOD every surface vertex samples this way (deeper ones with 2
// candidates): a single draw from the LOD'd CDF would make far emitters rare and heavy, which the radiance cache's
// clamps cut.
fn sample_emitter_ris(x: vec3<f32>, n: vec3<f32>, m: u32, sf: Surface, lb: Lobes, v: vec3<f32>) -> ES {
    var r = ES(vec3<f32>(0.0), vec3<f32>(0.0, 1.0, 0.0), 0.0, 0.0, 0.0);
    var wsum = 0.0;
    var pick_ti = 0u;
    var pick_b = vec2<f32>(0.0);
    var pick_ph = 0.0;
    var pick_e = vec4<f32>(0.0);
    var pick_sel = 0.0;
    for (var k = 0u; k < m; k++) {
        let pk = emitter_draw(x, n);
        let e = pk.e;
        var b1 = rnd();
        var b2 = rnd();
        if (pk.p <= 0.0) { continue; }
        if (b1 + b2 > 1.0) { b1 = 1.0 - b1; b2 = 1.0 - b2; }
        let ti = bitcast<u32>(e.x);
        let e1 = tri_verts[ti * 3u + 1u].xyz;
        let e2 = tri_verts[ti * 3u + 2u].xyz;
        let lp = tri_verts[ti * 3u].xyz + e1 * b1 + e2 * b2;
        let ln = normalize(cross(e1, e2));
        let dl = lp - x;
        let d2 = max(dot(dl, dl), 1e-8);
        let l = dl * inverseSqrt(d2);
        var g = abs(dot(ln, l)) * max(dot(n, l), 0.0) / d2;
        if (m > 1u) { g = abs(dot(ln, l)) * luminance(eval_bsdf(sf, lb, v, l)) / d2; }
        // target (emitted luminance x G) over source pdf (luminance x light LOD / total, per area) = total x G / LOD;
        // with the tree the source pdf is its selection probability / area (weight / area = luminance x power)
        let lk = emitter_lod(ti);
        var w = select(P.misc.y * g / lk, pk.w * g / pk.p, lt_on());
        // Particle emitters (e.w < 0): weighed by a nominal area -e.w, sampled over the actual one (0 when not shown).
        var ae = e.w;
        if (has(F_PEMIT) && e.w < 0.0) {
            ae = 0.5 * length(cross(e1, e2));
            if (ae <= 0.0) { continue; }
            w *= ae / -e.w;
        }
        if (w <= 0.0) { continue; }
        wsum += w;
        if (rnd() * wsum < w) {
            pick_ti = ti;
            pick_b = vec2<f32>(b1, b2);
            pick_ph = pk.w / (abs(e.w) * lk) * g;
            pick_e = vec4<f32>(e.x, e.y, e.z / lk, ae);
            pick_sel = pk.p;
        }
    }
    if (pick_ph <= 0.0) { return r; }
    let ti = pick_ti;
    let e1 = tri_verts[ti * 3u + 1u].xyz;
    let e2 = tri_verts[ti * 3u + 2u].xyz;
    let lp = tri_verts[ti * 3u].xyz + e1 * pick_b.x + e2 * pick_b.y;
    let ln = normalize(cross(e1, e2));
    let dl = lp - x;
    let d2 = dot(dl, dl);
    let dist = sqrt(d2);
    let l = dl / dist;
    var cos_l = dot(ln, -l);
    let m_card = materials[bitcast<u32>(attrs[ti].uv.w)].tex1.z == MAT_EMITTER_CARD;
    if (!m_card) { cos_l = abs(cos_l); }
    if (cos_l <= 0.0) { return r; }
    r.le = emitter_radiance(ti, pick_b.x, pick_b.y, l);
    r.l = l;
    r.dist = dist;
    // MIS by the source pdf: power alone without the tree (emit_mis), the tree's pmf with it
    r.pdf_w = select(pick_e.z / P.emit_power.x / pick_e.w * d2 / cos_l, pick_sel / pick_e.w * d2 / cos_l, lt_on());
    r.w = (wsum / f32(m)) / pick_ph * cos_l / d2;
    return r;
}

// One emissive triangle point by the emitter CDF (area x emissive-factor luminance); radiance includes the emissive
// map (coarse mip) and animation, so signs spill their own colours.
fn sample_emitter(x: vec3<f32>, n: vec3<f32>) -> ES {
    var r = ES(vec3<f32>(0.0), vec3<f32>(0.0, 1.0, 0.0), 0.0, 0.0, 0.0);
    let pk = emitter_draw(x, n);
    let e = pk.e;
    if (pk.p <= 0.0) { return r; } // switched off (power 0)
    let ti = bitcast<u32>(e.x);
    let v0 = tri_verts[ti * 3u].xyz;
    let e1 = tri_verts[ti * 3u + 1u].xyz;
    let e2 = tri_verts[ti * 3u + 2u].xyz;
    var b1 = rnd();
    var b2 = rnd();
    if (b1 + b2 > 1.0) { b1 = 1.0 - b1; b2 = 1.0 - b2; }
    let lp = v0 + e1 * b1 + e2 * b2;
    let cr = cross(e1, e2);
    let area = 0.5 * length(cr);
    let ln = cr / (2.0 * area);
    let dl = lp - x;
    let d2 = dot(dl, dl);
    let dist = sqrt(d2);
    let l = dl / dist;
    let a = attrs[ti];
    let mi = bitcast<u32>(a.uv.w);
    let m = materials[mi];
    var cos_l = dot(ln, -l);
    if (m.tex1.z != MAT_EMITTER_CARD) { cos_l = abs(cos_l); }
    if (cos_l <= 0.0 || (has(F_PEMIT) && !(cos_l > 0.0))) { return r; } // also a particle not shown (no area: NaN)
    // 5 cm footprint: a blurred texel is plenty for lighting and keeps NEE cheap.
    let pw = obj_power(a);
    r.le = emission(m, mesh_uv(a, m, b1, b2), mesh_lod(a, m) + log2(0.05)) * pw;
    if (has(F_FUNC) && m.tex1.w > 0) {
        var mb = mat_base(m);
        mb.emissive *= pw;
        r.le = emissive_material_function(m.tex1.w - 1, mat_in_at(ti, b1, b2, -l, log2(0.05)), m, mb).emissive;
    }
    r.l = l;
    r.dist = dist;
    r.pdf_w = pk.p / area * d2 / cos_l;
    r.w = 1.0 / r.pdf_w;
    return r;
}

fn hg_phase(g: f32, c: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * c;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(d));
}

fn direct_light(s: Surface, lb: Lobes, v: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    dl_spec = vec3<f32>(0.0);
    let o = offset_origin(s.p, s.ng);
    // Directional lights: uniformly sampled cone (soft shadows), MIS against BSDF sampling.
    for (var i = 0u; i < P.counts2.x; i++) {
        let li = lights[i];
        let cos_max = li.dir.w;
        let to_light = -li.dir.xyz;
        let cos_t = 1.0 - rnd() * (1.0 - cos_max);
        let sin_t = sqrt(max(0.0, 1.0 - cos_t * cos_t));
        let phi = 2.0 * PI * rnd();
        let tb = basis(to_light);
        let l = tb * vec3<f32>(sin_t * cos(phi), sin_t * sin(phi), cos_t);
        // A light gone black (a moon that has set, the sun at night) costs neither its BSDF nor its shadow ray. (After
        // the cone's random numbers: every other light sees the same ones as before.)
        if (all(li.color.rgb <= vec3<f32>(0.0))) { continue; }
        let f = eval_bsdf_k(s, lb, v, l, li.x.y);
        if (any(f > vec3<f32>(0.0))) {
            let vis = light_vis(li.ext.w >= 0.5, o, l, INF);
            if (any(vis > vec3<f32>(0.0))) {
                let pdf_l = 1.0 / (2.0 * PI * (1.0 - cos_max));
                let frac = split_frac;
                let wgt = power_heuristic(pdf_l, pdf_bsdf(s, lb, v, l));
                sum += li.color.rgb * f * wgt * gtint(vis);
                dl_spec += li.color.rgb * f * wgt * gtint(vis) * frac;
            }
        }
    }
    if (cur_depth == 0u) {
        dl_det += sum;
        dl_det_spec += dl_spec;
    }
    sum += local_lights(s, lb, v, o, s.p, false, vec3<f32>(0.0));
    // Emissive triangles (incl. rect lights)
    mis_x = o;
    mis_n = s.n;
    if (P.counts.y > 0u && restir_here()) {
        sum += restir_emitters(s, lb, v, o);
    } else if (P.counts.y > 0u) {
        var es: ES;
        // With the light tree its candidates are already near the point: P.ltree.y of them at camera hits, one elsewhere.
        if (lt_on()) { es = sample_emitter_ris(o, s.n, select(1u, u32(P.ltree.w), cur_depth == 0u), s, lb, v); } else if (seen_directly()) { es = sample_emitter_ris(o, s.n, select(EMIT_RIS_M / 2u, EMIT_RIS_M, cur_depth == 0u), s, lb, v); } else if (P.light_lod.w > 0.0) { es = sample_emitter_ris(o, s.n, 2u, s, lb, v); } else { es = sample_emitter(o, s.n); }
        if (es.pdf_w > 0.0) {
            let f = eval_bsdf(s, lb, v, es.l);
            if (any(f > vec3<f32>(0.0)) && !occluded_emitter(o, es.l, es.dist * 0.999)) {
                let frac = split_frac;
                let wgt = power_heuristic(es.pdf_w, pdf_bsdf(s, lb, v, es.l));
                sum += es.le * f * wgt * es.w;
                dl_spec += es.le * f * wgt * es.w * frac;
            }
        }
    }
    // Environment
    if (P.counts2.y != 0u && (P.env.x > 0.5 || luminance(P.sky_color.rgb) > 0.0)) {
        let l = sample_env();
        let f = eval_bsdf(s, lb, v, l);
        if (any(f > vec3<f32>(0.0))) {
            let pdf_e = env_pdf(l);
            if (pdf_e > 0.0 && !occluded_sky(o, l)) {
                let frac = split_frac;
                let wgt = power_heuristic(pdf_e, pdf_bsdf(s, lb, v, l));
                sum += env_radiance(l) * f * wgt / pdf_e * gtint(shadow_tint);
                dl_spec += env_radiance(l) * f * wgt / pdf_e * gtint(shadow_tint) * frac;
            }
        }
    }
    return sum;
}

// ------------------------------------------------------------------------------------------------
// The sky's aerial perspective (F_AERIAL): the atmosphere between a viewer and a surface. Its extinction is the
// SkyAtmosphere's own (sky.rs Atmosphere::earth): Rayleigh 5.8 / 13.6 / 33.1 per 1000 km and Mie 4.0 x scale + 0.44 per
// 1000 km at sea level (y = 0), thinning with scale heights of 8 km and 1.2 km, times P.aerial.x. The light it
// scatters toward the viewer is taken as the sky's own radiance at the horizon in the ray's direction, (1 - T) of it:
// a horizontal ray through the same air ends on that horizon, so far land fades into the sky behind it.
fn aerial_od(y0: f32, dy: f32, t: f32, h: f32) -> f32 {
    // the integral of exp(-(y0 + dy s) / h) over s < t
    let a = exp(-max(y0, -500.0) / h);
    let k = dy / h;
    if (abs(k * t) < 1e-4) { return a * t; }
    return a * (1.0 - exp(clamp(-k * t, -80.0, 80.0))) / k;
}

fn aerial_tr(o: vec3<f32>, d: vec3<f32>, t: f32) -> vec3<f32> {
    let rayleigh = vec3<f32>(5.802e-6, 13.558e-6, 33.1e-6) * P.aerial.y;
    let mie = 3.996e-6 * P.aerial.z + 0.444e-6;
    return exp(-P.aerial.x * (rayleigh * aerial_od(o.y, d.y, t, 8000.0) + mie * aerial_od(o.y, d.y, t, 1200.0)));
}

// The sky a little above the horizon (2.3 degrees: clear of the baked sky's fade to its black lower half), or where
// the ray points if higher: the haze's light, and what camera rays see of the sky near the horizon with F_AERIAL, so
// that the hazed land meets the sky without the fade's dark seam.
fn aerial_light(d: vec3<f32>) -> vec3<f32> {
    return env_radiance(normalize(vec3<f32>(d.x, max(d.y, 0.04), d.z)));
}

// Exponential height fog: extinction density * exp(-falloff * (y - height)).

fn fog_base(o: vec3<f32>, d: vec3<f32>, t0: f32) -> f32 {
    return P.fog.x * exp(clamp(-P.fog.y * (o.y + d.y * t0 - P.fog.z), -80.0, 80.0));
}

// Optical depth along o + d t, t in [t0, t1].
fn fog_depth(o: vec3<f32>, d: vec3<f32>, t0: f32, t1: f32) -> f32 {
    let len = t1 - t0;
    if (len <= 0.0) { return 0.0; }
    let a = fog_base(o, d, t0);
    let k = P.fog.y * d.y;
    if (abs(k * len) < 1e-4) { return a * len; }
    return a * (1.0 - exp(clamp(-k * len, -80.0, 80.0))) / k;
}

// Single scattering along a camera ray segment: one distance sampled by optical depth, then lights (RIS), one
// emissive point and the directional lights, each with a shadow ray. `tr` = segment transmittance.
fn fog_scatter(o: vec3<f32>, d: vec3<f32>, t0: f32, t1: f32, tr: f32) -> vec3<f32> {
    let pe = 1.0 - tr;
    if (pe < 1e-4) { return vec3<f32>(0.0); }
    let tau = -log(max(1.0 - rnd() * pe, 1e-8));
    let a = fog_base(o, d, t0) * max(P.vol.z, 1e-3);
    let k = P.fog.y * d.y;
    var t = tau / max(a, 1e-12);
    if (abs(k) > 1e-6) { t = -log(max(1.0 - tau * k / a, 1e-8)) / k; }
    let x = o + d * (t0 + clamp(t, 0.0, t1 - t0));
    var ls = vec3<f32>(0.0);
    for (var i = 0u; i < P.counts2.x; i++) {
        let li = lights[i];
        let l = -li.dir.xyz;
        if (all(li.color.rgb <= vec3<f32>(0.0))) { continue; } // black: no shadow ray (direct_light)
        if (li.ext.w < 0.5 || !occluded(x, l, INF)) { ls += li.color.rgb * hg_phase(P.vol.y, dot(l, d)) * li.x.x; }
    }
    // One shadow ray: either the local lights (RIS) or one emissive point, chosen with equal odds when both exist.
    let has_l = P.counts.x > P.counts2.x;
    let has_e = P.counts.y > 0u;
    let both = has_l && has_e;
    let use_e = has_e && (!has_l || rnd() < 0.5);
    let wk = select(1.0, 2.0, both);
    if (use_e) {
        // Emitters picked by global power are mostly far away from a fog point: resample 4 candidates by their
        // unshadowed contribution here (RIS), one shadow ray for the winner.
        var wsum = 0.0;
        var pick: ES;
        var pick_t = 0.0;
        for (var c = 0u; c < 4u; c++) {
            mis_x = x;
            mis_n = vec3<f32>(0.0);
            let es = sample_emitter(x, vec3<f32>(0.0));
            if (es.pdf_w <= 0.0) { continue; }
            let t = luminance(es.le) * hg_phase(P.vol.y, dot(es.l, d));
            let w = t / es.pdf_w;
            if (w <= 0.0) { continue; }
            wsum += w;
            if (rnd() * wsum < w) {
                pick = es;
                pick_t = t;
            }
        }
        if (pick_t > 0.0 && !occluded_any(x, pick.l, pick.dist * 0.999)) {
            ls += wk * pick.le * hg_phase(P.vol.y, dot(pick.l, d)) * (wsum / (4.0 * pick_t));
        }
    } else if (has_l) {
        var dummy_s: Surface;
        var dummy_l: Lobes;
        ls += wk * local_lights(dummy_s, dummy_l, -d, x, x, true, d);
    }
    return P.vol.x * P.vol2.rgb * pe * ls;
}

// ------------------------------------------------------------------------------------------------
// Bounded participating media (F_MEDIA): `VolumeMesh` nodes, a box or sphere each, whose volume function gives
// extinction, albedo and emission at a point (docs/MATERIAL_FUNCTIONS.md, "Volume functions"). Paths sample them by
// delta tracking, shadow rays by ratio tracking (Novák et al. 2018); single and multiple scattering, emission.

struct VIn {
    p: vec3<f32>,      // world position, m
    lp: vec3<f32>,     // position in the volume's own space: the box [-0.5, 0.5]^3 / the sphere of radius 0.5
    time: f32,         // scene time, s
    custom: vec4<f32>, // the node's customData
};

struct VOut {
    extinction: f32,    // 1/m (clamped to the volume's majorant)
    albedo: vec3<f32>,  // scattering / extinction
    emission: vec3<f32>, // emitted radiance per metre (absorption x emitted radiance)
};

// @volume-functions-begin
fn volume_function(f: u32, i: VIn, p: array<vec4<f32>, 8>) -> VOut { return VOut(0.0, vec3<f32>(1.0), vec3<f32>(0.0)); }
// @volume-functions-end

fn medium_local(m: Medium, x: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(m.w0.xyz, x) + m.w0.w, dot(m.w1.xyz, x) + m.w1.w, dot(m.w2.xyz, x) + m.w2.w);
}

// The ray's (o + d t) parameter interval inside medium m (x > y: it misses).
fn medium_span(m: Medium, o: vec3<f32>, d: vec3<f32>) -> vec2<f32> {
    let lo = medium_local(m, o);
    let ld = vec3<f32>(dot(m.w0.xyz, d), dot(m.w1.xyz, d), dot(m.w2.xyz, d));
    if (m.a.y == 1u) {
        let a = dot(ld, ld);
        let b = dot(lo, ld);
        let disc = b * b - a * (dot(lo, lo) - 0.25);
        if (disc < 0.0 || a <= 0.0) { return vec2<f32>(1.0, 0.0); }
        let r = sqrt(disc);
        return vec2<f32>((-b - r) / a, (-b + r) / a);
    }
    let inv = 1.0 / select(ld, vec3<f32>(1e-20), abs(ld) < vec3<f32>(1e-20));
    let ta = (vec3<f32>(-0.5) - lo) * inv;
    let tb = (vec3<f32>(0.5) - lo) * inv;
    let lo_t = min(ta, tb);
    let hi_t = max(ta, tb);
    return vec2<f32>(max(max(lo_t.x, lo_t.y), lo_t.z), min(min(hi_t.x, hi_t.y), hi_t.z));
}

fn medium_eval(k: u32, x: vec3<f32>) -> VOut {
    let m = media[k];
    var i: VIn;
    i.p = x;
    i.lp = medium_local(m, x);
    i.time = P.time.x;
    i.custom = m.c;
    var r = volume_function(m.a.x, i, m.p);
    r.extinction = clamp(r.extinction, 0.0, m.b.x);
    return r;
}

struct Track { t: f32, albedo: vec3<f32>, g: f32, emission: vec3<f32> };

// Delta tracking along o + d t, t < tmax: the first real collision (t < 0: none) with its albedo and HG anisotropy,
// and the emission met on the way (emission / majorant at every tentative collision). The media are taken in the
// order the ray enters them (they do not overlap).
fn media_track(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> Track {
    var r = Track(-1.0, vec3<f32>(1.0), 0.0, vec3<f32>(0.0));
    let n = min(u32(P.time.w), 32u);
    var done = 0u;
    for (var it = 0u; it < n; it++) {
        var best = 0xffffffffu;
        var span = vec2<f32>(0.0);
        for (var k = 0u; k < n; k++) {
            if (((done >> k) & 1u) != 0u) { continue; }
            let sp = medium_span(media[k], o, d);
            let a = max(sp.x, 0.0);
            let b = min(sp.y, tmax);
            if (a < b && (best == 0xffffffffu || a < span.x)) {
                best = k;
                span = vec2<f32>(a, b);
            }
        }
        if (best == 0xffffffffu) { break; }
        done |= 1u << best;
        let mj = media[best].b.x;
        if (mj <= 0.0) { continue; }
        var t = span.x;
        loop {
            t -= log(1.0 - rnd()) / mj;
            if (t >= span.y) { break; }
            let v = medium_eval(best, o + d * t);
            r.emission += v.emission / mj;
            if (rnd() * mj < v.extinction) {
                r.t = t;
                r.albedo = v.albedo;
                r.g = media[best].b.y;
                return r;
            }
        }
    }
    return r;
}

// Transmittance through the media along o + d t, t < tmax (ratio tracking; an unbiased estimate).
fn media_tr(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> f32 {
    var tr = 1.0;
    let n = min(u32(P.time.w), 32u);
    for (var k = 0u; k < n; k++) {
        let sp = medium_span(media[k], o, d);
        let b = min(sp.y, tmax);
        let mj = media[k].b.x;
        if (max(sp.x, 0.0) >= b || mj <= 0.0) { continue; }
        var t = max(sp.x, 0.0);
        loop {
            t -= log(1.0 - rnd()) / mj;
            if (t >= b) { break; }
            tr *= 1.0 - medium_eval(k, o + d * t).extinction / mj;
            if (tr < 1e-4) { return 0.0; }
        }
    }
    return tr;
}

// Live camera rays through media: transmittance and in-scattered light by ray marching (MEDIA_STEPS jittered steps
// per volume) instead of delta tracking, so the surface behind stays the pixel's first hit for the temporal filter.
// In-scattering takes the radiance cache's volume voxels (the mean incident radiance: isotropic scattering) plus the
// medium's emission. The voxels are trained by `media_train`, a pass of its own (point_radiance inlined here would
// copy surface shading and every material function into the kernel: minutes of compile on Kabukichō).
const MEDIA_STEPS: u32 = 8u;
const MEDIA_STEPS_INDIRECT: u32 = 4u; // later segments: transmittance only
const MEDIA_TRAIN_POINTS: u32 = 4096u;
struct March { tr: f32, light: vec3<f32> };

fn media_march(o: vec3<f32>, d: vec3<f32>, tmax: f32, camera: bool) -> March {
    var r = March(1.0, vec3<f32>(0.0));
    let n = min(u32(P.time.w), 32u);
    let u = rnd();
    var done = 0u;
    for (var it = 0u; it < n; it++) {
        var best = 0xffffffffu;
        var span = vec2<f32>(0.0);
        for (var k = 0u; k < n; k++) {
            if (((done >> k) & 1u) != 0u) { continue; }
            let sp = medium_span(media[k], o, d);
            let a = max(sp.x, 0.0);
            let b = min(sp.y, tmax);
            if (a < b && (best == 0xffffffffu || a < span.x)) {
                best = k;
                span = vec2<f32>(a, b);
            }
        }
        if (best == 0xffffffffu) { break; }
        done |= 1u << best;
        let steps = select(MEDIA_STEPS_INDIRECT, MEDIA_STEPS, camera);
        let dt = (span.y - span.x) / f32(steps);
        for (var k = 0u; k < steps; k++) {
            let x = o + d * (span.x + (f32(k) + u) * dt);
            let v = medium_eval(best, x);
            let st = v.extinction;
            let ti = exp(-st * dt);
            // Over the step (constant medium): emitted + scattered radiance x (1 - e^-st dt) / st.
            let w = select(dt, (1.0 - ti) / st, st > 1e-6);
            var lin = vec3<f32>(0.0);
            if (camera && cache_on()) {
                // Looked up at a point jittered within a voxel: the accumulation filters the voxels instead of
                // showing them as blocks. A jitter out of the medium finds a voxel no training point reached (dark):
                // then the voxel of the point itself. Near the boundary that lost light, at every scattering order
                // through the training's feedback (volume/furnace-live 0.931 of 1 without the emission clamp).
                let vs = exp2(floor(log2(max(length(x - P.cam_pos.xyz) * CACHE_ANGLE, CACHE_MIN_VOXEL))) + VOLUME_FINE);
                let jv = cache_point(x + (vec3<f32>(rnd(), rnd(), rnd()) - 0.5) * vs);
                lin = jv.rgb;
                if (jv.w < CACHE_WARM) { lin = cache_point(x).rgb; }
            }
            r.light += r.tr * (v.albedo * st * lin + v.emission) * w;
            r.tr *= ti;
        }
    }
    return r;
}

// A direction scattered from `d` by the Henyey-Greenstein phase function (its pdf: hg_phase(g, dot(d, result))).
fn sample_hg(d: vec3<f32>, g: f32) -> vec3<f32> {
    let u = rnd();
    var ct = 1.0 - 2.0 * u;
    if (abs(g) > 1e-3) {
        let q = (1.0 - g * g) / (1.0 - g + 2.0 * g * u);
        ct = (1.0 + g * g - q * q) / (2.0 * g);
    }
    let st = sqrt(max(0.0, 1.0 - ct * ct));
    let phi = 2.0 * PI * rnd();
    return basis(d) * vec3<f32>(st * cos(phi), st * sin(phi), ct);
}

// Direct light scattered at x along a path travelling in direction d (phase function HG g; shadow rays through the
// media), MIS against phase sampling where the light can also be hit: directional disks, emitters, the environment.
fn phase_direct(x: vec3<f32>, d: vec3<f32>, g: f32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < P.counts2.x; i++) {
        let li = lights[i];
        let cos_max = li.dir.w;
        let cos_t = 1.0 - rnd() * (1.0 - cos_max);
        let sin_t = sqrt(max(0.0, 1.0 - cos_t * cos_t));
        let phi = 2.0 * PI * rnd();
        let l = basis(-li.dir.xyz) * vec3<f32>(sin_t * cos(phi), sin_t * sin(phi), cos_t);
        if (all(li.color.rgb <= vec3<f32>(0.0))) { continue; } // black: no shadow ray (direct_light)
        let ph = hg_phase(g, dot(l, d));
        let vis = light_vis(li.ext.w >= 0.5, x, l, INF);
        if (any(vis > vec3<f32>(0.0))) {
            sum += li.color.rgb * gtint(vis) * ph * power_heuristic(1.0 / (2.0 * PI * (1.0 - cos_max)), ph);
        }
    }
    let nd = P.counts2.x;
    let nl = P.counts.x - nd;
    if (nl > 0u) {
        var wsum = 0.0;
        var pick = 0u;
        var pick_w = 0.0;
        var pick_ls: LS;
        for (var k = 0u; k < 3u; k++) {
            let idx = nd + min(u32(rnd() * f32(nl)), nl - 1u);
            let ls = local_light(lights[idx], x);
            let wk = luminance(ls.c) * hg_phase(g, dot(ls.l, d));
            if (wk <= 0.0) { continue; }
            wsum += wk;
            if (rnd() * wsum < wk) {
                pick = idx;
                pick_w = wk;
                pick_ls = ls;
            }
        }
        if (pick_w > 0.0) {
            let vis = light_vis(lights[pick].ext.w >= 0.5, x, pick_ls.sl, pick_ls.dist * 0.999);
            if (any(vis > vec3<f32>(0.0))) { sum += pick_ls.c * gtint(vis) * hg_phase(g, dot(pick_ls.l, d)) * (wsum * f32(nl) / (3.0 * pick_w)); }
        }
    }
    if (P.counts.y > 0u) {
        mis_x = x;
        mis_n = vec3<f32>(0.0);
        let es = sample_emitter(x, vec3<f32>(0.0));
        if (es.pdf_w > 0.0 && !occluded_any(x, es.l, es.dist * 0.999)) {
            let ph = hg_phase(g, dot(es.l, d));
            sum += es.le * ph * power_heuristic(es.pdf_w, ph) * es.w;
        }
    }
    if (P.counts2.y != 0u && (P.env.x > 0.5 || luminance(P.sky_color.rgb) > 0.0)) {
        let l = sample_env();
        let pdf_e = env_pdf(l);
        if (pdf_e > 0.0 && !occluded_sky(x, l)) {
            let ph = hg_phase(g, dot(l, d));
            sum += env_radiance(l) * ph * power_heuristic(pdf_e, ph) / pdf_e * gtint(shadow_tint);
        }
    }
    return sum;
}

// ------------------------------------------------------------------------------------------------
// Volume voxels of the radiance cache (translucent.wgsl, participating media in the live viewport).
// Lighting at points in the air (translucency now, participating media later): keyed by position alone
// (non-directional) and coarser than surface voxels at the same distance (UE's translucency lighting volume has ~0.5 m
// cells near the camera). Value: the mean radiance arriving from all directions, i.e. what a white isotropic scatterer
// there sends back. Two levels are trained together; lookups take the fine one once it is warm.
const VOLUME_FINE: f32 = 2.0;   // levels above the surface voxel at the same distance
const VOLUME_COARSE: f32 = 4.0;

fn cache_key_point(p: vec3<f32>, coarser: f32) -> vec2<u32> {
    let lvl = floor(log2(max(length(p - P.cam_pos.xyz) * CACHE_ANGLE, CACHE_MIN_VOXEL))) + coarser;
    let q = bitcast<vec3<u32>>(vec3<i32>(floor(p * exp2(-lvl))));
    // Side code 6: no surface side (surface voxels use 0..5).
    let tag = (u32(lvl + 64.0) << 3u) | 6u;
    let h = pcg(q.x ^ pcg(q.y ^ pcg(q.z ^ pcg(tag))));
    let c = pcg((q.x * 73856093u) ^ (q.y * 19349663u) ^ (q.z * 83492791u) ^ (tag * 2654435761u)) | 1u;
    return vec2<u32>(h, c);
}

// Mean incident radiance at p (w = history samples, 0: none yet).
fn cache_point(p: vec3<f32>) -> vec4<f32> {
    let f = cache_find(cache_key_point(p, VOLUME_FINE), false);
    var v = vec4<f32>(0.0);
    if (f != 0xffffffffu) { v = cache_val[f * 2u]; }
    if (v.w >= CACHE_WARM) { return v; }
    let c = cache_find(cache_key_point(p, VOLUME_COARSE), false);
    if (c != 0xffffffffu) {
        let cv = cache_val[c * 2u];
        if (cv.w > v.w) { return cv; }
    }
    return v;
}

// Direct light at a point for an isotropic receiver (phase 1 / 4π): the directional lights (cone), the local lights
// and the sampled emitters (each resampled from a few candidates, one shadow ray). The environment and emission that
// no light sampling covers come from the sphere ray of `point_radiance`.
fn point_direct(x: vec3<f32>) -> vec3<f32> {
    let ph = 1.0 / (4.0 * PI);
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < P.counts2.x; i++) {
        let li = lights[i];
        let cos_max = li.dir.w;
        let cos_t = 1.0 - rnd() * (1.0 - cos_max);
        let sin_t = sqrt(max(0.0, 1.0 - cos_t * cos_t));
        let phi = 2.0 * PI * rnd();
        let l = basis(-li.dir.xyz) * vec3<f32>(sin_t * cos(phi), sin_t * sin(phi), cos_t);
        if (all(li.color.rgb <= vec3<f32>(0.0))) { continue; } // black: no shadow ray (direct_light)
        if (li.ext.w < 0.5 || !occluded(x, l, INF)) { sum += li.color.rgb * ph; }
    }
    let nd = P.counts2.x;
    let nl = P.counts.x - nd;
    if (nl > 0u) {
        var wsum = 0.0;
        var pick = 0u;
        var pick_w = 0.0;
        var pick_ls: LS;
        for (var k = 0u; k < 3u; k++) {
            let idx = nd + min(u32(rnd() * f32(nl)), nl - 1u);
            let ls = local_light(lights[idx], x);
            let wk = luminance(ls.c);
            if (wk <= 0.0) { continue; }
            wsum += wk;
            if (rnd() * wsum < wk) {
                pick = idx;
                pick_w = wk;
                pick_ls = ls;
            }
        }
        if (pick_w > 0.0 && (lights[pick].ext.w < 0.5 || !occluded(x, pick_ls.sl, pick_ls.dist * 0.999))) {
            sum += pick_ls.c * ph * (wsum * f32(nl) / (3.0 * pick_w));
        }
    }
    if (P.counts.y > 0u) {
        var wsum = 0.0;
        var pick: ES;
        var pick_t = 0.0;
        for (var c = 0u; c < 4u; c++) {
            mis_x = x;
            mis_n = vec3<f32>(0.0);
            let es = sample_emitter(x, vec3<f32>(0.0));
            if (es.pdf_w <= 0.0) { continue; }
            let t = luminance(es.le);
            let w = t / es.pdf_w;
            if (w <= 0.0) { continue; }
            wsum += w;
            if (rnd() * wsum < w) {
                pick = es;
                pick_t = t;
            }
        }
        if (pick_t > 0.0 && !occluded_any(x, pick.l, pick.dist * 0.999)) {
            sum += pick.le * ph * (wsum / (4.0 * pick_t));
        }
    }
    return sum;
}

// One estimate of the mean incident radiance at x: the direct light, plus one uniformly sampled direction for the
// rest (the environment, emission that light sampling does not cover, and light reflected off the surface it hits:
// that surface's cached irradiance when warm, else its direct light).
fn point_radiance(x: vec3<f32>) -> vec3<f32> {
    cur_depth = 2u;
    let e = point_direct(x);
    return e + point_sphere(x, false);
}

// A training estimate for the volume voxels: point_radiance with the firefly clamp (`clamp_v`, luminance) on the sphere
// ray alone. Direct light is sampled and needs no clamp; clamping it lost the light of bright lamps and suns
// (volume/box-sun-scatter-live: 1000 lux at EV100 0 read 21 % low).
fn point_radiance_train(x: vec3<f32>, clamp_v: f32) -> vec3<f32> {
    cur_depth = 2u;
    let e = point_direct(x);
    // Scattering orders past the first only when the scene allows bounces (max bounces 0: direct light only).
    var r = point_sphere(x, media_voxels && P.counts.z > 0u);
    let lum = luminance(r);
    if (lum > clamp_v) { r *= clamp_v / lum; }
    return e + r;
}

// The sphere-ray part of point_radiance. In the media voxel training (`media`) the ray crosses the media too: it is
// attenuated and gathers their in-scattered light from the voxels. Trained frame after frame, that feedback adds every
// scattering order, so the voxels converge to multiple scattering (a lamp inside a sphere of albedo-0.9 medium read 22 %
// of the offline reference with single scattering alone).
fn point_sphere(x: vec3<f32>, media: bool) -> vec3<f32> {
    let z = 1.0 - 2.0 * rnd();
    let r = sqrt(max(0.0, 1.0 - z * z));
    let phi = 2.0 * PI * rnd();
    var l = vec3<f32>(r * cos(phi), r * sin(phi), z);
    var o = x;
    var light = vec3<f32>(0.0);
    var tr = 1.0;
    for (var k = 0u; k < 4u; k++) {
        let hit = trace(o, l, INF, TRACE_CLOSEST);
        if (media) {
            let mm = media_march(o, l, select(hit.t, 1.0e30, hit.tri == 0xffffffffu), true);
            light += tr * mm.light;
            tr *= mm.tr;
        }
        // In the media training, glass (a water surface over its body) passes the ray or reflects it, as the paths'
        // BSDF does (transmission share 1 - E_spec, none past the critical angle): stopped there, the voxels under
        // water saw neither the sky through the surface nor what it reflects back (a white furnace read 0.29).
        if (media && has(F_GLASS) && hit.tri != 0xffffffffu && k < 3u) {
            let g = glass_hit(hit, l, hit.t);
            if (g.share > 0.0) {
                let p = o + l * hit.t;
                if (g.tir || rnd() >= g.share) { l = reflect(l, g.ng); }
                o = offset_origin(p, select(g.ng, -g.ng, dot(g.ng, l) < 0.0));
                continue;
            }
        }
        return light + tr * sphere_hit(o, l, hit);
    }
    return light;
}

// Light arriving at x along l from the sphere ray's hit (the environment where it hits nothing).
fn sphere_hit(x: vec3<f32>, l: vec3<f32>, hit: Hit) -> vec3<f32> {
    var e = vec3<f32>(0.0);
    if (hit.tri == 0xffffffffu) {
        return env_radiance(l);
    }
    cur_depth = 3u;
    let s = surface(x, l, hit, hit.t * 0.05);
    // Emission that emitter sampling covers is in the direct part (its MIS weight against any other pdf is below 1).
    mis_x = x;
    mis_n = vec3<f32>(0.0);
    if (any(s.emissive > vec3<f32>(0.0)) && (s.front || !s.card) && emit_mis(hit, l, 1.0 / (4.0 * PI)) >= 1.0) {
        e += s.emissive;
    }
    if (s.card) { return e; }
    let c = cache_get(s, false);
    var irr = c.rgb;
    if (!cache_on() || c.w < CACHE_WARM) { irr = direct_light(s, white_lobes(), -l); }
    return e + cache_reflectance(lobes(s, max(dot(s.n, -l), 1e-4))) * irr;
}

// ------------------------------------------------------------------------------------------------
// Emitter reservoirs at live camera hits (ReSTIR DI, Bitterli et al. 2020; docs/ROADMAP.md § 1, step 1: direct light
// from the many emissive surfaces as clean as the exact light loop). Each pixel resamples RESTIR_M emitter points by
// their unshadowed contribution, then the previous frame's reservoir at its reprojected position and RESTIR_SPATIAL
// neighbours of it (same surface by normal and distance), and shades the winner with one shadow ray; occluded winners
// are stored with weight 0. The target is cheap (the emitter's factor and animation, not its texture); the winner gets
// the full emitted radiance, and its MIS weight against BSDF sampling uses the emitter CDF's pdf, as before. Reuse
// without visibility or pairwise MIS: slightly biased at shadow and surface boundaries. An experiment (THREERT_RESTIR=1):
// on Kabukicho it changed neither the k = 1 disocclusion frame (no history there) nor motion error (+0.1 ΔE), for
// +0.44 ms at 60 %; reuse among this frame's reservoirs needs a pass of its own.

const RESTIR_M: u32 = 4u;
const RESTIR_SPATIAL: u32 = 2u;
const RESTIR_RADIUS: f32 = 12.0; // traced pixels
const RESTIR_CAP: f32 = 80.0;    // history, in candidates (20 x RESTIR_M)

// Two halves (this frame's, the previous one's) of 2 entries per traced pixel: (triangle, barycentrics unorm16x2,
// weight W, count M), (octahedral normal snorm16x2, distance to the camera, -, -).
@group(1) @binding(4) var<storage, read_write> resv: array<vec4<u32>>;

// The camera ray's pixel (traced grid) and whether this path vertex uses the reservoirs.
var<private> cur_pix: u32;

// Reflection resolve inputs (live; live.wgsl `refl`), 2 per traced pixel: the camera hit's specular-lobe ray
// (radiance it brought back, its lobe pdf; pdf 0 = the pixel sampled another lobe), (specular albedo E at the hit,
// octahedral ray direction). Neighbours reuse each other's rays weighted by their own BRDF (Lumen / SSSR).
@group(1) @binding(5) var<storage, read_write> refl: array<vec4<f32>>;
// Live motion vectors per traced pixel: where the camera hit was the previous frame, as a world-space offset (w = 1:
// on a moving triangle).
@group(1) @binding(6) var<storage, read_write> motion_out: array<vec4<f32>>;
// Live deterministic direct light per traced pixel (`dl_det` at the camera hit).
@group(1) @binding(7) var<storage, read_write> direct_out: array<vec4<f32>>;

fn resolve_bound() -> bool { return LIVE && arrayLength(&refl) >= 2u * P.size.x * P.size.y; }
// THREERT_CACHE_FLAGS bit 16 turns the resolve off (rays then stay in their pixel's specular, the resolve reads zeros).
fn resolve_on() -> bool { return resolve_bound() && (u32(P.cache.w) & 16u) == 0u; }

// Density of l under the base specular lobe alone (VNDF sampling).
fn pdf_spec(s: Surface, lb: Lobes, v: vec3<f32>, l: vec3<f32>) -> f32 {
    let nov = max(dot(s.n, v), 1e-4);
    let noh = max(dot(s.n, normalize(v + l)), 0.0);
    return g1(lb.a2, nov) * d_ggx(lb.a2, noh) / (4.0 * nov);
}

fn restir_here() -> bool {
    return LIVE && has(F_RESTIR) && P.restir.x > 0.5 && cur_depth == 0u && P.misc.w < 0.5;
}

fn oct_enc(n: vec3<f32>) -> u32 {
    let p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    let q = select(p, (1.0 - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0)), n.z < 0.0);
    return pack2x16snorm(q);
}

fn oct_dec(u: u32) -> vec3<f32> {
    let p = unpack2x16snorm(u);
    var n = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    let t = max(-n.z, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    return normalize(n);
}

// An emitter point: triangle (0xffffffff: none), barycentrics, and its selection probability at the point that drew it
// (-1: unknown, a reused reservoir sample; emitter_eval asks the light tree then).
struct Cand { tri: u32, b: vec2<f32>, sel: f32 };

// One emitter point for x (normal n) by emitter_draw (the light tree or the CDF: area x emitted luminance).
fn emitter_point(x: vec3<f32>, n: vec3<f32>) -> Cand {
    let pk = emitter_draw(x, n);
    var b1 = rnd();
    var b2 = rnd();
    if (b1 + b2 > 1.0) { b1 = 1.0 - b1; b2 = 1.0 - b2; }
    return Cand(select(0xffffffffu, bitcast<u32>(pk.e.x), pk.p > 0.0), vec2<f32>(b1, b2), pk.p);
}

// Emitter point c seen from x on surface s, unshadowed: target (luminance of factor x animation x BSDF x geometry x
// MIS weight), the CDF's area pdf, direction, distance, the BSDF's specular share; with `full`, the contribution with
// the emitted radiance (texture) in `f`.
struct EmitEval { f: vec3<f32>, ph: f32, pdf_a: f32, l: vec3<f32>, dist: f32, frac: vec3<f32> };

fn emitter_eval(s: Surface, lb: Lobes, v: vec3<f32>, x: vec3<f32>, c: Cand, full: bool) -> EmitEval {
    var r: EmitEval;
    if (c.tri == 0xffffffffu || c.tri >= arrayLength(&attrs)) { return r; }
    let ti = c.tri;
    let e1 = tri_verts[ti * 3u + 1u].xyz;
    let e2 = tri_verts[ti * 3u + 2u].xyz;
    let lp = tri_verts[ti * 3u].xyz + e1 * c.b.x + e2 * c.b.y;
    let cr = cross(e1, e2);
    let dl = lp - x;
    let d2 = max(dot(dl, dl), 1e-8);
    let dist = sqrt(d2);
    let l = dl / dist;
    let a = attrs[ti];
    let mi = bitcast<u32>(a.uv.w);
    let mat = materials[mi];
    var cos_l = dot(normalize(cr), -l);
    if (mat.tex1.z != MAT_EMITTER_CARD) { cos_l = abs(cos_l); }
    if (cos_l <= 0.0) { return r; }
    let f = eval_bsdf(s, lb, v, l);
    if (all(f <= vec3<f32>(0.0))) { return r; }
    r.frac = split_frac;
    // Source pdf of the candidate (the CDF, light LOD included); MIS against the BSDF rays weighs by power alone (emit_mis).
    var lum_p = luminance(mat.emissive.rgb) * obj_power(a);
    // Particle emitters: nominal area (x.z) in the CDF, uniform over the actual one.
    if (has(F_PEMIT) && a.x.z > 0.0) { lum_p *= a.x.z / max(0.5 * length(cr), 1e-12); }
    var pdf_m = lum_p / P.emit_power.x;
    if (lt_on()) {
        // The light tree: its pmf at x per area, for the candidate's weight and for MIS (as emit_mis weighs).
        var sel = c.sel;
        if (sel < 0.0) { sel = emitter_sel(ti, x, s.n); }
        r.pdf_a = sel / max(0.5 * length(cr), 1e-12);
        pdf_m = r.pdf_a;
    } else {
        r.pdf_a = lum_p * emitter_lod(ti) / P.misc.y;
    }
    let m = power_heuristic(pdf_m * d2 / cos_l, pdf_bsdf(s, lb, v, l));
    let g = cos_l / d2 * m;
    r.ph = luminance(mat.emissive.rgb) * obj_power(a) * luminance(f) * g;
    if (full) { r.f = emitter_radiance(ti, c.b.x, c.b.y, l) * f * g; }
    r.l = l;
    r.dist = dist;
    return r;
}

fn restir_emitters(s: Surface, lb: Lobes, v: vec3<f32>, x: vec3<f32>) -> vec3<f32> {
    var pick = Cand(0xffffffffu, vec2<f32>(0.0), -1.0);
    var pick_ph = 0.0;
    var wsum = 0.0;
    var m = 0.0;
    for (var k = 0u; k < RESTIR_M; k++) {
        let c = emitter_point(x, s.n);
        m += 1.0;
        let ev = emitter_eval(s, lb, v, x, c, false);
        if (ev.ph <= 0.0 || ev.pdf_a <= 0.0) { continue; }
        let w = ev.ph / ev.pdf_a;
        wsum += w;
        if (rnd() * wsum < w) {
            pick = c;
            pick_ph = ev.ph;
        }
    }
    let half = arrayLength(&resv) / 4u;
    if (P.restir.x > 1.5) {
        // Previous frame's traced-grid position of this surface point.
        let dp = s.p - P.prev_pos.xyz;
        let z = dot(dp, P.prev_fwd.xyz);
        let dist = length(dp);
        if (z > 1e-4) {
            let pw = P.restir.y;
            let ph = P.restir.z;
            let c0 = vec2<f32>((dot(dp, P.prev_right.xyz) / (z * P.prev_pos.w * P.prev_fwd.w) * 0.5 + 0.5) * pw, (0.5 - dot(dp, P.prev_up.xyz) / (z * P.prev_pos.w) * 0.5) * ph);
            let prev = (1u - u32(P.restir.w)) * half;
            for (var j = 0u; j <= RESTIR_SPATIAL; j++) {
                var q = c0;
                if (j > 0u) {
                    let r = RESTIR_RADIUS * sqrt(rnd());
                    let a = 2.0 * PI * rnd();
                    q += r * vec2<f32>(cos(a), sin(a));
                }
                if (q.x < 0.0 || q.y < 0.0 || q.x >= pw || q.y >= ph) { continue; }
                let qi = prev + u32(q.y) * u32(pw) + u32(q.x);
                let e1 = resv[qi * 2u + 1u];
                if (dot(oct_dec(e1.x), s.n) < 0.9 || abs(bitcast<f32>(e1.y) - dist) > 0.1 * dist) { continue; }
                let e0 = resv[qi * 2u];
                let mq = min(bitcast<f32>(e0.w), RESTIR_CAP);
                let c = Cand(e0.x, unpack2x16unorm(e0.y), -1.0);
                m += mq;
                let ev = emitter_eval(s, lb, v, x, c, false);
                let w = ev.ph * bitcast<f32>(e0.z) * mq;
                if (!(w > 0.0)) { continue; }
                wsum += w;
                if (rnd() * wsum < w) {
                    pick = c;
                    pick_ph = ev.ph;
                }
            }
        }
    }
    var wgt = 0.0;
    var c = vec3<f32>(0.0);
    if (pick_ph > 0.0) {
        wgt = wsum / (m * pick_ph);
        let ev = emitter_eval(s, lb, v, x, pick, true);
        if (ev.ph > 0.0 && !occluded_any(x, ev.l, ev.dist * 0.999)) {
            c = ev.f * wgt;
            dl_spec += c * ev.frac;
        } else {
            wgt = 0.0;
        }
    }
    if (non_finite(c)) { c = vec3<f32>(0.0); wgt = 0.0; }
    let cur = u32(P.restir.w) * half + cur_pix;
    resv[cur * 2u] = vec4<u32>(pick.tri, pack2x16unorm(pick.b), bitcast<u32>(wgt), bitcast<u32>(min(m, RESTIR_CAP)));
    resv[cur * 2u + 1u] = vec4<u32>(oct_enc(s.n), bitcast<u32>(length(s.p - P.cam_pos.xyz)), 0u, 0u);
    return c;
}

// ------------------------------------------------------------------------------------------------
// World-space radiance cache (live; docs/ROADMAP.md § 1, step 1). Spatially hashed voxels, sized by distance to the
// camera (CACHE_ANGLE, rounded down to a power of two), x 6 normal directions. Each holds the cosine-weighted incident
// radiance (irradiance / π, direct + indirect) at its surfaces: a surface that ends a path there reflects its diffuse +
// directional specular albedo times that. Training paths start at those surfaces: white-Lambert next-event estimation
// plus one cosine-sampled ray per vertex, CACHE_DEPTH vertices deep, ending in the cache again (so it holds every
// bounce); each vertex's estimate is added. Sums are fixed point with stochastic rounding (`cache_acc`);
// `cache_resolve` folds them into `cache_val` once per frame.

const CACHE_SIZE: u32 = 262144u;     // entries, a power of two (live.rs CACHE_SIZE)
const CACHE_PROBE: u32 = 8u;         // linear probing window
const CACHE_ANGLE: f32 = 0.02;       // voxel size / distance to the camera
const CACHE_MIN_VOXEL: f32 = 0.0625; // m
const CACHE_WARM: f32 = 4.0;         // samples before lookups trust an entry
const CACHE_TRAIN_MASK: u32 = 7u;    // 1 in 8 tiles of 8x4 pixels (one SIMD group: the others really skip) per frame
const CACHE_FRAME_CAP: u32 = 255u;   // samples per entry per frame (fixed-point headroom)
const CACHE_TRAIN_CLAMP: f32 = 4.0;  // training samples' clamp, in indirect clamps (cache_fixed's headroom)
const CACHE_DEPTH: u32 = 2u;         // vertices per training path
const CACHE_EVICT: f32 = 120.0;      // frames without samples before an entry is freed

// cache_keys[i]: checksum of the voxel key (0 = free). cache_acc[8i ..]: this frame's sums (fixed point) of the total
// r, g, b, of its indirect part r, g, b, and the sample count. cache_val[2i] = (irradiance / π, history samples),
// cache_val[2i + 1] = (its indirect part: all but the voxel's own direct light, frames since the last sample).
@group(1) @binding(1) var<storage, read_write> cache_keys: array<atomic<u32>>;
@group(1) @binding(2) var<storage, read_write> cache_acc: array<atomic<u32>>;
@group(1) @binding(3) var<storage, read_write> cache_val: array<vec4<f32>>;

fn cache_on() -> bool { return LIVE && P.cache.x > 0.5; }

// (slot hash, checksum) of the voxel holding p, on the side ng faces most.
fn cache_key(p: vec3<f32>, ng: vec3<f32>) -> vec2<u32> {
    return cache_key_at(p, ng, 0.0);
}

// The key `step` levels coarser (negative: finer) than p's own.
fn cache_key_at(p: vec3<f32>, ng: vec3<f32>, step: f32) -> vec2<u32> {
    let lvl = floor(log2(max(length(p - P.cam_pos.xyz) * CACHE_ANGLE, CACHE_MIN_VOXEL))) + step;
    let q = bitcast<vec3<u32>>(vec3<i32>(floor(p * exp2(-lvl))));
    let a = abs(ng);
    let axis = select(select(2u, 1u, a.y >= a.z), 0u, a.x >= a.y && a.x >= a.z);
    let neg = select(select(ng.z, ng.y, axis == 1u), ng.x, axis == 0u) < 0.0;
    let tag = (u32(lvl + 64.0) << 3u) | (axis * 2u + u32(neg));
    let h = pcg(q.x ^ pcg(q.y ^ pcg(q.z ^ pcg(tag))));
    let c = pcg((q.x * 73856093u) ^ (q.y * 19349663u) ^ (q.z * 83492791u) ^ (tag * 2654435761u)) | 1u;
    return vec2<u32>(h, c);
}

// Slot of voxel `k`; 0xffffffff when absent (or, inserting, when its probe window is full). Lookups scan the whole
// window: eviction leaves holes in it.
fn cache_find(k: vec2<u32>, insert: bool) -> u32 {
    var free = 0xffffffffu;
    for (var i = 0u; i < CACHE_PROBE; i++) {
        let s = (k.x + i) & (CACHE_SIZE - 1u);
        let c = atomicLoad(&cache_keys[s]);
        if (c == k.y) { return s; }
        if (c == 0u && free == 0xffffffffu) { free = s; }
    }
    if (!insert || free == 0xffffffffu) { return 0xffffffffu; }
    let r = atomicCompareExchangeWeak(&cache_keys[free], 0u, k.y);
    if (r.exchanged || r.old_value == k.y) { return free; }
    return 0xffffffffu;
}

// Cached irradiance / π at the surface (or its indirect part), w = history samples (0: none).
fn cache_get(s: Surface, indirect: bool) -> vec4<f32> {
    var best = cache_entry(cache_key(s.p, s.ng), indirect);
    if (best.w >= CACHE_WARM) { return best; }
    // Voxel sizes follow the distance to the camera in powers of two: a surface the camera comes closer to (or moves
    // away from) gets a new, cold voxel while the one a level coarser (finer) is warm. Use the warmer meanwhile; the
    // training tiles warm the new one. Pixels on cold voxels traced a fresh estimate each: Bistro's camera path cost
    // 2 ms more trace moving than held still at the same poses (now only after a reset or a cut, see the path).
    for (var k = 0u; k < 2u; k++) {
        let e = cache_entry(cache_key_at(s.p, s.ng, select(-1.0, 1.0, k == 0u)), indirect);
        if (e.w > best.w) { best = e; }
    }
    return best;
}

fn cache_entry(k: vec2<u32>, indirect: bool) -> vec4<f32> {
    let i = cache_find(k, false);
    if (i == 0xffffffffu) { return vec4<f32>(0.0); }
    let v = cache_val[i * 2u];
    if (indirect) { return vec4<f32>(cache_val[i * 2u + 1u].rgb, v.w); }
    return v;
}

// Fixed point: 2^21 units per firefly clamp, so 255 samples of up to 4 clamps per channel fit 32 bits.
fn cache_fixed(e: vec3<f32>) -> vec3<u32> {
    return vec3<u32>(min(e, vec3<f32>(4.0 * P.misc.x)) * (2097152.0 / P.misc.x) + vec3<f32>(rnd(), rnd(), rnd()));
}

fn cache_add(i: u32, total: vec3<f32>, indirect: vec3<f32>) {
    if (i == 0xffffffffu || non_finite(total) || non_finite(indirect)) { return; }
    if (atomicAdd(&cache_acc[i * 8u + 6u], 1u) >= CACHE_FRAME_CAP) { return; }
    let a = cache_fixed(total);
    let b = cache_fixed(indirect);
    atomicAdd(&cache_acc[i * 8u], a.x);
    atomicAdd(&cache_acc[i * 8u + 1u], a.y);
    atomicAdd(&cache_acc[i * 8u + 2u], a.z);
    atomicAdd(&cache_acc[i * 8u + 3u], b.x);
    atomicAdd(&cache_acc[i * 8u + 4u], b.y);
    atomicAdd(&cache_acc[i * 8u + 5u], b.z);
}

// White Lambertian lobes: lighting gathered with them is irradiance / π, and a cosine-sampled ray has weight 1.
fn white_lobes() -> Lobes {
    var l: Lobes;
    l.diffuse = vec3<f32>(1.0);
    l.f0 = vec3<f32>(0.0);
    l.f90 = vec3<f32>(0.0);
    l.w = vec3<f32>(1.0);
    l.e_spec = vec3<f32>(0.0);
    l.a2 = 1.0;
    l.p_spec = 0.0;
    l.coat_alpha = 1.0;
    return l;
}

// What a surface reflects of irradiance / π toward the viewer, the lighting taken as uniform over the hemisphere:
// diffuse + directional specular albedo.
fn cache_reflectance(lb: Lobes) -> vec3<f32> {
    return lb.diffuse * (1.0 - lb.e_spec) + lb.e_spec;
}

struct Fresh { total: vec3<f32>, direct: vec3<f32> };

// A fresh irradiance / π estimate at surface s0 (reached along d0; ray-cone width there + spread) and its direct part,
// by a path of `depth` (1 ..= CACHE_DEPTH) vertices, also added to the cache at each of them.
// `insert`: whether the path's voxels are inserted when absent (training tiles), or only trained when present (a
// cold pixel's fresh estimate).
fn cache_train(s0: Surface, d0: vec3<f32>, cone0: vec2<f32>, depth: u32, insert: bool) -> Fresh {
    var slot: array<u32, CACHE_DEPTH>;
    // Per vertex: its direct light; that + the emission / sky its ray brought back; what the next vertex reflects.
    var direct: array<vec3<f32>, CACHE_DEPTH>;
    var gather: array<vec3<f32>, CACHE_DEPTH>;
    var refl: array<vec3<f32>, CACHE_DEPTH>;
    let wl = white_lobes();
    var s = s0;
    var d = d0;
    var cone = cone0;
    var tail = vec3<f32>(0.0);
    var n = 0u;
    for (var k = 0u; k < depth; k++) {
        cur_depth = 2u + k;
        let v = -d;
        slot[k] = cache_find(cache_key(s.p, s.ng), insert);
        var g = direct_light(s, wl, v);
        direct[k] = g;
        refl[k] = vec3<f32>(0.0);
        gather[k] = g;
        n = k + 1u;
        let l = sample_bsdf(s, wl, v);
        let pdf = pdf_bsdf(s, wl, v, l);
        if (pdf <= 0.0) { break; }
        let o = offset_origin(s.p, s.ng);
        let hit = trace(o, l, INF, TRACE_CLOSEST);
        let miss = hit.tri == 0xffffffffu;
        // Height fog on the segment (attenuation + ambient inscattering; no lit volumetrics off camera rays).
        var tr = 1.0;
        if (has(F_FOG) && P.fog.x > 0.0) {
            tr = max(exp(-fog_depth(o, l, 0.0, select(hit.t, 2.0e4, miss))), 1.0 - P.fog_color.w);
            g += (1.0 - tr) * P.fog_color.rgb;
        }
        if (has(F_AERIAL) && !miss) {
            let ta = aerial_tr(o, l, hit.t);
            g += tr * (1.0 - ta) * aerial_light(l);
            tr *= dot(ta, vec3<f32>(1.0 / 3.0));
        }
        // Homogeneous media (water) on the segment, as on the paths: without them the voxels under water gathered
        // their surroundings unattenuated (a canal bed read 2.4x the offline image).
        if (LIVE && has(F_HMEDIA)) {
            let mh = media_homogeneous(o, l, select(hit.t, 1.0e30, miss), true);
            g += tr * mh.light;
            tr *= mh.tr;
        }
        if (miss) {
            gather[k] = g + tr * sky_mis(l, pdf);
            break;
        }
        cur_depth = 3u + k;
        cone = vec2<f32>(cone.x + (cone.y + 2.0) * hit.t, cone.y + 2.0);
        let s2 = surface(o, l, hit, cone.x);
        if (any(s2.emissive > vec3<f32>(0.0)) && (s2.front || !s2.card)) {
            g += tr * s2.emissive * emit_mis(hit, l, pdf);
        }
        gather[k] = g;
        if (s2.card) { break; }
        refl[k] = tr * cache_reflectance(lobes(s2, max(dot(s2.n, -l), 1e-4)));
        // Every vertex is traced (each update carries `depth` more bounces into the cache, so multiple bounces build up
        // quickly); the path ends in the cache, or on a cold voxel in that vertex's direct light.
        if (k + 1u == depth) {
            let c = cache_get(s2, false);
            if (c.w >= CACHE_WARM) {
                tail = c.rgb;
            } else {
                tail = direct_light(s2, wl, -l);
            }
            break;
        }
        s = s2;
        d = l;
    }
    var e = tail;
    for (var j = i32(n) - 1; j >= 0; j--) {
        e = gather[j] + refl[j] * e;
        let lum = luminance(e);
        // Training samples are clamped at CACHE_TRAIN_CLAMP x the indirect clamp, what the fixed-point sums leave room
        // for (cache_fixed). At 1 x the voxels near bright small lights stored their light clamped, and every pixel
        // reading them inherited the loss: the Bistro crowd's held frame read diffuse 0.81 of our offline render,
        // 0.86 at 4 x; layer B 3.50 -> 3.47.
        let tcl = CACHE_TRAIN_CLAMP * P.misc.x;
        if (lum > tcl) { e *= tcl / lum; }
        cache_add(slot[j], e, max(e - direct[j], vec3<f32>(0.0)));
    }
    return Fresh(e, min(direct[0], e));
}

// Live, after the trace: MEDIA_TRAIN_POINTS fresh estimates (point_radiance) into the volume voxels, each at a random
// point where a camera ray toward a random traced pixel crosses the media before its first hit.
@compute @workgroup_size(64, 1, 1)
fn media_train(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (!has(F_MEDIA) || !cache_on() || P.time.w < 0.5 || gid.x >= MEDIA_TRAIN_POINTS) { return; }
    media_voxels = true; // the voxels' light is shadowed by the media
    rng_state = pcg(gid.x ^ pcg(P.size.z * 6151u + P.size.w * 3079u + 0x2545f491u));
    mirror_path = false;
    let w = P.size.x;
    let h = P.size.y;
    let px = min(u32(rnd() * f32(w)), w - 1u);
    let py = min(u32(rnd() * f32(h)), h - 1u);
    let ndc = vec2<f32>((f32(px) + rnd()) / f32(w) * 2.0 - 1.0, 1.0 - (f32(py) + rnd()) / f32(h) * 2.0);
    let d = normalize(P.cam_fwd.xyz + P.cam_right.xyz * (ndc.x * P.cam_pos.w * P.cam_fwd.w) + P.cam_up.xyz * (ndc.y * P.cam_pos.w));
    let o = P.cam_pos.xyz;
    let fd = accum[py * w + px].color.w;
    let tmax = select(1.0e30, fd, fd > 0.0);
    // Homogeneous media (water) lie behind the glass the camera sees them through: their points are taken along the
    // straight ray up to the first opaque surface (stopped at the glass, the voxels learned the water's top layer
    // alone, and its light lit the whole depth: 2.5x the offline single scattering).
    var tmax_h = tmax;
    if (has(F_HMEDIA)) {
        let hh = trace(o, d, INF, TRACE_SHADOW);
        tmax_h = select(1.0e30, hh.t, hh.tri != 0xffffffffu);
    }
    let n = min(u32(P.time.w), 32u);
    var total = 0.0;
    for (var k = 0u; k < n; k++) {
        let sp = medium_span(media[k], o, d);
        total += max(min(sp.y, select(tmax, tmax_h, media[k].a.z != 0u)) - max(sp.x, 0.0), 0.0);
    }
    if (total <= 0.0) { return; }
    var u = rnd() * total;
    var x = o;
    for (var k = 0u; k < n; k++) {
        let sp = medium_span(media[k], o, d);
        let a = max(sp.x, 0.0);
        let len = max(min(sp.y, select(tmax, tmax_h, media[k].a.z != 0u)) - a, 0.0);
        if (u < len) {
            x = o + d * (a + u);
            break;
        }
        u -= len;
    }
    let e = point_radiance_train(x, P.misc.x);
    cache_add(cache_find(cache_key_point(x, VOLUME_FINE), true), e, e);
    cache_add(cache_find(cache_key_point(x, VOLUME_COARSE), true), e, e);
}

// Live participating media, a pass of its own after the frame's last trace sample. Per traced pixel the camera ray
// of its first sample (the jitter stored with it) is marched up to the pixel's first hit (media_march): the
// transmittance scales every signal behind the medium (diffuse, specular, direct light, the reflection resolve's
// rays) and the medium's light joins what the camera sees directly (the emission signal, once per sample). Reflection,
// GI and shadow rays do not see media live (offline paths do).
@compute @workgroup_size(8, 8, 1)
fn media_composite(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = P.size.x;
    let h = P.size.y;
    if (!has(F_MEDIA) || P.time.w < 0.5 || gid.x >= w || gid.y >= h) { return; }
    let pix = gid.y * w + gid.x;
    rng_state = pcg(pix ^ pcg(P.size.z * 9781u + P.size.w * 6271u + 0x51ed27u));
    mirror_path = false;
    var px = accum[pix];
    let jit = u32(px.normal.w);
    let ndc = vec2<f32>((f32(gid.x) + f32(jit / 4096u) / 4095.0) / f32(w) * 2.0 - 1.0, 1.0 - (f32(gid.y) + f32(jit % 4096u) / 4095.0) / f32(h) * 2.0);
    let d = normalize(P.cam_fwd.xyz + P.cam_right.xyz * (ndc.x * P.cam_pos.w * P.cam_fwd.w) + P.cam_up.xyz * (ndc.y * P.cam_pos.w));
    let ft = px.color.w;
    let mm = media_march(P.cam_pos.xyz, d, select(1.0e30, ft, ft > 0.0), true);
    if (mm.tr >= 1.0 && all(mm.light == vec3<f32>(0.0))) { return; }
    let samples = P.misc.w + 1.0; // this dispatch runs with the frame's last sample's parameters
    px.color = vec4<f32>(px.color.rgb * mm.tr, px.color.w);
    px.spec = vec4<f32>(px.spec.rgb * mm.tr, px.spec.w);
    px.emit = vec4<f32>(px.emit.rgb * mm.tr + mm.light * samples, px.emit.w);
    accum[pix] = px;
    if (arrayLength(&direct_out) >= w * h) { direct_out[pix] = vec4<f32>(direct_out[pix].rgb * mm.tr, direct_out[pix].w); }
    if (resolve_bound()) { refl[pix * 2u] = vec4<f32>(refl[pix * 2u].xyz * mm.tr, refl[pix * 2u].w); }
}

// Once per frame after the trace: folds the frame's training sums into each entry (history capped at P.cache.y
// samples and P.cache.z frames, so lighting changes show within a few frames), ages entries, frees stale ones.
@compute @workgroup_size(256, 1, 1)
fn cache_resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= CACHE_SIZE || atomicLoad(&cache_keys[i]) == 0u) { return; }
    let n = min(atomicLoad(&cache_acc[i * 8u + 6u]), CACHE_FRAME_CAP);
    var v = cache_val[i * 2u];
    var ind = cache_val[i * 2u + 1u];
    if (n > 0u) {
        var sum: array<f32, 6>;
        for (var c = 0u; c < 6u; c++) { sum[c] = f32(atomicLoad(&cache_acc[i * 8u + c])) * (P.misc.x / 2097152.0) / f32(n); }
        let hist = min(v.w + f32(n), P.cache.y);
        // The frame cap counts frames, also those without samples since the last update (sparse voxels).
        let a = max(f32(n) / hist, min((ind.w + 1.0) / P.cache.z, 1.0));
        v = vec4<f32>(mix(v.rgb, vec3<f32>(sum[0], sum[1], sum[2]), a), hist);
        ind = vec4<f32>(mix(ind.rgb, vec3<f32>(sum[3], sum[4], sum[5]), a), 0.0);
        for (var c = 0u; c < 8u; c++) { atomicStore(&cache_acc[i * 8u + c], 0u); }
    } else {
        ind.w += 1.0;
        if (ind.w > CACHE_EVICT) {
            atomicStore(&cache_keys[i], 0u);
            v = vec4<f32>(0.0);
            ind = vec4<f32>(0.0);
        }
    }
    cache_val[i * 2u] = v;
    cache_val[i * 2u + 1u] = ind;
}

// ------------------------------------------------------------------------------------------------

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = P.size.x;
    let h = P.size.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let pix = gid.y * w + gid.x;
    cur_pix = pix;
    // Every pixel's deterministic direct light is written each frame (0 unless the camera hits a surface): a pixel that
    // turned to sky must not keep what a surface left there.
    if (LIVE && P.misc.w < 0.5 && arrayLength(&direct_out) >= w * h) { direct_out[pix] = vec4<f32>(0.0); }
    rng_state = pcg(pix ^ pcg(P.size.z * 9781u + P.size.w * 6271u));

    // Raster primary: one visibility sample per pixel at the frame's jitter (lens sampling would need its own rays).
    let raster = LIVE && P.raster.x > 0.5 && P.cam_right.w <= 0.0;
    var jx = rnd();
    var jy = rnd();
    if (raster) {
        jx = P.raster.y;
        jy = P.raster.z;
    }
    let ndc = vec2<f32>((f32(gid.x) + jx) / f32(w) * 2.0 - 1.0, 1.0 - (f32(gid.y) + jy) / f32(h) * 2.0);
    var d = normalize(P.cam_fwd.xyz + P.cam_right.xyz * (ndc.x * P.cam_pos.w * P.cam_fwd.w) + P.cam_up.xyz * (ndc.y * P.cam_pos.w));
    var o = P.cam_pos.xyz;
    if (P.cam_right.w > 0.0) {
        let focus = o + d * (P.cam_up.w / dot(d, P.cam_fwd.xyz));
        let r = sqrt(rnd()) * P.cam_right.w;
        let phi = 2.0 * PI * rnd();
        o += P.cam_right.xyz * (r * cos(phi)) + P.cam_up.xyz * (r * sin(phi));
        d = normalize(focus - o);
    }

    var radiance = vec3<f32>(0.0);
    var first_t = 0.0;
    var thr = vec3<f32>(1.0);
    var albedo = vec3<f32>(0.0);
    var normal = vec3<f32>(0.0);
    var anim = false;
    var e0 = vec3<f32>(0.0); // emission / sky seen directly (camera ray): the live denoiser leaves it unfiltered
    // Diffuse / specular split at the first hit: specular direct light, the specular share of the first BSDF sample
    // (everything gathered after it scales the same way), radiance before it, roughness, reflection hit distance.
    var spec0 = vec3<f32>(0.0);
    var k_spec = vec3<f32>(0.0);
    var rad0 = vec3<f32>(0.0);
    var split = false;
    var rough0 = 1.0;
    var spec_lobe = false;
    var spec_t = 0.0;
    var last_pdf = 0.0;
    // The camera hit's specular-lobe sample for the reflection resolve: direction, lobe pdf, path weight, albedo E;
    // and whether the camera hit's sample went through glass.
    var refl_l = vec3<f32>(0.0);
    var refl_pdf = 0.0;
    var refl_w = vec3<f32>(0.0);
    var refl_e = vec3<f32>(0.0);
    var through0 = false;
    // The camera hit is transmissive (glass, water): its reflections stay in the specular signal too (the resolve's
    // neighbours there mostly went through, so it had no rays to rebuild them from: live glass lost its reflection).
    var glass0 = false;
    var mv = vec4<f32>(0.0);
    dl_det = vec3<f32>(0.0);
    dl_det_spec = vec3<f32>(0.0);
    var specular_bounce = true;
    // Glass passed since the last scattering vertex: any (behind_glass), and whether an odd number of surfaces (bent).
    // The vertex's shadow rays toward emitters stopped at the glass and those toward the sky at bending glass, so
    // what the path finds there counts in full; behind glass that does not bend (a pane) the sky and the directional
    // lights' disks are weighed against the vertex's samples, which passed it (counted in full, they were counted
    // twice: a white furnace through water read 1.53).
    var behind_glass = false;
    var bent = false;
    // Only glass passed so far (straight through): what lies behind is seen directly, not clamped as indirect.
    var straight = true;
    let clamp_v = P.misc.x;
    // Ray cone (Akenine-Moller et al. 2019) for texture LOD: footprint width at the origin + spread angle.
    var cone_w = 0.0;
    var cone_a = 2.0 * P.cam_pos.w / lod_height();

    mirror_path = false;
    for (var depth = 0u; depth <= P.counts.z; depth++) {
        cur_depth = depth;
        var hit: Hit;
        if (raster && depth == 0u) {
            let id = textureLoad(vis_tex, vec2<i32>(gid.xy), 0).x;
            if (id == 0u) {
                hit = Hit(INF, 0xffffffffu, 0.0, 0.0, NO_INST);
            } else {
                hit = tri_hit(id - 1u, o, d);
                // The raster pass draws cutout triangles whole: the alpha test decides here, else trace past them.
                if (has(F_MASKED) && (bitcast<u32>(tri_verts[hit.tri * 3u].w) & TRI_MASKED) != 0u && !alpha_pass(hit.tri, NO_INST, hit.u, hit.v, hit.t)) {
                    hit = trace(o, d, INF, TRACE_CLOSEST);
                }
            }
        } else {
            hit = trace(o, d, INF, TRACE_CLOSEST);
        }
        // Ray tracing off: the camera ray above was the last one traced for this pixel.
        if (depth == 0u) { rays_off = LIVE && P.raster.w > 0.5; }
        if (has(F_FOG) && P.fog.x > 0.0) {
            // Height fog on every segment (attenuation + ambient inscattering); lit volumetric scattering on camera rays.
            let t0 = select(0.0, P.fog.w, depth == 0u);
            let t1 = select(hit.t, 2.0e4, hit.tri == 0xffffffffu);
            // Inside the volumetric range (camera rays) the medium is denser by the extinction scale (UE).
            var od = fog_depth(o, d, t0, t1);
            var tv = t1;
            var trv = 1.0;
            if (has(F_VOLUME) && depth == 0u && P.vol.x > 0.0) {
                tv = select(t1, min(t1, P.vol2.w), P.vol2.w > 0.0);
                let odv = fog_depth(o, d, t0, max(tv, t0));
                od += odv * (P.vol.z - 1.0);
                trv = exp(-odv * P.vol.z);
            }
            let tr = max(exp(-od), 1.0 - P.fog_color.w);
            // Interleaved over 8x4 tiles (one SIMD group each, so skipped tiles really idle), rotating per sample:
            // a quarter of the pixels at 4x weight; the low-frequency fog converges in the accumulation.
            if (has(F_VOLUME) && depth == 0u && P.vol.x > 0.0 && (((gid.x >> 3u) + 2u * (gid.y >> 2u) + pcg(P.size.z)) & VOL_RATE_MASK) == 0u) {
                // Firefly clamp: a fog point next to a bright sign is rare and huge; haze is a few nits.
                var fs = f32(VOL_RATE_MASK + 1u) * fog_scatter(o, d, t0, max(tv, t0), trv);
                let fl = luminance(fs);
                let fmax = 0.02 * clamp_v;
                if (fl > fmax) { fs *= fmax / fl; }
                radiance += thr * fs;
            }
            radiance += thr * (1.0 - tr) * P.fog_color.rgb;
            thr *= tr;
        }
        if (has(F_AERIAL) && hit.tri != 0xffffffffu) {
            let ta = aerial_tr(o, d, hit.t);
            let haze = thr * (1.0 - ta) * aerial_light(d);
            radiance += haze;
            // Live: the camera ray's haze is light the camera sees directly, not the surface's: it joins the emission
            // signal (left out of the diffuse one, which the denoiser divides by the albedo).
            if (depth == 0u) { e0 += haze; }
            thr *= ta;
        }
        // Live media are composited after the trace (media_composite); past the camera ray, homogeneous ones (water)
        // attenuate and light the path here. Offline paths track every medium.
        if (LIVE && has(F_HMEDIA) && depth > 0u) {
            let mh = media_homogeneous(o, d, select(hit.t, 1.0e30, hit.tri == 0xffffffffu), true);
            radiance += thr * mh.light;
            thr *= mh.tr;
        }
        if (!LIVE && has(F_MEDIA) && P.time.w > 0.5) {
            // Media on this segment: emission on the way; a real collision scatters the path there.
            let tk = media_track(o, d, select(hit.t, 1.0e30, hit.tri == 0xffffffffu));
            radiance += thr * tk.emission;
            if (tk.t >= 0.0) {
                let x = o + d * tk.t;
                thr *= tk.albedo;
                if (depth == 0u) {
                    first_t = tk.t;
                    albedo = tk.albedo;
                    normal = -d;
                }
                var c = thr * phase_direct(x, d, tk.g);
                if (depth > 0u && !straight) {
                    let lum = luminance(c);
                    if (lum > clamp_v) { c *= clamp_v / lum; }
                }
                radiance += c;
                if (depth == P.counts.z) { break; }
                let nd = sample_hg(d, tk.g);
                last_pdf = hg_phase(tk.g, dot(nd, d));
                specular_bounce = false;
                behind_glass = false;
                bent = false;
                straight = false;
                mirror_path = false;
                cone_w += cone_a * tk.t;
                cone_a += 2.0;
                if (depth >= P.counts.w) {
                    let q = min(max(thr.x, max(thr.y, thr.z)), 0.95);
                    if (rnd() > q) { break; }
                    thr /= q;
                }
                o = x;
                d = nd;
                continue;
            }
        }
        if (hit.tri == 0xffffffffu) {
            var c = thr * sky_mis(d, select(0.0, last_pdf, depth > 0u && !bent));
            if (depth > 1u && !straight) {
                let lum = luminance(c);
                if (lum > clamp_v) { c *= clamp_v / lum; }
            }
            if (depth == 0u) { e0 += c; }
            if (depth == 1u && spec_lobe) { spec_t = 1.0e4; }
            radiance += c;
            break;
        }
        let width = cone_w + cone_a * hit.t;
        let s = surface(o, d, hit, width);
        if (depth == 0u) {
            first_t = hit.t;
            let ti = hit.tri;
            // The first dynamic triangle is the first of traversal group 4 (a u32: an index carried as f32 bits would be a
            // denormal below 2^23, which a driver may flush to zero).
            // Dynamic triangles (and deforming instanced meshes, in model space) keep their previous vertices.
            let deforms = LIVE && P.motion.x > 0.5 && !is_proc(hit) && ti >= P.group_first[1].x && (bitcast<u32>(tri_verts[ti * 3u].w) & TRI_DYNAMIC) != 0u;
            let kp = (ti - P.group_first[1].x) * 3u;
            if (LIVE && P.motion.x > 0.5 && is_proc(hit)) {
                // Procedural geometry: its object-space hit point through the previous frame's rows.
                let r = instances[hit.inst];
                if (any(r.p0 != r.m0) || any(r.p1 != r.m1) || any(r.p2 != r.m2)) {
                    let lp = vec4<f32>(proc_point(r, s.p), 1.0);
                    mv = vec4<f32>(vec3<f32>(dot(r.p0, lp), dot(r.p1, lp), dot(r.p2, lp)) - s.p, 1.0);
                }
            } else if (LIVE && P.motion.x > 0.5 && has(F_INST) && hit.inst != NO_INST) {
                // A movable instance: its model-space hit point (the previous pose's for a deforming mesh) through the
                // previous frame's transform (p rows; the same as m when it did not move).
                let r = instances[hit.inst];
                if (deforms || any(r.p0 != r.m0) || any(r.p1 != r.m1) || any(r.p2 != r.m2)) {
                    var lp = vec4<f32>(tri_verts[ti * 3u].xyz + tri_verts[ti * 3u + 1u].xyz * hit.u + tri_verts[ti * 3u + 2u].xyz * hit.v, 1.0);
                    if (deforms) { lp = vec4<f32>(prev_verts[kp].xyz + prev_verts[kp + 1u].xyz * hit.u + prev_verts[kp + 2u].xyz * hit.v, 1.0); }
                    mv = vec4<f32>(vec3<f32>(dot(r.p0, lp), dot(r.p1, lp), dot(r.p2, lp)) - s.p, 1.0);
                }
            } else if (deforms) {
                let pp = prev_verts[kp].xyz + prev_verts[kp + 1u].xyz * hit.u + prev_verts[kp + 2u].xyz * hit.v;
                mv = vec4<f32>(pp - s.p, 1.0);
            }
            albedo = select(s.base, s.emissive / max(max(s.emissive.r, max(s.emissive.g, s.emissive.b)), 1e-6), s.card);
            normal = s.n;
            anim = s.anim;
            rough0 = s.roughness;
        }
        if (depth == 1u && spec_lobe) { spec_t = hit.t; }
        // Emission (one-sided for rect light cards)
        if (any(s.emissive > vec3<f32>(0.0)) && (s.front || !s.card)) {
            var wgt = 1.0;
            // Behind glass an emitter counts in full: shadow rays toward emitters stop at glass (occluded_any).
            if (depth > 0u && !specular_bounce && !behind_glass) {
                wgt = emit_mis(hit, d, last_pdf);
            }
            var c = thr * s.emissive * wgt;
            if (depth > 1u && !straight) {
                let lum = luminance(c);
                if (lum > clamp_v) { c *= clamp_v / lum; }
            }
            if (depth == 0u) { e0 += c; }
            radiance += c;
        }
        if (s.card) { break; }
        if (depth == P.counts.z) { break; }
        // Live: the path ends in the radiance cache at its first bounce. Surfaces seen in mirror-like reflections
        // (specular lobe below roughness 0.4, where UE traces reflections) get exact direct light, as UE's hit
        // lighting does, and the cache's indirect part; others reflect the cache's total. Rotating tiles train the
        // cache. For the first frames after a reset or a camera cut (live.rs COLD_FALLBACK_FRAMES; flag 1 later),
        // pixels that find their voxel cold trace a fresh estimate and use it meanwhile, without inserting the voxel:
        // the low-sample entries they inserted kept later frames slower until evicted (the crowd's square ~2 ms).
        let cfl = u32(P.cache.w);
        if (cache_on() && depth >= 1u && !(has(F_GLASS) && s.transmission > 0.0)) {
            let v1 = -d;
            let lb1 = lobes(s, max(dot(s.n, v1), 1e-4));
            let hit_light = spec_lobe && rough0 < 0.4 && depth == 1u && (cfl & 2u) == 0u;
            var e = cache_get(s, hit_light);
            let train = (((gid.x >> 3u) + 3u * (gid.y >> 2u) + pcg(P.size.z)) & CACHE_TRAIN_MASK) == 0u && (cfl & 4u) == 0u;
            if ((cfl & 1u) != 0u) { e.w = max(e.w, CACHE_WARM); }
            if (train || e.w < CACHE_WARM) {
                // Training tiles trace deep (coherent: whole SIMD groups); scattered cold pixels one vertex.
                let fresh = cache_train(s, d, vec2<f32>(width, cone_a), select(1u, CACHE_DEPTH, train), train);
                if (e.w < CACHE_WARM) { e = vec4<f32>(select(fresh.total, fresh.total - fresh.direct, hit_light), 1.0); }
            }
            cur_depth = depth;
            var c = thr * cache_reflectance(lb1) * e.rgb;
            // Resampled lights, not the exact loop camera hits get (that costs 2x here: mirror pixels are scattered).
            mirror_path = mirror_path && (cfl & 8u) != 0u;
            if (hit_light) { c += thr * direct_light(s, lb1, v1); }
            let lum = luminance(c);
            if (lum > clamp_v && !straight) { c *= clamp_v / lum; }
            radiance += c;
            break;
        }

        let v = -d;
        let nov = max(dot(s.n, v), 1e-4);
        let lb = lobes(s, nov);
        var dl = thr * direct_light(s, lb, v);
        if (depth > 0u && !straight) {
            let lum = luminance(dl);
            if (lum > clamp_v) { dl *= clamp_v / lum; }
        }
        radiance += dl;
        // Live: the deterministic direct light leaves the radiance (and its specular part the specular) for a signal of
        // its own, written here so nothing more stays live through the path. It carries the transmittance to the camera
        // (fog, media) as the radiance did: left out, the diffuse signal's clamp at 0 dropped the attenuation.
        if (LIVE && depth == 0u && arrayLength(&direct_out) >= P.size.x * P.size.y && !non_finite(dl_det) && !non_finite(dl_det_spec)) {
            radiance -= thr * dl_det;
            dl_spec = max(dl_spec - dl_det_spec, vec3<f32>(0.0));
            if (P.misc.w < 0.5) { direct_out[gid.y * P.size.x + gid.x] = vec4<f32>(thr * dl_det, 0.0); }
        }

        let l = sample_bsdf(s, lb, v);
        // Through glass: a delta lobe (weight tint / its sampling share, no MIS: what lies behind counts in full) whose
        // light the live viewport files with the specular, like a reflection.
        let through = has(F_GLASS) && sampled_trans;
        var pdf = 0.0;
        var wgt = lb.tint / max(lb.p_trans, 1e-6);
        if (through) {
            split_frac = vec3<f32>(1.0);
        } else {
            pdf = pdf_bsdf(s, lb, v, l);
            wgt = eval_bsdf(s, lb, v, l) / max(pdf, 1e-20);
        }
        if (depth == 0u) {
            spec0 = thr * dl_spec;
            k_spec = split_frac;
            rad0 = radiance;
            split = true;
            spec_lobe = sampled_spec || through;
            through0 = through;
            glass0 = has(F_GLASS) && s.transmission > 0.0;
            refl_e = lb.e_spec;
            if (sampled_spec && !through && lb.coat <= 0.0) {
                refl_l = l;
                refl_pdf = pdf_spec(s, lb, v, l);
                refl_w = wgt;
            }
        }
        if ((!through && pdf <= 0.0) || all(wgt <= vec3<f32>(0.0))) { break; }
        thr *= wgt;
        if (through && dot(l, s.ng) < 0.0) {
            behind_glass = true;
            bent = !bent;
        } else if (through) {
            // Total internal reflection: a specular vertex (no light sampling covers what it reflects).
            last_pdf = 0.0;
            specular_bounce = true;
            behind_glass = false;
            bent = false;
        } else {
            last_pdf = pdf;
            specular_bounce = false;
            behind_glass = false;
            bent = false;
        }
        straight = straight && through;
        cone_w = width;
        cone_a += 2.0 * s.roughness * s.roughness;
        mirror_path = (depth == 0u || mirror_path) && s.roughness < 0.3 && (sampled_spec || through);
        if (depth >= P.counts.w) {
            let q = min(max(thr.x, max(thr.y, thr.z)), 0.95);
            if (rnd() > q) { break; }
            thr /= q;
        }
        o = offset_origin(s.p, select(s.ng, -s.ng, dot(l, s.ng) < 0.0));
        d = l;
    }

    // Integer tests: Metal's fast-math folds `x != x` away, so float NaN checks are unreliable.
    if (non_finite(radiance)) { radiance = vec3<f32>(0.0); }
    if (non_finite(albedo)) { albedo = vec3<f32>(0.0); }
    if (non_finite(normal)) { normal = vec3<f32>(0.0, 1.0, 0.0); }
    if (non_finite(vec3<f32>(first_t))) { first_t = 0.0; }
    if (LIVE) {
        if (non_finite(e0)) { e0 = vec3<f32>(0.0); }
        var spec = spec0;
        // With the reflection resolve, the sampled ray's specular share is left out of both signals: the resolve
        // estimates the whole specular integral from the neighbourhood's specular-lobe rays. Its diffuse share stays.
        let resolve = resolve_on() && split && !through0 && !glass0;
        var dropped = vec3<f32>(0.0);
        if (split) {
            if (resolve) { dropped = (radiance - rad0) * k_spec; } else { spec += (radiance - rad0) * k_spec; }
        }
        if (non_finite(spec)) { spec = vec3<f32>(0.0); }
        if (non_finite(dropped)) { dropped = vec3<f32>(0.0); }
        spec = clamp(spec, vec3<f32>(0.0), max(radiance - e0, vec3<f32>(0.0)));
        let diff = max(radiance - e0 - spec - dropped, vec3<f32>(0.0));
        if (P.misc.w < 0.5 && arrayLength(&motion_out) >= w * h) {
            motion_out[pix] = select(vec4<f32>(0.0), mv, !non_finite(mv.xyz));
        }
        if (resolve_bound() && P.misc.w < 0.5) {
            // Radiance along the specular ray: what the path gathered after it, over the ray's weight.
            var li = vec3<f32>(0.0);
            if (resolve && refl_pdf > 0.0) {
                li = select((radiance - rad0) / refl_w, vec3<f32>(0.0), refl_w < vec3<f32>(1e-6));
            }
            if (non_finite(li)) { li = vec3<f32>(0.0); }
            // A sample that took the diffuse lobe leaves its specular share (dropped) for the resolve to return when no
            // specular-lobe ray around it can stand in (w = -1 marks it).
            if (resolve && !(refl_pdf > 0.0)) { li = dropped; }
            refl[pix * 2u] = vec4<f32>(li, select(select(0.0, -1.0, resolve), min(refl_pdf, 1e6), resolve && refl_pdf > 0.0));
            refl[pix * 2u + 1u] = vec4<f32>(select(vec3<f32>(0.0), refl_e, resolve), bitcast<f32>(oct_enc(select(vec3<f32>(0.0, 0.0, 1.0), refl_l, refl_pdf > 0.0))));
        }
        // misc.w = index of this sample within the frame: the first one also writes the g-buffer.
        if (P.misc.w < 0.5) {
            // normal.w = this sample's sub-pixel jitter (12 bits each), for the upsampling reconstruction.
            let jit = round(jx * 4095.0) * 4096.0 + round(jy * 4095.0);
            accum[pix] = Pixel(vec4<f32>(diff, first_t), vec4<f32>(albedo, select(1.0, 2.0, anim)), vec4<f32>(normal, jit), vec4<f32>(e0, rough0), vec4<f32>(spec, spec_t));
        } else {
            var px = accum[pix];
            px.color = vec4<f32>(px.color.rgb + diff, px.color.w);
            px.emit = vec4<f32>(px.emit.rgb + e0, px.emit.w);
            px.spec = vec4<f32>(px.spec.rgb + spec, px.spec.w);
            accum[pix] = px;
        }
        return;
    }
    var px = accum[pix];
    px.color += vec4<f32>(radiance, 1.0);
    let lum = luminance(radiance);
    px.albedo += vec4<f32>(albedo, lum);
    px.normal += vec4<f32>(normal, lum * lum);
    // `render --signals` (P.motion.y, a live-only lane otherwise): what the camera ray saw directly and the specular
    // share at the camera hit, split as the live denoiser splits them (no reflection resolve), in the slots only live
    // renders use. The diffuse signal is the rest.
    if (P.motion.y > 0.5) {
        var sp = spec0;
        if (split) { sp += (radiance - rad0) * k_spec; }
        let em = select(e0, vec3<f32>(0.0), non_finite(e0));
        sp = select(clamp(sp, vec3<f32>(0.0), max(radiance - em, vec3<f32>(0.0))), vec3<f32>(0.0), non_finite(sp));
        px.emit += vec4<f32>(em, 0.0);
        px.spec += vec4<f32>(sp, 0.0);
    }
    accum[pix] = px;
}

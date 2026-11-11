// Particle pass (gpu.rs `ParticleGpu`): every frame, before the trace, each particle system's particle function places
// its particles. A particle is a quad of two dynamic triangles (scene.rs `ParticleSystem`) and its own object (pivot,
// bounds, per-particle data for material functions). The pass writes the kernel's triangle data (positions and shading
// attributes), last frame's positions as `prev_verts` (motion vectors; from its own copy `last`, since a repose
// re-uploads the dynamic tail from the CPU), the BLAS input vertices and the objects; the dynamic BLASes are rebuilt
// after it.
//
// A particle function is a WGSL body (`ParticleFunction` in @threert/scene) run per particle:
//     fn pf_<name>(i: PIn, p: array<vec4<f32>, 8>) -> POut { var o = POut(...); <code> return o; }
// with its parameters in `p` ($Name in the DSL). It is a function of time, the camera and the particle's fixed random
// numbers: no state, so every client places the same particles.

struct PIn {
    index: u32,       // particle index, 0 .. count
    count: u32,
    time: f32,        // scene time, s
    dt: f32,          // frame time, s
    cam: vec3<f32>,   // camera position, m
    fwd: vec3<f32>,   // camera forward
    rain: f32,        // scene.rain (0..1)
    wetness: f32,
    rand: vec4<f32>,  // four uniform random numbers, fixed per particle
    pivot: vec3<f32>, // the node's world position
    custom: vec4<f32>, // the node's per-instance data (`customData`; e.g. a burst's start time, set live by the game)
};

struct POut {
    center: vec3<f32>,
    // Half extents of the quad: uv.x grows along `u`, image up (uv.y = 0) is at +`v`.
    u: vec3<f32>,
    v: vec3<f32>,
    visible: bool,
    custom: vec4<f32>, // per-particle data: the material function's `i.custom`
};

struct PP {
    cam: vec4<f32>,   // position, scene time
    fwd: vec4<f32>,   // forward, frame time
    env: vec4<f32>,   // rain, wetness, first run (1: no last frame, motion vectors 0), -
};

// scene.rs GpuTriAttr: normals (w: uv parts), uv parts + material, tangent + bitangent sign, (texel density, object).
struct TriAttr { n0: vec4<f32>, n1: vec4<f32>, n2: vec4<f32>, uv: vec4<f32>, t: vec4<f32>, x: vec4<f32> };

// a: (first triangle in the GPU order, count, first object, function); b: (seed, first dynamic triangle, translucent
// (1: its triangles are the translucency pass's, `tblas` from the first translucent triangle), first translucent
// triangle); c: the node's pivot, w > 0: collapse every particle (the system was just hidden: gpu.rs
// `set_particles_hidden`); p: parameters; d: the node's customData.
struct System { a: vec4<u32>, b: vec4<u32>, c: vec4<f32>, p: array<vec4<f32>, 8>, d: vec4<f32> };

@group(0) @binding(0) var<uniform> PPS: PP;
@group(0) @binding(1) var<storage, read> systems: array<System>;
@group(0) @binding(2) var<storage, read_write> verts: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> prev_verts: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> blas: array<f32>;
@group(0) @binding(5) var<storage, read_write> objects: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> attrs: array<TriAttr>;
// This pass's last positions of the dynamic tail's particle triangles (prev_verts' indexing).
@group(0) @binding(7) var<storage, read_write> last: array<vec4<f32>>;
// The translucent particles' BLAS input (gpu.rs `Translucent`).
@group(0) @binding(8) var<storage, read_write> tblas: array<f32>;

const OBJECT_STRIDE: u32 = 3u; // scene.rs OBJECT_STRIDE

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn unit(h: u32) -> f32 { return f32(h >> 8u) * (1.0 / 16777216.0); }

// @particle-functions-begin
fn particle_function(f: u32, i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(i.pivot, vec3<f32>(0.0), vec3<f32>(0.0), false, vec4<f32>(0.0)); }
// @particle-functions-end

// One triangle of a quad: positions, last frame's as prev_verts, shading normal / tangent / texel density (`lod_k`, as
// scene.rs tri_attr), BLAS input.
// Translucent triangles (`trans` = first translucent triangle) have no motion vectors (the pass draws them over the
// image each frame) and their own BLAS input.
fn put_tri(t: u32, dyn_first: u32, trans: u32, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, n: vec3<f32>, tg: vec3<f32>, lod_k: f32) {
    let flags = verts[t * 3u].w;
    let now = array<vec4<f32>, 3>(vec4<f32>(a, flags), vec4<f32>(b - a, 0.0), vec4<f32>(c - a, 0.0));
    let first = PPS.env.z > 0.5;
    for (var k = 0u; k < 3u; k++) {
        verts[t * 3u + k] = now[k];
        if (trans == 0xffffffffu) {
            let j = (t - dyn_first) * 3u + k;
            prev_verts[j] = select(last[j], now[k], first);
            last[j] = now[k];
        }
    }
    // Component writes: the w's hold the uv corners (and x.y the object), left as built.
    attrs[t].n0.x = n.x;
    attrs[t].n0.y = n.y;
    attrs[t].n0.z = n.z;
    attrs[t].n1.x = n.x;
    attrs[t].n1.y = n.y;
    attrs[t].n1.z = n.z;
    attrs[t].n2.x = n.x;
    attrs[t].n2.y = n.y;
    attrs[t].n2.z = n.z;
    attrs[t].t = vec4<f32>(tg, 1.0);
    attrs[t].x.x = lod_k;
    let corners = array<vec3<f32>, 3>(a, b, c);
    for (var k = 0u; k < 3u; k++) {
        for (var j = 0u; j < 3u; j++) {
            if (trans == 0xffffffffu) {
                blas[t * 9u + k * 3u + j] = corners[k][j];
            } else {
                tblas[(t - trans) * 9u + k * 3u + j] = corners[k][j];
            }
        }
    }
}

@compute @workgroup_size(64, 1, 1)
fn particles(@builtin(global_invocation_id) gid: vec3<u32>) {
    let s = systems[gid.y];
    let k = gid.x;
    if (k >= s.a.y) { return; }
    var i: PIn;
    i.index = k;
    i.count = s.a.y;
    i.time = PPS.cam.w;
    i.dt = PPS.fwd.w;
    i.cam = PPS.cam.xyz;
    i.fwd = PPS.fwd.xyz;
    i.rain = PPS.env.x;
    i.wetness = PPS.env.y;
    let h = pcg(s.b.x * 2654435761u ^ pcg(k));
    i.rand = vec4<f32>(unit(pcg(h)), unit(pcg(h + 1u)), unit(pcg(h + 2u)), unit(pcg(h + 3u)));
    i.pivot = s.c.xyz;
    i.custom = s.d;
    var o = particle_function(s.a.w, i, s.p);
    if (!o.visible || s.c.w > 0.5) {
        o.u = vec3<f32>(0.0);
        o.v = vec3<f32>(0.0);
    }
    // Two triangles, uv (0, 1) (1, 1) (1, 0) and (0, 1) (1, 0) (0, 0) as the scene build gave them.
    let c00 = o.center - o.u - o.v;
    let c10 = o.center + o.u - o.v;
    let c11 = o.center + o.u + o.v;
    let c01 = o.center - o.u + o.v;
    // Normal u x v, tangent (dP/du) along u, bitangent (image up) along v; uv area 1 over world area 4 |u x v|.
    let uxv = cross(o.u, o.v);
    let area = length(uxv);
    let n = select(vec3<f32>(0.0, 0.0, 1.0), uxv / area, area > 1e-12);
    let tg = select(vec3<f32>(1.0, 0.0, 0.0), normalize(o.u), dot(o.u, o.u) > 1e-24);
    let lod_k = select(0.0, 0.5 * log2(1.0 / (4.0 * area)), area > 1e-12);
    let t = s.a.x + 2u * k;
    let trans = select(0xffffffffu, s.b.w, s.b.z == 1u);
    put_tri(t, s.b.y, trans, c00, c10, c11, n, tg, lod_k);
    put_tri(t + 1u, s.b.y, trans, c00, c11, c01, n, tg, lod_k);
    let oi = (s.a.z + k) * OBJECT_STRIDE;
    objects[oi] = vec4<f32>(o.center, objects[oi].w);
    objects[oi + 1u] = vec4<f32>(2.0 * (abs(o.u) + abs(o.v)), objects[oi + 1u].w);
    objects[oi + 2u] = o.custom;
}

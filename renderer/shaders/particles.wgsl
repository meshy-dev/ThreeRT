// Particle pass (gpu.rs `ParticleGpu`): every frame, before the trace, each particle system's particle function places
// its particles. A particle is a quad of its system's mesh (scene.rs `ParticleSystem`: four vertices) and its own
// object (pivot, bounds, per-particle data for material functions). The pass writes the vertices (positions and
// shading attributes; the systems' BLASes are built from them after it), last frame's positions as `prev_verts`
// (motion vectors; from its own copy `last`, since a repose re-uploads the dynamic tail from the CPU) and the
// objects.
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

// (`TriAttr`: records.wgsl.)

// a: (its mesh's first vertex (four a particle), count, first object, function); b: (seed, where its vertices'
// previous positions begin (the instance's, `scene::Instance::prev`), -, -); c: the node's pivot, w: 1 collapse every particle (the system was just hidden), 2 its first run since it
// was shown (no last frame: motion vectors 0) (gpu.rs `set_particles_hidden`); p: parameters; d: the node's customData.
struct System { a: vec4<u32>, b: vec4<u32>, c: vec4<f32>, p: array<vec4<f32>, 8>, d: vec4<f32> };

@group(0) @binding(0) var<uniform> PPS: PP;
@group(0) @binding(1) var<storage, read> systems: array<System>;
// The scene's vertices (pathtrace.wgsl `vertices`: three rows each) and those that move as they were the previous
// frame (their positions are read).
@group(0) @binding(2) var<storage, read_write> vertices: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> prev_verts: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> objects: array<vec4<f32>>;
// This pass's last positions of the particles' vertices (the previous positions' indexing).
@group(0) @binding(7) var<storage, read_write> last: array<vec4<f32>>;

// vec4s an object in `objects` (scene.rs `object_stride`: 4 in a scene with instance colours), the pipeline's constant.
override OBJECT_STRIDE: u32 = 3u;

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn unit(h: u32) -> f32 { return f32(h >> 8u) * (1.0 / 16777216.0); }

// @particle-functions-begin
fn particle_function(f: u32, i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(i.pivot, vec3<f32>(0.0), vec3<f32>(0.0), false, vec4<f32>(0.0)); }
// @particle-functions-end

// One vertex of a system's mesh (`v`; `j`: where its previous position is kept): its position, the last frame's as
// the previous position, its normal and tangent.
fn put_vertex(v: u32, j: u32, p: vec3<f32>, n: vec3<f32>, tg: vec3<f32>, first: bool) {
    let i = v * 3u;
    // (The w's hold the uv, left as built.)
    vertices[i] = vec4<f32>(p, vertices[i].w);
    vertices[i + 1u] = vec4<f32>(n, vertices[i + 1u].w);
    vertices[i + 2u] = vec4<f32>(tg, 1.0);
    let now = vec4<f32>(p, 0.0);
    prev_verts[j * 3u] = select(last[j], now, first);
    last[j] = now;
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
    let h = pcg((s.b.x * 2654435761u) ^ pcg(k));
    i.rand = vec4<f32>(unit(pcg(h)), unit(pcg(h + 1u)), unit(pcg(h + 2u)), unit(pcg(h + 3u)));
    i.pivot = s.c.xyz;
    i.custom = s.d;
    var o = particle_function(s.a.w, i, s.p);
    if (!o.visible || (s.c.w > 0.5 && s.c.w < 1.5)) {
        o.u = vec3<f32>(0.0);
        o.v = vec3<f32>(0.0);
    }
    // The quad's corners, uv (0, 1) (1, 1) (1, 0) (0, 0) as the scene build gave them.
    let c00 = o.center - o.u - o.v;
    let c10 = o.center + o.u - o.v;
    let c11 = o.center + o.u + o.v;
    let c01 = o.center - o.u + o.v;
    // Normal u x v, tangent (dP/du) along u, bitangent (image up) along v.
    let uxv = cross(o.u, o.v);
    let area = length(uxv);
    let n = select(vec3<f32>(0.0, 0.0, 1.0), uxv / area, area > 1e-12);
    let tg = select(vec3<f32>(1.0, 0.0, 0.0), normalize(o.u), dot(o.u, o.u) > 1e-24);
    let first = PPS.env.z > 0.5 || s.c.w > 1.5;
    // Its mesh's quad: the corners in the order the build indexed them.
    let t = s.a.x + 4u * k;
    let j = s.b.y + 4u * k;
    put_vertex(t, j, c00, n, tg, first);
    put_vertex(t + 1u, j + 1u, c10, n, tg, first);
    put_vertex(t + 2u, j + 2u, c11, n, tg, first);
    put_vertex(t + 3u, j + 3u, c01, n, tg, first);
    let oi = (s.a.z + k) * OBJECT_STRIDE;
    objects[oi] = vec4<f32>(o.center, objects[oi].w);
    objects[oi + 1u] = vec4<f32>(2.0 * (abs(o.u) + abs(o.v)), objects[oi + 1u].w);
    objects[oi + 2u] = o.custom;
}

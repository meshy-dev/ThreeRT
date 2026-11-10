// Stateful GPU particles (gpu.rs `GpuParticlesGpu`; ROADMAP § 6 step 5a): each `GpuParticles` node's particles live in
// a state buffer on the GPU (position and age, velocity and life, colour and size), emitted, updated, collided and
// retired here every frame; the CPU keeps no particle. Like `Particles` (particles.wgsl), a particle is a quad of two
// dynamic triangles and its own object (pivot, bounds, per-particle data: its colour, the material function's
// `i.custom`); this pass writes the kernel's triangle data, last frame's positions (motion vectors), the BLAS input and
// the objects, and the dynamic BLASes are rebuilt after it.
//
// A particle program is two WGSL bodies (`ParticleProgram` in @threert/scene), parameters `$Name` in `p`:
//     fn pe_<name>(i: EIn, p: array<vec4<f32>, 8>) -> PState { var s: PState; <emit code> return s; }
//     fn pu_<name>(s_in: PState, i: UIn, p: array<vec4<f32>, 8>) -> PState { var s = s_in; <update code> return s; }
// `emit` makes a new particle: from the rate (the node's position, its random numbers) or from a spawn the game sent
// (`i.spawned`: its position and colour in `i.at` / `i.color`). `update` changes velocity (forces), colour and size each
// frame; the pass then moves it, collides it and ages it, and retires it at its life's end.

struct PState {
    pos: vec3<f32>,
    vel: vec3<f32>,
    age: f32,   // s since it was emitted
    life: f32,  // s it lives
    color: vec4<f32>,
    size: f32,  // half the quad's side, m
};

struct EIn {
    index: u32,       // slot, 0 .. capacity
    time: f32,
    rand: vec4<f32>,  // four uniform random numbers, fresh for each emission
    pivot: vec3<f32>, // the node's position
    spawned: bool,    // a spawn the game sent (`GpuParticles.spawn`)
    at: vec3<f32>,    // its position (spawned), else the pivot
    tag: f32,         // its free number (spawned: e.g. a size or a kind), else 0
    color: vec4<f32>, // its colour (spawned), else 1
};

struct UIn {
    index: u32,
    time: f32,
    dt: f32,
    cam: vec3<f32>,
    pivot: vec3<f32>,
};

struct PP {
    cam: vec4<f32>,   // position, scene time
    fwd: vec4<f32>,   // forward, frame time (the step)
    env: vec4<f32>,   // first run (1: no last frame, motion vectors 0), -, -, -
    // The voxel occupancy grid (`collide.voxels`): its origin (m) and cell size, its cells per axis (words: 32 cells
    // along x each, rows x then z then y).
    grid_o: vec4<f32>,
    grid_n: vec4<u32>,
};

// a: (first triangle in the GPU order, capacity, first object, program); b: (seed, first dynamic triangle, this frame's
// first emitted slot, emitted count); c: pivot, w = collision flags (1 plane, 2 voxels); d: (plane height, restitution,
// friction, -); e: (first spawn, spawned count, frame number, -); p: parameters.
struct System { a: vec4<u32>, b: vec4<u32>, c: vec4<f32>, d: vec4<f32>, e: vec4<u32>, p: array<vec4<f32>, 8> };

// scene.rs GpuTriAttr: normals (w: uv parts), uv parts + material, tangent + bitangent sign, (texel density, object).
struct TriAttr { n0: vec4<f32>, n1: vec4<f32>, n2: vec4<f32>, uv: vec4<f32>, t: vec4<f32>, x: vec4<f32> };

@group(0) @binding(0) var<uniform> PPS: PP;
@group(0) @binding(1) var<storage, read> systems: array<System>;
@group(0) @binding(2) var<storage, read_write> verts: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> prev_verts: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> blas: array<f32>;
@group(0) @binding(5) var<storage, read_write> objects: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> attrs: array<TriAttr>;
// This pass's last positions of its triangles (prev_verts' indexing; a repose re-uploads the dynamic tail).
@group(0) @binding(7) var<storage, read_write> last: array<vec4<f32>>;
// Per particle: (pos, age), (vel, life), color, (size, alive, -, -); from each system's first state.
@group(0) @binding(8) var<storage, read_write> state: array<vec4<f32>>;
// The game's spawns this frame: (position, tag), colour, each.
@group(0) @binding(9) var<storage, read> spawns: array<vec4<f32>>;
@group(0) @binding(10) var<storage, read> grid: array<u32>;

const OBJECT_STRIDE: u32 = 3u; // scene.rs OBJECT_STRIDE (particles' objects; no instance colours with them)

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn unit(h: u32) -> f32 { return f32(h >> 8u) * (1.0 / 16777216.0); }

// @particle-programs-begin
fn particle_emit(f: u32, i: EIn, p: array<vec4<f32>, 8>) -> PState { var s: PState; s.pos = i.at; s.life = 1.0; s.color = i.color; s.size = 0.05; return s; }
fn particle_update(f: u32, s: PState, i: UIn, p: array<vec4<f32>, 8>) -> PState { return s; }
// @particle-programs-end

// Whether world point `x` is in an occupied cell of the voxel grid.
fn solid(x: vec3<f32>) -> bool {
    let c = (x - PPS.grid_o.xyz) / PPS.grid_o.w;
    if (any(c < vec3<f32>(0.0)) || c.x >= f32(PPS.grid_n.x) || c.y >= f32(PPS.grid_n.y) || c.z >= f32(PPS.grid_n.z)) { return false; }
    let q = vec3<u32>(c);
    let row = (q.y * PPS.grid_n.z + q.z) * PPS.grid_n.w + q.x / 32u;
    return (grid[row] >> (q.x % 32u) & 1u) != 0u;
}

// One step of a particle that moves from `s.pos` by `s.vel` x dt, against the voxel grid and the plane (y = height): on
// contact the velocity along the surface's normal is reflected and scaled by `restitution`, the rest scaled by
// 1 - `friction`, and the particle is put against the surface (its centre `size` from it: the quad's half side).
fn collide(s_in: PState, dt: f32, flags: f32, d: vec4<f32>) -> PState {
    var s = s_in;
    let start = s.pos;
    var to = start + s.vel * dt;
    let fl = u32(flags);
    if ((fl & 2u) != 0u && PPS.grid_n.x > 0u) {
        // The particle's leading corner: its centre and `size` along each axis it moves on.
        let ext = sign(s.vel) * s.size;
        if (solid(to + ext)) {
            // The normal: the axis whose move alone takes the corner into a solid cell (y first: resting on a floor).
            var a = 1u;
            if (!solid(vec3<f32>(start.x, to.y, start.z) + ext)) {
                if (solid(vec3<f32>(to.x, start.y, start.z) + ext)) { a = 0u; }
                else if (solid(vec3<f32>(start.x, start.y, to.z) + ext)) { a = 2u; }
            }
            let v = s.vel[a];
            // Against the face of the cell it entered on that axis.
            let o = PPS.grid_o[a];
            let cell = PPS.grid_o.w;
            let idx = floor((to[a] + ext[a] - o) / cell);
            let face = o + cell * (idx + select(0.0, 1.0, v < 0.0));
            to[a] = face - ext[a] - sign(v) * 1e-4;
            var n = vec3<f32>(0.0);
            n[a] = 1.0;
            s.vel = (s.vel - v * n) * (1.0 - d.z) - v * n * d.y;
            // Still in a cell along another axis (a corner): it stays where it was.
            if (solid(to + ext)) { to = start; }
        }
    }
    if ((fl & 1u) != 0u && to.y < d.x + s.size) {
        to.y = d.x + s.size;
        if (s.vel.y < 0.0) {
            s.vel = vec3<f32>(s.vel.x * (1.0 - d.z), -s.vel.y * d.y, s.vel.z * (1.0 - d.z));
        }
    }
    s.pos = to;
    return s;
}

// One triangle of a quad (particles.wgsl `put_tri`, opaque particles only).
fn put_tri(t: u32, dyn_first: u32, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, n: vec3<f32>, tg: vec3<f32>, lod_k: f32) {
    let flags = verts[t * 3u].w;
    let now = array<vec4<f32>, 3>(vec4<f32>(a, flags), vec4<f32>(b - a, 0.0), vec4<f32>(c - a, 0.0));
    let first = PPS.env.x > 0.5;
    for (var k = 0u; k < 3u; k++) {
        verts[t * 3u + k] = now[k];
        let j = (t - dyn_first) * 3u + k;
        prev_verts[j] = select(last[j], now[k], first);
        last[j] = now[k];
    }
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
            blas[t * 9u + k * 3u + j] = corners[k][j];
        }
    }
}

@compute @workgroup_size(64, 1, 1)
fn gpu_particles(@builtin(global_invocation_id) gid: vec3<u32>) {
    let sys = systems[gid.y];
    let k = gid.x;
    let cap = sys.a.y;
    if (k >= cap) { return; }
    // The system's states: from its first object (one per particle, as its objects).
    let si = (sys.a.z + k) * 4u;
    var s: PState;
    s.pos = state[si].xyz;
    s.age = state[si].w;
    s.vel = state[si + 1u].xyz;
    s.life = state[si + 1u].w;
    s.color = state[si + 2u];
    s.size = state[si + 3u].x;
    var alive = state[si + 3u].y > 0.5;
    // Emitted this frame: the slots from b.z on (b.w of them, wrapping), the first e.y of them from the game's spawns.
    let rel = (k + cap - sys.b.z) % cap;
    if (rel < sys.b.w) {
        var i: EIn;
        i.index = k;
        i.time = PPS.cam.w;
        let h = pcg(sys.b.x * 2654435761u ^ pcg(k ^ pcg(sys.e.z)));
        i.rand = vec4<f32>(unit(pcg(h)), unit(pcg(h + 1u)), unit(pcg(h + 2u)), unit(pcg(h + 3u)));
        i.pivot = sys.c.xyz;
        i.spawned = rel < sys.e.y;
        i.at = sys.c.xyz;
        i.color = vec4<f32>(1.0);
        if (i.spawned) {
            let sp = (sys.e.x + rel) * 2u;
            i.at = spawns[sp].xyz;
            i.tag = spawns[sp].w;
            i.color = spawns[sp + 1u];
        }
        s = particle_emit(sys.a.w, i, sys.p);
        s.age = 0.0;
        alive = true;
    } else if (alive) {
        var u: UIn;
        u.index = k;
        u.time = PPS.cam.w;
        u.dt = PPS.fwd.w;
        u.cam = PPS.cam.xyz;
        u.pivot = sys.c.xyz;
        s = particle_update(sys.a.w, s, u, sys.p);
        s = collide(s, PPS.fwd.w, sys.c.w, sys.d);
        s.age += PPS.fwd.w;
        alive = s.age < s.life;
    }
    state[si] = vec4<f32>(s.pos, s.age);
    state[si + 1u] = vec4<f32>(s.vel, s.life);
    state[si + 2u] = s.color;
    state[si + 3u] = vec4<f32>(s.size, select(0.0, 1.0, alive), 0.0, 0.0);
    // Its quad, facing the camera (collapsed when it is not alive).
    let to_cam = normalize(PPS.cam.xyz - s.pos);
    var right = cross(PPS.fwd.xyz, vec3<f32>(0.0, 1.0, 0.0));
    right = select(vec3<f32>(1.0, 0.0, 0.0), normalize(right), dot(right, right) > 1e-8);
    let up = normalize(cross(right, PPS.fwd.xyz));
    let half = select(0.0, s.size, alive);
    let u = right * half;
    let v = up * half;
    let c00 = s.pos - u - v;
    let c10 = s.pos + u - v;
    let c11 = s.pos + u + v;
    let c01 = s.pos - u + v;
    let uxv = cross(u, v);
    let area = length(uxv);
    let n = select(to_cam, uxv / area, area > 1e-12);
    let tg = select(vec3<f32>(1.0, 0.0, 0.0), normalize(u), dot(u, u) > 1e-24);
    let lod_k = select(0.0, 0.5 * log2(1.0 / (4.0 * area)), area > 1e-12);
    let t = sys.a.x + 2u * k;
    put_tri(t, sys.b.y, c00, c10, c11, n, tg, lod_k);
    put_tri(t + 1u, sys.b.y, c00, c11, c01, n, tg, lod_k);
    let oi = (sys.a.z + k) * OBJECT_STRIDE;
    objects[oi] = vec4<f32>(s.pos, objects[oi].w);
    objects[oi + 1u] = vec4<f32>(vec3<f32>(4.0 * half), objects[oi + 1u].w);
    objects[oi + 2u] = s.color;
}

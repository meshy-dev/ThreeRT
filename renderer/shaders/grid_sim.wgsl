// Density grids changed on the GPU (gpu.rs `GridsGpu`; ROADMAP § 6 step 5b): a `DensityGrid` with a grid program holds
// two fields of four numbers a cell, A (what the volume function reads with `vgrid`: e.g. density, temperature) and B
// (the program's own: e.g. a velocity). Every frame the program runs once per cell and gives the cell's next A and B
// from the fields as they were: a cellular update, so advection, diffusion, forces and sources are all the program's.
// A frame is two half steps (A, B -> A', B' -> A, B), so the fields the renderer reads are always the current ones.
//
// A grid program is a WGSL body (`GridProgram` in @threert/scene), parameters `$Name` in `p`:
//     fn gp_<name>(s_in: Cell, i: CIn, p: array<vec4<f32>, 8>) -> Cell { var s = s_in; <code> return s; }
// It reads the fields anywhere with `ga(uvw)` / `gb(uvw)` (trilinear, clamped to the grid), goes between world
// positions and grid coordinates with `to_uvw(p)` / `to_world(uvw)`, and reads the game's sources with `source(k)`.

struct Cell {
    a: vec4<f32>, // field A: what the volume function reads
    b: vec4<f32>, // field B: the program's own
};

struct CIn {
    cell: vec3<u32>,   // the cell
    dims: vec3<u32>,   // cells per axis
    uvw: vec3<f32>,    // its centre in the grid, 0..1
    p: vec3<f32>,      // its centre in the world, m
    size: vec3<f32>,   // a cell's size in the world, m
    time: f32,         // scene time, s
    dt: f32,           // this step, s (half a frame)
    rand: vec4<f32>,   // four uniform random numbers, fresh for each cell and step
    custom: vec4<f32>, // the volume node's customData
    sources: u32,      // sources the game gave this frame (`source(k)`, k < sources): in its first half step only
};

// A source the game gave (`DensityGrid.emit`): a sphere in the world and two values, for the program to use as it likes
// (e.g. A set to `a` inside it, B pushed by `b`). A program sees each source in one half step: what it adds, it adds once.
struct Source {
    p: vec3<f32>,
    radius: f32,
    a: vec4<f32>,
    b: vec4<f32>,
};

struct SP {
    l2w: array<vec4<f32>, 3>, // the box [-0.5, 0.5]^3 -> world, affine rows
    w2l: array<vec4<f32>, 3>, // world -> the box
    dims: vec4<u32>,          // cells per axis, sources
    at: vec4<u32>,            // field A's first layer in the source texture, in the destination texture, program, step number
    t: vec4<f32>,             // scene time, step (s), -, -
    cell: vec4<f32>,          // a cell's size in the world, m
    custom: vec4<f32>,
    p: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> S: SP;
@group(0) @binding(1) var src_a: texture_3d<f32>;
@group(0) @binding(2) var src_b: texture_3d<f32>;
@group(0) @binding(3) var smp: sampler;
@group(0) @binding(4) var dst_a: texture_storage_3d<rgba16float, write>;
@group(0) @binding(5) var dst_b: texture_storage_3d<rgba16float, write>;
// Three vec4 a source: (position, radius), a, b.
@group(0) @binding(6) var<storage, read> srcs: array<vec4<f32>>;

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn unit(h: u32) -> f32 { return f32(h >> 8u) * (1.0 / 16777216.0); }

// Field A / B as they were before this step, at grid coordinates uvw (0..1; trilinear, clamped to the grid's cells).
fn ga(uvw: vec3<f32>) -> vec4<f32> {
    let n = vec3<f32>(S.dims.xyz);
    let c = clamp(uvw * n, vec3<f32>(0.5), n - 0.5) + vec3<f32>(0.0, 0.0, f32(S.at.x));
    return textureSampleLevel(src_a, smp, c / vec3<f32>(textureDimensions(src_a)), 0.0);
}

fn gb(uvw: vec3<f32>) -> vec4<f32> {
    let n = vec3<f32>(S.dims.xyz);
    let c = clamp(uvw * n, vec3<f32>(0.5), n - 0.5);
    return textureSampleLevel(src_b, smp, c / vec3<f32>(textureDimensions(src_b)), 0.0);
}

// World position <-> grid coordinates (0..1 across the volume's box).
fn to_uvw(p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(S.w2l[0].xyz, p) + S.w2l[0].w, dot(S.w2l[1].xyz, p) + S.w2l[1].w, dot(S.w2l[2].xyz, p) + S.w2l[2].w) + 0.5;
}

fn to_world(uvw: vec3<f32>) -> vec3<f32> {
    let l = uvw - 0.5;
    return vec3<f32>(dot(S.l2w[0].xyz, l) + S.l2w[0].w, dot(S.l2w[1].xyz, l) + S.l2w[1].w, dot(S.l2w[2].xyz, l) + S.l2w[2].w);
}

fn source(k: u32) -> Source {
    let s = srcs[k * 3u];
    return Source(s.xyz, s.w, srcs[k * 3u + 1u], srcs[k * 3u + 2u]);
}

// @grid-programs-begin
fn grid_program(f: u32, s: Cell, i: CIn, p: array<vec4<f32>, 8>) -> Cell { return s; }
// @grid-programs-end

@compute @workgroup_size(4, 4, 4)
fn grid_sim(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (any(gid >= S.dims.xyz)) { return; }
    var i: CIn;
    i.cell = gid;
    i.dims = S.dims.xyz;
    i.uvw = (vec3<f32>(gid) + 0.5) / vec3<f32>(S.dims.xyz);
    i.p = to_world(i.uvw);
    i.size = S.cell.xyz;
    i.time = S.t.x;
    i.dt = S.t.y;
    let h = pcg(gid.x ^ pcg(gid.y ^ pcg(gid.z ^ pcg(S.at.w))));
    i.rand = vec4<f32>(unit(pcg(h)), unit(pcg(h + 1u)), unit(pcg(h + 2u)), unit(pcg(h + 3u)));
    i.custom = S.custom;
    i.sources = S.dims.w;
    let c = vec3<i32>(gid);
    var s: Cell;
    s.a = textureLoad(src_a, c + vec3<i32>(0, 0, i32(S.at.x)), 0);
    s.b = textureLoad(src_b, c, 0);
    s = grid_program(S.at.z, s, i, S.p);
    textureStore(dst_a, c + vec3<i32>(0, 0, i32(S.at.y)), s.a);
    textureStore(dst_b, c, s.b);
}

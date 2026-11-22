// The skinning pass: the vertices of meshes that deform, from their model's vertices (skin.rs `SkinCorner`, as the
// file has them) and their poses' palettes, into the scene's positions and vertex attributes. One invocation a vertex;
// a workgroup is 64 vertices of one mesh (`blocks`: the frame's work, the meshes with a new pose alone). What it
// writes is what the CPU skin writes (skin.rs `skin_corner`).

struct Params {
    // x: the frame's blocks.
    n: vec4<u32>,
}

// A mesh that deforms: its first vertex among the scene's and how many, its model's first vertex among `corners`, its
// palette's first matrix.
struct Job {
    first: u32,
    count: u32,
    source: u32,
    palette: u32,
}

struct Corner {
    // Position; w: the first two palette indices (16 bits each).
    p: vec4<f32>,
    // Normal; w: the other two.
    n: vec4<f32>,
    // Tangent; w: CORNER_NORMAL | CORNER_TANGENT, which of the two the file has.
    t: vec4<f32>,
    // The four weights.
    w: vec4<f32>,
}

const CORNER_NORMAL: u32 = 1u;
const CORNER_TANGENT: u32 = 2u;
// Workgroups a row of the dispatch (a dimension holds 65,535).
const ROW: u32 = 32768u;

@group(0) @binding(0) var<uniform> PP: Params;
// (mesh, its first vertex of the block)
@group(0) @binding(1) var<storage, read> blocks: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read> jobs: array<Job>;
@group(0) @binding(3) var<storage, read> corners: array<Corner>;
// Three rows a matrix.
@group(0) @binding(4) var<storage, read> palette: array<vec4<f32>>;
// The scene's vertices (pathtrace.wgsl `positions`, `vattrs`): (position, u); (normal, v), (tangent, hand).
@group(0) @binding(5) var<storage, read_write> positions: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> vattrs: array<vec4<f32>>;

struct Placed {
    p: vec3<f32>,
    n: vec3<f32>,
    t: vec3<f32>,
}

fn unit(v: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    if l > 0.0 && finite(l) {
        return v / l;
    }
    return vec3<f32>(0.0);
}

fn finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

// A corner by its four matrices, blended by its weights; its normal by the blend's inverse transpose.
fn place(c: Corner, base: u32) -> Placed {
    let a = bitcast<u32>(c.p.w);
    let b = bitcast<u32>(c.n.w);
    let j0 = (base + (a & 0xffffu)) * 3u;
    let j1 = (base + (a >> 16u)) * 3u;
    let j2 = (base + (b & 0xffffu)) * 3u;
    let j3 = (base + (b >> 16u)) * 3u;
    let r0 = palette[j0] * c.w.x + palette[j1] * c.w.y + palette[j2] * c.w.z + palette[j3] * c.w.w;
    let r1 = palette[j0 + 1u] * c.w.x + palette[j1 + 1u] * c.w.y + palette[j2 + 1u] * c.w.z + palette[j3 + 1u] * c.w.w;
    let r2 = palette[j0 + 2u] * c.w.x + palette[j1 + 2u] * c.w.y + palette[j2 + 2u] * c.w.z + palette[j3 + 2u] * c.w.w;
    let p = vec4<f32>(c.p.xyz, 1.0);
    let c12 = cross(r1.xyz, r2.xyz);
    var n = vec3<f32>(dot(c12, c.n.xyz), dot(cross(r2.xyz, r0.xyz), c.n.xyz), dot(cross(r0.xyz, r1.xyz), c.n.xyz));
    if dot(r0.xyz, c12) < 0.0 {
        n = -n;
    }
    return Placed(vec3<f32>(dot(r0, p), dot(r1, p), dot(r2, p)), unit(n), unit(vec3<f32>(dot(r0.xyz, c.t.xyz), dot(r1.xyz, c.t.xyz), dot(r2.xyz, c.t.xyz))));
}

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let b = wg.x + wg.y * ROW;
    if b >= PP.n.x {
        return;
    }
    let block = blocks[b];
    let job = jobs[block.x];
    let local = block.y + li;
    if local >= job.count {
        return;
    }
    let k = job.first + local;
    let c = corners[job.source + local];
    let flags = bitcast<u32>(c.t.w);
    let q = place(c, job.palette);
    // (The BLASes are built from the positions: one that is not finite is the mesh's origin there.)
    var p = q.p;
    if !(finite(p.x) && finite(p.y) && finite(p.z)) {
        p = vec3<f32>(0.0);
    }
    positions[k] = vec4<f32>(p, positions[k].w);
    // A model without normals keeps zeros (the triangle's own is made where it is hit); one without tangents too.
    if (flags & CORNER_NORMAL) != 0u {
        vattrs[k * 2u] = vec4<f32>(q.n, vattrs[k * 2u].w);
    }
    if (flags & CORNER_TANGENT) != 0u {
        vattrs[k * 2u + 1u] = vec4<f32>(q.t, vattrs[k * 2u + 1u].w);
    }
}

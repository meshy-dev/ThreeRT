// The skinning pass: the triangles of meshes that deform, from their corners (skin.rs `SkinCorner`, as the file has
// them) and their poses' palettes, into the scene's triangles, shading attributes and BLAS vertices. One invocation a
// triangle; a workgroup is 64 triangles of one mesh (`blocks`: the frame's work, the meshes with a new pose alone).
// What it writes is what the CPU skin writes (skin.rs `Model::triangles`, scene.rs `tri_attr`).

struct Params {
    // x: the frame's blocks.
    n: vec4<u32>,
}

// A mesh that deforms: its first triangle in the scene's triangles and how many, its first triangle among the
// corners, its palette's first matrix.
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

// (pathtrace.wgsl `TriAttr`.)
struct TriAttr {
    n0: vec4<f32>,
    n1: vec4<f32>,
    n2: vec4<f32>,
    uv: vec4<f32>,
    t: vec4<f32>,
    x: vec4<f32>,
}

// (pathtrace.wgsl `TRI_DYNAMIC`.)
const TRI_DYNAMIC: u32 = 4u;
const CORNER_NORMAL: u32 = 1u;
const CORNER_TANGENT: u32 = 2u;
// Workgroups a row of the dispatch (a dimension holds 65,535).
const ROW: u32 = 32768u;
// Whether the BLASes' input is written too: hardware traversal. Software traversal has none (its meshes' trees take
// their boxes on the CPU), and binds a word there.
override BLAS: bool = true;

@group(0) @binding(0) var<uniform> PP: Params;
// (mesh, its first triangle of the block)
@group(0) @binding(1) var<storage, read> blocks: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read> jobs: array<Job>;
@group(0) @binding(3) var<storage, read> corners: array<Corner>;
// Three rows a matrix.
@group(0) @binding(4) var<storage, read> palette: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> verts: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> attrs: array<TriAttr>;
@group(0) @binding(7) var<storage, read_write> blas: array<f32>;

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
    let s = (job.source + local) * 3u;
    let flags = bitcast<u32>(corners[s].t.w);
    let c0 = place(corners[s], job.palette);
    let c1 = place(corners[s + 1u], job.palette);
    let c2 = place(corners[s + 2u], job.palette);
    let e1 = c1.p - c0.p;
    let e2 = c2.p - c0.p;
    let area2 = cross(e1, e2);
    var n0 = c0.n;
    var n1 = c1.n;
    var n2 = c2.n;
    if (flags & CORNER_NORMAL) == 0u {
        n0 = unit(area2);
        n1 = n0;
        n2 = n0;
    }
    var at = attrs[k];
    let uv0 = vec2<f32>(at.n0.w, at.n1.w);
    let d1 = vec2<f32>(at.n2.w, at.uv.x) - uv0;
    let d2 = vec2<f32>(at.uv.y, at.uv.z) - uv0;
    let det = d1.x * d2.y - d2.x * d1.y;
    // The tangent frame: the file's, its first corner's (its hand stays); or the triangle's own from its UVs.
    var t = c0.t;
    var hand = at.t.w;
    if (flags & CORNER_TANGENT) == 0u {
        if abs(det) < 1e-12 {
            var any = vec3<f32>(0.0, 1.0, 0.0);
            if abs(n0.x) < 0.9 {
                any = vec3<f32>(1.0, 0.0, 0.0);
            }
            t = normalize(cross(n0, any));
            hand = 1.0;
        } else {
            let r = 1.0 / det;
            t = unit((e1 * d2.y - e2 * d1.y) * r);
            let bt = (e2 * d1.x - e1 * d2.x) * r;
            hand = -1.0;
            if dot(cross(n0, t), bt) < 0.0 {
                hand = 1.0;
            }
        }
    }
    // Texel density for the mip choice: 0.5 log2(uv area / area).
    let area = 0.5 * length(area2);
    let uv_area = 0.5 * abs(det);
    var lod = 0.0;
    if area > 1e-12 && uv_area > 1e-14 {
        lod = 0.5 * log2(uv_area / area);
    }
    at.n0 = vec4<f32>(n0, at.n0.w);
    at.n1 = vec4<f32>(n1, at.n1.w);
    at.n2 = vec4<f32>(n2, at.n2.w);
    at.t = vec4<f32>(t, hand);
    at.x.x = lod;
    attrs[k] = at;
    // (The flag word: the kernel takes a deforming mesh's motion from the previous frame's vertices.)
    verts[k * 3u] = vec4<f32>(c0.p, bitcast<f32>(TRI_DYNAMIC));
    verts[k * 3u + 1u] = vec4<f32>(e1, 0.0);
    verts[k * 3u + 2u] = vec4<f32>(e2, 0.0);
    if !BLAS {
        return;
    }
    // The BLAS's vertices: a triangle that is not finite has no size there.
    var q0 = c0.p;
    var q1 = c1.p;
    var q2 = c2.p;
    if !(finite(q0.x) && finite(q0.y) && finite(q0.z) && finite(q1.x) && finite(q1.y) && finite(q1.z) && finite(q2.x) && finite(q2.y) && finite(q2.z)) {
        q0 = vec3<f32>(0.0);
        q1 = q0;
        q2 = q0;
    }
    let v = k * 9u;
    blas[v] = q0.x;
    blas[v + 1u] = q0.y;
    blas[v + 2u] = q0.z;
    blas[v + 3u] = q1.x;
    blas[v + 4u] = q1.y;
    blas[v + 5u] = q1.z;
    blas[v + 6u] = q2.x;
    blas[v + 7u] = q2.y;
    blas[v + 8u] = q2.z;
}

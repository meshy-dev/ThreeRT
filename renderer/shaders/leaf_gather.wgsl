// The leaf-gather pass (software traversal): the trees' leaves of meshes whose vertices a pass has just written (a
// new pose: skin.wgsl) take their triangles again from those vertices, through the triangles' indices: v0, e1, e2
// (pathtrace.wgsl `leaves`; indexed.rs `Leaf`). A leaf's ids stay. One invocation a triangle; a workgroup is 64
// triangles of one mesh (`blocks`: the frame's work, the meshes with a new pose alone).

struct Params {
    // x: the frame's blocks; y: the first word of the triangles' indices in `words`.
    n: vec4<u32>,
}

// A mesh: its first triangle among the scene's and how many, its first vertex.
struct Job {
    first: u32,
    count: u32,
    vertex: u32,
    spare: u32,
}

// Workgroups a row of the dispatch (a dimension holds 65,535).
const ROW: u32 = 32768u;

@group(0) @binding(0) var<uniform> PP: Params;
// (mesh, its first triangle of the block)
@group(0) @binding(1) var<storage, read> blocks: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read> jobs: array<Job>;
// The scene's vertices, three rows each, the first (position, u); the words' heap, the triangles' indices in it.
@group(0) @binding(3) var<storage, read> vertices: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> words: array<u32>;
@group(0) @binding(5) var<storage, read_write> leaves: array<vec4<f32>>;

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
    let t = job.first + local;
    let w = PP.n.y + t * 3u;
    let p0 = vertices[(job.vertex + words[w]) * 3u].xyz;
    let p1 = vertices[(job.vertex + words[w + 1u]) * 3u].xyz;
    let p2 = vertices[(job.vertex + words[w + 2u]) * 3u].xyz;
    let k = t * 3u;
    leaves[k] = vec4<f32>(p0, leaves[k].w);
    leaves[k + 1u] = vec4<f32>(p1 - p0, leaves[k + 1u].w);
    leaves[k + 2u] = vec4<f32>(p2 - p0, 0.0);
}

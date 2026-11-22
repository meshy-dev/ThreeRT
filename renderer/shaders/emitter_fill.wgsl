// The light list's triangles (lib.rs `GpuEmitterTri`) taken again from where their triangles are now: what real-time
// engines' "prepare lights" pass does. One run of the list an invocation row (gpu.rs `EmitterFill`): the emitters
// something has moved since the list was written (the triangles the CPU reposed, the particles their passes place,
// the instances that moved).
struct Run { first: u32, count: u32 };
@group(0) @binding(0) var<storage, read> runs: array<Run>;
// The scene's vertices (three rows each: the first has the position), and its triangles' indices (three a triangle,
// counted from the first vertex of the mesh an instance places; a world-space triangle's from 0).
@group(0) @binding(1) var<storage, read> vertices: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> indices: array<u32>;
// Three vec4s an emitter: (v0, its triangle), (e1, its instance), (e2, its path code). The words stay.
@group(0) @binding(2) var<storage, read_write> list: array<vec4<u32>>;
// The instances' records (scene.rs `GpuInstance`: the first three vec4s are the rows of the instance's matrix). Read
// only: they are a section of the buffer the list is one of, and a buffer is not bound to read and to write at once.
@group(0) @binding(3) var<storage, read_write> instances: array<vec4<f32>>;
const INSTANCE_VEC4: u32 = 10u;
const NO_INST: u32 = 0xffffffffu;

@compute @workgroup_size(64)
fn fill(@builtin(global_invocation_id) id: vec3<u32>) {
    let run = runs[id.y];
    if (id.x >= run.count) { return; }
    let r = (run.first + id.x) * 3u;
    let t = list[r].w * 3u;
    // An instance's emitter: its mesh's triangle (v0, e1, e2 in model space) placed by the instance. (The record's
    // seventh vec4: its mesh's first triangle, its object, its mesh's first vertex, its rows.)
    let inst = list[r + 1u].w;
    var base = 0u;
    if (inst != NO_INST) { base = bitcast<u32>(instances[inst * INSTANCE_VEC4 + 6u].z); }
    let v0 = vertices[(base + indices[t]) * 3u].xyz;
    var rows = array<vec3<f32>, 3>(v0, vertices[(base + indices[t + 1u]) * 3u].xyz - v0, vertices[(base + indices[t + 2u]) * 3u].xyz - v0);
    if (inst != NO_INST) {
        let m0 = instances[inst * INSTANCE_VEC4];
        let m1 = instances[inst * INSTANCE_VEC4 + 1u];
        let m2 = instances[inst * INSTANCE_VEC4 + 2u];
        for (var k = 0u; k < 3u; k++) {
            rows[k] = vec3<f32>(dot(m0.xyz, rows[k]), dot(m1.xyz, rows[k]), dot(m2.xyz, rows[k]));
        }
        rows[0] += vec3<f32>(m0.w, m1.w, m2.w);
    }
    for (var k = 0u; k < 3u; k++) {
        list[r + k] = vec4<u32>(bitcast<vec3<u32>>(rows[k]), list[r + k].w);
    }
}

// The light list's triangles (lib.rs `GpuEmitterTri`) taken again from where their triangles are now: what real-time
// engines' "prepare lights" pass does. One run of the list an invocation row (gpu.rs `EmitterFill`): the emitters
// something has moved since the list was written (the triangles the CPU reposed, the particles their passes place).
struct Run { first: u32, count: u32 };
@group(0) @binding(0) var<storage, read> runs: array<Run>;
@group(0) @binding(1) var<storage, read> verts: array<vec4<f32>>;
// Three vec4s an emitter: (v0, its triangle), (e1, its object), (e2, its path code). The words stay.
@group(0) @binding(2) var<storage, read_write> list: array<vec4<u32>>;

@compute @workgroup_size(64)
fn fill(@builtin(global_invocation_id) id: vec3<u32>) {
    let run = runs[id.y];
    if (id.x >= run.count) { return; }
    let r = (run.first + id.x) * 3u;
    let t = list[r].w * 3u;
    for (var k = 0u; k < 3u; k++) {
        list[r + k] = vec4<u32>(bitcast<vec3<u32>>(verts[t + k].xyz), list[r + k].w);
    }
}

// The light list's weights made again on the GPU from what the kernel shades with: each emitter's weight (its
// triangle's area as its record has it now x its material's emissive luminance x its object's power, x the light LOD
// without a tree), the cumulative table and its guide, the light tree's node powers, and the table's header (the
// total, the share outside the tree). What lib.rs `emitter_cdf` and ltree.rs `refit` make on the CPU at load; run when
// something they depend on changed (gpu.rs `EmitterWeights`). The tables are sections of one buffer, so all are bound
// to write, as in emitter_fill.wgsl.
struct Params {
    // Emitters (= entries), the tree's nodes (0: no tree), vec4s an object, bit 0: light LOD.
    n: u32,
    nodes: u32,
    stride: u32,
    flags: u32,
    // Light LOD (lib.rs `emitter_lod`): its reference and d0; the cull's radius, far distance and cone cosine; the
    // cone's direction.
    lod: vec4<f32>,
    cull: vec4<f32>,
    dir: vec4<f32>,
};
@group(0) @binding(0) var<uniform> W: Params;
// Four vec4s an emitter (lib.rs `GpuEmitterTri`): (v0, its triangle), (e1, its instance), (e2, its code), (its
// section, flags, its nominal area, -).
@group(0) @binding(1) var<storage, read_write> list: array<vec4<u32>>;
// An entry an emitter in the table's order (its emitter: bits, cumulative weight, weight, area), the guide (four
// indices a vec4), the header.
@group(0) @binding(2) var<storage, read_write> entries: array<vec4<f32>>;
// A vec4 a node: its sphere packed in two words, its power, its child word.
@group(0) @binding(3) var<storage, read_write> nodes: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> instances: array<vec4<u32>>;
@group(0) @binding(5) var<storage, read_write> materials: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> objects: array<vec4<f32>>;
// Four words a row of an instance: the first has its material.
@group(0) @binding(7) var<storage, read> rows: array<u32>;
// The level at work, the number of levels, each level's (first, count) in the list that follows: the tree's nodes
// by depth, the deepest first (ltree.rs `levels`).
@group(0) @binding(8) var<storage, read_write> levels: array<u32>;
// A block of the table: (its sum, the sum before it); after the last, the total.
@group(0) @binding(9) var<storage, read_write> blocks: array<vec4<f32>>;

const INSTANCE_VEC4: u32 = 10u;
const MATERIAL_VEC4: u32 = 18u;
const NO_INST: u32 = 0xffffffffu;
const EMITTER_MOVES: u32 = 0x80000000u;
const EMITTER_GONE: u32 = 2u;
const EMITTER_NOMINAL: u32 = 4u;
const ROW_MATERIAL: u32 = 0xffffffu;
const LEAF: u32 = 0x80000000u;
const GUIDE: u32 = 65536u;
const BLOCK: u32 = 256u;
const LEVELS: u32 = 32u;

fn block_count() -> u32 { return (W.n + BLOCK - 1u) / BLOCK; }

fn light_lod(c: vec3<f32>) -> f32 {
    let d = distance(c, W.lod.xyz);
    if (W.cull.x > 0.0 && d > W.cull.x && !(d <= W.cull.y && dot(c - W.lod.xyz, W.dir.xyz) >= W.cull.z * d)) { return 0.0; }
    let k = W.lod.w / max(d, W.lod.w);
    return k * k;
}

@compute @workgroup_size(64)
fn weigh(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= W.n) { return; }
    let e = entries[id.x];
    let r = bitcast<u32>(e.x) * 4u;
    let r0 = list[r];
    let r1 = list[r + 1u];
    let r2 = list[r + 2u];
    let r3 = list[r + 3u];
    let e1 = bitcast<vec3<f32>>(r1.xyz);
    let e2 = bitcast<vec3<f32>>(r2.xyz);
    var area = 0.5 * length(cross(e1, e2));
    if ((r3.y & EMITTER_NOMINAL) != 0u) { area = bitcast<f32>(r3.z); }
    var w = 0.0;
    if ((r3.y & EMITTER_GONE) != 0u || r1.w == NO_INST) {
        area = 0.0;
    } else {
        // (The instance's record: its seventh vec4 has its object and its rows.)
        let info = instances[r1.w * INSTANCE_VEC4 + 6u];
        let m = rows[(info.w + r3.x) * 4u] & ROW_MATERIAL;
        w = area * dot(materials[m * MATERIAL_VEC4 + 1u].rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) * objects[info.y * W.stride].w;
        if ((W.flags & 1u) != 0u && (r2.w & EMITTER_MOVES) == 0u) { w *= light_lod(bitcast<vec3<f32>>(r0.xyz) + (e1 + e2) / 3.0); }
    }
    entries[id.x] = vec4<f32>(e.x, 0.0, max(w, 0.0), area);
}

// The cumulative weights by blocks: inside each block, then the blocks' own, then both over the total.
@compute @workgroup_size(64)
fn scan_a(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= block_count()) { return; }
    let lo = id.x * BLOCK;
    let hi = min(lo + BLOCK, W.n);
    var sum = 0.0;
    for (var k = lo; k < hi; k++) {
        sum += entries[k].z;
        entries[k].y = sum;
    }
    blocks[id.x] = vec4<f32>(sum, 0.0, 0.0, 0.0);
}

@compute @workgroup_size(1)
fn scan_b(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x > 0u) { return; }
    var sum = 0.0;
    let nb = block_count();
    for (var b = 0u; b < nb; b++) {
        blocks[b].y = sum;
        sum += blocks[b].x;
    }
    blocks[nb] = vec4<f32>(sum, 0.0, 0.0, 0.0);
}

@compute @workgroup_size(64)
fn scan_c(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= W.n) { return; }
    let total = blocks[block_count()].x;
    // (Exactly 1 from the last entry that weighs something on, whatever the division rounds to.)
    let c = blocks[id.x / BLOCK].y + entries[id.x].y;
    entries[id.x].y = select(0.0, select(c / total, 1.0, c >= total), total > 0.0);
}

// The guide: index k is the first entry whose cumulative weight is not below k / GUIDE. Four a vec4, a vec4 an
// invocation.
@compute @workgroup_size(64)
fn guide(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x > GUIDE / 4u) { return; }
    var out = vec4<u32>(0u);
    for (var c = 0u; c < 4u; c++) {
        let k = id.x * 4u + c;
        if (k > GUIDE) { break; }
        let u = f32(k) / f32(GUIDE);
        var lo = 0u;
        var hi = W.n;
        for (var s = 0u; s < 32u && lo < hi; s++) {
            let mid = (lo + hi) / 2u;
            if (entries[mid].y < u) { lo = mid + 1u; } else { hi = mid; }
        }
        out[c] = lo;
    }
    entries[W.n + id.x] = bitcast<vec4<f32>>(out);
}

// The tree's powers, a level a dispatch, the deepest first: a leaf's from its entries, a node's from its two
// children. `next` moves to the level above.
@compute @workgroup_size(64)
fn tree(@builtin(global_invocation_id) id: vec3<u32>) {
    let level = levels[0];
    if (W.nodes == 0u || level >= min(levels[1], LEVELS)) { return; }
    if (id.x >= levels[3u + 2u * level]) { return; }
    let i = levels[2u + 2u * LEVELS + levels[2u + 2u * level] + id.x];
    var nd = nodes[i];
    let w = bitcast<u32>(nd.w);
    var power = 0.0;
    if ((w & LEAF) != 0u) {
        let lo = w & 0x1fffffffu;
        let hi = lo + ((w >> 29u) & 3u) + 1u;
        for (var j = lo; j < hi; j++) { power += entries[j].z; }
    } else {
        power = nodes[w].z + nodes[w + 1u].z;
    }
    nd.z = power;
    nodes[i] = nd;
}

@compute @workgroup_size(1)
fn next(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x > 0u) { return; }
    levels[0] += 1u;
}

// The header: the total, and the share of it that is not the tree's.
@compute @workgroup_size(1)
fn finish(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x > 0u) { return; }
    let total = blocks[block_count()].x;
    var tail = 0.0;
    if (W.nodes > 0u && total > 0.0) { tail = max((total - nodes[0].z) / total, 0.0); }
    entries[W.n + GUIDE / 4u + 1u] = vec4<f32>(total, tail, 0.0, 0.0);
}

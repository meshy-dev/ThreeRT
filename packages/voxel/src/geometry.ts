// The voxel geometry function: a ray against the bricks inside one box of a VoxelGrid layout (grid.ts), a DDA over
// bricks, then over the 8³ voxels of each non-empty brick. The same body serves every box granularity: it walks only the
// bricks the box covers, so neighbouring boxes never report each other's voxels.
import { GeometryFunction } from "@threert/scene";

export const VOXEL_BRICKS = new GeometryFunction("voxel_bricks", {
  helpers: `
// Voxel v (0..511) of brick record w: set? (occupancy bit), and its material.
fn vx_set(w: u32, v: u32) -> bool { return ((gdata(w + (v >> 5u)) >> (v & 31u)) & 1u) != 0u; }
fn vx_mat(base: u32, w: u32, v: u32) -> u32 {
    let m = gdata(w + 16u);
    if ((m & 0x80000000u) != 0u) { return m & 0xffu; }
    return (gdata(base + m + (v >> 2u)) >> (8u * (v & 3u))) & 0xffu;
}
// Slab entry / exit of [lo, hi] for the ray o + d t (inv = 1 / d).
fn vx_slab(o: vec3<f32>, inv: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> vec2<f32> {
    let a = (lo - o) * inv;
    let b = (hi - o) * inv;
    let n = min(a, b);
    let f = max(a, b);
    return vec2<f32>(max(max(n.x, n.y), n.z), min(min(f.x, f.y), f.z));
}`,
  code: `
    let H = i.data;
    let nb = vec3<i32>(i32(gdata(H)), i32(gdata(H + 1u)), i32(gdata(H + 2u)));
    let s = gdataf(H + 3u);
    let corner = vec3<f32>(gdataf(H + 4u), gdataf(H + 5u), gdataf(H + 6u));
    let grid = H + gdata(H + 7u);
    let bricks = H + gdata(H + 8u);
    // The ray in voxel units (t unchanged); the box clipped to the grid, in voxels.
    let ro = (i.o - corner) / s;
    let rd = i.d / s;
    let inv = 1.0 / select(rd, vec3<f32>(1e-20), abs(rd) < vec3<f32>(1e-20));
    let glo = max((i.lo - corner) / s, vec3<f32>(0.0));
    let ghi = min((i.hi - corner) / s, vec3<f32>(nb * 8));
    let span = vx_slab(ro, inv, glo, ghi);
    let t0 = max(span.x, i.tmin);
    let t1 = min(span.y, i.tmax);
    if (t0 < t1) {
        let blo = vec3<i32>(floor(glo / 8.0 + 1e-4));
        let bhi = vec3<i32>(ceil(ghi / 8.0 - 1e-4)) - 1;
        let st = vec3<i32>(sign(rd));
        // Brick DDA from the entry point.
        var b = clamp(vec3<i32>(floor((ro + rd * t0) / 8.0)), blo, bhi);
        let bdt = abs(inv) * 8.0;
        var btm = (vec3<f32>(b * 8) + select(vec3<f32>(0.0), vec3<f32>(8.0), rd > vec3<f32>(0.0)) - ro) * inv;
        btm = select(btm, vec3<f32>(1e30), rd == vec3<f32>(0.0));
        var bt = t0;
        var done = false;
        for (var k = 0; k < 1024 && !done; k++) {
            let bexit = min(min(btm.x, btm.y), min(btm.z, t1));
            let idx = gdata(grid + u32(b.x + nb.x * (b.y + nb.y * b.z)));
            if (idx != 0u) {
                // Voxel DDA inside brick b, between bt and bexit.
                let w = bricks + (idx - 1u) * 17u;
                let vlo = b * 8;
                var v = clamp(vec3<i32>(floor(ro + rd * (bt + 1e-5 * max(1.0, bt)))), vlo, vlo + 7);
                let vdt = abs(inv);
                var vtm = (vec3<f32>(v) + select(vec3<f32>(0.0), vec3<f32>(1.0), rd > vec3<f32>(0.0)) - ro) * inv;
                vtm = select(vtm, vec3<f32>(1e30), rd == vec3<f32>(0.0));
                for (var q = 0; q < 24; q++) {
                    let l = v - vlo;
                    let vi = u32(l.x + 8 * l.y + 64 * l.z);
                    if (vx_set(w, vi)) {
                        // Entry into this voxel: its latest slab entry, the face it is crossed through.
                        // An axis the ray does not move along never bounds it (its slab is entered at -inf, left at
                        // +inf), as the DDA steps never take it.
                        let flat = abs(rd) < vec3<f32>(1e-20);
                        let en = select((vec3<f32>(v) + select(vec3<f32>(1.0), vec3<f32>(0.0), rd > vec3<f32>(0.0)) - ro) * inv, vec3<f32>(-1e30), flat);
                        let ex = select((vec3<f32>(v) + select(vec3<f32>(0.0), vec3<f32>(1.0), rd > vec3<f32>(0.0)) - ro) * inv, vec3<f32>(1e30), flat);
                        let te = max(max(en.x, en.y), en.z);
                        // The ray must cross this voxel (te <= its exit): the start cell comes from a clamped floor of
                        // the entry point, which at a brick's face can be a neighbour the ray only grazes.
                        if (te > i.tmin && te < i.tmax && te <= min(min(ex.x, ex.y), ex.z)) {
                            o.t = te;
                            let ax = select(select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), en.y >= en.z), vec3<f32>(1.0, 0.0, 0.0), en.x >= en.y && en.x >= en.z);
                            o.normal = -ax * sign(rd);
                            o.material = vx_mat(H, w, vi);
                            o.attr = vec2<f32>(f32(o.material), 0.0);
                            done = true;
                            break;
                        }
                    }
                    let tn = min(min(vtm.x, vtm.y), vtm.z);
                    if (tn >= bexit) { break; }
                    if (vtm.x <= vtm.y && vtm.x <= vtm.z) { v.x += st.x; vtm.x += vdt.x; }
                    else if (vtm.y <= vtm.z) { v.y += st.y; vtm.y += vdt.y; }
                    else { v.z += st.z; vtm.z += vdt.z; }
                    // Out of the brick by the cells themselves: the summed step times drift from the brick's exit time,
                    // and a cell outside would read another voxel's bit.
                    if (any(v < vlo) || any(v > vlo + 7)) { break; }
                }
            }
            if (done || bexit >= t1) { break; }
            bt = bexit;
            if (btm.x <= btm.y && btm.x <= btm.z) { b.x += st.x; btm.x += bdt.x; }
            else if (btm.y <= btm.z) { b.y += st.y; btm.y += bdt.y; }
            else { b.z += st.z; btm.z += bdt.z; }
            if (any(b < blo) || any(b > bhi)) { break; }
        }
    }`,
});

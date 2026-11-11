// Units: software traversal's second level (gpu.rs F_UNITS, renderer/src/units.rs; docs/RUNTIME.md "Software traversal
// with units"). Spliced into pathtrace.wgsl's software traversal by `kernel_source` for scenes that have units; a scene
// without them, and hardware traversal, never see this file.
//
// A unit is a model instance (`instances`, as hardware traversal's): its mesh's triangles in model space in
// `tri_verts`, from the record's first triangle; its mesh's tree in `nodes`, from the root in the record's `n0.w`
// (child indices count from that root, a leaf's triangles from the mesh's first). `n1.w`: the unit casts shadows.
// The top tree, over the units' world boxes, starts at node `P.motion.z` (bits; child indices count from it); each of
// its leaves is one unit, `left_or_first` its record. `P.motion.w` (bits): how many units there are. (`P.motion.y`
// is `render --signals`'.)
//
// `trace` walks the still scene first and calls this with its hit: the ray is already cut to it. Both levels share one
// stack: a unit's walk pushes above what the top walk has pending and stops when it is back down to it. Every index
// read here was checked on the CPU when it was written (units.rs `check_mesh`, `check_top`).
fn trace_units(o: vec3<f32>, d: vec3<f32>, hit_in: Hit, mode: u32) -> Hit {
    var hit = hit_in;
    if (bitcast<u32>(P.motion.w) == 0u) { return hit; }
    let top = bitcast<u32>(P.motion.z);
    let inv_d = 1.0 / select(d, vec3<f32>(1e-20), abs(d) < vec3<f32>(1e-20));
    var stack: array<u32, 48>;
    var sp = 0u;
    var node = top;
    loop {
        let n = nodes[node];
        if (n.count > 0u) {
            // A top leaf: one unit.
            let rec = n.left_or_first;
            let r = instances[rec];
            if (mode != TRACE_SHADOW || bitcast<u32>(r.n1.w) != 0u) {
                // The ray in the unit's space (its direction not renormalised: t is the world's).
                let lo = proc_point(r, o);
                let ld = proc_dir(r, d);
                let linv = 1.0 / select(ld, vec3<f32>(1e-20), abs(ld) < vec3<f32>(1e-20));
                let root = bitcast<u32>(r.n0.w);
                let first = r.info.x;
                let base = sp;
                var m = root;
                loop {
                    let q = nodes[m];
                    if (q.count > 0u) {
                        for (var k = 0u; k < q.count; k++) {
                            let ti = first + q.left_or_first + k;
                            let x = intersect_tri(lo, ld, ti, hit.t);
                            if (x.x < hit.t) {
                                let flags = bitcast<u32>(tri_verts[ti * 3u].w);
                                if (has(F_MASKED) && (flags & TRI_MASKED) != 0u && !cutouts_opaque() && mode != TRACE_ANY_OPAQUE && !alpha_pass(ti, rec, x.y, x.z, x.x)) { continue; }
                                hit = Hit(x.x, ti, x.y, x.z, rec);
                                if (mode != TRACE_CLOSEST) { return hit; }
                            }
                        }
                        if (sp == base) { break; }
                        sp--;
                        m = stack[sp];
                        continue;
                    }
                    let ml = root + q.left_or_first;
                    let mr = ml + 1u;
                    let ql = nodes[ml];
                    let qr = nodes[mr];
                    var ul = ray_aabb(lo, linv, ql.bmin, ql.bmax, hit.t);
                    var ur = ray_aabb(lo, linv, qr.bmin, qr.bmax, hit.t);
                    if (ul == INF && ur == INF) {
                        if (sp == base) { break; }
                        sp--;
                        m = stack[sp];
                        continue;
                    }
                    var mnear = ml;
                    var mfar = mr;
                    if (ur < ul) {
                        mnear = mr;
                        mfar = ml;
                        let tmp = ul; ul = ur; ur = tmp;
                    }
                    m = mnear;
                    if (ur != INF && sp < 48u) {
                        stack[sp] = mfar;
                        sp++;
                    }
                }
            }
            if (sp == 0u) { break; }
            sp--;
            node = stack[sp];
            continue;
        }
        let l = top + n.left_or_first;
        let r = l + 1u;
        let nl = nodes[l];
        let nr = nodes[r];
        var tl = ray_aabb(o, inv_d, nl.bmin, nl.bmax, hit.t);
        var tr = ray_aabb(o, inv_d, nr.bmin, nr.bmax, hit.t);
        if (tl == INF && tr == INF) {
            if (sp == 0u) { break; }
            sp--;
            node = stack[sp];
            continue;
        }
        var near = l;
        var far = r;
        if (tr < tl) {
            near = r;
            far = l;
            let tmp = tl; tl = tr; tr = tmp;
        }
        node = near;
        if (tr != INF && sp < 48u) {
            stack[sp] = far;
            sp++;
        }
    }
    return hit;
}

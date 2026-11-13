// Units: software traversal's second level (gpu.rs F_UNITS, renderer/src/units.rs; docs/RUNTIME.md "Software traversal
// with units"). Part of the kernel under `@if(units)` (pathtrace.wgsl's head): a scene with units under software
// traversal has it; a scene without them, and hardware traversal, do not.
//
// A unit is a model instance (`instances`, as hardware traversal's): its mesh's triangles in model space in
// `tri_verts`, from the record's first triangle; its mesh's tree in `nodes`, from the root in the record's `n0.w`
// (child indices count from that root, a leaf's triangles from the mesh's first). `n1.w`: the unit casts shadows.
// The top tree, over the units' world boxes, starts at node `P.motion.z` (bits; child indices count from it); each of
// its leaves is one unit, `left_or_first` its record. `P.motion.w` (bits): how many units there are. (`P.motion.y`
// is `render --signals`'.) `n2.w` (bits): the unit is placed by a translation alone, its normal rows the identity's
// (units.rs `unit_record`): the ray keeps its direction and only its origin moves, which is what the rows would have
// made of it.
//
// `trace` walks the still scene first and calls this with its hit: the ray is already cut to it.
//
// The stack is the module's, one for an invocation of the kernel, and in a kernel with units the still walk uses it
// too (`@if(!units)` takes its own away). An array a function declares is set to zero each time the
// function is entered, whatever the ray then meets, and a ray's walk is entered many times a pixel: the unit walk's
// own 64 words cost hello 0.3 ms a frame and the city 0.75, and the still walk's 48 cost every scene as much again,
// flattened ones too. This one is set to zero once. A walk reads only what it has pushed: what an earlier ray left
// is never read. The still walk has left it empty; a unit's walk pushes above what the top walk has pending and is
// done when it is back down to it. 64 entries: 19 for the top tree, 45 for a mesh's (units.rs `STACK`); the still
// walk uses 48 of them, as before. Every index read here was checked on the CPU when it was written (units.rs
// `check_mesh`, `check_top`), the depths with them.
//
// One loop walks both levels: a GPU runs a group of rays through a loop together, and a ray that has left an inner
// loop waits for the others of its group to leave it; with the unit's walk as a loop inside the top walk's, every
// ray of a group waited at every unit for the group's slowest (the voxel level, 10,459 units: the trace pass 6.8 ms
// for as many iterations as the flattened scene's 4.5 ms). Here each turn of the loop is a node of whichever tree
// the ray is in: `rec` says which (NO_INST: the top tree), and with it the ray (`ro`, `rd`, `rinv`: the world's, or
// the unit's), what child indices count from (`off`) and the unit's first triangle. A top leaf sets these and goes
// to the unit's root; when the stack is back down to where the unit was entered (`base`), they are set back.
//
// The root's box is tested before the walk: a ray that meets no unit reads one node, and a scene of one unit is a
// root that is a leaf, whose record only the rays that meet its box read.
@if(units) var<private> stack: array<u32, 64>;

@if(units) fn trace_units(o: vec3<f32>, d: vec3<f32>, inv_d: vec3<f32>, hit_in: Hit, mode: u32) -> Hit {
    var hit = hit_in;
    if (bitcast<u32>(P.motion.w) == 0u) { return hit; }
    let top = bitcast<u32>(P.motion.z);
    let whole = nodes[top];
    if (ray_aabb(o, inv_d, whole.bmin, whole.bmax, hit.t) == INF) { return hit; }
    var sp = 0u;
    var node = top;
    var rec = NO_INST;
    var ro = o;
    var rd = d;
    var rinv = inv_d;
    var off = top;
    var first = 0u;
    var base = 0u;
    loop {
        let n = nodes[node];
        if (n.count == 0u) {
            let l = off + n.left_or_first;
            let r = l + 1u;
            let nl = nodes[l];
            let nr = nodes[r];
            var tl = ray_aabb(ro, rinv, nl.bmin, nl.bmax, hit.t);
            var tr = ray_aabb(ro, rinv, nr.bmin, nr.bmax, hit.t);
            if (tl != INF || tr != INF) {
                var near = l;
                var far = r;
                if (tr < tl) {
                    near = r;
                    far = l;
                    let tmp = tl; tl = tr; tr = tmp;
                }
                node = near;
                if (tr != INF && sp < 64u) {
                    stack[sp] = far;
                    sp++;
                }
                continue;
            }
        } else if (rec == NO_INST) {
            // A top leaf: one unit. The ray in the unit's space (its direction not renormalised: t is the world's):
            // `proc_point`, `proc_dir`, or the origin moved alone.
            let u = instance_at(n.left_or_first);
            if (mode != TRACE_SHADOW || bitcast<u32>(u.n1.w) != 0u) {
                rec = n.left_or_first;
                ro = o - vec3<f32>(u.m0.w, u.m1.w, u.m2.w);
                if (bitcast<u32>(u.n2.w) == 0u) {
                    ro = proc_dir(u, ro);
                    rd = proc_dir(u, d);
                    rinv = 1.0 / select(rd, vec3<f32>(1e-20), abs(rd) < vec3<f32>(1e-20));
                }
                off = bitcast<u32>(u.n0.w);
                first = u.info.x;
                base = sp;
                node = off;
                continue;
            }
        } else {
            for (var k = 0u; k < n.count; k++) {
                let ti = first + n.left_or_first + k;
                let x = intersect_tri(ro, rd, ti, hit.t);
                if (x.x < hit.t) {
                    let flags = bitcast<u32>(tri_verts[ti * 3u].w);
                    if (has(F_MASKED) && (flags & TRI_MASKED) != 0u && !cutouts_opaque() && mode != TRACE_ANY_OPAQUE && !alpha_pass(ti, rec, x.y, x.z, x.x)) { continue; }
                    hit = Hit(x.x, ti, x.y, x.z, rec);
                    if (mode != TRACE_CLOSEST) { return hit; }
                }
            }
        }
        // Nothing more under this node. The unit's walk is done when the stack is back where it began: the top's again.
        if (rec != NO_INST && sp == base) {
            rec = NO_INST;
            ro = o;
            rd = d;
            rinv = inv_d;
            off = top;
        }
        if (sp == 0u) { break; }
        sp--;
        node = stack[sp];
    }
    return hit;
}

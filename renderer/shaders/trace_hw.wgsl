// Hardware ray tracing (wgpu ray query). Up to four instances (see Prepared::groups): shadow casters mask 1, the rest
// mask 2; opaque groups never report candidates, alpha-cutout groups are confirmed by `alpha_pass`.
// instance_custom_data holds the instance's first global triangle index, or in scenes where a group starts beyond
// 2^24 triangles (F_HUGE: custom data has 24 bits) its traversal group, whose first triangle is in P.group_first.
@if(hwrt) @group(0) @binding(13) var acc: acceleration_structure;

@if(hwrt) fn trace(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, mode: u32) -> Hit {
    // Ray tracing off (pathtrace.wgsl `rays_off`): past the camera ray, every ray misses.
    if (rays_off) { return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST); }
    let mask = select(0xffu, 1u, mode == TRACE_SHADOW);
    if (!has(F_MASKED)) {
        let h = trace_opaque(o, d, tmax_in, mask, mode != TRACE_CLOSEST);
        return h;
    }
    // Live secondary rays, and live emitter shadow rays (TRACE_ANY_OPAQUE): cutouts opaque (pathtrace.wgsl
    // `cutouts_opaque`, `occluded_emitter`).
    @if(!cutout_lite) if (cutouts_opaque() || mode == TRACE_ANY_OPAQUE) { return trace_opaque(o, d, tmax_in, mask, mode != TRACE_CLOSEST); }
    @if(cutout_lite) if (cutouts_opaque() || mode != TRACE_CLOSEST) { return trace_opaque(o, d, tmax_in, mask, mode != TRACE_CLOSEST); }
    // Alpha-cutout triangles: every query forces opacity, and a cutout hit that fails its alpha test is stepped
    // through. The ray query's own candidate loop (non-opaque traversal) lost hits on Metal: ~0.1 % of rays near
    // cutout foliage missed even the opaque ground behind it or found it unlit (black pixels on the ground; the
    // software BVH had none, and confirming every candidate did not help). Fewer whole-scene steps (the opaque
    // instances once, then the cutout instances alone; or one candidate loop on them) came out within ±0.4 ms on
    // Bistro: the leaves' alpha tests and the traversal past them cost it, however the queries are arranged. Culling
    // the cutout geometry by its non-opaque flag (RAY_FLAG_CULL_NO_OPAQUE) is slow on Metal (Bistro's trace +1.7 ms).
    if (mode != TRACE_CLOSEST) {
        // Any hit blocks a shadow ray, except a cutout one that its alpha lets through.
        let h = trace_opaque(o, d, tmax_in, mask, true);
        if (h.tri == 0xffffffffu || is_proc(h) || !cutout(h.tri) || alpha_pass(h.tri, h.inst, h.u, h.v, h.t)) { return h; }
        // Live: one query whose candidate loop alpha-tests the cutouts (`trace_any_cutout`). It loses hits on Metal
        // from run to run, confirmed ones too: offline it made two runs of one render differ in 75 % of Kabukichō's
        // pixels and 98 % of Bistro's at 64 spp (the ordered steps below: 0–0.06 % and 2–10 %, Bistro's own share of
        // ties). Live it is 1.6 ms of Bistro's trace less at its worst stretch, its image ΔE +0.04 against the
        // unbiased render; offline, which is that render, steps in order.
        if (LIVE) { return trace_any_cutout(o, d, tmax_in, mask); }
    }
    // In order, each query starting past the last cutout hit its alpha let through. Shadow rays take up to 64 of them
    // and count as blocked past that: with 16, as closest hits take, the rest counted as unblocked (light through dense
    // foliage: Bistro's offline render up to 2.3 % bright in a region against the software BVH).
    @if(!cutout_lite2) let steps = select(16u, 64u, mode != TRACE_CLOSEST);
    @if(cutout_lite2) let steps = 4u;
    var t0 = 0.0;
    for (var k = 0u; k < steps; k++) {
        let h = trace_opaque(o + d * t0, d, tmax_in - t0, mask, false);
        if (h.tri == 0xffffffffu) { return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST); }
        let t = t0 + h.t;
        if (is_proc(h) || !cutout(h.tri) || alpha_pass(h.tri, h.inst, h.u, h.v, t)) { return Hit(t, h.tri, h.u, h.v, h.inst); }
        t0 = t + 1e-4 * max(1.0, t);
    }
    if (mode != TRACE_CLOSEST) { return Hit(t0, 0u, 0.0, 0.0, NO_INST); }
    return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST);
}

// One query, not forced opaque; each cutout candidate that passes its alpha test is confirmed, which ends it (as does any
// opaque triangle). Live only (see `trace`).
@if(hwrt) fn trace_any_cutout(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, mask: u32) -> Hit {
    @if(strict) if (ray_bad(o, d, tmax_in)) { return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST); }
    var rq: ray_query;
    rayQueryInitialize(&rq, acc, RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT, mask, 0.0, tmax_in, o, d));
    var pg = GOut(-1.0, vec3<f32>(0.0, 1.0, 0.0), 0u, vec2<f32>(0.0));
    var pprim = 0u;
    while (rayQueryProceed(&rq)) {
        let c = rayQueryGetCandidateIntersection(&rq);
        if (has(F_PROC) && c.kind == RAY_QUERY_INTERSECTION_AABB) {
            let g = proc_box(c.instance_custom_data, c.primitive_index, c.world_to_object * vec4<f32>(o, 1.0), c.world_to_object * vec4<f32>(d, 0.0), 0.0, tmax_in);
            // (naga: the hit distance as its own value, not `g.t`, which it scopes wrongly for the ray-query statement.)
            let gt = g.t;
            if (gt >= 0.0) {
                pg = g;
                pprim = c.primitive_index;
                rayQueryGenerateIntersection(&rq, gt);
            }
            continue;
        }
        if (c.kind != RAY_QUERY_INTERSECTION_TRIANGLE) { continue; }
        let ti = hit_tri(c.instance_custom_data, c.geometry_index, c.primitive_index);
        if (alpha_pass(ti, hit_inst(c.instance_custom_data), c.barycentrics.x, c.barycentrics.y, c.t)) { rayQueryConfirmIntersection(&rq); }
    }
    let it = rayQueryGetCommittedIntersection(&rq);
    if (it.kind == RAY_QUERY_INTERSECTION_NONE) { return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST); }
    if (has(F_PROC) && it.kind == RAY_QUERY_INTERSECTION_GENERATED) { return proc_hit(it.t, pg, pprim, it.instance_custom_data); }
    return Hit(it.t, hit_tri(it.instance_custom_data, it.geometry_index, it.primitive_index), it.barycentrics.x, it.barycentrics.y, hit_inst(it.instance_custom_data));
}

// The closest (or with `any`, the first found) hit, every triangle opaque.
@if(hwrt) fn trace_opaque(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, mask: u32, any: bool) -> Hit {
    @if(strict) if (ray_bad(o, d, tmax_in)) { return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST); }
    var rq: ray_query;
    let flags = select(RAY_FLAG_FORCE_OPAQUE, RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT, any);
    rayQueryInitialize(&rq, acc, RayDesc(flags, mask, 0.0, tmax_in, o, d));
    var pg = GOut(-1.0, vec3<f32>(0.0, 1.0, 0.0), 0u, vec2<f32>(0.0));
    var pprim = 0u;
    if (!has(F_PROC)) {
        rayQueryProceed(&rq);
    } else {
        // Procedural boxes come up as candidates although the query forces opacity (that applies to triangles; checked
        // on Metal: one AABB candidate loop is deterministic run to run, unlike cutout-triangle candidates). Each one's
        // geometry function answers up to the closest hit so far; the last hit generated is the committed one unless a
        // triangle committed after it.
        while (rayQueryProceed(&rq)) {
            let c = rayQueryGetCandidateIntersection(&rq);
            if (c.kind != RAY_QUERY_INTERSECTION_AABB) { continue; }
            let cur = rayQueryGetCommittedIntersection(&rq);
            let tmax = select(tmax_in, cur.t, cur.kind != RAY_QUERY_INTERSECTION_NONE);
            let g = proc_box(c.instance_custom_data, c.primitive_index, c.world_to_object * vec4<f32>(o, 1.0), c.world_to_object * vec4<f32>(d, 0.0), 0.0, tmax);
            // (naga: the hit distance as its own value, not `g.t`, which it scopes wrongly for the ray-query statement.)
            let gt = g.t;
            if (gt >= 0.0) {
                pg = g;
                pprim = c.primitive_index;
                rayQueryGenerateIntersection(&rq, gt);
            }
        }
    }
    let it = rayQueryGetCommittedIntersection(&rq);
    if (it.kind == RAY_QUERY_INTERSECTION_NONE) {
        return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST);
    }
    if (has(F_PROC) && it.kind == RAY_QUERY_INTERSECTION_GENERATED) { return proc_hit(it.t, pg, pprim, it.instance_custom_data); }
    let ti = hit_tri(it.instance_custom_data, it.geometry_index, it.primitive_index);
    // Model instance (F_INST, custom data bit 23 | record): ti is its model-space pool triangle.
    return Hit(it.t, ti, it.barycentrics.x, it.barycentrics.y, hit_inst(it.instance_custom_data));
}

// An alpha-cutout triangle: its material has an alpha test (as the build flags TRI_MASKED; model instances' pool
// triangles carry no flags).
@if(hwrt) fn cutout(ti: u32) -> bool {
    return materials[bitcast<u32>(attrs[ti].uv.w)].uv2.w > 0.0;
}

// The model-instance record a hit's custom data names (F_INST), or NO_INST for a traversal group.
@if(hwrt) fn hit_inst(custom: u32) -> u32 {
    return select(NO_INST, custom & (INSTANCE_BIT - 1u), has(F_INST) && (custom & INSTANCE_BIT) != 0u);
}

@if(hwrt) const INSTANCE_BIT: u32 = 0x800000u;

// Global triangle of a hit: a model instance's pool triangle (its cutout geometry 1 after the opaque one), or a
// traversal group's.
@if(hwrt) fn hit_tri(custom: u32, geometry: u32, prim: u32) -> u32 {
    if (has(F_INST) && (custom & INSTANCE_BIT) != 0u) {
        let r = instances[custom & (INSTANCE_BIT - 1u)].info;
        return r.x + select(0u, r.w, geometry == 1u) + prim;
    }
    return group_tri(custom, prim);
}

@if(hwrt) fn group_tri(custom: u32, prim: u32) -> u32 {
    if (has(F_HUGE)) { return P.group_first[custom >> 2u][custom & 3u] + prim; }
    return custom + prim;
}

// gpu.rs `KernelSwitches::strict`: a ray a ray query must not be given (not finite, or a length that is negative or NaN).
@if(hwrt && strict) fn ray_bad(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> bool {
    let tb = bitcast<u32>(tmax);
    return non_finite(o) || non_finite(d) || (tb & 0x80000000u) != 0u || (tb & 0x7fffffffu) > 0x7f800000u;
}

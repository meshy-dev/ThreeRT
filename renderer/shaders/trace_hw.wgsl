// Hardware ray tracing (wgpu ray query). Up to four instances (see Prepared::groups): shadow casters mask 1, the rest
// mask 2 (glass among them; a model's mesh's glass is a geometry of its BLAS that a shadow ray culls, an instance's
// glass among its other sections is stepped through by its row); opaque groups
// never report candidates, alpha-cutout groups are confirmed by `alpha_pass`.
// instance_custom_data holds the instance's first global triangle index, or in scenes where a group starts beyond
// 2^24 triangles (F_HUGE: custom data has 24 bits) its traversal group, whose first triangle is in P.group_first.
@if(hwrt) @group(0) @binding(19) var acc: acceleration_structure;

@if(hwrt) fn trace(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, mode: u32) -> Hit {
    // Ray tracing off (pathtrace.wgsl `rays_off`): past the camera ray, every ray misses.
    if (rays_off) { return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST); }
    let mask = select(0xffu, 1u, mode == TRACE_SHADOW);
    let any = mode != TRACE_CLOSEST;
    // (A scene without cutouts: one query. A feature bit: the kernel has this or what follows, not both. Glass among an
    // instance's other sections is stepped through as a cutout is: `passes`.)
    if (!has(F_MASKED) && !(has(F_IGLASS) && has(F_ROWS))) { return trace_opaque(o, d, tmax_in, mask, any); }
    // Whether a cutout stops this ray as any triangle does: A_CUTOUT_PATHS, A_CUTOUT_EMITTERS (TRACE_ANY_OPAQUE)
    // (pathtrace.wgsl `cutouts_opaque`, `emitter_tr`).
    @if(!cutout_lite) let solid = cutouts_opaque() || mode == TRACE_ANY_OPAQUE;
    @if(cutout_lite) let solid = cutouts_opaque() || any;
    // Alpha-cutout triangles: every query forces opacity, and a cutout hit that fails its alpha test is stepped
    // through. The ray query's own candidate loop (non-opaque traversal) lost hits on Metal: ~0.1 % of rays near
    // cutout foliage missed even the opaque ground behind it or found it unlit (black pixels on the ground; the
    // software BVH had none, and confirming every candidate did not help), differently from run to run (two offline
    // renders of Kabukichō differed in 75 % of their pixels). Fewer whole-scene steps (the opaque instances once, then
    // the cutout instances alone; or one candidate loop on them) came out within ±0.4 ms on Bistro: the leaves' alpha
    // tests and the traversal past them cost it, however the queries are arranged. Culling the cutout geometry by its
    // non-opaque flag (RAY_FLAG_CULL_NO_OPAQUE) was slow on Metal (Bistro's trace +1.7 ms); every geometry but an
    // instanced mesh's glass is flagged opaque now (`trace_opaque`).
    //
    // In order, each query starting past the last cutout hit its alpha let through. Shadow rays take up to 64 of them
    // and count as blocked past that: with 16, as closest hits take, the rest counted as unblocked (light through dense
    // foliage: Bistro's offline render up to 2.3 % bright in a region against the software BVH).
    @if(!cutout_lite2) let steps = select(16u, 64u, any);
    @if(cutout_lite2) let steps = 4u;
    // One ray query in the source for every kind of ray. `trace` is inlined where it is called, and each query written
    // in it costs every ray, taken or not: with a query of its own for opaque cutouts and one for a shadow ray's first
    // hit beside the ordered ones, Zero Day's trace was 9.40 ms for 8.39 and Bistro's 8.56 for 8.05. A shadow ray's
    // first query takes any hit; if that is a cutout its alpha lets through, the ordered ones start from the ray's
    // origin.
    var first = any;
    var t0 = 0.0;
    var hole = false;
    for (var k = 0u; k < steps + u32(any); k++) {
        let h = trace_opaque(o + d * t0, d, tmax_in - t0, mask, first);
        if (h.tri == 0xffffffffu) {
            // (A shadow ray comes here in order only past a cutout that let it through, `shadow_hole`, or past glass.)
            if (any && !first && hole) { shadow_hole = true; }
            return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST);
        }
        let t = t0 + h.t;
        // A shadow ray passes the glass among an instance's sections (its row says so; an instance that is all glass is
        // out of its way by its mask).
        let passes = has(F_IGLASS) && has(F_ROWS) && mask == 1u && hit_row != NO_ROW && (hit_row & ROW_GLASS) != 0u;
        if (!passes) {
            if (!has(F_MASKED) || solid || is_proc(h) || !cutout_hit(h) || alpha_pass(h.tri, h.inst, h.u, h.v, t)) { return Hit(t, h.tri, h.u, h.v, h.inst); }
            hole = true;
        }
        if (first) { first = false; } else { t0 = t + 1e-4 * max(1.0, t); }
    }
    if (any) { return Hit(t0, 0u, 0.0, 0.0, NO_INST); }
    return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST);
}

// The closest (or with `any`, the first found) hit, every triangle opaque.
@if(hwrt) fn trace_opaque(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, mask: u32, any: bool) -> Hit {
    hit_row = NO_ROW;
    @if(strict) if (ray_bad(o, d, tmax_in)) { return Hit(tmax_in, 0xffffffffu, 0.0, 0.0, NO_INST); }
    var rq: ray_query;
    // A shadow ray (mask 1) where a model's mesh mixes glass with other triangles (F_IGLASS): it culls the geometry
    // that is not flagged opaque, the glass parts alone (gpu.rs), instead of forcing every geometry opaque. A mesh that
    // is all glass is out of its way by the instance's mask. (A mesh with sections has every geometry flagged opaque:
    // `trace` steps through its glass.) (Measured on Metal with every geometry opaque: the same
    // pictures, at most 0.5 % of a frame's GPU time.)
    let opacity = select(RAY_FLAG_FORCE_OPAQUE, RAY_FLAG_CULL_NO_OPAQUE, has(F_IGLASS) && mask == 1u);
    let flags = select(opacity, opacity | RAY_FLAG_TERMINATE_ON_FIRST_HIT, any);
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
@if(hwrt) fn cutout(ti: u32, inst: u32) -> bool {
    return material_at(material_of(attrs[ti], inst)).uv2.w > 0.0;
}

// The first word of the row of the hit `trace_opaque` last found (its material and class bits), NO_ROW where its
// instance has no rows: the trace's tests of a hit read no attribute and no material then.
@if(hwrt) var<private> hit_row: u32;
@if(hwrt) const NO_ROW: u32 = 0xffffffffu;
@if(hwrt) fn cutout_hit(h: Hit) -> bool {
    if (has(F_ROWS) && hit_row != NO_ROW) { return (hit_row & ROW_CUTOUT) != 0u; }
    return cutout(h.tri, h.inst);
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
        // (Its mesh's geometries: where the second and the third start, scene.rs `InstMesh::part_offsets`; a third
        // only with glass.)
        let rec = instance_at(custom & (INSTANCE_BIT - 1u));
        // (With rows: a geometry is a section, and its row has where its triangles start.)
        if (has_rows(rec)) {
            hit_row = row_word(rec.info.w, geometry, 0u);
            return row_word(rec.info.w, geometry, 1u) + prim;
        }
        var at = select(0u, rec.info.w, geometry == 1u);
        if (has(F_IGLASS) && geometry == 2u) { at = bitcast<u32>(rec.n0.w); }
        return rec.info.x + at + prim;
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

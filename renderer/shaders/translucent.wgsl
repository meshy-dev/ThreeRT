
// ------------------------------------------------------------------------------------------------
// Translucency pass (gpu.rs `Translucent`; hardware RT only, appended to the kernel module after the trace).
//
// Particles drawn with a `transparent` material are in the kernel's triangle tables (after everything else, from
// TPS.size.z) but in none of the scene's acceleration structures: they cast no shadow and show in no reflection or GI
// (UE's default for translucency). This pass draws them, per output pixel, over the opaque image:
// - depth: the particle ray stops at the opaque surface the pixel sees (live: the traced first hits under the pixel;
//   offline: a ray), so nothing behind a wall shows;
// - overlap: ordered blending, the TL_LAYERS nearest particle hits sorted by distance, front to back;
// - material: the material (maps, opacity) and its material function, with `i.custom` per particle;
// - lighting: non-directional, from the radiance cache's volume voxels at the hit (UE's translucency lighting
//   volume); offline (no cache), a fresh estimate per sample of the same quantity.
// Live, it runs on the composed HDR image after the temporal filter (no history: nothing to leave trails); offline it
// accumulates (colour, transmittance) per sample, laid over the converged image.

@if(hwrt) struct TP {
    size: vec4<u32>, // output width, height, first translucent triangle, mode (0 offline: accumulate; 1 live: over the image)
    view: vec4<u32>, // live: traced width, height (the opaque first hits); training budget (points per frame); 1 = at the traced resolution
    misc: vec4<f32>, // live: share of hit pixels that train the volume voxels, 1 = train only (THREERT_TL_HIDE: what is behind), -, -
};

@if(hwrt) @group(2) @binding(0) var tacc: acceleration_structure;
@if(hwrt) @group(2) @binding(1) var<uniform> TPS: TP;
// Live: the composed image (in place; w kept). Offline: per-pixel sums of (colour, transmittance).
@if(hwrt) @group(2) @binding(2) var<storage, read_write> timg: array<vec4<f32>>;
// Volume training points of this frame (live) and their count.
@if(hwrt) @group(2) @binding(3) var<storage, read_write> vpts: array<vec4<f32>>;
@if(hwrt) @group(2) @binding(4) var<storage, read_write> vcount: array<atomic<u32>>;
// Live at a reduced render scale: (colour, transmittance) per traced pixel, laid over the image by translucent_up.
@if(hwrt) @group(2) @binding(5) var<storage, read_write> tlow: array<vec4<f32>>;

@if(hwrt) const TL_LAYERS: u32 = 4u;

// Volume voxels (cache_point, point_radiance): pathtrace.wgsl, shared with participating media.

// Live: the nearest opaque first hit around output pixel (x, y): of the 2 x 2 traced pixels under it, the nearest
// (sky = none), so nothing shows through at an edge.
@if(hwrt) fn opaque_depth(x: u32, y: u32) -> f32 {
    let iw = TPS.view.x;
    let ih = TPS.view.y;
    let fx = (f32(x) + 0.5) * f32(iw) / f32(TPS.size.x) - 0.5;
    let fy = (f32(y) + 0.5) * f32(ih) / f32(TPS.size.y) - 0.5;
    let x0 = u32(clamp(floor(fx), 0.0, f32(iw - 1u)));
    let y0 = u32(clamp(floor(fy), 0.0, f32(ih - 1u)));
    let x1 = min(x0 + 1u, iw - 1u);
    let y1 = min(y0 + 1u, ih - 1u);
    var t = INF;
    for (var k = 0u; k < 4u; k++) {
        let q = select(y0, y1, k >= 2u) * iw + select(x0, x1, (k & 1u) == 1u);
        let d = accum[q].color.w;
        if (d > 0.0) { t = min(t, d); }
    }
    return t;
}

// Material of a translucent particle at triangle ti (barycentrics b): colour, opacity and emission after maps and its
// material function.
@if(hwrt) fn particle_material(ti: u32, b: vec2<f32>, view: vec3<f32>, lod: f32) -> MatOut {
    let a = attrs[ti];
    let m = material_at(bitcast<u32>(a.uv.w));
    let uv = mesh_uv(a, m, b.x, b.y);
    let tl = mesh_lod(a, m) + lod - LOD_BIAS;
    var r = mat_base(m);
    if (has(F_TEX) && m.tex0.x >= 0) {
        let c = tex(m.tex0.x, uv, tl);
        r.base *= srgb_to_linear(c.rgb);
        r.opacity *= c.a;
    }
    r.emissive = emission(m, uv, tl) * obj_power(a);
    if (has(F_FUNC) && m.tex1.w > 0) {
        r = translucent_material_function(m.tex1.w - 1, mat_in_at(ti, b.x, b.y, view, lod), m, r);
    }
    return r;
}

@if(hwrt) @compute @workgroup_size(8, 8, 1)
fn translucent(@builtin(global_invocation_id) gid: vec3<u32>) {
    let live = TPS.size.w == 1u;
    // Live below full resolution the particles are drawn per traced pixel (as UE draws translucency at the screen
    // percentage) and laid over the output image by translucent_up: a quarter of the rays at the 0.5 floor.
    let low = live && TPS.view.w == 1u;
    let w = select(TPS.size.x, TPS.view.x, low);
    let h = select(TPS.size.y, TPS.view.y, low);
    if (gid.x >= w || gid.y >= h) { return; }
    let pix = gid.y * w + gid.x;
    rng_start(gid.xy, P.size.z, P.size.w ^ 0x5bd1e995u, 4u);
    cur_depth = 0u;
    let cam = rng2(rng_dim(D_CAMERA));
    mirror_path = false;
    // Live: the pixel centre (stable streaks; the image has passed the temporal filter). Offline: a random point in it.
    var jx = 0.5;
    var jy = 0.5;
    if (!live) {
        jx = cam.x;
        jy = cam.y;
    }
    let ndc = vec2<f32>((f32(gid.x) + jx) / f32(w) * 2.0 - 1.0, 1.0 - (f32(gid.y) + jy) / f32(h) * 2.0);
    let d = normalize(P.cam_fwd.xyz + P.cam_right.xyz * (ndc.x * P.cam_pos.w * P.cam_fwd.w) + P.cam_up.xyz * (ndc.y * P.cam_pos.w));
    let o = P.cam_pos.xyz;
    var tmax = INF;
    if (low) {
        let od = accum[pix].color.w;
        if (od > 0.0) { tmax = od; }
    } else if (live) {
        tmax = opaque_depth(gid.x, gid.y);
    } else {
        let hit = trace(o, d, INF, TRACE_CLOSEST);
        if (hit.tri != 0xffffffffu) { tmax = hit.t; }
    }
    // The nearest TL_LAYERS hits, by distance.
    var lt: array<f32, TL_LAYERS>;
    var lp: array<u32, TL_LAYERS>;
    var lb: array<vec2<f32>, TL_LAYERS>;
    var n = 0u;
    var rq: ray_query;
    @if(!strict) rayQueryInitialize(&rq, tacc, RayDesc(0u, 0xffu, 0.0, tmax, o, d));
    @if(strict) let tbad = ray_bad(o, d, tmax);
    @if(strict) rayQueryInitialize(&rq, tacc, RayDesc(0u, 0xffu, 0.0, select(tmax, 0.0, tbad), select(o, vec3<f32>(0.0), tbad), select(d, vec3<f32>(0.0, 0.0, 1.0), tbad)));
    while (rayQueryProceed(&rq)) {
        let c = rayQueryGetCandidateIntersection(&rq);
        if (n == TL_LAYERS && c.t >= lt[TL_LAYERS - 1u]) { continue; }
        var k = min(n, TL_LAYERS - 1u);
        loop {
            if (k == 0u || lt[k - 1u] <= c.t) { break; }
            lt[k] = lt[k - 1u];
            lp[k] = lp[k - 1u];
            lb[k] = lb[k - 1u];
            k -= 1u;
        }
        lt[k] = c.t;
        lp[k] = c.primitive_index;
        lb[k] = c.barycentrics;
        n = min(n + 1u, TL_LAYERS);
    }
    if (n == 0u) {
        if (low) { tlow[pix] = vec4<f32>(0.0, 0.0, 0.0, 1.0); }
        if (!live) { timg[pix] += vec4<f32>(0.0, 0.0, 0.0, 1.0); }
        return;
    }
    // Front to back.
    var col = vec3<f32>(0.0);
    var tr = 1.0;
    var train = live && rng1(rng_dim(D_CACHE)) < TPS.misc.x;
    let foot = 2.0 * P.cam_pos.w / f32(h);
    for (var k = 0u; k < n; k++) {
        let ti = TPS.size.z + lp[k];
        let r = particle_material(ti, lb[k], -d, log2(max(lt[k] * foot, 1e-7)));
        let alpha = clamp(r.opacity, 0.0, 1.0);
        if (alpha <= 0.0) { continue; }
        let x = o + d * lt[k];
        var lit: vec3<f32>;
        if (live) {
            lit = cache_point(x).rgb;
            if (train) {
                let slot = atomicAdd(&vcount[0], 1u);
                if (slot < TPS.view.z) { vpts[slot] = vec4<f32>(x, 0.0); }
                train = false;
            }
        } else {
            lit = point_radiance(x);
        }
        col += tr * alpha * (r.base * lit + r.emissive);
        tr *= 1.0 - alpha;
    }
    if (low) {
        tlow[pix] = vec4<f32>(col, tr);
    } else if (live) {
        if (TPS.misc.y < 0.5) {
            let c = timg[pix];
            timg[pix] = vec4<f32>(c.rgb * tr + col, c.w);
        }
    } else {
        timg[pix] += vec4<f32>(col, tr);
    }
}

// Live, after `translucent` at the traced resolution: its (colour, transmittance) bilinearly up to each output pixel,
// over the image.
@if(hwrt) @compute @workgroup_size(8, 8, 1)
fn translucent_up(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = TPS.size.x;
    let h = TPS.size.y;
    if (gid.x >= w || gid.y >= h || TPS.misc.y >= 0.5) { return; }
    let iw = TPS.view.x;
    let ih = TPS.view.y;
    let fx = clamp((f32(gid.x) + 0.5) * f32(iw) / f32(w) - 0.5, 0.0, f32(iw - 1u));
    let fy = clamp((f32(gid.y) + 0.5) * f32(ih) / f32(h) - 0.5, 0.0, f32(ih - 1u));
    let x0 = u32(fx);
    let y0 = u32(fy);
    let x1 = min(x0 + 1u, iw - 1u);
    let y1 = min(y0 + 1u, ih - 1u);
    let ax = fx - f32(x0);
    let ay = fy - f32(y0);
    let t = mix(mix(tlow[y0 * iw + x0], tlow[y0 * iw + x1], ax), mix(tlow[y1 * iw + x0], tlow[y1 * iw + x1], ax), ay);
    let pix = gid.y * w + gid.x;
    let c = timg[pix];
    timg[pix] = vec4<f32>(c.rgb * t.w + t.rgb, c.w);
}

// Live, after `translucent`: one fresh estimate per training point into its volume voxels (both levels). The points
// are capped per frame (TPS.view.z), so the cost is bounded whatever the particles cover.
@if(hwrt) @compute @workgroup_size(64, 1, 1)
fn volume_train(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    if (k >= min(atomicLoad(&vcount[0]), TPS.view.z)) { return; }
    rng_start(vec2<u32>(k, 0u), P.size.z, P.size.w ^ 0x68e31da4u, 5u);
    mirror_path = false;
    let x = vpts[k].xyz;
    let e = point_radiance_train(x, P.misc.x);
    cache_add(cache_find(cache_key_point(x, VOLUME_FINE), true), e, e);
    cache_add(cache_find(cache_key_point(x, VOLUME_COARSE), true), e, e);
}

//! threert-render as a library: scene loading, the GPU path tracer, the real-time LiveCore (used by the
//! `threert-render` CLI and by the native playground client).

pub mod api;
pub mod bvh;
#[cfg(not(target_arch = "wasm32"))]
pub mod compare;
#[cfg(not(target_arch = "wasm32"))]
pub mod denoise;
pub mod desc;
pub mod files;
pub mod gpu;
#[cfg(not(target_arch = "wasm32"))]
pub mod host;
pub mod live;
pub mod ltree;
pub mod merged;
pub mod post;
pub mod procasset;
pub mod scene;
#[cfg(not(target_arch = "wasm32"))]
pub mod script;
pub mod skin;
pub mod sky;
pub mod tables;
pub mod units;
#[cfg(test)]
mod units_cost;
pub mod texpack;
#[cfg(not(target_arch = "wasm32"))]
pub mod view;
pub mod physics;
pub mod pipeline_cache;
pub mod voxel;
pub mod world;

use glam::{Vec3, Vec3A};
use std::path::PathBuf;
use web_time::Instant;

/// `presets/` next to (an ancestor of) the executable, else ./presets.
pub fn default_presets_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    for anc in exe.ancestors() {
        let p = anc.join("presets");
        if p.join("cinematic.json").exists() {
            return p;
        }
    }
    PathBuf::from("presets")
}


/// Traversal groups (see `group_of`).
pub const GROUPS: usize = 8;

/// Everything the GPU kernels need for a loaded scene: triangle layout for the active traversal backend, emitter CDF,
/// baked sky + its sampling CDF, and the base kernel parameters (camera from the scene).
pub struct Prepared {
    pub tri_verts: Vec<[f32; 4]>,
    /// Triangle counts of the traversal groups, in order: shadow-casting opaque, shadow-casting cutout,
    /// non-shadowing opaque, non-shadowing cutout; then the same four for dynamic triangles (posed models).
    pub groups: [usize; GROUPS],
    pub nodes: Vec<bvh::GpuNode>,
    /// The emitter CDF (`emitter_cdf`): one entry per `emitter_tris`, then the guide table.
    pub emitters: Vec<[f32; 4]>,
    pub env: gpu::EnvMap,
    pub env_cdf: Vec<f32>,
    pub params: gpu::Params,
    pub energy: Vec<[f32; 4]>,
    /// Emissive triangles (also those at power 0), for re-weighting `emitters` after power / material edits.
    pub emitter_tris: Vec<EmitterTri>,
    /// The light tree over the static emitters (`ltree`), its nodes after the CDF in `emitters`.
    pub light_tree: Option<ltree::LightTree>,
    /// Material -> runs of its flattened static triangles (first, end in the traversal order, object), and the
    /// materials that also cover pool or dynamic triangles: a material that starts emitting gets emitter entries in
    /// place from its runs (live/edits.rs `add_material_emitters`), unless fixed.
    pub mat_runs: std::collections::HashMap<u32, Vec<[u32; 3]>>,
    pub fixed_mats: std::collections::HashSet<u32>,
    /// The flattened triangles' build indices in traversal order (`Scene::tris` order: positions for new entries).
    pub tri_order: Vec<u32>,
    /// The brightest sky texel (luminance) from which the environment is light-sampled (`Params::counts2[1]`): 0 when
    /// there are no emitters. A dynamic sky re-decides with each bake (live/load.rs `DynamicSky`).
    pub env_nee_floor: f32,
}

/// The emitter entry of triangle `k` (traversal order) with attributes `a`, or None if it does not emit. Particle
/// triangles are collapsed until the particle pass places them: their nominal area (attrs x.z).
pub fn emitter_tri(k: usize, tri_verts: &[[f32; 4]], a: &scene::GpuTriAttr, materials: &[scene::GpuMaterial]) -> Option<EmitterTri> {
    let mat = a.uv[3].to_bits();
    let m = materials.get(mat as usize)?;
    let lum = 0.2126 * m.emissive[0] + 0.7152 * m.emissive[1] + 0.0722 * m.emissive[2];
    if !(lum > 0.0) {
        return None;
    }
    let v = |j: usize| Vec3::new(tri_verts[k * 3 + j][0], tri_verts[k * 3 + j][1], tri_verts[k * 3 + j][2]);
    let (e1, e2) = (v(1), v(2));
    let nominal = a.x[2] > 0.0;
    let area = if nominal { a.x[2] } else { 0.5 * e1.cross(e2).length() };
    if !(area * lum > 0.0) {
        return None;
    }
    let c = v(0) + (e1 + e2) / 3.0;
    let moves = tri_verts[k * 3][3].to_bits() & scene::TRI_DYNAMIC != 0;
    Some(EmitterTri { tri: k as u32, mat, obj: a.x[1].to_bits(), area, c: c.to_array(), moves, nominal, in_tree: false })
}

/// An emissive triangle of the emitter CDF: triangle (traversal order), material, object, area.
#[derive(Clone, Copy, Debug)]
pub struct EmitterTri {
    pub tri: u32,
    pub mat: u32,
    pub obj: u32,
    pub area: f32,
    /// Centroid (light LOD: distance to the reference camera position).
    pub c: [f32; 3],
    /// On a moving object (TRI_DYNAMIC): no light LOD, its centroid would go stale (the kernel skips it too).
    pub moves: bool,
    /// A particle's: `area` is the nominal one (Particles emitterArea); its CDF entry carries it negated so the kernel
    /// samples the actual area (pathtrace.wgsl sample_emitter_ris, emit_mis).
    pub nominal: bool,
    /// One of the light tree's (`ltree`): its CDF entry is in the tree's range, at the start of the CDF.
    pub in_tree: bool,
}

/// An emitter triangle's power: area x emissive-factor luminance x object power (its CDF weight without light LOD).
pub fn emitter_power(e: &EmitterTri, materials: &[scene::GpuMaterial], objects: &[[f32; 4]], stride: usize) -> f32 {
    let m = &materials[e.mat as usize];
    let lum = 0.2126 * m.emissive[0] + 0.7152 * m.emissive[1] + 0.0722 * m.emissive[2];
    e.area * lum * objects.get(e.obj as usize * stride).map_or(1.0, |o| o[3])
}

/// An emitter's power with its object switched on: its power now, or at full power if its object is off (0). What the
/// light sampling is set up for at load (the light tree, the sky's light-sampling floor), so that it does not depend on
/// which lights were on then: a time-of-day room switched in place samples as one loaded in that state.
pub fn switchable_power(e: &EmitterTri, materials: &[scene::GpuMaterial], objects: &[[f32; 4]], stride: usize) -> f32 {
    let p = emitter_power(e, materials, objects, stride);
    if p > 0.0 { p } else { emitter_power(e, materials, &[], stride) }
}

/// The light tree over a scene's emitters (`ltree::build`; `slot`: with the slot for emitters added in place). Its
/// members and shape go by `switchable_power`: a time-of-day room's lamps and windows by day are in the tree at zero
/// weight until they switch on, instead of in the flat CDF beyond it, where the kernel draws them by power share alone
/// (no importance at the shading point) once they are on. Then, if some are off, the node powers are refit to the
/// objects' powers now; a scene with every emitter on at load gets the tree it had before, bit for bit.
pub fn light_tree_of(tris: &[EmitterTri], tri_verts: &[[f32; 4]], materials: &[scene::GpuMaterial], objects: &[[f32; 4]], stride: usize, slot: bool) -> Option<ltree::LightTree> {
    let build = if slot { ltree::build_slotted } else { ltree::build };
    let mut t = build(tris, tri_verts, &|i| switchable_power(&tris[i], materials, objects, stride))?;
    let now: Vec<f32> = t.order.iter().map(|&i| emitter_power(&tris[i as usize], materials, objects, stride)).collect();
    if now.iter().any(|&w| !(w > 0.0)) {
        t.refit(&|k| now[k]);
    }
    Some(t)
}

/// The live emitter cull: emitters farther than `radius` from the light LOD's reference are not proposed at all, unless
/// they lie in the view cone (`dir`, cosine of the half-angle `cos`) within `far`. Far emitters behind or beside the
/// camera are mostly occluded and cost long shadow rays; the ones ahead light what the camera sees (Bistro's square,
/// 40-90 m down the street, lost 60 % of its light in the live image when they were culled too: the BSDF rays that
/// alone bring a culled emitter's light are rare and the firefly clamp cuts them). radius 0 = no cull; far 0 = no cone.
/// The kernel's `emitter_culled` makes the same test.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct EmitterCull {
    pub radius: f32,
    pub far: f32,
    pub dir: [f32; 3],
    pub cos: f32,
}

impl EmitterCull {
    /// As the kernel reads it from the parameters.
    pub fn of(p: &gpu::Params) -> Self {
        Self { radius: p.emit_power[1], far: p.emit_power[2], dir: [p.lod_view[0], p.lod_view[1], p.lod_view[2]], cos: p.lod_view[3] }
    }

    /// Whether an emitter at `c`, `d` from the reference `r`, is culled.
    fn culls(&self, c: Vec3, r: Vec3, d: f32) -> bool {
        self.radius > 0.0 && d > self.radius && !(d <= self.far && (c - r).dot(Vec3::from(self.dir)) >= self.cos * d)
    }
}

/// Light LOD (live): emitters are sampled by power x (d0 / max(d, d0))^2, d = distance to the reference position, so
/// far emitters (mostly occluded, long shadow rays) are drawn rarely. Unbiased: the kernel's emitter pdfs apply the
/// same factor (`emitter_lod`). xyz = reference position, w = d0 (0 = off).
/// Emitters the cull drops (`EmitterCull`) are not proposed at all (factor 0). The kernel gives the BSDF rays that hit
/// them their whole light (emit_mis), so the estimator stays unbiased; their light just comes by those rays alone.
pub fn emitter_lod(c: [f32; 3], lod: [f32; 4], cull: EmitterCull) -> f32 {
    if lod[3] <= 0.0 {
        return 1.0;
    }
    let (c, r) = (Vec3::from(c), Vec3::new(lod[0], lod[1], lod[2]));
    let d = c.distance(r);
    if cull.culls(c, r, d) {
        return 0.0;
    }
    let d = d.max(lod[3]);
    (lod[3] / d) * (lod[3] / d)
}

/// Seed of the kernel's random numbers (Params::size.w). Renders are deterministic; THREERT_SEED gives the same build
/// another sample sequence, for noise floors (two seeds, same estimator).
pub fn rng_seed() -> u32 {
    static S: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *S.get_or_init(|| std::env::var("THREERT_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x5eed))
}

/// Local lights are evaluated in groups of LIGHT_GROUP at camera hits (pathtrace.wgsl `local_lights`): spatially sorted,
/// each group with a sphere bounding its lights' reach, so a shading point skips whole groups it is outside of instead
/// of testing every light's range. Same result as testing each light (a skipped light is one its range test rejects).
pub const LIGHT_GROUP: usize = 16;

/// Distance beyond which a local light gives nothing (the kernel's range test in `local_lights`); f32::MAX without a
/// range. (A black light gives nothing anywhere: `lights_with_groups` leaves it out.)
fn light_reach(l: &scene::GpuLight) -> f32 {
    if l.color[3] <= 0.0 {
        return f32::MAX;
    }
    let size = if l.pos[3] == 3.0 { Vec3::from_slice(&l.ext[..3]).length() + l.dir[3] } else { l.dir[3] };
    l.color[3] + size
}

/// Orders `idx` (lights at positions base.. of the light table) so that every LIGHT_GROUP-sized chunk counted from
/// `origin` is spatially compact: median splits at chunk boundaries along the longest axis.
fn sort_lights_spatially(idx: &mut [usize], base: usize, origin: usize, lights: &[scene::GpuLight]) {
    let (lo, hi) = (base, base + idx.len());
    // The chunk boundary nearest the middle, strictly inside.
    let mid = lo + idx.len() / 2;
    let k = ((mid - origin) as f32 / LIGHT_GROUP as f32).round() as usize;
    let mut split = origin + k * LIGHT_GROUP;
    if split <= lo {
        split += LIGHT_GROUP;
    }
    if split >= hi {
        split = split.saturating_sub(LIGHT_GROUP);
    }
    if split <= lo || split >= hi {
        return;
    }
    let p = |i: usize| Vec3::from_slice(&lights[i].pos[..3]);
    let (mut a, mut b) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &i in idx.iter() {
        a = a.min(p(i));
        b = b.max(p(i));
    }
    let ext = b - a;
    let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
    idx.select_nth_unstable_by(split - lo, |&i, &j| p(i)[axis].total_cmp(&p(j)[axis]));
    let (l, r) = idx.split_at_mut(split - lo);
    sort_lights_spatially(l, lo, origin, lights);
    sort_lights_spatially(r, split, origin, lights);
}

/// 0 directional, 1 local without shadows, 2 local with shadows.
fn light_class(l: &scene::GpuLight) -> u8 {
    if l.pos[3] == 0.0 { 0 } else if l.ext[3] < 0.5 { 1 } else { 2 }
}

/// The kernel's light order: directional lights first (it loops over them), then local lights without shadows (evaluated
/// exactly at camera hits, like UE's deferred lights), then shadowed ones (one shadow ray each when few, else sampled);
/// each class spatially sorted into the kernel's light groups (`lights_with_groups`).
pub fn light_order(lights: &[scene::GpuLight]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..lights.len()).collect();
    order.sort_by_key(|&i| light_class(&lights[i]));
    let nd = order.iter().take_while(|&&i| light_class(&lights[i]) == 0).count();
    let ns = nd + order[nd..].iter().take_while(|&&i| light_class(&lights[i]) == 1).count();
    let (unshadowed, shadowed) = order[nd..].split_at_mut(ns - nd);
    sort_lights_spatially(unshadowed, nd, nd, lights);
    sort_lights_spatially(shadowed, ns, nd, lights);
    order
}

/// The light table as uploaded: the lights, then per group of LIGHT_GROUP local lights (counted from the first local
/// one) an entry whose `pos` is the sphere (centre, radius) holding every point its lights reach. Black lights (off, or
/// hidden: `scene::gpu_light_shown`) reach nothing and are in no sphere; a group of them alone has radius -1 (no point
/// is in it). THREERT_LIGHT_GROUPS=0: unbounded spheres, every light tested (A/B).
pub fn lights_with_groups(lights: &[scene::GpuLight]) -> Vec<scene::GpuLight> {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let off = *OFF.get_or_init(|| std::env::var("THREERT_LIGHT_GROUPS").is_ok_and(|v| v == "0"));
    let nd = lights.iter().take_while(|l| l.pos[3] == 0.0).count();
    let mut out = lights.to_vec();
    for g in lights[nd..].chunks(LIGHT_GROUP) {
        let lit = |l: &&scene::GpuLight| l.color[..3].iter().any(|c| *c > 0.0);
        if !off && !g.iter().any(|l| lit(&l)) {
            out.push(scene::GpuLight { pos: [0.0, 0.0, 0.0, -1.0], ..Default::default() });
            continue;
        }
        let (mut a, mut b) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for l in g.iter().filter(lit) {
            a = a.min(Vec3::from_slice(&l.pos[..3]));
            b = b.max(Vec3::from_slice(&l.pos[..3]));
        }
        let c = (a + b) * 0.5;
        let mut r = 0.0f32;
        for l in g.iter().filter(lit) {
            let reach = light_reach(l);
            r = if reach == f32::MAX { f32::MAX } else { r.max(c.distance(Vec3::from_slice(&l.pos[..3])) + reach) };
        }
        if off {
            r = f32::MAX;
        }
        // Margin for rounding in the kernel's distance (positions up to a few km).
        if r < f32::MAX {
            r = r * (1.0 + 1e-5) + 1e-3 * (1.0 + c.abs().max_element() * 1e-3);
        }
        out.push(scene::GpuLight { pos: [c.x, c.y, c.z, r], ..Default::default() });
    }
    out
}

/// Buckets of the emitter CDF's guide table (pathtrace.wgsl `EMITTER_GUIDE`, a power of two).
pub const EMITTER_GUIDE: usize = 65536;

/// Emitter CDF entries (triangle bits, normalized cumulative weight, weight, area), one per triangle, then the guide
/// table; the total weight and the total without light LOD (Params::emit_power). Weight = area x emissive luminance x
/// object power (x light LOD). Triangles switched off keep a zero-weight entry.
/// Guide table (the cutpoint method): EMITTER_GUIDE + 1 indices, four per vec4 (bits); index k is where the kernel's
/// binary search lands for u = k / EMITTER_GUIDE, so a draw u only searches between the indices of its bucket and the
/// next, and finds the same entry as a search over the whole CDF.
pub fn emitter_cdf(tris: &[EmitterTri], materials: &[scene::GpuMaterial], objects: &[[f32; 4]], stride: usize, lod: [f32; 4], cull: EmitterCull) -> (Vec<[f32; 4]>, f32, f32) {
    let (mut total, mut power) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(tris.len());
    for e in tris {
        let lod_k = if e.moves { 1.0 } else { emitter_lod(e.c, lod, cull) };
        let p = emitter_power(e, materials, objects, stride);
        power += p;
        total += p * lod_k;
        out.push([f32::from_bits(e.tri), total, p * lod_k, if e.nominal { -e.area } else { e.area }]);
    }
    if total > 0.0 {
        for e in &mut out {
            e[1] /= total;
        }
    }
    // The kernel's search: the first entry whose cumulative weight is not below u.
    let (n, mut i) = (out.len(), 0usize);
    let guide: Vec<u32> = (0..=EMITTER_GUIDE)
        .map(|k| {
            let u = k as f32 / EMITTER_GUIDE as f32;
            while i < n && out[i][1] < u {
                i += 1;
            }
            i as u32
        })
        .collect();
    out.extend(guide.chunks(4).map(|c| std::array::from_fn(|j| f32::from_bits(c.get(j).copied().unwrap_or(0)))));
    (out, total, power)
}

/// Height fog and volumetric fog kernel parameters (`Params::fog`, `fog_color`, `vol`, `vol2`).
pub fn fog_params(fog: Option<&desc::Fog>) -> [[f32; 4]; 4] {
    [
        fog.map_or([0.0; 4], |f| [f.density.max(0.0), f.height_falloff, f.height, f.start_distance]),
        fog.map_or([0.0; 4], |f| {
            let k = f.intensity.unwrap_or(1.0);
            [f.color[0] * k, f.color[1] * k, f.color[2] * k, f.max_opacity.unwrap_or(1.0).clamp(0.0, 1.0)]
        }),
        fog.and_then(|f| f.volumetric.as_ref()).map_or([0.0; 4], |v| [v.scattering.max(0.0), v.anisotropy.clamp(-0.95, 0.95), v.extinction_scale.max(0.0), 0.0]),
        fog.and_then(|f| f.volumetric.as_ref()).map_or([1.0, 1.0, 1.0, 0.0], |v| {
            let a = v.albedo.unwrap_or([1.0; 3]);
            [a[0], a[1], a[2], v.distance.max(0.0)]
        }),
    ]
}

/// Whether a baked environment map (an HDRI, a `SkyAtmosphere`'s bake) is light-sampled: THREERT_ENV_NEE=1|0, on
/// everywhere but Android. On Adreno 740 the kernel's reads of `env_cdf` for it went to the wrong memory (tens of
/// thousands of GPU page faults a second under a `SkyAtmosphere`, a trace up to twice as slow). Without it the sky
/// still lights through the paths' own rays, with their full weight; the sun is a directional light, sampled as ever.
pub fn env_nee_on() -> bool {
    env_nee_of(std::env::var("THREERT_ENV_NEE").ok().as_deref(), cfg!(target_os = "android"))
}

fn env_nee_of(var: Option<&str>, android: bool) -> bool {
    var.map_or(!android, |v| v != "0")
}

/// `prepare` for an offline render: under software traversal every instance flattened unless THREERT_UNITS=1.
pub fn prepare(g: &gpu::Gpu, sc: &mut scene::Scene, timings: &mut Vec<(String, f64)>) -> Prepared {
    prepare_as(g.hwrt, units_on(false), sc, timings)
}

/// `prepare` for a live session: under software traversal the instances are units unless THREERT_UNITS=0.
pub fn prepare_live(g: &gpu::Gpu, sc: &mut scene::Scene, timings: &mut Vec<(String, f64)>) -> Prepared {
    prepare_as(g.hwrt, units_on(true), sc, timings)
}

/// Whether software traversal keeps a scene's instances as units (units.rs). A live session and a browser do: units
/// are what they edit in place, and what a browser's memory and load time need. An offline render does not: it
/// edits nothing, it is the reference the catalog's bytes are compared by, and its kernel and bytes stay what they
/// were before units. THREERT_UNITS=1 has an offline render use them (the A/B against a live frame's units);
/// THREERT_UNITS=0 flattens everywhere (the A/B against units).
pub(crate) fn units_on(live: bool) -> bool {
    units_said(std::env::var("THREERT_UNITS").ok().as_deref(), live)
}

fn units_said(var: Option<&str>, live: bool) -> bool {
    match var {
        Some("0") => false,
        Some("1") => true,
        _ => live || cfg!(target_arch = "wasm32"),
    }
}

/// Whether `prepare_live` will keep the scene's instances as units (software traversal, and it has instances), asked
/// before it runs (live/load.rs `load_scene`: the light tree's part for emitters edits add).
pub fn units_wanted(hwrt: bool, sc: &scene::Scene) -> bool {
    !hwrt && units_on(true) && !sc.instances.is_empty() && sc.instances.len() <= units::MAX_UNITS
}

/// `prepare` for a traversal: hardware ray queries, or software traversal with units (units.rs) or without.
pub fn prepare_as(hwrt: bool, units: bool, sc: &mut scene::Scene, timings: &mut Vec<(String, f64)>) -> Prepared {
    let t = Instant::now();
    // Software traversal: the instances stay instances, units of a second level (units.rs; docs/RUNTIME.md "Software
    // traversal with units"), each mesh with a tree of its own over its model-space triangles, which are put in the
    // tree's order here, before anything counts them. Without units (THREERT_UNITS=0, the A/B; a mesh no tree can
    // be made of; more instances than a top tree holds) the instances become ordinary world-space triangles, as
    // they all did before.
    let mut trees = None;
    if !hwrt && !sc.instances.is_empty() {
        trees = if units && sc.instances.len() <= units::MAX_UNITS { unit_trees(sc) } else { None };
        match &trees {
            Some(_) => flatten_deforming(sc),
            None => flatten_instances(sc),
        }
    }
    // Software traversal walks each procedural object's boxes through a tree of its own, and the tree over no box is
    // not one the kernel can walk (bvh.rs `EMPTY`): an object without boxes has nothing to meet, and is left out.
    if !hwrt {
        sc.procs.retain(|p| !p.blob.boxes.is_empty());
    }
    // Hardware RT only needs triangles grouped by shadow casting; software traversal needs our BVH.
    let bvh = if hwrt {
        let mut order: Vec<u32> = (0..sc.tris.len() as u32).collect();
        order.sort_by_key(|&i| group_of(sc.tri_flags[i as usize]));
        bvh::Bvh { nodes: vec![bvh::GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 }], order }
    } else {
        bvh::build(&sc.tris)
    };
    let mut groups = [0usize; GROUPS];
    for f in &sc.tri_flags {
        groups[group_of(*f)] += 1;
    }
    let mut tri_verts: Vec<[f32; 4]> = Vec::with_capacity(sc.tris.len() * 3);
    let mut attrs = Vec::with_capacity(sc.attrs.len());
    for &i in &bvh.order {
        let [a, b, c] = sc.tris[i as usize];
        let (e1, e2): (Vec3A, Vec3A) = (b - a, c - a);
        tri_verts.push([a.x, a.y, a.z, f32::from_bits(sc.tri_flags[i as usize])]);
        tri_verts.push([e1.x, e1.y, e1.z, 0.0]);
        tri_verts.push([e2.x, e2.y, e2.z, 0.0]);
        attrs.push(sc.attrs[i as usize]);
    }
    // Instanced meshes (hardware RT): the model-space pool after every traversal group, one BLAS per mesh.
    let pool_first = tri_verts.len() / 3;
    // Deforming meshes come first (scene build): flagged dynamic, their previous vertices kept for motion vectors.
    let deforming = scene::deforming_pool(&sc.meshes);
    for (k, (t, a)) in sc.pool_tris.iter().zip(&sc.pool_attrs).enumerate() {
        let [a0, b, c] = *t;
        let (e1, e2): (Vec3A, Vec3A) = (b - a0, c - a0);
        // (A unit's triangles are in its tree's order, cutouts among the rest: each carries its own flag, as the still
        // scene's do. Hardware traversal has them apart, in a geometry of their own.)
        let cutout = trees.is_some() && sc.materials.get(a.uv[3].to_bits() as usize).is_some_and(|m| m.uv2[3] > 0.0);
        tri_verts.push([a0.x, a0.y, a0.z, f32::from_bits(if k < deforming { scene::TRI_DYNAMIC } else { 0 } | if cutout { scene::TRI_MASKED } else { 0 })]);
        tri_verts.push([e1.x, e1.y, e1.z, 0.0]);
        tri_verts.push([e2.x, e2.y, e2.z, 0.0]);
        attrs.push(*a);
    }
    sc.gpu_instances = sc.instances.iter().map(|i| { let m = sc.meshes[i.mesh as usize]; i.gpu(pool_first as u32 + m.first, m.opaque) }).collect();
    // A scene whose triangles are all units': the still tree is then the leaf of one triangle, triangle 0 (bvh.rs
    // `EMPTY`), which no ray meets while triangle 0 is the zeros of an empty buffer. Here triangle 0 is the pool's
    // first, a mesh's in its model space, and the still walk would meet it standing in the world. The leaf gets a
    // triangle of its own after the pool, without area.
    let no_still = trees.is_some() && pool_first == 0;
    let nothing = tri_verts.len() / 3;
    if no_still {
        tri_verts.extend([[0.0; 4]; 3]);
        attrs.push(bytemuck::Zeroable::zeroed());
    }
    // The units: the mesh trees after the still tree's nodes, room for the trees and units edits add (the edit room's
    // triangles and records, gpu.rs `EditRoom`), the top tree; each unit's record gets its tree's root.
    let mut nodes = bvh.nodes;
    if no_still {
        nodes[0].left_or_first = nothing as u32;
    }
    sc.units = None;
    if let Some(trees) = trees {
        let room = gpu::EditRoom::for_scene(sc, tri_verts.len() / 3, sc.instances.len() + sc.procs.len(), false, true);
        let used: usize = trees.iter().flatten().map(|t| t.nodes.len()).sum();
        let mut u = units::Units { layout: units::Layout::new(nodes.len(), used + room.tris, sc.instances.len() + room.records), trees: vec![None; trees.len()], slots: vec![None; sc.instances.len()], count: 0, records: sc.instances.len() + sc.procs.len() + room.records, kept: Default::default() };
        let still = nodes.len();
        nodes.resize(still + u.layout.nodes(), units::FILL);
        for (k, t) in trees.iter().enumerate() {
            if let Some(t) = t {
                let root = u.add_tree(k as u32, t).expect("the mesh range holds the build's trees") as usize;
                nodes[root..root + t.nodes.len()].copy_from_slice(&t.nodes);
            }
        }
        for (r, i) in sc.instances.iter().enumerate() {
            if let Some(unit) = u.unit(i.mesh, i.world, i.hidden) {
                sc.gpu_instances[r] = units::unit_record(sc.gpu_instances[r], unit.root, i.cast_shadow);
                u.slots[r] = Some(unit);
            }
        }
        // (A scene whose units the top tree cannot hold, or with one that is nowhere: none is drawn, and it says so.)
        match u.top() {
            Ok(top) => nodes[u.layout.top_first as usize..u.layout.top_first as usize + top.len()].copy_from_slice(&top),
            Err(e) => eprintln!("units: no top tree ({e}): the scene's instances are not drawn"),
        }
        sc.units = Some(u);
    }
    // Procedural geometry: its records after the model instances', its boxes / materials / data in `proc_data`.
    let layout = scene::proc_layout(&sc.procs, !hwrt);
    sc.proc_records = sc.procs.iter().zip(&layout.headers).map(|(p, &h)| p.gpu(h)).collect();
    sc.proc_layout = (!sc.procs.is_empty()).then_some(layout);
    // Translucent particles (hardware RT): after the pool, in no group (only the translucency pass traces them).
    sc.trans_first = (tri_verts.len() / 3) as u32;
    if hwrt {
        for (t, a) in sc.trans_tris.iter().zip(&sc.trans_attrs) {
            let [a0, b, c] = *t;
            let (e1, e2): (Vec3A, Vec3A) = (b - a0, c - a0);
            tri_verts.push([a0.x, a0.y, a0.z, f32::from_bits(0)]);
            tri_verts.push([e1.x, e1.y, e1.z, 0.0]);
            tri_verts.push([e2.x, e2.y, e2.z, 0.0]);
            attrs.push(*a);
        }
    }
    sc.attrs = attrs;
    // Vertex colours (scene.rs `tri_colors`), with the triangles: the flattened ones in traversal order, the pool, the
    // translucent particles (white).
    sc.gpu_vcolors = if sc.vcolors.is_empty() && sc.pool_vcolors.is_empty() {
        vec![]
    } else {
        let at = |v: &[[u32; 3]], k: usize| v.get(k).copied().unwrap_or(scene::WHITE);
        let mut v: Vec<u32> = Vec::with_capacity(sc.attrs.len() * 3);
        v.extend(bvh.order.iter().flat_map(|&i| at(&sc.vcolors, i as usize)));
        v.extend((0..sc.pool_tris.len()).flat_map(|k| at(&sc.pool_vcolors, k)));
        v.resize(sc.attrs.len() * 3, u32::MAX);
        v
    };
    // Posed models: where each of their triangles landed (they are re-skinned in place, hardware RT only).
    if hwrt && (!sc.retained.dynamic.is_empty() || !sc.retained.rigid.is_empty() || !sc.particles.is_empty() || !sc.gpu_particles.is_empty()) {
        let first_dynamic: usize = groups[..4].iter().sum();
        let finals: std::collections::HashMap<u32, u32> = bvh.order[first_dynamic..].iter().enumerate().map(|(k, &i)| (i, (first_dynamic + k) as u32)).collect();
        for d in sc.retained.dynamic.values_mut().chain(sc.retained.rigid.values_mut().map(|r| &mut r.rest)) {
            d.finals = (0..d.mats.len() as u32).map(|j| finals[&(d.first + j)]).collect();
        }
        // A particle system's triangles stay together (one group, the order within a group is the build order).
        for ps in sc.particles.iter_mut().filter(|p| !p.translucent) {
            ps.final_first = finals[&ps.first];
            debug_assert!((0..ps.count * 2).all(|j| finals[&(ps.first + j)] == ps.final_first + j));
        }
        for ps in sc.gpu_particles.iter_mut() {
            ps.final_first = finals[&ps.first];
            debug_assert!((0..ps.capacity * 2).all(|j| finals[&(ps.first + j)] == ps.final_first + j));
        }
    }
    for ps in sc.particles.iter_mut().filter(|p| p.translucent) {
        ps.final_first = sc.trans_first + ps.first;
    }
    timings.push(("bvh_ms".into(), t.elapsed().as_secs_f64() * 1000.0));

    // Emissive triangle CDF (weight = area * luminance of the emissive factor * object power): the flattened
    // triangles, then the emitting own_blas meshes' (world space in the pool, `InstMesh::emits`).
    let mut emitter_tris: Vec<EmitterTri> = vec![];
    let emitting_pool = sc.meshes.iter().filter(|m| m.emits).flat_map(|m| pool_first + m.first as usize..pool_first + (m.first + m.count) as usize);
    for k in (0..pool_first).chain(emitting_pool) {
        if let Some(e) = emitter_tri(k, &tri_verts, &sc.attrs[k], &sc.materials) {
            emitter_tris.push(e);
        }
    }
    // Where each material's flattened static triangles are (runs of the traversal order, with their object), for a
    // material that starts emitting in the live view (tables.rs `apply`): its emitter entries are made from them in
    // place. Materials that also cover pool or dynamic triangles cannot (`fixed_mats`).
    let mut mat_runs: std::collections::HashMap<u32, Vec<[u32; 3]>> = std::collections::HashMap::new();
    let mut fixed_mats: std::collections::HashSet<u32> = sc.pool_attrs.iter().map(|a| a.uv[3].to_bits()).collect();
    for (k, a) in sc.attrs[..pool_first].iter().enumerate() {
        let (mat, obj) = (a.uv[3].to_bits(), a.x[1].to_bits());
        if tri_verts[k * 3][3].to_bits() & scene::TRI_DYNAMIC != 0 {
            fixed_mats.insert(mat);
            continue;
        }
        let runs = mat_runs.entry(mat).or_default();
        match runs.last_mut() {
            Some(r) if r[1] == k as u32 && r[2] == obj => r[1] += 1,
            _ => runs.push([k as u32, k as u32 + 1, obj]),
        }
    }
    let tri_order = bvh.order[..pool_first.min(bvh.order.len())].to_vec();
    // Many static emitters: a light tree samples them by importance at the shading point (`ltree`); each one's path
    // code in its triangle's attrs x.w (emit_mis). THREERT_LTREE=0: the CDF alone (A/B).
    // (Switched-off objects at full power: `switchable_power`.)
    let all_power: f32 = emitter_tris.iter().map(|e| switchable_power(e, &sc.materials, &sc.objects, sc.object_stride())).sum();
    let use_tree = std::env::var("THREERT_LTREE").map_or(true, |v| v != "0");
    let mut light_tree = if use_tree { light_tree_of(&emitter_tris, &tri_verts, &sc.materials, &sc.objects, sc.object_stride(), sc.emitter_slot) } else { None };
    if let Some(t) = &mut light_tree {
        // The tree's emitters first, in leaf order (each node covers a range of entries), then the rest as they were.
        let mut taken = vec![false; emitter_tris.len()];
        let mut ordered: Vec<EmitterTri> = Vec::with_capacity(emitter_tris.len());
        for (k, &i) in t.order.iter().enumerate() {
            taken[i as usize] = true;
            let mut e = emitter_tris[i as usize];
            e.in_tree = true;
            sc.attrs[e.tri as usize].x[3] = f32::from_bits(t.code[k]);
            ordered.push(e);
        }
        ordered.extend(emitter_tris.iter().zip(&taken).filter(|(_, t)| !**t).map(|(e, _)| *e));
        emitter_tris = ordered;
        // From here on an entry of the tree is its position in `emitter_tris` (the live view's refits read it so).
        t.order = (0..t.order.len() as u32).collect();
    }
    let (mut emitters, total, _) = emitter_cdf(&emitter_tris, &sc.materials, &sc.objects, sc.object_stride(), [0.0; 4], EmitterCull::default());
    let ltree_param = match &light_tree {
        Some(t) => {
            let off = emitters.len() as u32;
            emitters.extend_from_slice(&t.nodes);
            // The moving emitters (after the tree's in the CDF) are picked with the probability of their share of power.
            [f32::from_bits(off), f32::from_bits(t.order.len() as u32), if total > 0.0 { ((total - t.power) / total).max(0.0) } else { 0.0 }, ltree::RIS_OFFLINE as f32]
        }
        None => [0.0; 4],
    };

    // Sky / environment.
    let t = Instant::now();
    let (env, sky_color, has_env) = match &sc.sky {
        scene::SkyData::Atmosphere { sun_dir, sun_lux, rayleigh, mie, g: mie_g, ground, sun_node, dynamic, glow } => {
            // Scene edits that leave the sky inputs unchanged (materials, transforms, lights) reuse the last bake.
            static LAST: std::sync::Mutex<Option<(Vec<u32>, std::sync::Arc<sky::SkyBake>)>> = std::sync::Mutex::new(None);
            let alt = sc.camera.position.y.max(0.0);
            let (bw, bh) = if *dynamic { sky::DYNAMIC_SKY } else { (1024, 512) };
            let key: Vec<u32> = [*rayleigh, *mie, *mie_g, ground.x, ground.y, ground.z, sun_dir.x, sun_dir.y, sun_dir.z, sun_lux.x, sun_lux.y, sun_lux.z, alt, glow.uniform.x, glow.uniform.y, glow.uniform.z, glow.horizon.x, glow.horizon.y, glow.horizon.z, glow.height, bw as f32, sc.cloud_cover].iter().map(|f| f.to_bits()).collect();
            let mut last = LAST.lock().unwrap();
            let bake = match last.as_ref() {
                Some((k, b)) if *k == key => b.clone(),
                _ => {
                    let atm = sky::Atmosphere::earth(*rayleigh, *mie, *mie_g, *ground);
                    let b = std::sync::Arc::new(if *dynamic || !glow.is_zero() || sc.cloud_cover > 0.0 {
                        sky::bake_atmosphere_tables(&atm, &sky::SkyTables::new(&atm), *sun_dir, *sun_lux, alt, bw, bh, *glow, sc.cloud_cover)
                    } else {
                        sky::bake_atmosphere(&atm, *sun_dir, *sun_lux, alt, bw, bh)
                    });
                    *last = Some((key, b.clone()));
                    b
                }
            };
            drop(last);
            if let Some(i) = sun_node {
                let s = bake.sun_illuminance;
                sc.lights[*i].color = [s.x, s.y, s.z, 0.0];
            }
            (gpu::EnvMap { width: bake.width, height: bake.height, texels: bake.texels.clone() }, Vec3::ZERO, true)
        }
        scene::SkyData::Gradient(gr) => {
            static LAST: std::sync::Mutex<Option<(sky::Gradient, std::sync::Arc<sky::SkyBake>)>> = std::sync::Mutex::new(None);
            let mut last = LAST.lock().unwrap();
            let bake = match last.as_ref() {
                Some((k, b)) if k == gr => b.clone(),
                _ => {
                    let b = std::sync::Arc::new(sky::bake_gradient(gr, 1024, 512));
                    *last = Some((gr.clone(), b.clone()));
                    b
                }
            };
            (gpu::EnvMap { width: bake.width, height: bake.height, texels: bake.texels.clone() }, Vec3::ZERO, true)
        }
        scene::SkyData::Color(c) => (gpu::EnvMap { width: 1, height: 1, texels: vec![[c.x, c.y, c.z, 1.0]] }, *c, false),
        scene::SkyData::None => (gpu::EnvMap { width: 1, height: 1, texels: vec![[0.0; 4]] }, Vec3::ZERO, false),
    };
    // The procedural region rides after the environment's CDF (its binding is the kernel's one flat 32-bit array, and
    // the kernel has no binding to spare: Metal's 29 with the translucency pass); gpu.rs `upload_procs` lays it out.
    let env_cdf = build_env_cdf(&env);
    // (w: the vertex colours' first vec4 in `prev_verts`, after the dynamic tail's previous vertices; gpu.rs F_VCOLOR.)
    if !sc.gpu_vcolors.is_empty() {
        sc.vcolor_base = ((groups[4..].iter().sum::<usize>() + scene::deforming_pool(&sc.meshes)) * 3) as u32;
    }
    let procs = [env_cdf.len() as u32, sc.procs.len() as u32, sc.gpu_instances.len() as u32, sc.vcolor_base];
    // Environment light sampling (one shadow ray per vertex) is skipped when the sky is negligible next to the emitters
    // (night streets): escaping BSDF rays still pick it up, with full MIS weight. The emitters as if all on: a
    // time-of-day room loaded by day decides at dusk as one loaded at night does (live/edits.rs `put_sky`).
    let env_max = env.texels.iter().map(|t| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).fold(0.0f32, f32::max).max(0.2126 * sky_color.x + 0.7152 * sky_color.y + 0.0722 * sky_color.z);
    let em_area: f32 = emitters[..emitter_tris.len()].iter().map(|e| e[3]).sum();
    let env_nee_floor = if emitter_tris.is_empty() { 0.0 } else { 0.01 * all_power / em_area.max(1e-6) };
    // (A uniform sky is sampled regardless of `env_nee_on`: it reads no table.)
    let env_nee = env_max >= env_nee_floor && (!has_env || env_nee_on());
    // The kernel's light order (`light_order`).
    let class = light_class;
    let order = light_order(&sc.lights);
    sc.lights = order.iter().map(|&i| sc.lights[i]).collect();
    if sc.retained.light_nodes.len() == order.len() {
        sc.retained.light_nodes = order.iter().map(|&i| sc.retained.light_nodes[i].clone()).collect();
    }
    let n_dir = sc.lights.iter().filter(|l| l.pos[3] == 0.0).count() as u32;
    let first_shadowed = sc.lights.iter().position(|l| class(l) == 2).unwrap_or(sc.lights.len()) as u32;
    timings.push(("sky_ms".into(), t.elapsed().as_secs_f64() * 1000.0));

    let preset = sc.preset.clone();
    let fog = fog_params(sc.fog.as_ref());
    let cam = &sc.camera;
    let aspect = sc.width as f32 / sc.height as f32;
    let (lens_radius, focus) = match &cam.dof {
        Some(d) => {
            let sensor_h = d.sensor_width / aspect;
            let focal_mm = sensor_h / (2.0 * cam.tan_half_fov);
            (0.5 * focal_mm / d.f_stop / 1000.0, d.focus_distance)
        }
        None => (0.0, 1.0),
    };
    // Firefly clamp on indirect paths, expressed relative to the exposure so it is scene independent.
    let ev_guess = match &cam.exposure {
        desc::Exposure::Manual { ev100 } => *ev100,
        desc::Exposure::Auto { max_ev100, .. } => *max_ev100,
    };
    let params = gpu::Params {
        cam_pos: [cam.position.x, cam.position.y, cam.position.z, cam.tan_half_fov],
        cam_fwd: [cam.forward.x, cam.forward.y, cam.forward.z, aspect],
        cam_right: [cam.right.x, cam.right.y, cam.right.z, lens_radius],
        cam_up: [cam.up.x, cam.up.y, cam.up.z, focus],
        size: [sc.width, sc.height, 0, rng_seed()],
        counts: [sc.lights.len() as u32, if total > 0.0 { emitter_tris.len() as u32 } else { 0 }, preset.native.max_bounces, preset.native.russian_roulette_depth],
        env: [if has_env { 1.0 } else { 0.0 }, env.width as f32, env.height as f32, -1.0],
        misc: [preset.native.clamp_indirect * 2f32.powf(ev_guess), total, preset.native.energy_compensation, 0.0],
        sky_color: [sky_color.x, sky_color.y, sky_color.z, 0.0],
        counts2: [n_dir, env_nee as u32, 0, first_shadowed],
        time: [sc.time, sc.wetness, sc.rain, sc.media.len() as f32],
        fog: fog[0],
        fog_color: fog[1],
        vol: fog[2],
        vol2: fog[3],
        raster: [0.0; 4],
        cache: [0.0; 4],
        // (z, w: software traversal's units: the top tree's root and their count, as bits; trace_units.wgsl. y is
        // `render --signals`'.)
        motion: sc.units.as_ref().map_or([0.0; 4], |u| [0.0, 0.0, f32::from_bits(u.layout.top_first), f32::from_bits(u.count)]),
        group_first: [[0; 4]; 2],
        light_lod: [0.0; 4],
        emit_power: [total, 0.0, 0.0, 0.0],
        aerial: sc.aerial,
        background: sc.background.map_or([0.0; 4], |b| [b.x, b.y, b.z, 1.0]),
        lod_view: [0.0; 4],
        ltree: ltree_param,
        procs,
    };
    let mut params = params;
    let mut first = 0u32;
    for (g, &count) in groups.iter().enumerate() {
        params.group_first[g / 4][g % 4] = first;
        first += u32::try_from(count).expect("more than 2^32 triangles");
    }
    let energy = energy_lut();
    Prepared { tri_verts, groups, nodes, emitters, env, env_cdf, params, energy, emitter_tris, light_tree, env_nee_floor, mat_runs, fixed_mats, tri_order }
}

/// Instances as world-space triangles (software traversal), appended to the flattened ones with their objects.
/// Software traversal's units (units.rs): a tree for each instanced mesh that keeps its shape, its triangles (and
/// what goes with them: attributes, vertex colours) put in the tree's order. None: no units (a mesh no tree can be
/// made of: a vertex that is not a number); the caller flattens every instance then.
fn unit_trees(sc: &mut scene::Scene) -> Option<Vec<Option<units::MeshTree>>> {
    let mut trees = Vec::with_capacity(sc.meshes.len());
    for m in &sc.meshes {
        if m.deforms || m.count == 0 {
            trees.push(None);
            continue;
        }
        match units::mesh_tree(&sc.pool_tris[m.first as usize..(m.first + m.count) as usize]) {
            Ok(t) => trees.push(Some(t)),
            Err(e) => {
                eprintln!("units: {e}: the scene's instances are flattened");
                return None;
            }
        }
    }
    if !sc.pool_vcolors.is_empty() {
        sc.pool_vcolors.resize(sc.pool_tris.len(), scene::WHITE);
    }
    for (m, t) in sc.meshes.iter().zip(&trees) {
        let Some(t) = t else { continue };
        let r = m.first as usize..(m.first + m.count) as usize;
        let tris: Vec<_> = t.order.iter().map(|&i| sc.pool_tris[r.start + i as usize]).collect();
        sc.pool_tris[r.clone()].copy_from_slice(&tris);
        let attrs: Vec<_> = t.order.iter().map(|&i| sc.pool_attrs[r.start + i as usize]).collect();
        sc.pool_attrs[r.clone()].copy_from_slice(&attrs);
        if !sc.pool_vcolors.is_empty() {
            let colors: Vec<_> = t.order.iter().map(|&i| sc.pool_vcolors[r.start + i as usize]).collect();
            sc.pool_vcolors[r].copy_from_slice(&colors);
        }
    }
    Some(trees)
}

/// With units: the instances of meshes that deform (a pose a crowd shares) are still written out as world-space
/// triangles, as every instance was. They keep their place among the instances (edits name instances by index) and
/// are no unit.
fn flatten_deforming(sc: &mut scene::Scene) {
    for k in 0..sc.instances.len() {
        let inst = sc.instances[k].clone();
        let m = sc.meshes[inst.mesh as usize];
        if inst.hidden || !m.deforms {
            continue;
        }
        let first = sc.tris.len() as u32;
        for k in m.first..m.first + m.count {
            if let Some(&c) = sc.pool_vcolors.get(k as usize) {
                scene::put_colors(&mut sc.vcolors, sc.tris.len(), Some(vec![c]));
            }
            sc.tris.push(inst.tri(&sc.pool_tris[k as usize]));
            sc.tri_flags.push(scene::instance_tri_flags(&inst, &sc.pool_attrs[k as usize], &sc.materials));
            sc.attrs.push(scene::instance_attr(&sc.pool_attrs[k as usize], inst.world, inst.object));
        }
        sc.tri_nodes.push((inst.name.clone(), first, sc.tris.len() as u32));
        if let Some(i) = inst.sub {
            sc.tri_instances.push((first, sc.tris.len() as u32, i));
        }
    }
}

fn flatten_instances(sc: &mut scene::Scene) {
    for inst in std::mem::take(&mut sc.instances) {
        if inst.hidden {
            continue;
        }
        let m = sc.meshes[inst.mesh as usize];
        let first = sc.tris.len() as u32;
        for k in m.first..m.first + m.count {
            if let Some(&c) = sc.pool_vcolors.get(k as usize) {
                scene::put_colors(&mut sc.vcolors, sc.tris.len(), Some(vec![c]));
            }
            sc.tris.push(inst.tri(&sc.pool_tris[k as usize]));
            sc.tri_flags.push(scene::instance_tri_flags(&inst, &sc.pool_attrs[k as usize], &sc.materials));
            sc.attrs.push(scene::instance_attr(&sc.pool_attrs[k as usize], inst.world, inst.object));
        }
        // A second range for the node when its glass / emissive part was flattened at build: node_tris joins them.
        sc.tri_nodes.push((inst.name, first, sc.tris.len() as u32));
        if let Some(i) = inst.sub {
            sc.tri_instances.push((first, sc.tris.len() as u32, i));
        }
    }
    sc.pool_tris.clear();
    sc.pool_attrs.clear();
    sc.meshes.clear();
}

/// Traversal group: shadow casting x alpha cutout, static (0..4) or dynamic (4..8: posed models, own BLASes).
fn group_of(flags: u32) -> usize {
    ((((flags & scene::TRI_NO_SHADOW) << 1) | ((flags & scene::TRI_MASKED) >> 1)) + (flags & scene::TRI_DYNAMIC)) as usize
}

/// Marginal (rows) + conditional (per row) CDFs over luminance * sin(theta).
fn build_env_cdf(env: &gpu::EnvMap) -> Vec<f32> {
    let (w, h) = (env.width as usize, env.height as usize);
    let mut cond = vec![0f32; w * h];
    let mut rows = vec![0f32; h];
    for y in 0..h {
        let sin_t = (((y as f32) + 0.5) / h as f32 * std::f32::consts::PI).sin();
        let mut acc = 0.0f32;
        for x in 0..w {
            let t = env.texels[y * w + x];
            acc += (0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).max(0.0) * sin_t + 1e-12;
            cond[y * w + x] = acc;
        }
        rows[y] = acc;
        for x in 0..w {
            cond[y * w + x] /= acc;
        }
    }
    let total: f32 = rows.iter().sum();
    let mut marg = vec![0f32; h];
    let mut acc = 0.0;
    for y in 0..h {
        acc += rows[y] / total;
        marg[y] = acc;
    }
    marg.extend(cond);
    marg
}

/// UE GGXEnergyLookup equivalent: E.x = integral of D*Vis*NoL (F = 1), E.y = same weighted by (1-VoH)^5.
pub fn energy_lut() -> Vec<[f32; 4]> {
    let n = 32usize;
    let mut vals = vec![[0f32; 2]; n * n];
    for ry in 0..n {
        let roughness = (ry as f32 / (n - 1) as f32).max(0.02);
        let a = roughness * roughness;
        let a2 = a * a;
        for vx in 0..n {
            let nov = (vx as f32 / (n - 1) as f32).max(1e-3);
            let v = Vec3::new((1.0 - nov * nov).sqrt(), 0.0, nov);
            let (mut ex, mut ey) = (0.0f64, 0.0f64);
            let samples = 1024;
            for i in 0..samples {
                // Hammersley + GGX NDF sampling of H.
                let u1 = (i as f32 + 0.5) / samples as f32;
                let u2 = (i as u32).reverse_bits() as f32 / 4294967296.0;
                let phi = 2.0 * std::f32::consts::PI * u2;
                let cos_t = ((1.0 - u1) / (1.0 + (a2 - 1.0) * u1)).sqrt();
                let sin_t = (1.0 - cos_t * cos_t).sqrt();
                let hv = Vec3::new(sin_t * phi.cos(), sin_t * phi.sin(), cos_t);
                let l = 2.0 * v.dot(hv) * hv - v;
                let (nol, noh, voh) = (l.z, hv.z, v.dot(hv));
                if nol > 0.0 && voh > 0.0 {
                    let av = a;
                    let vis_v = nol * (nov * (1.0 - av) + av);
                    let vis_l = nov * (nol * (1.0 - av) + av);
                    let vis = 0.5 / (vis_v + vis_l);
                    // pdf_L = D * NoH / (4 VoH)  =>  D*Vis*NoL / pdf = Vis * NoL * 4 VoH / NoH
                    let est = (vis * nol * 4.0 * voh / noh) as f64;
                    let fc = (1.0 - voh).powi(5) as f64;
                    ex += est;
                    ey += est * fc;
                }
            }
            vals[ry * n + vx] = [(ex / samples as f64).min(1.0) as f32, (ey / samples as f64) as f32];
        }
    }
    vals.chunks(2).map(|c| [c[0][0], c[0][1], c[1][0], c[1][1]]).collect()
}

#[cfg(test)]
mod light_lod_tests {
    use super::*;

    fn light(p: [f32; 3], range: f32, shadowed: bool) -> scene::GpuLight {
        scene::GpuLight { pos: [p[0], p[1], p[2], 1.0], dir: [0.0, 0.0, 0.0, 0.05], color: [1.0, 1.0, 1.0, range], ext: [0.0, 0.0, 0.0, shadowed as u32 as f32], x: [1.0; 4] }
    }

    #[test]
    fn emitters_switched_off_at_load_are_in_the_light_tree() {
        // 600 triangles in a row, 100 on object 0 (on) and 500 on object 1 (switched off at load): too few on for a
        // tree of their own (MIN_STATIC), yet all are in it, at zero weight. Switched on in place and refit, the tree is
        // the one a load with all of them on builds: the plain build's, its powers summed bottom-up.
        let mats = [scene::GpuMaterial { emissive: [1.0, 1.0, 1.0, 0.0], ..Default::default() }];
        let tris: Vec<EmitterTri> = (0..600u32)
            .map(|i| EmitterTri { tri: i, mat: 0, obj: (i % 6 != 0) as u32, area: 0.5, c: [2.0 * i as f32 + 1.0 / 3.0, 1.0 / 3.0, 0.0], moves: false, nominal: false, in_tree: false })
            .collect();
        let verts: Vec<[f32; 4]> = (0..600).flat_map(|i| [[2.0 * i as f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]).collect();
        let (off, on) = ([[0.0, 0.0, 0.0, 1.0], [0.0; 4]], [[0.0, 0.0, 0.0, 1.0]; 2]);
        assert!(ltree::build(&tris, &verts, &|i| emitter_power(&tris[i], &mats, &off, 1)).is_none());
        let mut t = light_tree_of(&tris, &verts, &mats, &off, 1, false).unwrap();
        assert_eq!(t.order.len(), 600);
        assert!((t.power - 100.0 * 0.5).abs() < 1e-3, "{}", t.power);
        let loaded = light_tree_of(&tris, &verts, &mats, &on, 1, false).unwrap();
        let plain = ltree::build(&tris, &verts, &|i| emitter_power(&tris[i], &mats, &on, 1)).unwrap();
        let bits = |t: &ltree::LightTree| t.nodes.iter().map(|n| n.map(f32::to_bits)).collect::<Vec<_>>();
        assert_eq!((bits(&loaded), &loaded.order), (bits(&plain), &plain.order));
        let w: Vec<f32> = t.order.iter().map(|&i| emitter_power(&tris[i as usize], &mats, &on, 1)).collect();
        t.refit(&|k| w[k]);
        assert_eq!(t.order, loaded.order);
        for (a, b) in t.nodes.iter().zip(&loaded.nodes) {
            assert_eq!([a[0], a[1], a[3]].map(f32::to_bits), [b[0], b[1], b[3]].map(f32::to_bits));
            assert!((a[2] - b[2]).abs() <= 1e-4 * b[2], "{} {}", a[2], b[2]);
        }
    }

    #[test]
    fn emitter_guide_finds_what_the_full_search_finds() {
        // 3000 triangles over six decades of power, light LOD on, some switched off (material 0 is black).
        let mut seed = 11u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed
        };
        let mut unit = || (rnd() >> 8) as f32 / (1 << 24) as f32;
        let mats: Vec<scene::GpuMaterial> = (0..4).map(|k| scene::GpuMaterial { emissive: [k as f32 * 3.0, k as f32, 0.5 * k as f32, 0.0], ..Default::default() }).collect();
        let tris: Vec<EmitterTri> = (0..3000)
            .map(|i| EmitterTri { tri: i, mat: if i % 9 == 0 { 0 } else { 1 + i % 3 }, obj: 0, area: 10f32.powf(-6.0 * unit()), c: [300.0 * unit(), 5.0, 100.0 * unit()], moves: i % 50 == 0, nominal: false, in_tree: false })
            .collect();
        let (t, total, _) = emitter_cdf(&tris, &mats, &[], 3, [20.0, 0.0, 30.0, 10.0], EmitterCull::default());
        let n = tris.len() as u32;
        assert!(total > 0.0);
        assert_eq!(t.len(), tris.len() + (EMITTER_GUIDE + 1).div_ceil(4));
        // pathtrace.wgsl: the search over [lo, hi), the guide table, emitter_pick.
        let search = |mut lo: u32, mut hi: u32, u: f32| {
            while lo < hi {
                let mid = (lo + hi) / 2;
                if t[mid as usize][1] < u { lo = mid + 1 } else { hi = mid }
            }
            lo.min(n - 1)
        };
        let guide = |k: u32| t[(n + k / 4) as usize][(k % 4) as usize].to_bits();
        let pick = |u: f32| {
            let k = ((u * EMITTER_GUIDE as f32) as u32).min(EMITTER_GUIDE as u32 - 1);
            search(guide(k), guide(k + 1), u)
        };
        // Random 24-bit u, the bucket edges and their neighbours, and every entry's own cumulative weight.
        let mut us: Vec<u32> = (0..200_000).map(|_| rnd() >> 8).collect();
        for k in 0..=EMITTER_GUIDE as u32 {
            let m = k << 12;
            us.extend([m.saturating_sub(1), m, m + 1].into_iter().filter(|&m| m < 1 << 24));
        }
        us.extend(t[..tris.len()].iter().map(|e| ((e[1] * (1 << 24) as f32) as u32).min((1 << 24) - 1)));
        for m in us {
            let u = m as f32 / (1 << 24) as f32;
            assert_eq!(pick(u), search(0, n, u), "u = {u}");
        }
    }

    #[test]
    fn emitter_lod_falls_off_beyond_d0() {
        let none = EmitterCull::default();
        let cull = EmitterCull { radius: 60.0, ..none };
        assert_eq!(emitter_lod([50.0, 0.0, 0.0], [0.0; 4], none), 1.0);
        assert_eq!(emitter_lod([5.0, 0.0, 0.0], [0.0, 0.0, 0.0, 10.0], none), 1.0);
        assert!((emitter_lod([40.0, 0.0, 0.0], [0.0, 0.0, 0.0, 10.0], none) - 1.0 / 16.0).abs() < 1e-6);
        assert!((emitter_lod([40.0, 0.0, 0.0], [0.0, 0.0, 0.0, 10.0], cull) - 1.0 / 16.0).abs() < 1e-6);
        assert_eq!(emitter_lod([70.0, 0.0, 0.0], [0.0, 0.0, 0.0, 10.0], cull), 0.0);
    }

    #[test]
    fn emitter_cull_keeps_the_view_cone() {
        // Looking down +x, cone half-angle 60 degrees, reach 240 m.
        let cull = EmitterCull { radius: 60.0, far: 240.0, dir: [1.0, 0.0, 0.0], cos: 0.5 };
        let lod = [0.0, 0.0, 0.0, 10.0];
        assert!((emitter_lod([100.0, 0.0, 0.0], lod, cull) - 0.01).abs() < 1e-6); // ahead: kept
        assert_eq!(emitter_lod([50.0, 140.0, 0.0], lod, cull), 0.0); // 70 degrees off, 149 m: outside the cone
        assert!(emitter_lod([100.0, 100.0, 0.0], lod, cull) > 0.0); // 45 degrees, 141 m: kept
        assert_eq!(emitter_lod([-100.0, 0.0, 0.0], lod, cull), 0.0); // behind
        assert_eq!(emitter_lod([300.0, 0.0, 0.0], lod, cull), 0.0); // beyond the reach
        assert!(emitter_lod([0.0, 50.0, 0.0], lod, cull) > 0.0); // beside, within the radius
    }

    #[test]
    fn light_groups_hold_every_light_they_skip() {
        // Pseudo-random lights over a 500 x 200 m district: directional, unshadowed, shadowed, one without a range.
        let mut seed = 7u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        let mut lights = vec![scene::GpuLight { pos: [0.0, 0.0, 0.0, 0.0], ..Default::default() }];
        for k in 0..300 {
            lights.push(light([500.0 * rnd(), 10.0 * rnd(), 200.0 * rnd() - 100.0], 5.0 + 25.0 * rnd(), k % 10 == 0));
        }
        lights[150].color[3] = 0.0;
        let order = light_order(&lights);
        let mut seen = order.clone();
        seen.sort();
        assert_eq!(seen, (0..lights.len()).collect::<Vec<_>>(), "a permutation");
        let sorted: Vec<_> = order.iter().map(|&i| lights[i]).collect();
        let classes: Vec<u8> = sorted.iter().map(light_class).collect();
        assert!(classes.windows(2).all(|w| w[0] <= w[1]), "classes stay in order");
        let table = lights_with_groups(&sorted);
        let nd = 1;
        assert_eq!(table.len(), sorted.len() + (sorted.len() - nd).div_ceil(LIGHT_GROUP));
        for (i, l) in sorted.iter().enumerate().skip(nd) {
            let g = table[sorted.len() + (i - nd) / LIGHT_GROUP].pos;
            let d = Vec3::new(g[0], g[1], g[2]).distance(Vec3::from_slice(&l.pos[..3]));
            assert!(d + light_reach(l) <= g[3], "light {i} reaches out of its group's sphere");
        }
        // Spatially compact: the groups' mean radius well under that of the same classes in their original order.
        let mut plain: Vec<usize> = (0..lights.len()).collect();
        plain.sort_by_key(|&i| light_class(&lights[i]));
        let unsorted = lights_with_groups(&plain.iter().map(|&i| lights[i]).collect::<Vec<_>>());
        let mean = |t: &[scene::GpuLight]| {
            let r: Vec<f32> = t[sorted.len()..].iter().map(|g| g.pos[3]).filter(|r| *r < f32::MAX).collect();
            r.iter().sum::<f32>() / r.len() as f32
        };
        assert!(mean(&table) < 0.5 * mean(&unsorted), "groups are compact: {} vs {}", mean(&table), mean(&unsorted));
    }

    /// Black lights (off, or hidden) are in no group's sphere: one parked far away does not widen its group, and a
    /// group of them alone holds no point.
    #[test]
    fn black_lights_reach_nothing() {
        let at = |x: f32, lit: bool| scene::GpuLight { pos: [x, 0.0, 0.0, 1.0], color: [if lit { 1.0 } else { 0.0 }, 0.0, 0.0, 10.0], ..Default::default() };
        // 16 lit lights within 15 m and, in their group's place, one of them black 10 km away: the sphere as without it.
        let mut near: Vec<_> = (0..LIGHT_GROUP).map(|k| at(k as f32, true)).collect();
        let lit = lights_with_groups(&near)[LIGHT_GROUP].pos;
        near[3] = at(-10_000.0, false);
        let with_black = lights_with_groups(&near)[LIGHT_GROUP].pos;
        assert!(with_black[3] <= lit[3] && with_black[3] > 0.0, "{with_black:?} for {lit:?}");
        // A group of black lights alone: radius -1.
        let dark: Vec<_> = (0..LIGHT_GROUP + 2).map(|k| at(k as f32, k < LIGHT_GROUP)).collect();
        let t = lights_with_groups(&dark);
        assert!(t[dark.len()].pos[3] > 0.0);
        assert_eq!(t[dark.len() + 1].pos[3], -1.0);
    }
}

#[cfg(test)]
mod units_tests {
    use super::*;
    use crate::units::cpu::{Kernel, INF, TRACE_CLOSEST, TRACE_SHADOW};
    use crate::world::{Geometry, Kind, World};
    use glam::{Mat4, Quat};

    /// A world with a flattened ground, meshes of their own (`own_blas`: units under software traversal) turned and
    /// scaled, one that casts no shadow, one hidden, and an instanced mesh of five instances of which four are drawn.
    fn world(ground: bool) -> World {
        let mut w = World::default();
        w.set_material(1, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.5, 0.5, 0.5], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })).unwrap());
        let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
        // A closed shape of 80 triangles (a strip of quads around an axis), for a tree with interior nodes.
        let ring = {
            let (mut p, mut n, mut idx) = (vec![], vec![], vec![]);
            for k in 0..=40u32 {
                let a = k as f32 / 40.0 * std::f32::consts::TAU;
                for y in [-0.5f32, 0.5] {
                    p.extend([a.cos() * (0.6 + 0.2 * y), y, a.sin() * 0.6]);
                    n.extend([a.cos(), 0.0, a.sin()]);
                }
            }
            for k in 0..40u32 {
                idx.extend([2 * k, 2 * k + 1, 2 * k + 2, 2 * k + 1, 2 * k + 3, 2 * k + 2]);
            }
            Geometry { uvs: vec![0.0; p.len() / 3 * 2], positions: p, normals: n, indices: idx, groups: vec![], colors: vec![] }
        };
        w.set_geometry(2, quad(6.0)).unwrap();
        w.set_geometry(3, quad(0.5)).unwrap();
        w.set_geometry(5, ring).unwrap();
        let mesh = |geometry, own_blas| Kind::Mesh { geometry, material: 1, own_blas, materials: vec![] };
        if ground {
            w.set_object(10, "ground".into(), None, mesh(2, false), true, false).unwrap();
            w.set_transform(10, Vec3::new(0.0, -1.0, 0.0), Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2), Vec3::ONE).unwrap();
        }
        let place = |w: &mut World, id: u32, geometry: u32, at: [f32; 3], turn: f32, scale: [f32; 3], shadow: bool| {
            w.set_object(id, format!("o{id}"), None, mesh(geometry, true), shadow, false).unwrap();
            w.set_transform(id, Vec3::from(at), Quat::from_euler(glam::EulerRot::XYZ, turn, turn * 0.7, 0.3), Vec3::from(scale)).unwrap();
        };
        place(&mut w, 11, 5, [0.0, 0.0, 0.0], 0.4, [1.0, 1.0, 1.0], true);
        place(&mut w, 12, 5, [2.0, 0.5, -1.0], 1.9, [0.5, 2.0, 0.7], true);
        place(&mut w, 13, 3, [-2.0, 0.2, 0.5], 0.9, [3.0, 1.0, 1.0], false);
        place(&mut w, 14, 5, [0.3, 0.1, 0.2], 2.5, [1.2, 0.4, 1.2], true);
        place(&mut w, 15, 3, [0.0, 3.0, 0.0], 0.0, [1.0, 1.0, 1.0], true);
        w.set_visible(15, false).unwrap();
        w.set_object(20, "many".into(), None, Kind::Instanced { geometry: 5, procedural: false, materials: vec![1], capacity: 5 }, true, false).unwrap();
        let m: Vec<f32> = (0..5).flat_map(|k| Mat4::from_scale_rotation_translation(Vec3::splat(0.6), Quat::from_rotation_y(k as f32), Vec3::new(-4.0 + 2.0 * k as f32, 1.5, -3.0)).to_cols_array()).collect();
        w.set_instances(20, 4, Some(m), None, None).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(4, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_transform(4, Vec3::new(0.0, 1.0, 9.0), Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_settings(crate::world::Settings { camera: Some(4), ..Default::default() });
        w
    }

    /// `prepare` under software traversal with units lays out what the kernel walks (the still tree, the meshes'
    /// trees and their triangles in tree order, the records with their roots, the top tree, the parameters): the
    /// kernel's walk over it, on the CPU with every read checked, meets what it meets in the same scene with every
    /// instance flattened into world-space triangles (the software traversal there was), ray for ray: the same
    /// distance (to the rounding of testing a triangle in its own space instead of the world's), and shadow rays
    /// blocked alike (a unit that casts none lets them through). No GPU.
    #[test]
    fn a_scene_with_units_is_the_flattened_scene_to_every_ray() {
        agree(&world(true), (3000, 400, 2000));
    }

    /// The same with no still triangle at all: the still tree is then the leaf of one triangle (bvh.rs `EMPTY`), and
    /// that triangle must be one no ray meets, not the first of the units' (a mesh's, in its model space). First seen
    /// on a GPU: `geometry/instanced`, two planes, darker towards a triangle standing at the origin.
    #[test]
    fn a_scene_of_units_alone_is_the_flattened_scene_to_every_ray() {
        agree(&world(false), (1000, 1000, 400));
    }

    /// Units are a live session's and a browser's: an offline render flattens a scene's instances as it did before
    /// units, unless told (THREERT_UNITS=1), and its kernel is then the one without the unit walk. THREERT_UNITS=0
    /// flattens everywhere.
    #[test]
    fn an_offline_render_has_no_units_unless_told() {
        let browser = cfg!(target_arch = "wasm32");
        assert_eq!((units_said(None, false), units_said(None, true)), (browser, true));
        assert_eq!((units_said(Some("1"), false), units_said(Some("1"), true)), (true, true));
        assert_eq!((units_said(Some("0"), false), units_said(Some("0"), true)), (false, false));
        assert_eq!(units_said(Some(""), false), browser);
        if std::env::var("THREERT_UNITS").is_ok() {
            return;
        }
        // What `prepare` hands an offline render of a scene with instances, and `prepare_live` a live session.
        let build = |live: bool| {
            let mut sc = scene::build(&world(true), std::path::Path::new("."), &default_presets_dir(), None).unwrap();
            let p = prepare_as(false, units_on(live), &mut sc, &mut vec![]);
            (sc, p)
        };
        let (off, po) = build(false);
        let (flat, pf) = {
            let mut sc = scene::build(&world(true), std::path::Path::new("."), &default_presets_dir(), None).unwrap();
            let p = prepare_as(false, false, &mut sc, &mut vec![]);
            (sc, p)
        };
        assert!(off.units.is_none() && off.instances.is_empty(), "offline: flattened");
        assert_eq!(gpu::instance_features(&off) & gpu::F_UNITS, 0, "offline: the kernel without the unit walk");
        assert_eq!(gpu::instance_features(&off), gpu::instance_features(&flat));
        assert!(bytemuck::cast_slice::<_, u8>(&po.tri_verts) == bytemuck::cast_slice::<_, u8>(&pf.tri_verts) && bytemuck::cast_slice::<_, u8>(&po.nodes) == bytemuck::cast_slice::<_, u8>(&pf.nodes), "offline: the flattened scene's bytes");
        let (live, _) = build(true);
        assert!(live.units.is_some() && gpu::instance_features(&live) & gpu::F_UNITS != 0, "live: units");
        assert!(units_wanted(false, &scene::build(&world(true), std::path::Path::new("."), &default_presets_dir(), None).unwrap()) && !units_wanted(true, &off));
    }

    fn agree(w: &World, least: (usize, usize, usize)) {
        let build = |units: bool| {
            let mut sc = scene::build(&w, std::path::Path::new("."), &default_presets_dir(), None).unwrap();
            let p = prepare_as(false, units, &mut sc, &mut vec![]);
            (sc, p)
        };
        let (su, pu) = build(true);
        let (sf, pf) = build(false);
        assert_eq!(su.tris.is_empty(), w.objects.get(&10).is_none(), "still triangles: the ground's only");
        let u = su.units.as_ref().expect("the scene has units");
        assert!(sf.units.is_none() && sf.instances.is_empty(), "flattened: no instance is left");
        // Nine instances: four own meshes shown, one hidden (no instance at all), five of the instanced mesh of which
        // one is hidden: eight units in the top tree.
        assert_eq!((su.instances.len(), u.count), (9, 8), "{:?}", su.instances.iter().map(|i| (i.name.clone(), i.hidden)).collect::<Vec<_>>());
        assert!(gpu::instance_features(&su) & gpu::F_UNITS != 0 && gpu::instance_features(&sf) & gpu::F_UNITS == 0);
        assert_eq!((pu.params.motion[2].to_bits(), pu.params.motion[3].to_bits()), (u.layout.top_first, 8));
        // The flattened scene holds every instance's triangles; the scene with units holds each mesh's once.
        assert!(pf.tri_verts.len() > pu.tri_verts.len());
        assert_eq!(pu.nodes.len(), u.layout.top_first as usize + u.layout.top_cap as usize);
        let kernel = |p: &Prepared, sc: &scene::Scene| {
            let (top, units) = (p.params.motion[2].to_bits(), p.params.motion[3].to_bits());
            (p.nodes.clone(), p.tri_verts.clone(), sc.gpu_instances.clone(), top, units)
        };
        let (ku, kf) = (kernel(&pu, &su), kernel(&pf, &sf));
        let walk = |k: &(Vec<bvh::GpuNode>, Vec<[f32; 4]>, Vec<scene::GpuInstance>, u32, u32), o: Vec3A, d: Vec3A, mode: u32| {
            let instances = if k.2.is_empty() { vec![scene::GpuInstance::default()] } else { k.2.clone() };
            let kernel = Kernel { nodes: &k.0, tri_verts: &k.1, instances: &instances, top: k.3, units: k.4, steps: Default::default(), budget: 1 << 20, deepest: Default::default(), reads: Default::default() };
            kernel.trace(o, d, INF, mode).unwrap_or_else(|e| panic!("ray {o} {d}, mode {mode}: the kernel {e}"))
        };
        let mut s = 12345u64;
        let mut rnd = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 40) as f32) / (1u64 << 24) as f32
        };
        let (mut hits, mut on_units, mut shadowed) = (0, 0, 0);
        for k in 0..9000 {
            // From around the camera into the scene, through the space around the origin (where a mesh's model-space
            // triangles would stand if anything took them for the world's), and from above straight down.
            let (o, d) = if k % 3 == 0 {
                (Vec3A::new(8.0 * rnd() - 4.0, 4.0 * rnd(), 9.0), Vec3A::new(3.0 * rnd() - 1.5, 3.0 * rnd() - 2.0, -4.0).normalize())
            } else if k % 3 == 1 {
                (Vec3A::new(2.0 * rnd() - 1.0, 2.0 * rnd() - 1.0, 4.0), Vec3A::new(0.1 * rnd() - 0.05, 0.1 * rnd() - 0.05, -1.0).normalize())
            } else {
                (Vec3A::new(12.0 * rnd() - 6.0, 6.0, 8.0 * rnd() - 5.0), Vec3A::new(0.2 * rnd() - 0.1, -1.0, 0.2 * rnd() - 0.1).normalize())
            };
            let (a, b) = (walk(&ku, o, d, TRACE_CLOSEST), walk(&kf, o, d, TRACE_CLOSEST));
            assert!((a.t - b.t).abs() <= 2e-4 * (1.0 + b.t.min(1e4)) || (a.t == INF) == (b.t == INF) && a.t == INF, "ray {o} {d}: with units {a:?}, flattened {b:?}");
            assert_eq!(a.t == INF, b.t == INF, "ray {o} {d}: with units {a:?}, flattened {b:?}");
            hits += (a.t < INF) as usize;
            on_units += (a.inst != units::cpu::NO_INST) as usize;
            let (sa, sb) = (walk(&ku, o, d, TRACE_SHADOW), walk(&kf, o, d, TRACE_SHADOW));
            assert_eq!(sa.t == INF, sb.t == INF, "shadow ray {o} {d}: with units {sa:?}, flattened {sb:?}");
            shadowed += (sa.t < INF) as usize;
        }
        assert!(hits > least.0 && on_units > least.1 && shadowed > least.2, "{hits} hits, {on_units} on units, {shadowed} shadow rays blocked");
    }
}

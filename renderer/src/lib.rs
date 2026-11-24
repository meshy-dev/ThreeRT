//! threert-render as a library: scene loading, the GPU path tracer, the real-time LiveCore (used by the
//! `threert-render` CLI and by the native clients).

pub mod api;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub mod audio;
pub mod bvh;
#[cfg(not(target_arch = "wasm32"))]
pub mod compare;
#[cfg(not(target_arch = "wasm32"))]
pub mod denoise;
pub mod desc;
pub mod files;
pub mod gpu;
pub mod heap;
#[cfg(not(target_arch = "wasm32"))]
pub mod host;
pub mod indexed;
pub mod live;
pub mod ltree;
pub mod merged;
pub mod post;
pub mod procasset;
pub mod scene;
#[cfg(not(target_arch = "wasm32"))]
pub mod script;
pub mod skin;
pub mod sample;
pub mod sky;
pub mod tables;
pub mod tangents;
pub mod units;
#[cfg(not(target_arch = "wasm32"))]
pub mod userfiles;
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
    /// The emitters' triangles as light sampling reads them, one per `emitter_tris`.
    pub gpu_emitter_tris: Vec<GpuEmitterTri>,
    /// The runs of the list that follow their triangles after the load.
    pub emitter_runs: EmitterRuns,
    pub env: gpu::EnvMap,
    pub env_cdf: Vec<f32>,
    pub params: gpu::Params,
    pub energy: Vec<[f32; 4]>,
    /// Emissive triangles (also those at power 0), for re-weighting `emitters` after power / material edits.
    pub emitter_tris: Vec<EmitterTri>,
    /// The light tree over the static emitters (`ltree`).
    pub light_tree: Option<ltree::LightTree>,
    /// Material -> runs of its flattened static triangles (first, end in the traversal order, object): a material
    /// that starts emitting gets emitter entries in place from its runs (live/edits.rs `add_material_emitters`).
    /// `moving_mats`: the materials of dynamic world-space triangles and of particle systems (`Tables::moving_mats`).
    pub mat_runs: std::collections::HashMap<u32, Vec<[u32; 3]>>,
    pub moving_mats: std::collections::HashSet<u32>,
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
    let area = 0.5 * e1.cross(e2).length();
    if !(area * lum > 0.0) {
        return None;
    }
    let c = v(0) + (e1 + e2) / 3.0;
    let moves = tri_verts[k * 3][3].to_bits() & scene::TRI_DYNAMIC != 0;
    Some(EmitterTri { tri: k as u32, mat, obj: a.x[1].to_bits(), area, c: c.to_array(), moves, nominal: false, in_tree: false, inst: NO_INST, part: 0, card: is_card(m) })
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
    /// A particle's: `area` is the nominal one (Particles emitterArea), what the CDF weighs it by; its pass places
    /// its triangle, of whatever size.
    pub nominal: bool,
    /// One of the light tree's (`ltree`): its CDF entry is in the tree's range, at the start of the CDF.
    pub in_tree: bool,
    /// The instance that places its triangle (its record; the triangle is the mesh's, in model space), or `NO_INST`:
    /// a triangle in world space.
    pub inst: u32,
    /// Its section of that instance's mesh (0 without one): a mesh's triangle does not say which section it is of,
    /// so a light sample is told here, as a ray is by its hit.
    pub part: u32,
    /// A rect light's card (its material: `scene::MAT_EMITTER_CARD`): it lights its front side alone.
    pub card: bool,
}

fn is_card(m: &scene::GpuMaterial) -> bool {
    m.tex1[2] == scene::MAT_EMITTER_CARD
}

/// No instance (pathtrace.wgsl `NO_INST`).
pub const NO_INST: u32 = u32::MAX;

/// `GpuEmitterTri::code`: the emitter moves (`EmitterTri::moves`). Path codes are below it.
pub const EMITTER_MOVES: u32 = 1 << 31;
/// `GpuEmitterTri::flags`: a rect light's card (`EmitterTri::card`).
pub const EMITTER_CARD: u32 = 1;
const _: () = assert!((ltree::PATH_BITS + 2) << ltree::PATH_BITS <= EMITTER_MOVES);

/// An emitter's triangle as the kernel's light sampling reads it (pathtrace.wgsl `EmitterTri`; `Table::EmitterTris`,
/// entry i for `emitter_tris[i]`): in world space, the light list's own copy, so a sample reads no placement. v0, e1,
/// e2 in world space; its triangle in the triangles' table (attributes); the instance that places it (`NO_INST`: the
/// triangle is in world space there too); where its weight is (`emitter_codes`) with `EMITTER_MOVES`; its section of
/// that instance's mesh; `EMITTER_CARD`. 64 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuEmitterTri {
    pub v0: [f32; 3],
    pub tri: u32,
    pub e1: [f32; 3],
    pub inst: u32,
    pub e2: [f32; 3],
    pub code: u32,
    pub part: u32,
    pub flags: u32,
    pub spare: [u32; 2],
}

impl GpuEmitterTri {
    /// `e`'s, from its triangle's rows (v0, e1, e2: `Prepared::tri_verts`), with path code `code`.
    pub fn new(e: &EmitterTri, rows: &[[f32; 4]], code: u32) -> GpuEmitterTri {
        let v = |r: usize| [rows[r][0], rows[r][1], rows[r][2]];
        // (As a hit has it: with an instance its section, without one its row, which is its triangle's.)
        let part = if e.inst == NO_INST { e.tri } else { e.part };
        GpuEmitterTri { v0: v(0), tri: e.tri, e1: v(1), inst: e.inst, e2: v(2), code: code | if e.moves { EMITTER_MOVES } else { 0 }, part, flags: if e.card { EMITTER_CARD } else { 0 }, spare: [0; 2] }
    }

    /// As `new`, from its vertices.
    pub fn of(e: &EmitterTri, [a, b, c]: [glam::Vec3A; 3], code: u32) -> GpuEmitterTri {
        let (e1, e2) = (b - a, c - a);
        GpuEmitterTri::new(e, &[[a.x, a.y, a.z, 0.0], [e1.x, e1.y, e1.z, 0.0], [e2.x, e2.y, e2.z, 0.0]], code)
    }

    /// Its vertices.
    pub fn vertices(&self) -> [[f32; 3]; 3] {
        let at = |e: [f32; 3]| [self.v0[0] + e[0], self.v0[1] + e[1], self.v0[2] + e[2]];
        [self.v0, at(self.e1), at(self.e2)]
    }
}

/// The emitters of an instance that has some (`count` of them: `scene::rows_emitters`, or a model's mesh's
/// `InstMesh::emitters`): one a numbered
/// triangle, in the numbers' order, each with its triangle as the instance places it. `tris`: the mesh's, in model
/// space; `first`: its first triangle in the triangles' table; `numbered(k)`: triangle k's number, material and
/// section, if it emits; `record`: the instance's; `nominal`: the area a triangle is weighed by where the mesh is a
/// particle system's (`scene::ParticleSystem::emitter_area`, half of it: the pass places the triangle, of whatever
/// size). A triangle of no area: no weight (area 0). A hidden instance's (an instanced
/// object's copy not shown) keep their mesh's own area: its object has no power, so they weigh nothing, and the light
/// tree has them (`switchable_power`) for when the copy is shown.
pub fn instance_emitters(inst: &scene::Instance, record: u32, tris: &[[glam::Vec3A; 3]], first: u32, numbered: &dyn Fn(usize) -> Option<(usize, u32, u32)>, count: usize, materials: &[scene::GpuMaterial], moves: bool, nominal: Option<f32>) -> Vec<(EmitterTri, [glam::Vec3A; 3])> {
    let dead = EmitterTri { tri: first, mat: 0, obj: inst.object, area: 0.0, c: [0.0; 3], moves, nominal: false, in_tree: false, inst: record, part: 0, card: false };
    let mut out = vec![(dead, [glam::Vec3A::ZERO; 3]); count];
    for (k, t) in tris.iter().enumerate() {
        let Some((slot, mat, part)) = numbered(k) else { continue };
        let (area, c, w) = placed_emitter(inst, t, mat, materials);
        // (A particle system's: weighed by its nominal area, whatever size its pass gives it.)
        out[slot] = (EmitterTri { tri: first + k as u32, mat, area: nominal.unwrap_or(area), c, part, nominal: nominal.is_some(), card: materials.get(mat as usize).is_some_and(is_card), ..dead }, w);
    }
    out
}

/// An instance's emitter of its mesh's triangle `t` (material `mat`): its area, its centre and the triangle as the
/// instance places it (`instance_emitters` has which area).
pub fn placed_emitter(inst: &scene::Instance, t: &[glam::Vec3A; 3], mat: u32, materials: &[scene::GpuMaterial]) -> (f32, [f32; 3], [glam::Vec3A; 3]) {
    let w = t.map(|v| glam::Vec3A::from(inst.world.transform_point3(v.into())));
    let emits = materials.get(mat as usize).is_some_and(|m| 0.2126 * m.emissive[0] + 0.7152 * m.emissive[1] + 0.0722 * m.emissive[2] > 0.0);
    let shape = if inst.hidden { *t } else { w };
    let area = if emits { 0.5 * (shape[1] - shape[0]).cross(shape[2] - shape[0]).length() } else { 0.0 };
    (area, ((w[0] + w[1] + w[2]) / 3.0).to_array(), w)
}

/// Runs of the light list (first emitter, count) whose triangles something moves after the load; the GPU takes them
/// again from where the triangles are (gpu.rs `EmitterFill`): the emitters of the triangles the CPU reposes, and the
/// particles', which their passes place. (An instance that moves has its run filled when it does: live/edits.rs.)
#[derive(Default)]
pub struct EmitterRuns {
    pub moved: Vec<[u32; 2]>,
    pub particles: Vec<[u32; 2]>,
}

impl EmitterRuns {
    pub fn of(tris: &[EmitterTri]) -> EmitterRuns {
        let runs = |which: &dyn Fn(&EmitterTri) -> bool| {
            let mut out: Vec<[u32; 2]> = vec![];
            for (i, _) in tris.iter().enumerate().filter(|(_, e)| which(e)) {
                match out.last_mut() {
                    Some(r) if r[0] + r[1] == i as u32 => r[1] += 1,
                    _ => out.push([i as u32, 1]),
                }
            }
            out
        };
        EmitterRuns { moved: runs(&|e| e.moves && !e.nominal && e.inst == NO_INST), particles: runs(&|e| e.nominal) }
    }
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
pub fn light_tree_of(tris: &[EmitterTri], verts: &dyn Fn(usize) -> [[f32; 3]; 3], materials: &[scene::GpuMaterial], objects: &[[f32; 4]], stride: usize, slot: bool) -> Option<ltree::LightTree> {
    let build = if slot { ltree::build_slotted } else { ltree::build };
    let mut t = build(tris, verts, &|i| switchable_power(&tris[i], materials, objects, stride))?;
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
/// range. (A black light gives nothing anywhere: `light_groups` leaves it out.)
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
/// each class spatially sorted into the kernel's light groups (`light_groups`).
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

/// The light groups' table: per group of LIGHT_GROUP local lights (counted from the first local one) the sphere
/// (centre, radius) holding every point its lights reach. Black lights (off, or hidden: `scene::gpu_light_shown`)
/// reach nothing and are in no sphere; a group of them alone has radius -1 (no point is in it).
pub fn light_groups(lights: &[scene::GpuLight]) -> Vec<[f32; 4]> {
    let nd = lights.iter().take_while(|l| l.pos[3] == 0.0).count();
    let mut out = vec![];
    for g in lights[nd..].chunks(LIGHT_GROUP) {
        let lit = |l: &&scene::GpuLight| l.color[..3].iter().any(|c| *c > 0.0);
        if !g.iter().any(|l| lit(&l)) {
            out.push([0.0, 0.0, 0.0, -1.0]);
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
        // Margin for rounding in the kernel's distance (positions up to a few km).
        if r < f32::MAX {
            r = r * (1.0 + 1e-5) + 1e-3 * (1.0 + c.abs().max_element() * 1e-3);
        }
        out.push([c.x, c.y, c.z, r]);
    }
    out
}

/// Buckets of the emitter CDF's guide table (pathtrace.wgsl `EMITTER_GUIDE`, a power of two).
pub const EMITTER_GUIDE: usize = 65536;

/// The order of a scene's emitters in the CDF: the light tree's in its leaf order (a node covers a run of entries),
/// then the rest as the list has them. The list itself keeps its order.
pub fn emitter_order(tris: &[EmitterTri], tree: Option<&ltree::LightTree>) -> Vec<u32> {
    let all = 0..tris.len() as u32;
    let Some(t) = tree else { return all.collect() };
    let mut placed = vec![false; tris.len()];
    for &i in &t.order {
        placed[i as usize] = true;
    }
    t.order.iter().copied().chain(all.filter(|&i| !placed[i as usize])).collect()
}

/// What each emitter of the list carries for the kernel to find its weight by (`GpuEmitterTri::code`), for the CDF's
/// `order`: one of the tree's its path code, any other its entry in the CDF (no depth bits: `emit_mis` reads the
/// entry's weight); with `EMITTER_MOVES`.
pub fn emitter_codes(tris: &[EmitterTri], order: &[u32], tree: Option<&ltree::LightTree>) -> Vec<u32> {
    let in_tree = tree.map_or(0, |t| t.order.len());
    assert!(order.len() < 1 << ltree::PATH_BITS, "an entry of the CDF is named in a code's path bits");
    let mut out = vec![0; tris.len()];
    for (k, &i) in order.iter().enumerate() {
        let code = if k < in_tree { tree.unwrap().code[k] } else { k as u32 };
        out[i as usize] = code | if tris[i as usize].moves { EMITTER_MOVES } else { 0 };
    }
    out
}

/// Emitter CDF entries (its emitter's index in `tris`: bits, normalized cumulative weight, weight, area), one per
/// emitter in `order` (`emitter_order`); then the guide table; the total weight and the total without light LOD
/// (Params::emit_power). Weight = area x emissive luminance x object power (x light LOD). Triangles switched off
/// keep a zero-weight entry.
/// Guide table (the cutpoint method): EMITTER_GUIDE + 1 indices, four per vec4 (bits); index k is where the kernel's
/// binary search lands for u = k / EMITTER_GUIDE, so a draw u only searches between the indices of its bucket and the
/// next, and finds the same entry as a search over the whole CDF.
pub fn emitter_cdf(tris: &[EmitterTri], order: &[u32], materials: &[scene::GpuMaterial], objects: &[[f32; 4]], stride: usize, lod: [f32; 4], cull: EmitterCull) -> (Vec<[f32; 4]>, f32, f32) {
    let (mut total, mut power) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(order.len());
    for &i in order {
        let e = &tris[i as usize];
        let lod_k = if e.moves { 1.0 } else { emitter_lod(e.c, lod, cull) };
        let p = emitter_power(e, materials, objects, stride);
        power += p;
        total += p * lod_k;
        out.push([f32::from_bits(i), total, p * lod_k, e.area]);
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
            Some(t) => flatten_deforming(sc, t),
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
        bvh::Bvh { nodes: vec![bvh::GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 }], order, depth: 0 }
    } else {
        let b = bvh::build(&sc.tris);
        timings.push(("bvh_depth".into(), b.depth as f64));
        b
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
        // (No class on a mesh's triangle: its instances' rows say which sections are cutouts or glass.)
        tri_verts.push([a0.x, a0.y, a0.z, f32::from_bits(if k < deforming { scene::TRI_DYNAMIC } else { 0 })]);
        tri_verts.push([e1.x, e1.y, e1.z, 0.0]);
        tri_verts.push([e2.x, e2.y, e2.z, 0.0]);
        attrs.push(*a);
    }
    // (A deforming mesh's previous vertices lie as its triangles do, counted from the first dynamic triangle: what
    // gpu.rs `prev_verts` holds at the build. Its instance's record says where.)
    sc.gpu_instances = sc.instances.iter().map(|i| i.gpu(pool_first as u32 + sc.meshes[i.mesh as usize].first)).collect();
    // A row's section starts here in the triangles' table (hardware traversal: a hit's geometry is its section, its
    // primitive counts from there).
    for i in &sc.instances {
        let m = sc.meshes[i.mesh as usize];
        for (s, row) in m.sections(&sc.sections).iter().zip(&mut sc.rows[i.rows as usize..]) {
            row[1] = pool_first as u32 + m.first + s.first;
        }
    }
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
    sc.attrs = attrs;
    // Vertex colours (scene.rs `tri_colors`), with the triangles: the flattened ones in traversal order, the pool.
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
    if hwrt && sc.retained.dynamics_mut().next().is_some() {
        let first_dynamic: usize = groups[..4].iter().sum();
        let finals: std::collections::HashMap<u32, u32> = bvh.order[first_dynamic..].iter().enumerate().map(|(k, &i)| (i, (first_dynamic + k) as u32)).collect();
        for d in sc.retained.dynamics_mut() {
            d.finals = (0..d.mats.len() as u32).map(|j| finals[&(d.first + j)]).collect();
        }
    }
    timings.push(("bvh_ms".into(), t.elapsed().as_secs_f64() * 1000.0));

    // The light list (an emitter's weight = area x luminance of the emissive factor x object power): the flattened
    // triangles that emit, then, an instance at a time, those of its triangles that emit: a run of the list an
    // instance, its first in its record, its triangles' numbers counted from it (a model's mesh), or from where its
    // section's start, which the section's row has (a mesh with sections). Each with its triangle in world space.
    let mut emitter_tris: Vec<EmitterTri> = vec![];
    let mut gpu_emitter_tris: Vec<GpuEmitterTri> = vec![];
    for k in 0..pool_first {
        if let Some(e) = emitter_tri(k, &tri_verts, &sc.attrs[k], &sc.materials) {
            gpu_emitter_tris.push(GpuEmitterTri::new(&e, &tri_verts[k * 3..][..3], 0));
            emitter_tris.push(e);
        }
    }
    // (And the instances hidden at the build, `Instance::masked`: where they stand when they are shown is theirs to
    // bound then, in the light tree's moving subtree.)
    let moving = sc.retained.moving_records();
    // (The particle systems the lights are sampled by: half a quad's nominal area a triangle.)
    let nominal: std::collections::HashMap<u32, f32> = sc.particles.iter().map(|p| (p.mesh, p.emitter_area)).chain(sc.gpu_particles.iter().map(|p| (p.mesh, p.emitter_area))).filter(|(_, a)| *a > 0.0).map(|(m, a)| (m, a * 0.5)).collect();
    for r in 0..sc.instances.len() {
        let m = sc.meshes[sc.instances[r].mesh as usize];
        // (With sections: the emitters of its sections whose material emits, a section after another; each one's row
        // has where they start.)
        let (sections, at) = (m.sections(&sc.sections), sc.instances[r].rows as usize);
        let rows = sc.rows.get(at..at + sections.len()).unwrap_or(&[]).to_vec();
        let starts = scene::section_emitters(sections, &rows, &sc.materials);
        let count = scene::rows_emitters(sections, &rows, &sc.materials);
        if count == 0 {
            continue;
        }
        let first = emitter_tris.len() as u32;
        sc.instances[r].first_emitter = first;
        let area = nominal.get(&sc.instances[r].mesh).copied();
        for (k, start) in starts.iter().enumerate() {
            sc.rows[at + k][2] = start.map_or(scene::NO_EMITTER, |s| first + s);
        }
        let (a, b) = (m.first as usize, (m.first + m.count) as usize);
        let attrs = &sc.attrs[pool_first + a..pool_first + b];
        let numbered = |k: usize| scene::section_slot(&attrs[k], &rows, &starts);
        for (e, w) in instance_emitters(&sc.instances[r], r as u32, &sc.pool_tris[a..b], (pool_first + a) as u32, &numbered, count as usize, &sc.materials, moving.contains(&(r as u32)) || sc.instances[r].masked || area.is_some(), area) {
            gpu_emitter_tris.push(GpuEmitterTri::of(&e, w, 0));
            emitter_tris.push(e);
        }
    }
    // Where each material's flattened static triangles are (runs of the traversal order, with their object), for a
    // material that starts emitting in the live view (tables.rs `apply`): its emitter entries are made from them in
    // place, as each instance with a row of it gets its own (live/edits.rs `add_material_emitters`). Not the
    // materials of dynamic world-space triangles: `moving_mats`. (A mesh that deforms is a model's, whose materials
    // are the file's: no edit names them.)
    let mut mat_runs: std::collections::HashMap<u32, Vec<[u32; 3]>> = std::collections::HashMap::new();
    let mut moving_mats: std::collections::HashSet<u32> = Default::default();
    for (k, a) in sc.attrs[..pool_first].iter().enumerate() {
        let (mat, obj) = (a.uv[3].to_bits(), a.x[1].to_bits());
        if tri_verts[k * 3][3].to_bits() & scene::TRI_DYNAMIC != 0 {
            moving_mats.insert(mat);
            continue;
        }
        let runs = mat_runs.entry(mat).or_default();
        match runs.last_mut() {
            Some(r) if r[1] == k as u32 && r[2] == obj => r[1] += 1,
            _ => runs.push([k as u32, k as u32 + 1, obj]),
        }
    }
    // (Nor a particle system's material: its triangles are its pass's.)
    // (Not a translucent system's, which has no emitter and no class: its material's function changes in place.)
    for i in sc.instances.iter().filter(|i| i.bits & scene::INST_PARTICLES != 0 && !i.hidden) {
        moving_mats.insert(scene::row_material(&sc.rows[i.rows as usize]));
    }
    let tri_order = bvh.order[..pool_first.min(bvh.order.len())].to_vec();
    // Many static emitters: a light tree samples them by importance at the shading point (`ltree`); each one's path
    // code with its triangle in the list (emit_mis). THREERT_LTREE=0: the CDF alone (A/B).
    // (Switched-off objects at full power: `switchable_power`.)
    let all_power: f32 = emitter_tris.iter().map(|e| switchable_power(e, &sc.materials, &sc.objects, sc.object_stride())).sum();
    let use_tree = std::env::var("THREERT_LTREE").map_or(true, |v| v != "0");
    let mut light_tree = if use_tree { light_tree_of(&emitter_tris, &|i| gpu_emitter_tris[i].vertices(), &sc.materials, &sc.objects, sc.object_stride(), sc.emitter_slot) } else { None };
    // (A hidden instance's emitters, an instanced object's copies not shown, bound nothing: the moving subtree's
    // spheres without them, as an edit leaves them.)
    let gone = |i: usize| sc.instances.get(emitter_tris[i].inst as usize).is_some_and(|x| x.hidden);
    if let Some(t) = light_tree.as_mut().filter(|_| (0..emitter_tris.len()).any(gone)) {
        t.refit_moving(&|i| (!gone(i)).then(|| gpu_emitter_tris[i].vertices()));
    }
    // The list keeps its own order (the tree and the CDF name its emitters: `emitter_order`); each emitter carries
    // where the kernel finds its weight (`emitter_codes`).
    if let Some(t) = &light_tree {
        for &i in &t.order {
            emitter_tris[i as usize].in_tree = true;
        }
    }
    let order = emitter_order(&emitter_tris, light_tree.as_ref());
    for (g, code) in gpu_emitter_tris.iter_mut().zip(emitter_codes(&emitter_tris, &order, light_tree.as_ref())) {
        g.code = code;
    }
    // A triangle in world space names its emitter in its attributes (x.w = the entry + 1), for the rays that hit it
    // (emit_mis); a mesh's triangle is numbered among its mesh's, and its instance has the first.
    for (i, e) in emitter_tris.iter().enumerate().filter(|(_, e)| e.inst == NO_INST) {
        sc.attrs[e.tri as usize].x[3] = f32::from_bits(i as u32 + 1);
    }
    let emitter_runs = EmitterRuns::of(&emitter_tris);
    let (emitters, total, _) = emitter_cdf(&emitter_tris, &order, &sc.materials, &sc.objects, sc.object_stride(), [0.0; 4], EmitterCull::default());
    // The moving emitters (after the tree's in the CDF) are picked with the probability of their share of power.
    let ltree_param = light_tree.as_ref().map_or([0.0; 4], |t| [0.0, f32::from_bits(t.order.len() as u32), if total > 0.0 { ((total - t.power) / total).max(0.0) } else { 0.0 }, ltree::RIS_OFFLINE as f32]);

    // Sky / environment.
    let t = Instant::now();
    let baked = bake_sky(&sc.sky, sc.environment.cloud_cover, sc.camera.position.y.max(0.0));
    // (The atmosphere's sun: its light's colour is what the bake leaves it.)
    if let (Some(i), Some(c)) = (sc.retained.sun.and_then(|s| sc.retained.light_nodes.iter().position(|n| *n == s)), baked.sun) {
        sc.lights[i].color = [c.x, c.y, c.z, 0.0];
    }
    let env_cdf = build_env_cdf(&baked.env);
    let procs = [0, sc.procs.len() as u32, sc.gpu_instances.len() as u32, 0];
    // Environment light sampling (one shadow ray per vertex) is skipped when the sky is negligible next to the emitters
    // (night streets): escaping BSDF rays still pick it up, with full MIS weight (`SkyBaked::write`). The emitters as
    // if all on: a time-of-day room loaded by day decides at dusk as one loaded at night does (live/edits.rs `put_sky`).
    let em_area: f32 = emitters[..emitter_tris.len()].iter().map(|e| e[3]).sum();
    let env_nee_floor = if emitter_tris.is_empty() { 0.0 } else { 0.01 * all_power / em_area.max(1e-6) };
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
    let params = gpu::Params {
        cam_pos: [cam.position.x, cam.position.y, cam.position.z, cam.tan_half_fov],
        cam_fwd: [cam.forward.x, cam.forward.y, cam.forward.z, aspect],
        cam_right: [cam.right.x, cam.right.y, cam.right.z, lens_radius],
        cam_up: [cam.up.x, cam.up.y, cam.up.z, focus],
        size: [sc.width, sc.height, 0, rng_seed()],
        // (The preset's and the exposure's: `Params::set_preset`; the sky's: `SkyBaked::write`; the environment's:
        // `EnvSettings::write`. Below.)
        counts: [sc.lights.len() as u32, if total > 0.0 { emitter_tris.len() as u32 } else { 0 }, 0, 0],
        env: [0.0, 0.0, 0.0, -1.0],
        misc: [0.0, total, 0.0, 0.0],
        sky_color: [0.0; 4],
        counts2: [n_dir, 0, 0, first_shadowed],
        time: [sc.time, 0.0, 0.0, sc.media.len() as f32],
        fog: [0.0; 4],
        fog_color: [0.0; 4],
        vol: [0.0; 4],
        vol2: [0.0; 4],
        jitter: [0.0; 4],
        cache: [0.0; 4],
        // (z, w: software traversal's units: the top tree's root and their count, as bits; trace_units.wgsl. y is
        // `render --signals`'.)
        motion: sc.units.as_ref().map_or([0.0; 4], |u| [0.0, 0.0, f32::from_bits(u.layout.top_first), f32::from_bits(u.count)]),
        group_first: [[0; 4]; 2],
        light_lod: [0.0; 4],
        emit_power: [total, 0.0, 0.0, 0.0],
        aerial: [0.0; 4],
        background: [0.0; 4],
        lod_view: [0.0; 4],
        ltree: ltree_param,
        procs,
        ltile: [0; 4],
        ltile2: [0; 4],
        lt_cam: [[0.0; 4]; 4],
        // (`gpu::Uploaded::place`, with the buffers.)
        table_first: [[0; 4]; 3],
        word_first: [[0; 4]; 2],
        // (`gpu::KernelBufs::place`, with what is traced into.)
        live_first: [[0; 4]; 2],
        live_len: [0; 4],
    };
    let mut params = params;
    params.set_preset(&sc.preset.native, &cam.exposure);
    baked.write(&mut params, env_nee_floor);
    sc.environment.write(&mut params);
    let env = baked.env;
    let mut first = 0u32;
    for (g, &count) in groups.iter().enumerate() {
        params.group_first[g / 4][g % 4] = first;
        first += u32::try_from(count).expect("more than 2^32 triangles");
    }
    let energy = energy_lut();
    let t = Instant::now();
    sc.indexed = index_geometry(sc, &tri_order, &tri_verts, &groups, pool_first, no_still);
    // Software traversal: the trees' leaves, every triangle as it was laid out for its tree above (v0, e1, e2), a
    // mesh's with its section and its index there, a world-space one's with its flags.
    if !hwrt {
        let pool = pool_first..pool_first + sc.pool_tris.len();
        let rows = &sc.indexed.rows;
        sc.indexed.leaves = tri_verts
            .chunks_exact(3)
            .enumerate()
            .map(|(k, t)| {
                let ids = if pool.contains(&k) { [rows[k][0], rows[k][3]] } else { [t[0][3].to_bits(), 0] };
                [[t[0][0], t[0][1], t[0][2], f32::from_bits(ids[0])], [t[1][0], t[1][1], t[1][2], f32::from_bits(ids[1])], [t[2][0], t[2][1], t[2][2], 0.0]]
            })
            .collect();
    }
    timings.push(("index_ms".into(), t.elapsed().as_secs_f64() * 1000.0));
    timings.push(("vertices".into(), sc.indexed.vertices.len() as f64));
    // An instance's triangles count their vertices from its mesh's first; the previous positions from the first
    // vertex that moves.
    // (A deforming mesh's previous positions: where its vertices are among those that move.)
    for (m, &(first, count)) in sc.meshes.iter_mut().zip(&sc.indexed.meshes) {
        (m.vfirst, m.vcount) = (first, count);
    }
    for (r, i) in sc.instances.iter_mut().enumerate() {
        i.verts = sc.indexed.meshes[i.mesh as usize].0;
        let deforms = sc.meshes[i.mesh as usize].deforms;
        i.prev = if deforms { i.verts - sc.indexed.dynamic.0 } else { 0 };
        i.bits = i.bits & !scene::INST_DEFORMS | if deforms { scene::INST_DEFORMS } else { 0 };
        let bits = sc.gpu_instances[r].n[1][3].to_bits();
        sc.gpu_instances[r].n[1][3] = f32::from_bits(bits & !scene::INST_DEFORMS | if deforms { scene::INST_DEFORMS } else { 0 });
        sc.gpu_instances[r].info[2] = i.verts;
        sc.gpu_instances[r].n[2][3] = f32::from_bits(i.prev);
    }
    params.word_first[1][2] = sc.indexed.dynamic.0;
    Prepared { tri_verts, groups, nodes, emitters, gpu_emitter_tris, emitter_runs, env, env_cdf, params, energy, emitter_tris, light_tree, env_nee_floor, mat_runs, moving_mats, tri_order }
}

/// The scene's triangles, in the GPU's order (`tri_verts`, `sc.attrs`), as vertices and indices (indexed.rs). The
/// still world-space triangles share their vertices among themselves, the dynamic ones have three each (the CPU
/// writes them whole), each mesh of the pool shares within itself (the meshes in parallel; a skinned one by its
/// model's corners), the translucent particles' have three each. World-space triangles count their vertices from
/// 0, a mesh's from the mesh's first. The vertices lie in that order, so those that move are one run: the dynamic
/// world-space triangles', then the deforming meshes' (which are the pool's first).
fn index_geometry(sc: &scene::Scene, order: &[u32], tri_verts: &[[f32; 4]], groups: &[usize; GROUPS], pool_first: usize, no_still: bool) -> indexed::Indexed {
    use rayon::prelude::*;
    let still: usize = groups[..4].iter().sum();
    let flat: Vec<[Vec3A; 3]> = order.iter().map(|&i| sc.tris[i as usize]).collect();
    let mut out = indexed::Indexed { meshes: vec![(0, 0); sc.meshes.len()], skins: vec![Default::default(); sc.skin_sources.len()], ..Default::default() };
    out.push(indexed::shared(&flat[..still], &sc.attrs[..still]), 0);
    let moving = out.vertices.len() as u32;
    out.push(indexed::apart(&flat[still..], &sc.attrs[still..pool_first]), moving);
    out.flat = out.vertices.len() as u32;
    // The pool, mesh by mesh as their triangles lie.
    let source_of: std::collections::HashMap<u32, u32> = sc.retained.skinned.iter().filter(|k| k.mesh != u32::MAX).map(|k| (k.mesh, k.source)).collect();
    let mut meshes: Vec<usize> = (0..sc.meshes.len()).filter(|&m| sc.meshes[m].count > 0).collect();
    meshes.sort_by_key(|&m| sc.meshes[m].first);
    // (A particle system's mesh: quads the pass writes.)
    let quads: std::collections::HashSet<u32> = sc.particles.iter().map(|p| p.mesh).chain(sc.gpu_particles.iter().map(|p| p.mesh)).collect();
    // (A skinned mesh with its source: the model's vertices are found with its first mesh.)
    let scopes: Vec<(indexed::Scope, Option<u32>)> = meshes
        .par_iter()
        .map(|&m| {
            let im = sc.meshes[m];
            let r = im.first as usize..(im.first + im.count) as usize;
            let (tris, attrs) = (&sc.pool_tris[r.clone()], &sc.attrs[pool_first + r.start..pool_first + r.end]);
            match source_of.get(&(m as u32)).copied().filter(|&s| im.deforms && sc.skin_sources[s as usize].len() == tris.len()) {
                Some(s) => (indexed::shared_skinned(&sc.skin_sources[s as usize], tris, attrs), Some(s)),
                None if quads.contains(&(m as u32)) => (indexed::quads(tris, attrs), None),
                None => (indexed::shared(tris, attrs), None),
            }
        })
        .collect();
    let mut deforming = 0;
    for (&m, (s, source)) in meshes.iter().zip(scopes) {
        if let Some(k) = source.filter(|&k| out.skins[k as usize].1.is_empty()) {
            out.skins[k as usize] = (indexed::source_vertices(&sc.skin_sources[k as usize], &s), s.indices.clone());
        }
        // (A mesh's triangles follow the mesh's before it: the pool has no triangle that is no mesh's.)
        assert_eq!(out.indices.len(), pool_first + sc.meshes[m].first as usize, "the pool's triangles are its meshes', one after another");
        let n = s.vertices.len() as u32;
        out.meshes[m] = (out.push(s, 0), n);
        if sc.meshes[m].deforms {
            assert_eq!(out.meshes[m].0, moving + (pool_first - still) as u32 * 3 + deforming, "the deforming meshes are the pool's first");
            deforming += n;
        }
    }
    out.dynamic = (moving, (pool_first - still) as u32 * 3 + deforming);
    assert_eq!(out.indices.len(), pool_first + sc.pool_tris.len());
    // The triangle of a still tree over nothing: without area.
    if no_still {
        out.indices.push([0; 3]);
    }
    assert_eq!((out.indices.len(), tri_verts.len()), (sc.attrs.len(), sc.attrs.len() * 3), "a row of indices a triangle");
    // A row a triangle. One no instance places: its material, its object, its flags, its emitter + 1. A mesh's: its
    // section and its index in it (whether it deforms is its instance's to say: `scene::INST_DEFORMS`).
    let pool = pool_first..pool_first + sc.pool_tris.len();
    out.rows = sc.attrs.iter().enumerate().map(|(k, a)| if pool.contains(&k) { [a.uv[3].to_bits(), 0, 0, a.x[3].to_bits()] } else { [a.uv[3].to_bits(), a.x[1].to_bits(), tri_verts[k * 3][3].to_bits(), a.x[3].to_bits()] }).collect();
    out.pool_first = pool_first as u32;
    out
}

/// Instances as world-space triangles (software traversal), appended to the flattened ones with their objects.
/// Software traversal's units (units.rs): a tree for each instanced mesh that keeps its shape, its triangles (and
/// what goes with them: attributes, vertex colours) put in the tree's order. None: no units (a mesh no tree can be
/// made of: a vertex that is not a number); the caller flattens every instance then.
fn unit_trees(sc: &mut scene::Scene) -> Option<Vec<Option<units::MeshTree>>> {
    // A mesh that deforms (a posed skinned model's) is a unit as any: its tree has the shape of the tree over its
    // model's bind pose, one shape for every mesh skinned from the source, and its boxes are the pose's (taken again
    // at each new one: `units::refit`; a tree made again a pose costs sixty times that, and traces 5 % faster).
    // The source's corners are put in that tree's order, and so are each such mesh's triangles: the skinning pass
    // writes a triangle where its corners are.
    let source_of: std::collections::HashMap<u32, u32> = sc.retained.skinned.iter().map(|k| (k.mesh, k.source)).collect();
    let mut shapes: Vec<Option<(Vec<u32>, usize)>> = vec![None; sc.skin_sources.len()];
    sc.skin_trees = vec![vec![]; sc.skin_sources.len()];
    for s in 0..sc.skin_sources.len() {
        if !source_of.values().any(|&x| x as usize == s) {
            continue;
        }
        let rest = &sc.skin_rest[s];
        let bind: Vec<[Vec3A; 3]> = sc.skin_sources[s].iter().map(|c| c.map(|c| Vec3A::from(skin::skin_corner(&c, rest).0))).collect();
        match units::mesh_tree(&bind) {
            Ok(t) => {
                sc.skin_sources[s] = t.order.iter().map(|&i| sc.skin_sources[s][i as usize]).collect();
                sc.skin_trees[s] = t.nodes;
                shapes[s] = Some((t.order, t.depth));
            }
            Err(e) => {
                eprintln!("units: {e}: the scene's instances are flattened");
                return None;
            }
        }
    }
    let mut trees = Vec::with_capacity(sc.meshes.len());
    for (k, m) in sc.meshes.iter().enumerate() {
        if m.count == 0 {
            trees.push(None);
            continue;
        }
        if m.deforms {
            // (One no source skins: flattened, `flatten_deforming`.)
            let shape = source_of.get(&(k as u32)).and_then(|&s| shapes[s as usize].as_ref().map(|x| (s as usize, x)));
            trees.push(shape.filter(|(_, (order, _))| order.len() == m.count as usize).map(|(s, (order, depth))| {
                let mut nodes = sc.skin_trees[s].clone();
                let (lo, hi) = units::refit(&mut nodes, |i| sc.pool_tris[m.first as usize + order[i] as usize]);
                units::MeshTree { nodes, order: order.clone(), depth: *depth, lo, hi }
            }));
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
        let mut attrs: Vec<_> = t.order.iter().map(|&i| sc.pool_attrs[r.start + i as usize]).collect();
        scene::rank_in_sections(&mut attrs);
        sc.pool_attrs[r.clone()].copy_from_slice(&attrs);
        if !sc.pool_vcolors.is_empty() {
            let colors: Vec<_> = t.order.iter().map(|&i| sc.pool_vcolors[r.start + i as usize]).collect();
            sc.pool_vcolors[r].copy_from_slice(&colors);
        }
    }
    // A skinned model's list of its mesh's triangles (the model's triangle, its section, its index in it) as they lie
    // now: one list a source, as before.
    let mut lists: Vec<Option<std::sync::Arc<Vec<(u32, u32, u32)>>>> = vec![None; shapes.len()];
    for k in &mut sc.retained.skinned {
        let (Some((order, _)), Some(Some(_))) = (&shapes[k.source as usize], trees.get(k.mesh as usize)) else { continue };
        let first = sc.meshes[k.mesh as usize].first as usize;
        let list = lists[k.source as usize].get_or_insert_with(|| {
            std::sync::Arc::new(order.iter().enumerate().map(|(at, &i)| (k.order[i as usize].0, sc.pool_attrs[first + at].uv[3].to_bits(), sc.pool_attrs[first + at].x[3].to_bits())).collect())
        });
        k.order = list.clone();
    }
    Some(trees)
}

/// With units: the instances of meshes that deform and have no tree (none skins them from a source) are still
/// written out as world-space triangles, as every instance was. They keep their place among the instances (edits
/// name instances by index) and are no unit.
fn flatten_deforming(sc: &mut scene::Scene, trees: &[Option<units::MeshTree>]) {
    for k in 0..sc.instances.len() {
        let inst = sc.instances[k].clone();
        let m = sc.meshes[inst.mesh as usize];
        if inst.hidden || !m.deforms || trees.get(inst.mesh as usize).is_some_and(|t| t.is_some()) {
            continue;
        }
        let first = sc.tris.len() as u32;
        for k in m.first..m.first + m.count {
            let own = inst.resolved(&sc.pool_attrs[k as usize], sc.rows.get(inst.rows as usize..).unwrap_or(&[]));
            let (t, a, c) = inst.flat(&sc.pool_tris[k as usize], &own, sc.pool_vcolors.get(k as usize).copied());
            if let Some(c) = c {
                scene::put_colors(&mut sc.vcolors, sc.tris.len(), Some(vec![c]));
            }
            sc.tris.push(t);
            sc.tri_flags.push(scene::instance_tri_flags(&inst, &own, &sc.materials));
            sc.attrs.push(a);
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
            let own = inst.resolved(&sc.pool_attrs[k as usize], sc.rows.get(inst.rows as usize..).unwrap_or(&[]));
            let (t, a, c) = inst.flat(&sc.pool_tris[k as usize], &own, sc.pool_vcolors.get(k as usize).copied());
            if let Some(c) = c {
                scene::put_colors(&mut sc.vcolors, sc.tris.len(), Some(vec![c]));
            }
            sc.tris.push(t);
            sc.tri_flags.push(scene::instance_tri_flags(&inst, &own, &sc.materials));
            sc.attrs.push(a);
        }
        // A second range for the node when a part of its mesh was flattened at build: node_tris joins them.
        sc.tri_nodes.push((inst.name, first, sc.tris.len() as u32));
        if let Some(i) = inst.sub {
            sc.tri_instances.push((first, sc.tris.len() as u32, i));
        }
    }
    sc.pool_tris.clear();
    sc.pool_attrs.clear();
    sc.meshes.clear();
    sc.sections.clear();
    sc.rows.clear();
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

/// A sky as the kernel takes it: the environment map (1 x 1 for a uniform sky or none), a uniform sky's radiance,
/// whether the map is one to sample, and the illuminance the atmosphere leaves its sun's light.
pub struct SkyBaked {
    pub env: gpu::EnvMap,
    pub color: Vec3,
    pub has_env: bool,
    pub sun: Option<Vec3>,
}

/// Bakes a scene's sky (`cloud_cover`: an atmosphere's; `altitude`: the camera's, m). The last atmosphere and the last
/// gradient are kept: edits that leave the sky's inputs as they were reuse the bake.
pub fn bake_sky(sky: &scene::SkyData, cloud_cover: f32, altitude: f32) -> SkyBaked {
    match sky {
        scene::SkyData::Atmosphere { sun_dir, sun_lux, rayleigh, mie, g: mie_g, ground, dynamic, glow, .. } => {
            static LAST: std::sync::Mutex<Option<(Vec<u32>, std::sync::Arc<sky::SkyBake>)>> = std::sync::Mutex::new(None);
            let alt = altitude;
            let (bw, bh) = if *dynamic { sky::DYNAMIC_SKY } else { (1024, 512) };
            let key: Vec<u32> = [*rayleigh, *mie, *mie_g, ground.x, ground.y, ground.z, sun_dir.x, sun_dir.y, sun_dir.z, sun_lux.x, sun_lux.y, sun_lux.z, alt, glow.uniform.x, glow.uniform.y, glow.uniform.z, glow.horizon.x, glow.horizon.y, glow.horizon.z, glow.height, bw as f32, cloud_cover].iter().map(|f| f.to_bits()).collect();
            let mut last = LAST.lock().unwrap();
            let bake = match last.as_ref() {
                Some((k, b)) if *k == key => b.clone(),
                _ => {
                    let atm = sky::Atmosphere::earth(*rayleigh, *mie, *mie_g, *ground);
                    let b = std::sync::Arc::new(if *dynamic || !glow.is_zero() || cloud_cover > 0.0 {
                        sky::bake_atmosphere_tables(&atm, &sky::SkyTables::new(&atm), *sun_dir, *sun_lux, alt, bw, bh, *glow, cloud_cover)
                    } else {
                        sky::bake_atmosphere(&atm, *sun_dir, *sun_lux, alt, bw, bh)
                    });
                    *last = Some((key, b.clone()));
                    b
                }
            };
            SkyBaked { env: gpu::EnvMap { width: bake.width, height: bake.height, texels: bake.texels.clone() }, color: Vec3::ZERO, has_env: true, sun: Some(bake.sun_illuminance) }
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
            SkyBaked { env: gpu::EnvMap { width: bake.width, height: bake.height, texels: bake.texels.clone() }, color: Vec3::ZERO, has_env: true, sun: None }
        }
        scene::SkyData::Color(c) => SkyBaked { env: gpu::EnvMap { width: 1, height: 1, texels: vec![[c.x, c.y, c.z, 1.0]] }, color: *c, has_env: false, sun: None },
        scene::SkyData::None => SkyBaked { env: gpu::EnvMap { width: 1, height: 1, texels: vec![[0.0; 4]] }, color: Vec3::ZERO, has_env: false, sun: None },
    }
}

impl SkyBaked {
    /// The kernel's parameters for it: whether it is a map and its size, a uniform sky's colour, whether it is
    /// light-sampled (`floor`: `Prepared::env_nee_floor`).
    pub fn write(&self, p: &mut gpu::Params, floor: f32) {
        p.env = [if self.has_env { 1.0 } else { 0.0 }, self.env.width as f32, self.env.height as f32, p.env[3]];
        p.sky_color = [self.color.x, self.color.y, self.color.z, 0.0];
        p.counts2[1] = sky_sampled(sky_brightest(&self.env, self.color), floor, self.has_env) as u32;
    }
}

/// The brightest luminance a sky has (its map's texels, its uniform colour).
pub fn sky_brightest(env: &gpu::EnvMap, color: Vec3) -> f32 {
    env.texels.iter().map(|t| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).fold(0.0f32, f32::max).max(0.2126 * color.x + 0.7152 * color.y + 0.0722 * color.z)
}

/// Whether the sky is light-sampled (`Params::counts2[1]`): not when it is negligible next to the emitters (`floor`:
/// `Prepared::env_nee_floor`), nor a baked sky with THREERT_ENV_NEE off. A uniform sky reads no table: always.
pub fn sky_sampled(brightest: f32, floor: f32, has_env: bool) -> bool {
    brightest >= floor && (!has_env || env_nee_on())
}

/// The energy table's nodes (pathtrace.wgsl `ENERGY_N`): along a row sqrt(cos(view)) from 0 to 1 (the albedo changes
/// fastest at grazing views, where a table over the cosine itself is 20 times further off), over the rows the
/// roughness from 0 to 1.
pub const ENERGY_N: usize = 32;

/// The specular lobe's directional albedo toward a view at cosine `mu`, for `alpha` = roughness^2, as the two terms of
/// Karis's split sum ("Real Shading in Unreal Engine 4", SIGGRAPH 2013 course notes, "Environment BRDF"): with
/// Schlick's Fresnel F = f0 + (f90 - f0) (1 - v.h)^5 the lobe reflects f0 x `.0` + f90 x `.1` of a uniform white
/// surround, and `.0 + .1` at F = 1 (E(mu) of Kulla & Conty 2017). The lobe is the kernel's, single scattering: GGX D
/// (Walter et al. 2007) with the square-root-free height-correlated Smith visibility (`vis_smith_joint_approx`:
/// Hammon 2017).
/// The integral is taken over visible normals (Heitz, "Sampling the GGX Distribution of Visible Normals", JCGT 2018):
/// a light direction mirrored off a visible normal has density G1(v) D / (4 n.v), so the lobe's value x cosine over
/// it is F x (V 4 n.v n.l) / G1(v), bounded. Midpoint rule over the unit disk the normals are drawn from, `nr` rings
/// x 2 `nphi` angles (half of them computed: the lobe is symmetric about the plane of incidence). The rings crowd
/// toward the rim (u = 1 - (1 - t)^2), where the normals tilt far and the reflected ray nears the horizon: the
/// approximate visibility falls to 0 there over a range of n.l about alpha wide, which even rings miss (48 x 16
/// points: 1.6e-4 from the converged value; 24 x 24 even ones: 9e-3).
pub fn specular_albedo(mu: f64, alpha: f64, nr: usize, nphi: usize) -> (f64, f64) {
    let mu = mu.clamp(1e-4, 1.0);
    if alpha < 1e-6 {
        // A mirror: the half vector is the normal.
        let fc = (1.0 - mu).powi(5);
        return (1.0 - fc, fc);
    }
    // The view direction (in the x-z plane, n = +z) in the space where the visible normals' hemisphere is a half ball
    // (Heitz 2018, section 3); the basis around it is t1 = +y, t2 = (-vh.z, 0, vh.x).
    let (vx, vz) = ((1.0 - mu * mu).max(0.0).sqrt(), mu);
    let k = (alpha * alpha * vx * vx + vz * vz).sqrt();
    let (hx, hz) = (alpha * vx / k, vz / k);
    let s = 0.5 * (1.0 + hz);
    // 1 / G1(v) = 1 + Lambda(v) (Heitz 2014, "Understanding the Masking-Shadowing Function", GGX's Lambda).
    let inv_g1 = 0.5 * (1.0 + (1.0 + alpha * alpha * (1.0 - mu * mu) / (mu * mu)).sqrt());
    let (mut a, mut b) = (0.0, 0.0);
    for i in 0..nr {
        let t = (i as f64 + 0.5) / nr as f64;
        let (r, w) = ((1.0 - (1.0 - t) * (1.0 - t)).sqrt(), 2.0 * (1.0 - t));
        for j in 0..nphi {
            let phi = std::f64::consts::PI * ((j as f64 + 0.5) / nphi as f64 - 0.5);
            let p1 = r * phi.cos();
            let p2 = (1.0 - s) * (1.0 - p1 * p1).max(0.0).sqrt() + s * r * phi.sin();
            let pz = (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
            // The visible normal, stretched back.
            let m = [alpha * (pz * hx - p2 * hz), alpha * p1, (p2 * hx + pz * hz).max(0.0)];
            let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
            let voh = (vx * m[0] + vz * m[2]) / len;
            let nol = 2.0 * voh * m[2] / len - vz;
            if nol <= 0.0 || voh <= 0.0 {
                continue;
            }
            // V x 4 n.v n.l, V = 0.5 / (n.l (n.v (1 - alpha) + alpha) + n.v (n.l (1 - alpha) + alpha)): the kernel's
            // 0.5 / mix(2 n.l n.v, n.l + n.v, alpha).
            let g2 = 2.0 * mu * nol / (nol * (mu * (1.0 - alpha) + alpha) + mu * (nol * (1.0 - alpha) + alpha));
            let fc = (1.0 - voh).powi(5);
            a += w * (1.0 - fc) * g2;
            b += w * fc * g2;
        }
    }
    let k = inv_g1 / (nr * nphi) as f64;
    (a * k, b * k)
}

/// The table pathtrace.wgsl's `energy_lookup` reads (`heap::Table::Energy`): `specular_albedo` at `ENERGY_N` x
/// `ENERGY_N` nodes, node (ix, iy) at cos(view) (ix / 31)^2 and roughness iy / 31, row after row, two nodes (four
/// numbers) a vec4. Computed once a process (about 10 ms).
pub fn energy_lut() -> Vec<[f32; 4]> {
    static LUT: std::sync::OnceLock<Vec<[f32; 4]>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let n = ENERGY_N;
        let mut vals = Vec::with_capacity(2 * n * n);
        for iy in 0..n {
            let r = iy as f64 / (n - 1) as f64;
            for ix in 0..n {
                let x = ix as f64 / (n - 1) as f64;
                let (a, b) = specular_albedo(x * x, r * r, 48, 8);
                vals.extend([a as f32, b as f32]);
            }
        }
        vals.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
    })
    .clone()
}

#[cfg(test)]
mod energy_tests {
    use super::*;

    /// The kernel's `energy_lookup`: the table between its nodes.
    fn lookup(lut: &[[f32; 4]], roughness: f64, mu: f64) -> (f64, f64) {
        let n = ENERGY_N;
        let node = |i: usize| (lut[i / 2][2 * (i % 2)] as f64, lut[i / 2][2 * (i % 2) + 1] as f64);
        let (x, y) = (mu.clamp(0.0, 1.0).sqrt() * (n - 1) as f64, roughness.clamp(0.0, 1.0) * (n - 1) as f64);
        let (x0, y0) = ((x as usize).min(n - 2), (y as usize).min(n - 2));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let mix = |p: (f64, f64), q: (f64, f64), t: f64| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
        let i = y0 * n + x0;
        mix(mix(node(i), node(i + 1), fx), mix(node(i + n), node(i + n + 1), fx), fy)
    }

    /// The lobe's value x cosine integrated over the hemisphere of light directions at F = 1 (`n` x 2 `n` points,
    /// midpoint rule over theta and half the azimuth): what the kernel's light samples sum. The table's estimator is
    /// its sampling.
    fn quadrature(mu: f64, alpha: f64, n: usize) -> f64 {
        let a2 = alpha * alpha;
        let v = [(1.0 - mu * mu).sqrt(), mu];
        let mut sum = 0.0;
        for i in 0..n {
            let (st, c) = (std::f64::consts::FRAC_PI_2 * (i as f64 + 0.5) / n as f64).sin_cos();
            for j in 0..2 * n {
                let phi = std::f64::consts::PI * (j as f64 + 0.5) / (2 * n) as f64;
                let (x, z) = (st * phi.cos(), st * phi.sin());
                let h = [v[0] + x, v[1] + c, z];
                let noh = h[1] / (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
                let d = (noh * a2 - noh) * noh + 1.0;
                let vis = 0.5 / (c * (mu * (1.0 - alpha) + alpha) + mu * (c * (1.0 - alpha) + alpha));
                sum += a2 / (std::f64::consts::PI * d * d) * vis * c * st;
            }
        }
        sum * std::f64::consts::PI * std::f64::consts::PI / (2 * n * n) as f64
    }

    #[test]
    fn the_albedo_is_the_lobes_integral() {
        for (r, mu) in [(0.3, 0.9), (0.5, 0.7071), (0.5, 0.2), (0.8, 0.5), (1.0, 1.0), (1.0, 0.1), (0.25, 0.4)] {
            let ((a, b), q) = (specular_albedo(mu, r * r, 48, 8), quadrature(mu, r * r, 800));
            eprintln!("roughness {r}, cos {mu}: {a:.4} + {b:.4} = {:.4}; quadrature {q:.4}", a + b);
            assert!((a + b - q).abs() < 1e-3, "roughness {r}, cos {mu}: {:.4} for {q:.4}", a + b);
        }
        // A mirror reflects everything, by Fresnel at the view's angle; a smooth lobe nearly so.
        let (a, b) = specular_albedo(0.5, 0.0, 48, 8);
        assert!((a + b - 1.0).abs() < 1e-12 && (b - 0.5f64.powi(5)).abs() < 1e-12);
        let (a, b) = specular_albedo(0.5, 0.02 * 0.02, 48, 8);
        assert!((a + b - 1.0).abs() < 2e-3 && (b - 0.5f64.powi(5)).abs() < 2e-3, "{a} {b}");
    }

    #[test]
    fn the_table_follows_the_albedo_between_its_nodes() {
        let t = std::time::Instant::now();
        let lut = energy_lut();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(lut.len(), 512);
        assert!(lut.iter().flatten().all(|e| (0.0..=1.0 + 1e-4).contains(e)));
        // Between the nodes, at the kernel's roughnesses (0.02 and up) and all but the last half degree of grazing
        // views (below it a smooth lobe's albedo falls from 1 to a half within a cosine of alpha: no table follows).
        let (mut worst, mut at, mut sum) = (0.0f64, (0.0, 0.0), 0.0);
        for iy in 0..48 {
            let r = 0.02 + 0.98 * (iy as f64 + 0.37) / 48.0;
            for ix in 0..48 {
                let mu = 0.01 + 0.99 * (ix as f64 + 0.61) / 48.0;
                let ((a, b), (ta, tb)) = (specular_albedo(mu, r * r, 96, 12), lookup(&lut, r, mu));
                let e = (ta - a).abs().max((tb - b).abs()).max((ta + tb - a - b).abs() / (a + b));
                sum += e / (48.0 * 48.0);
                if e > worst {
                    (worst, at) = (e, (r, mu));
                }
            }
        }
        eprintln!("energy table: {ms:.1} ms; error between nodes: mean {sum:.5}, worst {worst:.5} at roughness {:.3}, cos {:.3}", at.0, at.1);
        assert!(worst < 0.02 && sum < 0.002, "{worst} at {at:?}, mean {sum}");
        assert!(cfg!(debug_assertions) || ms < 50.0, "{ms} ms");
    }
}

/// The glass energy table's nodes (pathtrace.wgsl GLASS_E_MU, _ROUGH, _IOR): cos(view) as its square root, roughness,
/// the index of refraction as sqrt((ior - 1) / (ior + 1)) up to `GLASS_E_ZMAX` (ior 3); and the side (entering,
/// leaving).
pub const GLASS_E: [usize; 3] = [16, 16, 8];
pub const GLASS_E_ZMAX: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Unpolarised Fresnel reflectance of a dielectric interface: `c` the cosine on the incident side, `eta` = n_t / n_i;
/// 1 at total internal reflection (Walter et al. 2007, eq. 22).
fn fresnel_dielectric(c: f64, eta: f64) -> f64 {
    let g2 = eta * eta - 1.0 + c * c;
    if g2 <= 0.0 {
        return 1.0;
    }
    let g = g2.sqrt();
    let (a, b) = ((g - c) / (g + c), (c * (g + c) - 1.0) / (c * (g - c) + 1.0));
    0.5 * a * a * (1.0 + b * b)
}

/// What a rough dielectric interface (GGX, single scattering: the kernel's `dielectric_*`) returns of a uniform white
/// surround toward a direction at cosine `mu`: reflection + transmission, without the tint. `alpha` = roughness^2,
/// `eta` = n(far side) / n(view side). By `n` x `n` stratified visible normals (Heitz 2018), whose estimator is
/// bounded: G2 / G1, reflected or refracted by Fresnel at the normal.
pub fn glass_energy(mu: f64, alpha: f64, eta: f64, n: usize) -> f64 {
    let a2 = alpha * alpha;
    let lambda = |c: f64| {
        let c = c.abs().min(1.0);
        0.5 * (-1.0 + (1.0 + a2 * (1.0 - c * c) / (c * c)).sqrt())
    };
    let v = [(1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu];
    let k = (alpha * alpha * v[0] * v[0] + v[2] * v[2]).sqrt();
    let vh = [alpha * v[0] / k, 0.0, v[2] / k];
    // (t1 = (0, 1, 0), t2 = vh x t1.)
    let t2 = [-vh[2], 0.0, vh[0]];
    let (lv, s) = (lambda(mu), 0.5 * (1.0 + vh[2]));
    let mut sum = 0.0;
    for i in 0..n {
        for j in 0..n {
            let (r, phi) = (((i as f64 + 0.5) / n as f64).sqrt(), 2.0 * std::f64::consts::PI * (j as f64 + 0.5) / n as f64);
            let p1 = r * phi.cos();
            let p2 = (1.0 - s) * (1.0 - p1 * p1).max(0.0).sqrt() + s * r * phi.sin();
            let pz = (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
            let nh = [p2 * t2[0] + pz * vh[0], p1, p2 * t2[2] + pz * vh[2]];
            let m = [alpha * nh[0], alpha * nh[1], nh[2].max(0.0)];
            let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
            let m = [m[0] / len, m[1] / len, m[2] / len];
            let vm = v[0] * m[0] + v[2] * m[2];
            let f = fresnel_dielectric(vm, eta);
            // Reflection: the kernel's joint visibility (vis_smith_joint_approx x 4 nov nol) over G1(v).
            let nol = 2.0 * vm * m[2] - v[2];
            if nol > 0.0 {
                let g2 = 2.0 * mu * nol / (nol * (mu * (1.0 - alpha) + alpha) + mu * (nol * (1.0 - alpha) + alpha)).max(1e-7);
                sum += f * g2 * (1.0 + lv);
            }
            // Refraction (eq. 40): height-correlated Smith over G1(v).
            let c2 = 1.0 - (1.0 - vm * vm) / (eta * eta);
            if f < 1.0 && c2 > 0.0 {
                let lz = (vm / eta - c2.sqrt()) * m[2] - v[2] / eta;
                if lz < 0.0 {
                    sum += (1.0 - f) * (1.0 + lv) / (1.0 + lv + lambda(lz));
                }
            }
        }
    }
    sum / (n * n) as f64
}

/// `glass_energy` over the table's nodes, entering then leaving, four values a vec4 (`heap::Table::GlassEnergy`; once
/// a process, 35 ms).
pub fn glass_energy_lut() -> &'static [[f32; 4]] {
    static LUT: std::sync::OnceLock<Vec<[f32; 4]>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let [nm, nr, nz] = GLASS_E;
        let mut vals = Vec::with_capacity(2 * nz * nr * nm);
        for side in 0..2 {
            for iz in 0..nz {
                let z = (iz as f64 / (nz - 1) as f64 * GLASS_E_ZMAX).max(0.02);
                let ior = (1.0 + z * z) / (1.0 - z * z);
                for ir in 0..nr {
                    let r = (ir as f64 / (nr - 1) as f64).max(0.02);
                    for im in 0..nm {
                        let y = im as f64 / (nm - 1) as f64;
                        vals.push(glass_energy((y * y).max(1e-3), r * r, if side == 0 { ior } else { 1.0 / ior }, 24).clamp(0.05, 1.0) as f32);
                    }
                }
            }
        }
        vals.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
    })
}

#[cfg(test)]
mod glass_energy_tests {
    use super::*;

    fn lambda(a2: f64, c: f64) -> f64 {
        let c = c.abs().min(1.0);
        0.5 * (-1.0 + (1.0 + a2 * (1.0 - c * c) / (c * c)).sqrt())
    }

    /// The kernel's `dielectric_t`: the interface's transmission x |n.l| (Walter et al. 2007 eq. 21 from the view
    /// side; n = +y, v above, l below).
    fn transmitted(v: [f64; 3], l: [f64; 3], a2: f64, eta: f64) -> f64 {
        let h = [v[0] + eta * l[0], v[1] + eta * l[1], v[2] + eta * l[2]];
        let k = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt() * h[1].signum();
        let h = [h[0] / k, h[1] / k, h[2] / k];
        let (voh, loh) = (v[0] * h[0] + v[1] * h[1] + v[2] * h[2], l[0] * h[0] + l[1] * h[1] + l[2] * h[2]);
        if voh <= 0.0 || loh >= 0.0 {
            return 0.0;
        }
        let d = (h[1] * a2 - h[1]) * h[1] + 1.0;
        let den = voh + eta * loh;
        (1.0 - fresnel_dielectric(voh, eta)) * a2 / (std::f64::consts::PI * d * d) * voh * eta * eta * -loh / (v[1] * den * den * (1.0 + lambda(a2, v[1]) + lambda(a2, l[1])))
    }

    /// (R, T) of one interface toward a view at cosine `mu`: the BSDF's value x cosine integrated over the directions
    /// of each side (`n` x 2 `n` points). What the kernel's light samples sum; the table's estimator is its sampling.
    fn interface(mu: f64, alpha: f64, eta: f64, n: usize) -> (f64, f64) {
        let a2 = alpha * alpha;
        let v = [(1.0 - mu * mu).sqrt(), mu, 0.0];
        let (mut r, mut t) = (0.0, 0.0);
        for i in 0..n {
            let c = (i as f64 + 0.5) / n as f64;
            let st = (1.0 - c * c).sqrt();
            for j in 0..2 * n {
                let phi = std::f64::consts::PI * (j as f64 + 0.5) / n as f64;
                let (x, z) = (st * phi.cos(), st * phi.sin());
                // Reflection: D x the kernel's joint visibility x Fresnel at the half vector.
                let h = [v[0] + x, v[1] + c, z];
                let k = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
                let (noh, voh) = (h[1] / k, (v[0] * h[0] + v[1] * h[1]) / k);
                let d = (noh * a2 - noh) * noh + 1.0;
                let vis = 0.5 / (c * (mu * (1.0 - alpha) + alpha) + mu * (c * (1.0 - alpha) + alpha)).max(1e-7);
                r += a2 / (std::f64::consts::PI * d * d) * vis * fresnel_dielectric(voh, eta) * c;
                t += transmitted(v, [x, -c, z], a2, eta);
            }
        }
        let k = std::f64::consts::PI / (n * n) as f64;
        (r * k, t * k)
    }

    /// The table's estimator against the BSDF's integral; the numbers the `glass/rough-*` render tests expect; the
    /// table's cost.
    #[test]
    fn the_glass_energy_is_the_interfaces() {
        for (r, eta, mu) in [(0.3, 1.5, 0.7071), (0.3, 1.0 / 1.5, 0.7071), (0.6, 1.5, 0.4), (0.6, 1.0 / 1.5, 0.9), (1.0, 1.5, 0.9), (1.0, 1.0 / 1.5, 0.9), (0.5, 1.33, 0.2), (0.8, 1.0 / 1.33, 0.5)] {
            let (e, (qr, qt)) = (glass_energy(mu, r * r, eta, 24), interface(mu, r * r, eta, 700));
            assert!((e - qr - qt).abs() < 0.012 * (qr + qt), "roughness {r}, eta {eta:.3}, cos {mu}: {e:.4} for {:.4}", qr + qt);
        }
        // tests/render/glass-rough-enter.ts, -leave.ts, -leave-low.ts (roughness 0.6, ior 1.5).
        let c45 = std::f64::consts::FRAC_1_SQRT_2;
        for (eta, mu, want) in [(1.5, c45, 0.9573), (1.0 / 1.5, c45, 0.7390), (1.0 / 1.5, 0.4, 0.6474)] {
            let ((qr, qt), e) = (interface(mu, 0.36, eta, 1000), glass_energy(mu, 0.36, eta, 192));
            eprintln!("eta {eta:.3}, cos {mu:.4}: R {qr:.4} + T {qt:.4} = {:.4}; by visible normals {e:.4}", qr + qt);
            assert!((qr + qt - want).abs() < 1e-3 && (e - want).abs() < 5e-4, "eta {eta:.3}, cos {mu}: R {qr:.4} + T {qt:.4}, {e:.4} for {want}");
        }
        let t = std::time::Instant::now();
        let lut = glass_energy_lut();
        assert_eq!(lut.len() * 4, 2 * GLASS_E[0] * GLASS_E[1] * GLASS_E[2]);
        assert!(lut.iter().flatten().all(|e| (0.05..=1.0).contains(e)));
        eprintln!("glass energy table: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    }

    /// tests/render/glass-rough-lamp.ts and -wide.ts: what a square emitter of side `side`, 1 m under a sheet of
    /// glass (roughness 0.6, ior 1.5) where the 45-degree view's refracted ray arrives, sends through the sheet, as
    /// a fraction of its radiance: `transmitted` integrated over the emitter's area.
    #[test]
    fn the_lamp_through_frosted_glass() {
        let (a2, eta) = (0.6f64.powi(4), 1.5f64);
        let c45 = std::f64::consts::FRAC_1_SQRT_2;
        let v = [0.0, c45, c45];
        // The refracted ray's foot 1 m down (the test's lamp is centred there: z = -0.5345).
        let st = c45 / eta;
        let cz = -st / (1.0 - st * st).sqrt();
        assert!((cz + 0.5345).abs() < 1e-4);
        for (side, want) in [(0.3, 0.40415), (1.0, 0.83641)] {
            let n = 600;
            let mut sum = 0.0;
            for i in 0..n {
                let x = ((i as f64 + 0.5) / n as f64 - 0.5) * side;
                for j in 0..n {
                    let z = cz + ((j as f64 + 0.5) / n as f64 - 0.5) * side;
                    let d2 = x * x + 1.0 + z * z;
                    let d = d2.sqrt();
                    sum += transmitted(v, [x / d, -1.0 / d, z / d], a2, eta) / (d * d2);
                }
            }
            let got = sum * (side / n as f64).powi(2);
            assert!((got - want).abs() < 2e-4, "a {side} m lamp: {got:.5} for {want}");
        }
    }

    /// Whether the table's nodes are enough: the kernel's lookup (pathtrace.wgsl `glass_energy`, trilinear over the
    /// nodes; copied here) against the estimator between them.
    #[test]
    fn the_glass_energy_table_interpolates() {
        let lut = glass_energy_lut();
        let at = |i: usize| lut[i / 4][i % 4] as f64;
        let [nm, nr, nz] = GLASS_E;
        let look = |mu: f64, r: f64, ior: f64, leaving: bool| {
            let x = (mu.sqrt() * (nm - 1) as f64).clamp(0.0, (nm - 1) as f64);
            let y = (r * (nr - 1) as f64).clamp(0.0, (nr - 1) as f64);
            let z = (((ior - 1.0).max(0.0) / (ior + 1.0)).sqrt() * (nz - 1) as f64 / GLASS_E_ZMAX).clamp(0.0, (nz - 1) as f64);
            let (x0, y0, z0) = (x as usize, y as usize, z as usize);
            let (x1, y1, z1) = ((x0 + 1).min(nm - 1), (y0 + 1).min(nr - 1), (z0 + 1).min(nz - 1));
            let (fx, fy, fz) = (x - x0 as f64, y - y0 as f64, z - z0 as f64);
            let plane = |zi: usize| {
                let b = (if leaving { nz } else { 0 } + zi) * nr * nm;
                let row = |yi: usize| at(b + yi * nm + x0) * (1.0 - fx) + at(b + yi * nm + x1) * fx;
                row(y0) * (1.0 - fy) + row(y1) * fy
            };
            plane(z0) * (1.0 - fz) + plane(z1) * fz
        };
        let (mut worst, mut sum, mut n) = ((0.0f64, String::new()), 0.0, 0);
        for leaving in [false, true] {
            for ior in [1.2, 1.33, 1.5, 1.8, 2.4] {
                for ri in 0..12 {
                    let r = 0.08 + ri as f64 * 0.08;
                    for mi in 0..24 {
                        let mu = 0.02 + mi as f64 * 0.0408;
                        let (t, e) = (look(mu, r, ior, leaving), glass_energy(mu, r * r, if leaving { 1.0 / ior } else { ior }, 48));
                        let d = (t - e).abs() / e;
                        sum += d;
                        n += 1;
                        if d > worst.0 {
                            worst = (d, format!("ior {ior} r {r:.2} mu {mu:.3} leaving {leaving}: table {t:.4}, direct {e:.4}"));
                        }
                    }
                }
            }
        }
        eprintln!("glass energy table against the estimator: mean {:.4}, worst {:.4} ({})", sum / n as f64, worst.0, worst.1);
        assert!(sum / (n as f64) < 0.01 && worst.0 < 0.05, "mean {:.4}, worst {:.4} ({})", sum / n as f64, worst.0, worst.1);
    }
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
            .map(|i| EmitterTri { tri: i, mat: 0, obj: (i % 6 != 0) as u32, area: 0.5, c: [2.0 * i as f32 + 1.0 / 3.0, 1.0 / 3.0, 0.0], moves: false, nominal: false, in_tree: false, inst: NO_INST, part: 0, card: false })
            .collect();
        let verts: Vec<[f32; 4]> = (0..600).flat_map(|i| [[2.0 * i as f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]).collect();
        let (off, on) = ([[0.0, 0.0, 0.0, 1.0], [0.0; 4]], [[0.0, 0.0, 0.0, 1.0]; 2]);
        let rows = |i: usize| ltree::tri_vertices(&verts[i * 3..i * 3 + 3]);
        assert!(ltree::build(&tris, &rows, &|i| emitter_power(&tris[i], &mats, &off, 1)).is_none());
        let mut t = light_tree_of(&tris, &rows, &mats, &off, 1, false).unwrap();
        assert_eq!(t.order.len(), 600);
        assert!((t.power - 100.0 * 0.5).abs() < 1e-3, "{}", t.power);
        let loaded = light_tree_of(&tris, &rows, &mats, &on, 1, false).unwrap();
        let plain = ltree::build(&tris, &rows, &|i| emitter_power(&tris[i], &mats, &on, 1)).unwrap();
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
            .map(|i| EmitterTri { tri: i, mat: if i % 9 == 0 { 0 } else { 1 + i % 3 }, obj: 0, area: 10f32.powf(-6.0 * unit()), c: [300.0 * unit(), 5.0, 100.0 * unit()], moves: i % 50 == 0, nominal: false, in_tree: false, inst: NO_INST, part: 0, card: false })
            .collect();
        let (t, total, _) = emitter_cdf(&tris, &emitter_order(&tris, None), &mats, &[], 3, [20.0, 0.0, 30.0, 10.0], EmitterCull::default());
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
        let table = light_groups(&sorted);
        let nd = 1;
        assert_eq!(table.len(), (sorted.len() - nd).div_ceil(LIGHT_GROUP));
        for (i, l) in sorted.iter().enumerate().skip(nd) {
            let g = table[(i - nd) / LIGHT_GROUP];
            let d = Vec3::new(g[0], g[1], g[2]).distance(Vec3::from_slice(&l.pos[..3]));
            assert!(d + light_reach(l) <= g[3], "light {i} reaches out of its group's sphere");
        }
        // Spatially compact: the groups' mean radius well under that of the same classes in their original order.
        let mut plain: Vec<usize> = (0..lights.len()).collect();
        plain.sort_by_key(|&i| light_class(&lights[i]));
        let unsorted = light_groups(&plain.iter().map(|&i| lights[i]).collect::<Vec<_>>());
        let mean = |t: &[[f32; 4]]| {
            let r: Vec<f32> = t.iter().map(|g| g[3]).filter(|r| *r < f32::MAX).collect();
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
        let lit = light_groups(&near)[0];
        near[3] = at(-10_000.0, false);
        let with_black = light_groups(&near)[0];
        assert!(with_black[3] <= lit[3] && with_black[3] > 0.0, "{with_black:?} for {lit:?}");
        // A group of black lights alone: radius -1.
        let dark: Vec<_> = (0..LIGHT_GROUP + 2).map(|k| at(k as f32, k < LIGHT_GROUP)).collect();
        let t = light_groups(&dark);
        assert!(t[0][3] > 0.0);
        assert_eq!(t[1][3], -1.0);
    }
}

#[cfg(test)]
mod units_tests {
    use super::*;
    use crate::units::cpu::{Kernel, INF, TRACE_CLOSEST, TRACE_SHADOW};
    use crate::world::{Geometry, Kind, World};
    use glam::{Mat4, Quat};

    /// A world with a ground and a light's card (an instance each, as every mesh: units under software traversal, and
    /// no still triangle), meshes turned and scaled, one that casts no shadow, one hidden, and an instanced mesh of
    /// five instances of which four are drawn.
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
            let card = serde_json::from_value(serde_json::json!({ "type": "rect", "color": [1, 1, 1], "castShadow": true, "intensity": 1, "width": 3, "height": 3, "range": 0 })).unwrap();
            w.set_object(16, "card".into(), None, Kind::Light(card), true, false).unwrap();
            w.set_transform(16, Vec3::new(1.0, 2.5, -1.0), Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), Vec3::ONE).unwrap();
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

    /// `prepare`'s vertices and indices (indexed.rs) are its triangles: every triangle read through its indices (a
    /// mesh's from the mesh's first vertex, a world-space one's from 0) has the corners, the edges, the normals and
    /// the UVs the kernel's arrays have for it, bit for bit, its corners' tangents (zeros: it has none), and its row's
    /// four words. Flattened, with units, and as hardware traversal has the scene; with two posed skinned models. A mesh's
    /// vertices are fewer than its triangles' corners (the ring: 82 for 80 triangles), and the vertices that move are
    /// one run. No GPU.
    #[test]
    fn the_indexed_geometry_is_the_scenes_triangles() {
        let mut w = world(true);
        for (id, x) in [(30, -0.7), (31, 0.7)] {
            let pose = crate::desc::Animation { clip: "bend".into(), time: 0.25 * (id - 29) as f32, r#loop: true, nodes: Default::default() };
            w.set_object(id, format!("arm{id}"), None, Kind::Model { uri: "tests/render/data/skin-blend.glb".into(), nodes: None, animation: Some(pose), functions: vec![] }, true, false).unwrap();
            w.set_transform(id, Vec3::new(x, -1.0, 0.8), Quat::IDENTITY, Vec3::splat(2.0)).unwrap();
        }
        for (what, hwrt, units) in [("flattened", false, false), ("units", false, true), ("hardware traversal", true, false)] {
            let mut sc = scene::build(&w, std::path::Path::new(".."), &default_presets_dir(), None).unwrap();
            let p = prepare_as(hwrt, units, &mut sc, &mut vec![]);
            let x = &sc.indexed;
            assert_eq!((x.indices.len(), x.rows.len()), (sc.attrs.len(), sc.attrs.len()), "{what}");
            // (A pool triangle's mesh: its vertices count from the mesh's first.)
            let pool_first = sc.attrs.len() - sc.pool_tris.len() - usize::from(sc.units.is_some() && sc.tris.is_empty());
            let base = |k: usize| sc.meshes.iter().zip(&x.meshes).find(|(m, _)| m.count > 0 && (pool_first + m.first as usize..pool_first + (m.first + m.count) as usize).contains(&k)).map_or(0, |(_, v)| v.0);
            let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
            for (k, a) in sc.attrs.iter().enumerate() {
                let (q, n, uv, tangent) = x.triangle(k, base(k));
                let v = |r: usize| [p.tri_verts[k * 3 + r][0], p.tri_verts[k * 3 + r][1], p.tri_verts[k * 3 + r][2]];
                let edge = |c: usize| [q[c][0] - q[0][0], q[c][1] - q[0][1], q[c][2] - q[0][2]];
                // (The triangle of a still tree over nothing, after the pool: without area either way.)
                if sc.units.is_some() && sc.tris.is_empty() && k + 1 == sc.attrs.len() {
                    assert_eq!((edge(1), edge(2), v(1), v(2)), ([0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3]), "{what}: the still tree's own triangle has no area");
                    continue;
                }
                assert_eq!((bits(&q[0]), bits(&edge(1)), bits(&edge(2))), (bits(&v(0)), bits(&v(1)), bits(&v(2))), "{what}: triangle {k}'s first corner and its edges");
                let skinned_without_normals = n.iter().flatten().all(|c| *c == 0.0);
                if !skinned_without_normals {
                    assert_eq!([bits(&n[0]), bits(&n[1]), bits(&n[2])], [bits(&a.n0[..3]), bits(&a.n1[..3]), bits(&a.n2[..3])], "{what}: triangle {k}'s normals");
                }
                assert_eq!(bits(&uv.concat()), bits(&[a.n0[3], a.n1[3], a.n2[3], a.uv[0], a.uv[1], a.uv[2]]), "{what}: triangle {k}'s UVs");
                assert_eq!(tangent.map(|t| bits(&t)), [a.t, a.t1, a.t2].map(|t| bits(&t)), "{what}: triangle {k}'s tangents, a corner each, or none");
                // (A mesh's triangle: its section and its index in it, nothing else. One no instance places: its
                // material, its object, its flags, its emitter + 1.)
                let pool = (pool_first..pool_first + sc.pool_tris.len()).contains(&k);
                let row = if pool { [a.uv[3].to_bits(), 0, 0, a.x[3].to_bits()] } else { [a.uv[3].to_bits(), a.x[1].to_bits(), p.tri_verts[k * 3][3].to_bits(), a.x[3].to_bits()] };
                assert_eq!(x.rows[k], row, "{what}: triangle {k}'s row");
                // Software traversal: the triangle as its tree's leaf has it, with its ids (a mesh's: its section and
                // its index there; a world-space one's: its flags).
                if !hwrt {
                    let ids = if pool { [row[0], row[3]] } else { [row[2], 0] };
                    let corners = q.map(|c| Vec3A::new(c[0], c[1], c[2]));
                    assert_eq!(x.leaves[k].map(|r| bits(&r)), indexed::leaf(corners, ids).map(|r| bits(&r)), "{what}: triangle {k}'s leaf");
                }
            }
            assert_eq!(x.leaves.len(), if hwrt { 0 } else { x.indices.len() }, "{what}: a leaf a triangle under software traversal, none under hardware traversal");
            if sc.meshes.is_empty() {
                continue;
            }
            // The ring's mesh: 80 triangles of 82 vertices. Each skinned model's mesh: its own vertices, as many as
            // the other's (one model), fewer than its corners; and they are the vertices that move.
            assert!(sc.attrs.iter().any(|a| a.t[3] != 0.0) && sc.attrs.iter().any(|a| a.t[3] == 0.0), "{what}: triangles with tangents and without");
            let ring = sc.meshes.iter().zip(&x.meshes).find(|(m, _)| m.count == 80).expect("the ring's mesh");
            assert_eq!(ring.1 .1, 82, "{what}: the ring's vertices");
            let skinned: Vec<(u32, u32)> = sc.meshes.iter().zip(&x.meshes).filter(|(m, _)| m.deforms).map(|(_, v)| *v).collect();
            assert_eq!(skinned.len(), 2, "{what}: two skinned meshes");
            let corners = sc.meshes.iter().find(|m| m.deforms).unwrap().count * 3;
            assert!(skinned[0].1 == skinned[1].1 && skinned[0].1 < corners && skinned[0].1 >= corners / 6, "{what}: {} vertices for {corners} corners", skinned[0].1);
            assert_eq!((x.dynamic.0, x.dynamic.1), (skinned[0].0.min(skinned[1].0), skinned[0].1 * 2), "{what}: the vertices that move are the skinned meshes'");
        }
    }

    /// An opaque particle system is a mesh of its own and one instance of it: two triangles and four vertices a
    /// particle (a quad's corners once each, in the order the pass writes them), among the vertices that move; the
    /// instance places nothing and has the system's first object. One the lights are not sampled by has a row that
    /// keeps its material's emission out of the light list; one they are sampled by has an emitter a triangle, its
    /// instance's, weighed by half the particle's nominal area. Stateless and stateful systems alike. A translucent
    /// system's instance is hidden from the scene's acceleration structure (the translucency pass has it). An
    /// opaque particle system's material is not one whose emitters an edit makes in place. Under software traversal (the passes do
    /// not run there) no kernel asks for a particle's object. No GPU.
    #[test]
    fn a_particle_system_is_a_mesh_of_quads_and_an_instance() {
        let mut w = world(true);
        w.set_material(7, serde_json::from_value(serde_json::json!({ "name": "spark", "color": [0, 0, 0], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 0.5, 0.2], "emissiveIntensity": 20, "doubleSided": true })).unwrap());
        w.set_particle_fn("rise", "fn pf_rise(i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(i.pivot + vec3<f32>(f32(i.index), i.time, 0.0), vec3<f32>(0.1, 0.0, 0.0), vec3<f32>(0.0, 0.1, 0.0), true, vec4<f32>(0.0)); }".into(), String::new());
        for (id, count, emitter_area) in [(40, 3, 0.0), (41, 2, 0.09)] {
            w.set_object(id, format!("sparks{id}"), None, Kind::Particles { count, emitter: "rise".into(), params: vec![], material: 7, emitter_area }, true, false).unwrap();
            w.set_transform(id, Vec3::new(0.3, 0.2, 1.0), Quat::IDENTITY, Vec3::ONE).unwrap();
        }
        // (A translucent system, of a transparent material.)
        w.set_material(8, serde_json::from_value(serde_json::json!({ "name": "smoke", "color": [0.5, 0.5, 0.5], "opacity": 0.5, "transparent": true, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })).unwrap());
        w.set_object(44, "smoke".into(), None, Kind::Particles { count: 2, emitter: "rise".into(), params: vec![], material: 8, emitter_area: 0.0 }, true, false).unwrap();
        // (Stateful systems too: one of each kind.)
        w.set_particle_program("puff", "fn pe_puff(i: EIn, p: array<vec4<f32>, 8>) -> PState { return PState(i.pivot, vec3<f32>(0.0), 0.0, 1.0, vec4<f32>(1.0), 0.1); }".into(), "fn pu_puff(s: PState, i: UIn, p: array<vec4<f32>, 8>) -> PState { return s; }".into(), String::new());
        for (id, capacity, emitter_area) in [(42, 5, 0.0), (43, 1, 0.04)] {
            w.set_object(id, format!("puffs{id}"), None, Kind::GpuParticles { capacity, program: "puff".into(), params: vec![], material: 7, emitter_area, rate: 1.0, collide: Default::default() }, true, false).unwrap();
        }
        for (what, hwrt, units) in [("units", false, true), ("hardware traversal", true, false)] {
            let mut sc = scene::build(&w, std::path::Path::new(".."), &default_presets_dir(), None).unwrap();
            let p = prepare_as(hwrt, units, &mut sc, &mut vec![]);
            let (own, sampled) = (&sc.particles[0], &sc.particles[1]);
            let (stateful, stateful_sampled) = (&sc.gpu_particles[0], &sc.gpu_particles[1]);
            assert_eq!((stateful.capacity, stateful_sampled.capacity), (5, 1), "{what}: the stateful systems, as the stateless");
            assert!(sc.instances.iter().filter(|i| i.bits & scene::INST_PARTICLES != 0).count() == 5 && sc.tri_flags.iter().all(|f| f & scene::TRI_DYNAMIC == 0), "{what}: five systems, an instance each, none a world-space triangle");
            assert!(p.moving_mats.contains(&sc.retained.materials[&7]), "{what}: the particles' material is its passes'");
            assert_eq!((own.count, sampled.count), (3, 2));
            let part = gpu::instance_features(&sc) & gpu::F_PART;
            if !hwrt {
                assert_eq!(part, 0, "{what}: no kernel asks for a particle's object");
                continue;
            }
            assert_ne!(part, 0, "{what}: the kernel asks for a particle's object");
            let (m, v) = (sc.meshes[own.mesh as usize], sc.indexed.meshes[own.mesh as usize]);
            assert_eq!((m.count, m.deforms, m.sect_count, v.1), (6, true, 1, 12), "{what}: two triangles and four vertices a particle");
            let pool_first = sc.attrs.len() - sc.pool_tris.len();
            let tris = &sc.indexed.indices[pool_first + m.first as usize..][..6];
            assert_eq!(tris, [[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7], [8, 9, 10], [8, 10, 11]], "{what}: a quad's corners once each");
            let uv = |k: u32| [sc.indexed.vertices[(v.0 + k) as usize][0][3], sc.indexed.vertices[(v.0 + k) as usize][1][3]];
            assert_eq!([uv(4), uv(5), uv(6), uv(7)], [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], "{what}: in the order the pass writes them");
            assert!(v.0 >= sc.indexed.dynamic.0 && v.0 + v.1 <= sc.indexed.dynamic.0 + sc.indexed.dynamic.1, "{what}: among the vertices that move");
            let of: Vec<&scene::Instance> = sc.instances.iter().filter(|i| i.mesh == own.mesh).collect();
            assert_eq!(of.len(), 1, "{what}: one instance");
            let i = of[0];
            assert_eq!((i.world, i.object, i.bits & scene::INST_PARTICLES, i.prev), (glam::Mat4::IDENTITY, own.object, scene::INST_PARTICLES, v.0 - sc.indexed.dynamic.0), "{what}: it places nothing; the first particle's object; where its previous positions are");
            let row = sc.rows[i.rows as usize];
            assert_eq!((row[2], row[3]), (scene::NO_EMITTER, scene::ROW_UNSAMPLED), "{what}: its emission is not in the light list");
            assert!(p.emitter_tris.iter().all(|e| e.inst == NO_INST || sc.instances[e.inst as usize].mesh != own.mesh), "{what}: no emitter of it");
            // The stateful system: a mesh of five quads after the first's, its own instance and objects.
            let (m2, v2) = (sc.meshes[stateful.mesh as usize], sc.indexed.meshes[stateful.mesh as usize]);
            assert_eq!((m2.count, m2.deforms, v2.1), (10, true, 20), "{what}: the stateful system's quads");
            let i2 = sc.instances.iter().find(|i| i.mesh == stateful.mesh).expect("its instance");
            assert_eq!((i2.object, i2.bits & scene::INST_PARTICLES), (stateful.object, scene::INST_PARTICLES), "{what}: its first particle's object");
            assert!(stateful.object >= own.object + 3 || own.object >= stateful.object + 5, "{what}: the systems' objects apart");
            // The ones the lights are sampled by: an emitter a triangle, the instance's, from its row's first on, in
            // the triangles' order, weighed by half the particle's nominal area.
            for (mesh, count, area) in [(sampled.mesh, 2u32, 0.045f32), (stateful_sampled.mesh, 1, 0.02)] {
                let r = sc.instances.iter().position(|i| i.mesh == mesh).expect("its instance");
                let (i, m) = (&sc.instances[r], sc.meshes[mesh as usize]);
                let row = sc.rows[i.rows as usize];
                assert_eq!((row[2], row[3], i.bits & scene::INST_PARTICLES), (i.first_emitter, 0, scene::INST_PARTICLES), "{what}: its row has its first emitter");
                let mine = &p.emitter_tris[row[2] as usize..][..(count * 2) as usize];
                for (k, e) in mine.iter().enumerate() {
                    assert_eq!((e.inst, e.part, e.tri, e.nominal, e.moves, e.area, e.obj), (r as u32, 0, (pool_first as u32 + m.first) + k as u32, true, true, area, i.object), "{what}: emitter {k} of the system");
                }
                assert_eq!(p.emitter_runs.particles.iter().filter(|run| run[0] <= row[2] && row[2] + count * 2 <= run[0] + run[1]).count(), 1, "{what}: its emitters are taken again from its vertices every frame");
            }
            assert_eq!(p.emitter_tris.iter().filter(|e| e.nominal).count(), 6, "{what}: no other emitter by a nominal area");
            // The translucent system: a mesh and an instance too, the instance out of the scene's acceleration
            // structure (the translucency pass has it); no world-space translucent triangle; its material is the
            // translucency pass's, and one an edit may change in place.
            let smoke = &sc.particles[2];
            let i3 = sc.instances.iter().find(|i| i.mesh == smoke.mesh).expect("its instance");
            assert!(smoke.translucent && i3.hidden && i3.bits & scene::INST_PARTICLES != 0 && sc.rows[i3.rows as usize][3] == scene::ROW_UNSAMPLED, "{what}: the translucent system's instance");
            assert_eq!((sc.meshes[smoke.mesh as usize].count, i3.object), (4, smoke.object), "{what}: its quads are its mesh's");
            assert_eq!((sc.translucent_mats(), p.moving_mats.contains(&sc.retained.materials[&8])), (vec![sc.retained.materials[&8]], false), "{what}: its material");
            assert!(sc.instances.iter().filter(|i| i.bits & scene::INST_PARTICLES != 0 && i.mesh != smoke.mesh).all(|i| !i.hidden), "{what}: the opaque systems' instances are the scene's");
        }
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
        // (The ground and the light's card come together: `world`.)
        let ground = 2 * usize::from(w.objects.contains_key(&10));
        assert_eq!(su.tris.len(), 0, "no still triangle: the card is an instance too");
        let u = su.units.as_ref().expect("the scene has units");
        assert!(sf.units.is_none() && sf.instances.is_empty(), "flattened: no instance is left");
        // Ten instances, the ground and the card: four meshes shown, one hidden (an instance no ray sees), five of the
        // instanced mesh of which one is hidden: eight units in the top tree, the ground and the card.
        assert_eq!((su.instances.len(), u.count as usize), (10 + ground, 8 + ground), "{:?}", su.instances.iter().map(|i| (i.name.clone(), i.hidden)).collect::<Vec<_>>());
        assert!(gpu::instance_features(&su) & gpu::F_UNITS != 0 && gpu::instance_features(&sf) & gpu::F_UNITS == 0);
        assert_eq!((pu.params.motion[2].to_bits(), pu.params.motion[3].to_bits() as usize), (u.layout.top_first, 8 + ground));
        // The flattened scene holds every instance's triangles; the scene with units holds each mesh's once.
        assert!(pf.tri_verts.len() > pu.tri_verts.len());
        assert_eq!(pu.nodes.len(), u.layout.top_first as usize + u.layout.top_cap as usize);
        let kernel = |p: &Prepared, sc: &scene::Scene| {
            let (top, units) = (p.params.motion[2].to_bits(), p.params.motion[3].to_bits());
            (p.nodes.clone(), sc.indexed.leaves.clone(), sc.gpu_instances.clone(), top, units, sc.rows.clone())
        };
        let (ku, kf) = (kernel(&pu, &su), kernel(&pf, &sf));
        #[allow(clippy::type_complexity)]
        let walk = |k: &(Vec<bvh::GpuNode>, Vec<indexed::Leaf>, Vec<scene::GpuInstance>, u32, u32, Vec<scene::Row>), o: Vec3A, d: Vec3A, mode: u32| {
            let instances = if k.2.is_empty() { vec![scene::GpuInstance::default()] } else { k.2.clone() };
            let kernel = Kernel { nodes: &k.0, leaves: &k.1, instances: &instances, rows: &k.5, top: k.3, units: k.4, steps: Default::default(), budget: 1 << 20, deepest: Default::default(), reads: Default::default() };
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

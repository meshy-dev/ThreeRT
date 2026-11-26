//! A scene loaded for the live view (`load_scene`, `LoadedScene`), and the worker threads its loading and its
//! dynamic sky and emitters use (`DynamicSky`, `EmitterWorker`).

use super::*;

pub(super) struct LoadedScene {
    /// Load phases (parse, bvh, sky, accel build, upload) in ms.
    pub(super) timings: Vec<(String, f64)>,
    /// The meshes' triangles (model space) and their placements, kept on the CPU for picking and selection outlines.
    /// (A mesh the GPU skins has its build's pose here, but for its triangles that emit, which a new pose places:
    /// `repose_skinned`. `LiveCore::mesh_tris` has the mesh as it is now.)
    pub(super) pool_tris: Vec<[glam::Vec3A; 3]>,
    /// The vertices that move, the first and how many (`indexed::Indexed::dynamic`); by skin source, the model's
    /// vertices its meshes are skinned from and their triangles' indices (`Indexed::skins`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) dynamic_verts: (u32, u32),
    pub(super) skins: Vec<(Vec<crate::skin::SkinCorner>, Vec<[u32; 3]>)>,
    /// Software traversal with units: the shape of each skin source's meshes' trees (`scene::Scene::skin_trees`): a
    /// new pose takes a tree's boxes again from its vertices as posed (live/edits.rs `repose_skinned`).
    pub(super) skin_trees: Vec<Vec<crate::bvh::GpuNode>>,
    /// Software traversal: each pool triangle's section and its index in it (a unit's tree orders its mesh's
    /// triangles; live/edits.rs `mesh_places`). Empty under hardware traversal, where they lie by section.
    pub(super) pool_place: Vec<[u32; 2]>,
    pub(super) meshes: Vec<scene::InstMesh>,
    pub(super) instances: Vec<scene::Instance>,
    /// The meshes' sections and the instances' rows as the GPU has them (`scene::Scene::sections`, `rows`); what an
    /// edit places takes its rows from the room (`EditState::take_rows`).
    pub(super) sections: Vec<scene::Section>,
    pub(super) rows: Vec<scene::Row>,
    /// The instances' kernel records as uploaded (movable ones are rewritten when they move), then the procedural
    /// objects' (from `proc_first`).
    pub(super) gpu_instances: Vec<scene::GpuInstance>,
    /// Procedural geometry (moves, picking): the record `proc_first` + j is `procs[j]`'s, as record i is
    /// `instances[i]`'s. Both hold what edits place too, at its record (hidden fillers at the records between).
    pub(super) procs: Vec<scene::ProcObject>,
    pub(super) proc_first: usize,
    /// Some object's move is an instance's transform (`Retained::moves_instances`: kept where the proxies change,
    /// here and in a geometry edit): the frame's kernel then writes motion vectors.
    pub(super) moving_instances: bool,
    /// Kernel features of the scene's structure (gpu::instance_features), which edits cannot change.
    pub(super) instance_features: u32,
    pub(super) name: String,
    pub(super) triangles: usize,
    pub(super) up: gpu::Uploaded,
    pub(super) base: gpu::Params,
    /// GPU table copies that parameter edits rewrite in place (`commit`).
    pub(super) tables: tables::Tables,
    /// Kernel feature inputs of the scene as a whole: whether it has textures, whether it has material functions,
    pub(super) fixed_features: (bool, bool),
    /// The scene's material functions (name, code, helpers), each at its number (`Retained::shaders`): the build's
    /// by name, one that came while the scene runs after them (`set_functions`).
    pub(super) shaders: Vec<(String, String, String)>,
    /// The materials of translucent particles' triangles: their functions are the translucency pass's.
    pub(super) translucent_mats: Vec<u32>,
    /// The functions of those materials the scene's kernel was made with (`LiveCore::remake_functions`).
    pub(super) translucent_used: std::collections::BTreeSet<i32>,
    /// The scene's volume functions (name, code, helpers), each at its number: the build's by name, one that came
    /// while the scene runs after them (`set_volume_functions`).
    pub(super) volume_fns: Vec<(String, String, String)>,
    /// Where in-place geometry edits write next (`edit_geometry`); a rebuild starts it afresh.
    pub(super) edit: EditState,
    /// Power edits' emitter rebuilds on a worker (`EmitterWorker`), for scenes with many emitters.
    pub(super) emit: Option<EmitterWorker>,
    /// The light list's path codes as the GPU has them (`GpuEmitterTri::code`, by emitter): the upload's, then what
    /// `write_emitters` wrote, which writes only those that changed since.
    pub(super) codes_sent: Vec<u32>,
}

/// A planet's viewer altitude, when the sky has one (`desc::Sky::Atmosphere::altitude`).
pub(super) fn sky_altitude(sky: &scene::SkyData) -> Option<f32> {
    match sky {
        scene::SkyData::Atmosphere { altitude, .. } => *altitude,
        _ => None,
    }
}

/// Cores the sky's worker thread bakes with: the rest stay with the frame (scripts, uploads, the GPU's driver).
pub(super) const SKY_THREADS: usize = 3;

/// An atmosphere whose sun moves while the scene runs (time of day). The sun's light is rewritten in place at every
/// move, its direction and its colour through the atmosphere (`sky::sun_illuminance`, no bake needed), which carries
/// the direct light and the shadows. The sky's radiance (the environment map and its sampling CDF) is baked again on a
/// worker thread with `SKY_THREADS` cores, the newest sun first, and lands with a later frame (`take_sky`): the sky
/// trails the sun by one bake (~10–40 ms at `sky::DYNAMIC_SKY`). Lockstep runs wait for it, so they repeat.
pub(super) struct DynamicSky {
    pub(super) atm: Arc<crate::sky::Atmosphere>,
    pub(super) ask: mpsc::Sender<SkyAsk>,
    pub(super) done: mpsc::Receiver<SkyBaked>,
}

/// A bake asked of the worker: the sun's direction, its illuminance above the atmosphere, the cloud cover, the
/// viewer's altitude (m: a planet's, `SkyAtmosphere.altitude`, or the camera's when the sky was built).
pub(super) type SkyAsk = (glam::Vec3, glam::Vec3, f32, f32);

/// What the worker baked: the environment map, its CDF, its brightest texel, the limb's row (`sky::SkyBake::horizon_v`).
pub(super) type SkyBaked = (gpu::EnvMap, Vec<f32>, f32, f32);

impl DynamicSky {
    pub(super) fn new(world: &World, (w, h): (u32, u32)) -> Option<DynamicSky> {
        let crate::desc::Sky::Atmosphere { rayleigh_scale, mie_scale, mie_anisotropy, ground_albedo, glow, horizon_glow, sun: Some(_), altitude, .. } = &world.environment.sky else {
            return None;
        };
        let horizon = if altitude.is_some() { crate::sky::Horizon::Planet } else { crate::sky::Horizon::Level };
        let atm = Arc::new(crate::sky::Atmosphere::earth(*rayleigh_scale, *mie_scale, *mie_anisotropy, glam::Vec3::from(*ground_albedo)));
        let glow = crate::sky::glow_of(glow, horizon_glow);
        let (ask, asked) = mpsc::channel::<SkyAsk>();
        let (tx, done) = mpsc::channel();
        let a = atm.clone();
        std::thread::Builder::new()
            .name("sky bake".into())
            .spawn(move || {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(SKY_THREADS).build().ok();
                let tables = match &pool {
                    Some(p) => p.install(|| crate::sky::SkyTables::new(&a)),
                    None => crate::sky::SkyTables::new(&a),
                };
                let bake = |sun: SkyAsk| {
                    let b = crate::sky::bake_atmosphere_tables(&a, &tables, sun.0, sun.1, sun.3, w, h, glow, sun.2, horizon);
                    let env = gpu::EnvMap { width: b.width, height: b.height, texels: b.texels };
                    let cdf = crate::build_env_cdf(&env);
                    let max = env.texels.iter().map(|t| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).fold(0.0f32, f32::max);
                    (env, cdf, max, b.horizon_v)
                };
                // The newest sun only: bakes asked for while one ran are stale.
                while let Ok(mut sun) = asked.recv() {
                    while let Ok(s) = asked.try_recv() {
                        sun = s;
                    }
                    let out = match &pool {
                        Some(p) => p.install(|| bake(sun)),
                        None => bake(sun),
                    };
                    if tx.send(out).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(DynamicSky { atm, ask, done })
    }
}

/// Scenes with at least this many emitters rebuild their emitter CDF for power edits on a worker (`EmitterWorker`).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) const ASYNC_EMITTERS: usize = 100_000;

/// Power edits in a scene with many emitters (a city's lamps and windows switching with the clock): the emitter CDF and
/// the light tree's node powers are rebuilt on a worker thread (~13 ms over 1.4 M emitters, which held up the frame in
/// `write_emitters`), and the edits land with them, in a later frame (`take_emitters`): the objects' powers, the CDF
/// and the tree's powers together, so sampling and shading switch at once. The worker writes the CDF and the tree's
/// nodes into a mapped staging buffer; the frame only copies it on the GPU (a 22 MB `write_buffer` cost it ~5 ms).
/// Lockstep frames wait for it, so they repeat. Scenes whose emitters change in place (added in place: the tree's slot;
/// moving emitters, whose nodes the frame refits) keep `write_emitters`; so does any other emitter edit (materials),
/// which first lands the edits held back.
pub(super) struct EmitterWorker {
    pub(super) ask: mpsc::Sender<EmitterJob>,
    pub(super) done: mpsc::Receiver<EmitterBuilt>,
    /// The emitters it was made for: a list that grew (added in place) makes another.
    pub(super) count: usize,
    /// The edits of the job it is on (they land with its result).
    pub(super) flight: Option<Vec<(usize, f32, f32)>>,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) struct EmitterJob {
    pub(super) materials: Vec<scene::GpuMaterial>,
    pub(super) objects: Vec<[f32; 4]>,
    pub(super) lod: [f32; 4],
    pub(super) cull: crate::EmitterCull,
}

pub(super) struct EmitterBuilt {
    /// The CDF then the tree's nodes, as the emitters buffer holds them (`put_emitters`), and their bytes.
    pub(super) staging: wgpu::Buffer,
    pub(super) bytes: u64,
    /// The CDF's entries (its guide table included).
    pub(super) entries: usize,
    pub(super) total: f32,
    pub(super) power: f32,
    /// The light tree's node powers (`nodes[i][2]`), for the frame's copy of the tree.
    pub(super) node_powers: Vec<f32>,
}

impl EmitterWorker {
    /// wasm32: none, and power edits rebuild in the frame as before (the browser's WebGPU device and buffers stay on
    /// the thread that made them).
    #[cfg(target_arch = "wasm32")]
    pub(super) fn new(_e: &tables::Tables, _device: &wgpu::Device) -> Option<EmitterWorker> {
        None
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn new(e: &tables::Tables, device: &wgpu::Device) -> Option<EmitterWorker> {
        // (Nor where an instance's emitters change in place, an instanced object's copies: the worker weighs its own
        // copy of the list.)
        let changing = e.light_tree.as_ref().is_some_and(|t| t.dynamic.is_some() || t.moving_count > 0) || e.emitter_tris.iter().any(|t| t.moves && t.inst != crate::NO_INST);
        if e.emitter_tris.len() < ASYNC_EMITTERS || changing {
            return None;
        }
        let device = device.clone();
        let (tris, mut tree, stride) = (e.emitter_tris.clone(), e.light_tree.clone(), e.stride);
        let order = crate::emitter_order(&tris, tree.as_ref());
        let (ask, asked) = mpsc::channel::<EmitterJob>();
        let (tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("emitters".into())
            .spawn(move || {
                while let Ok(j) = asked.recv() {
                    let (em, total, power) = crate::emitter_cdf(&tris, &order, &j.materials, &j.objects, stride, j.lod, j.cull);
                    let node_powers = tree.as_mut().map_or(vec![], |t| {
                        let w: Vec<f32> = t.order.iter().map(|&i| crate::emitter_power(&tris[i as usize], &j.materials, &j.objects, stride)).collect();
                        t.refit(&|k| w[k]);
                        t.nodes.iter().map(|n| n[2]).collect()
                    });
                    let nodes: &[[f32; 4]] = tree.as_ref().map_or(&[], |t| &t.nodes);
                    let bytes = ((em.len() + nodes.len()) * 16) as u64;
                    let staging = device.create_buffer(&wgpu::BufferDescriptor { label: Some("emitters staging"), size: bytes.max(16), usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: true });
                    let at = em.len() as u64 * 16;
                    for (range, data) in [(0..at, bytemuck::cast_slice::<_, u8>(&em)), (at..bytes, bytemuck::cast_slice(nodes))] {
                        if !data.is_empty() {
                            let Ok(mut m) = staging.slice(range).get_mapped_range_mut() else { break };
                            m.copy_from_slice(data);
                        }
                    }
                    staging.unmap();
                    if tx.send(EmitterBuilt { staging, bytes, entries: em.len(), total, power, node_powers }).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(EmitterWorker { ask, done, count: e.emitter_tris.len(), flight: None })
    }
}

pub(super) fn load_scene(g: &gpu::Gpu, world: &World, assets: &Path, presets: &Path, w: u32, h: u32, targets: &Targets) -> Result<(LoadedScene, ViewState, Environment)> {
    let t0 = Instant::now();
    let mut sc = scene::build(world, assets, presets, None)?;
    sc.width = w;
    sc.height = h;
    // Emitters its edits add get a light-tree subtree of their own (ROADMAP § 6 step 4).
    // (Software traversal with units edits in place as hardware traversal does: `units_wanted`.)
    sc.emitter_slot = gpu::EditRoom::editable(&sc, g.hwrt || crate::units_wanted(g.hwrt, &sc));
    let mut timings = vec![("build_ms".to_string(), t0.elapsed().as_secs_f64() * 1000.0)];
    let mut prep = crate::prepare(g, &mut sc, &mut timings);
    let mut tables = tables::Tables::new(&sc, std::mem::take(&mut prep.emitter_tris), prep.light_tree.take());
    // (THREERT_EDIT_REBUILD=1: every edit rebuilds, a material that starts emitting too: the A/B.)
    tables.emit_in_place = std::env::var("THREERT_EDIT_REBUILD").map_or(true, |v| v != "1");
    tables.moving_mats = std::mem::take(&mut prep.moving_mats);
    let emit = EmitterWorker::new(&tables, &g.device);
    tables.defer_powers = emit.is_some();
    let fixed_features = (!sc.textures.is_empty(), !sc.shaders.is_empty());
    let shaders = sc.shaders.clone();
    let volume_fns = sc.volume_fns.clone();
    let translucent_mats = sc.translucent_mats();
    let translucent_used = translucent_mats.iter().filter_map(|&m| sc.materials.get(m as usize)).map(|m| m.tex1[3] - 1).filter(|&f| f >= 0).collect();
    // (The light tree is the tables' by now: its nodes from there.)
    // (The codes the upload writes with the list: `write_emitters` sends those that change after.)
    let codes_sent: Vec<u32> = prep.gpu_emitter_tris.iter().map(|t| t.code).collect();
    let up = g.upload(&sc, &targets.accum, bytemuck::cast_slice(&prep.nodes), &prep.emitters, &prep.gpu_emitter_tris, &prep.emitter_runs, crate::ltree::nodes_of(tables.light_tree.as_ref()), &prep.env, &prep.env_cdf, &prep.energy, &mut timings)?;
    let _ = g.pipeline(&up.kernel, true); // compile the scene's kernel variant now, not in the first frame
    let _ = g.kernel_pass(&up.kernel, "first_hit");
    if !g.merged {
        let _ = g.kernel_pass(&up.kernel, "finish");
    }
    // (And the media passes where the scene has a medium, shown or hidden: the frame that first shows one, an idle
    // fireball's first fire, would otherwise wait for them.)
    if up.kernel.features & gpu::F_MEDIA != 0 {
        let _ = g.kernel_pass(&up.kernel, "media_train");
        let _ = g.kernel_pass(&up.kernel, "media_composite");
    }
    let env = Environment {
        sky: SkyState { map: (prep.env.width, prep.env.height), altitude: sky_altitude(&sc.sky).unwrap_or(sc.camera.position.y.max(0.0)), floor: prep.env_nee_floor, cover: sc.environment.cloud_cover, worker: None },
        settings: sc.environment.clone(),
    };
    let view = ViewState::new(&sc.camera, Arc::new(sc.preset.clone()));
    let mut base = prep.params;
    // The live view's own count of light-tree candidates at camera hits (ltree.rs `RIS_LIVE`).
    if base.ltree[3] > 0.5 {
        base.ltree[3] = crate::ltree::live_candidates() as f32;
    }
    base.ltile[3] = super::LIST_CANDIDATES;
    base.cam_fwd[3] = w as f32 / h as f32;
    let instance_features = gpu::instance_features(&sc);
    let pool_place: Vec<[u32; 2]> = if up.units.is_some() { sc.pool_attrs.iter().map(|a| [a.uv[3].to_bits(), a.x[3].to_bits()]).collect() } else { vec![] };
    let (pool_tris, meshes, instances) = (std::mem::take(&mut sc.pool_tris), std::mem::take(&mut sc.meshes), std::mem::take(&mut sc.instances));
    let skin_trees = if up.units.is_some() { std::mem::take(&mut sc.skin_trees) } else { vec![] };
    // (The vertices and the indices themselves are the GPU's now.)
    let ix = std::mem::take(&mut sc.indexed);
    let (vertices, dynamic_verts, skins) = (ix.vertices.len(), ix.dynamic, ix.skins);
    let (sections, rows) = (std::mem::take(&mut sc.sections), std::mem::take(&mut sc.rows));
    let mut gpu_instances = std::mem::take(&mut sc.gpu_instances);
    let proc_first = gpu_instances.len();
    gpu_instances.extend_from_slice(&sc.proc_records);
    let procs = std::mem::take(&mut sc.procs);
    // (A proxy's index in `procs` is the build's: nothing between the build and here may take one out.)
    assert!(tables.retained.proxies.iter().all(|(id, p)| !matches!(p.draw, scene::Draw::Procedural(j) if procs.get(j as usize).map(|q| q.node) != Some(*id))), "a procedural object's proxy names another's place");
    let triangles = sc.stats.get("triangles").map_or(0, |t| *t as usize);
    let blobs = sc.proc_layout.as_ref().map_or_else(Default::default, |l| l.blobs.iter().map(|(b, off)| (b.hash, l.head.len() + off)).collect());
    // (A record's TLAS instance is at it: model instances, then procedural objects, then the room's.)
    let room_base = (prep.tri_verts.len() / 3, gpu_instances.len(), tables.objects.len() / tables.stride, up.region_words);
    let room = up.room;
    let mut users: std::collections::HashMap<u32, u32> = Default::default();
    for i in &instances {
        *users.entry(i.mesh).or_default() += 1;
    }
    let managed = tables.retained.own_mesh.values().copied().collect();
    let edit = EditState {
        tris: Holes::new(room_base.0, room_base.0 + room.tris),
        records: Holes::new(room_base.1, room_base.1 + room.records),
        objects: Holes::new(room_base.2, room_base.2 + room.objects),
        word: room_base.3,
        word_cap: room_base.3 + room.proc_words,
        users,
        managed,
        dead_meshes: vec![],
        grows: 0,
        rebind: false,
        base: room_base,
        rows: Holes::new(rows.len(), rows.len() + room.records),
        row_runs: Default::default(),
        blobs,
        // (The previous positions at the build: the vertices that move. No room: it grows with the first deforming
        // mesh an edit places.)
        prev: Holes::new(dynamic_verts.1 as usize, dynamic_verts.1 as usize),
        // (Three vertices a triangle of the room: `Gpu::upload`.)
        verts: Holes::new(vertices, vertices + room.tris * 3),
        free_skinned: vec![],
    };
    Ok((LoadedScene { timings, triangles, pool_tris, dynamic_verts, skins, skin_trees, pool_place, meshes, instances, sections, rows, gpu_instances, procs, proc_first, moving_instances: tables.retained.moves_instances(), instance_features, name: sc.name.clone(), up, base, tables, fixed_features, shaders, translucent_mats, translucent_used, volume_fns, edit, emit, codes_sent }, view, env))
}

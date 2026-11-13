//! A scene loaded for the live view (`load_scene`, `LoadedScene`), and the worker threads its loading and its
//! dynamic sky and emitters use (`DynamicSky`, `EmitterWorker`).

use super::*;

pub(super) struct LoadedScene {
    /// Load phases (parse, bvh, sky, accel build, upload) in ms.
    pub(super) timings: Vec<(String, f64)>,
    /// World-space triangles + per-node ranges, kept on the CPU for picking and selection outlines; instanced meshes
    /// (model space) and their placements likewise.
    pub(super) tris: Vec<[glam::Vec3A; 3]>,
    pub(super) tri_nodes: Vec<(String, u32, u32)>,
    /// Flattened instances of instanced objects: (first, end, instance index), for picking.
    pub(super) tri_instances: Vec<(u32, u32, u32)>,
    pub(super) pool_tris: Vec<[glam::Vec3A; 3]>,
    pub(super) meshes: Vec<scene::InstMesh>,
    pub(super) instances: Vec<scene::Instance>,
    /// The instances' kernel records as uploaded (movable ones are rewritten when they move), then the procedural
    /// objects' (from `proc_first`).
    pub(super) gpu_instances: Vec<scene::GpuInstance>,
    /// Procedural geometry (moves, picking), by node in `proc_of`; its records start at `proc_first`.
    pub(super) procs: Vec<scene::ProcObject>,
    pub(super) proc_of: std::collections::HashMap<u32, usize>,
    pub(super) proc_first: usize,
    /// The scene has instanced meshes or procedural geometry (the raster experiment draws only flattened triangles:
    /// primary visibility is traced).
    pub(super) instanced: bool,
    /// Kernel features of the instanced meshes (gpu::instance_features), which edits cannot change.
    pub(super) instance_features: u32,
    pub(super) name: String,
    pub(super) triangles: usize,
    pub(super) up: gpu::Uploaded,
    pub(super) base: gpu::Params,
    /// The live passes' bind groups, a pass each (`Targets::bind_groups`).
    pub(super) bgs: Vec<wgpu::BindGroup>,
    pub(super) exposure: f32,
    pub(super) exposure_settings: ExposureSettings,
    pub(super) tan_half_fov: f32,
    pub(super) scene_camera: Camera,
    /// The camera's depth of field (UE: off unless it sets a focus distance).
    pub(super) dof: Option<crate::desc::Dof>,
    /// GPU table copies that parameter edits rewrite in place (`commit`).
    pub(super) tables: tables::Tables,
    /// CPU copy of the dynamic triangles (posed models: the tail of the GPU triangle order from `dynamic_first`):
    /// vertices (kernel layout), shading attributes, BLAS vertices.
    pub(super) dynamic: (usize, Vec<[f32; 4]>, Vec<scene::GpuTriAttr>, Vec<[f32; 3]>),
    /// Kernel feature inputs edits cannot change: textures, material functions, traversal groups.
    pub(super) fixed_features: (bool, bool, [usize; crate::GROUPS]),
    /// Where in-place geometry edits write next (`edit_geometry`); a rebuild starts it afresh.
    pub(super) edit: EditState,
    /// The atmosphere's sky once its sun has moved in place (`move_sun` starts it), and what it bakes with: the
    /// environment map's size, the viewer's altitude and the environment's light-sampling floor, as the load had them.
    pub(super) sky: Option<DynamicSky>,
    /// Power edits' emitter rebuilds on a worker (`EmitterWorker`), for scenes with many emitters.
    pub(super) emit: Option<EmitterWorker>,
    pub(super) sky_inputs: ((u32, u32), f32, f32),
    /// For materials that start emitting (`add_material_emitters`): their flattened static triangles' runs (`prepare`)
    /// and the flattened triangles' build indices in traversal order (positions in `tris`).
    pub(super) mat_runs: std::collections::HashMap<u32, Vec<[u32; 3]>>,
    pub(super) tri_order: Vec<u32>,
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
    /// (sun direction, sun illuminance above the atmosphere, cloud cover)
    pub(super) ask: mpsc::Sender<(glam::Vec3, glam::Vec3, f32)>,
    pub(super) done: mpsc::Receiver<(gpu::EnvMap, Vec<f32>, f32)>,
    /// The brightest sky texel from which the environment is light-sampled (`Prepared::env_nee_floor`).
    pub(super) env_nee_floor: f32,
}

impl DynamicSky {
    pub(super) fn new(world: &World, (w, h): (u32, u32), altitude: f32, env_nee_floor: f32) -> Option<DynamicSky> {
        let crate::desc::Sky::Atmosphere { rayleigh_scale, mie_scale, mie_anisotropy, ground_albedo, glow, horizon_glow, sun: Some(_), .. } = &world.environment.sky else {
            return None;
        };
        let atm = Arc::new(crate::sky::Atmosphere::earth(*rayleigh_scale, *mie_scale, *mie_anisotropy, glam::Vec3::from(*ground_albedo)));
        let glow = crate::sky::glow_of(glow, horizon_glow);
        let (ask, asked) = mpsc::channel::<(glam::Vec3, glam::Vec3, f32)>();
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
                let bake = |sun: (glam::Vec3, glam::Vec3, f32)| {
                    let b = crate::sky::bake_atmosphere_tables(&a, &tables, sun.0, sun.1, altitude, w, h, glow, sun.2);
                    let env = gpu::EnvMap { width: b.width, height: b.height, texels: b.texels };
                    let cdf = crate::build_env_cdf(&env);
                    let max = env.texels.iter().map(|t| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).fold(0.0f32, f32::max);
                    (env, cdf, max)
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
        Some(DynamicSky { atm, ask, done, env_nee_floor })
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
        let changing = e.light_tree.as_ref().is_some_and(|t| t.dynamic.is_some() || t.moving_count > 0);
        if e.emitter_tris.len() < ASYNC_EMITTERS || changing {
            return None;
        }
        let device = device.clone();
        let (tris, mut tree, stride) = (e.emitter_tris.clone(), e.light_tree.clone(), e.stride);
        let (ask, asked) = mpsc::channel::<EmitterJob>();
        let (tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("emitters".into())
            .spawn(move || {
                while let Ok(j) = asked.recv() {
                    let (em, total, power) = crate::emitter_cdf(&tris, &j.materials, &j.objects, stride, j.lod, j.cull);
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

pub(super) fn load_scene(g: &gpu::Gpu, world: &World, assets: &Path, presets: &Path, w: u32, h: u32, targets: &Targets, passes: &Passes) -> Result<(LoadedScene, crate::desc::PostPreset)> {
    let t0 = Instant::now();
    let mut sc = scene::build(world, assets, presets, None)?;
    sc.width = w;
    sc.height = h;
    // Emitters its edits add get a light-tree subtree of their own (ROADMAP § 6 step 4).
    // (Software traversal with units edits in place as hardware traversal does: `units_wanted`.)
    sc.emitter_slot = gpu::EditRoom::editable(&sc, g.hwrt || crate::units_wanted(g.hwrt, &sc));
    let mut timings = vec![("build_ms".to_string(), t0.elapsed().as_secs_f64() * 1000.0)];
    let mut prep = crate::prepare_live(g, &mut sc, &mut timings);
    let mut tables = tables::Tables::new(&sc, std::mem::take(&mut prep.emitter_tris), prep.light_tree.take());
    // (THREERT_EDIT_REBUILD=1: every edit rebuilds, a material that starts emitting too: the A/B.)
    tables.emit_in_place = std::env::var("THREERT_EDIT_REBUILD").map_or(true, |v| v != "1");
    tables.fixed_mats = std::mem::take(&mut prep.fixed_mats);
    let emit = EmitterWorker::new(&tables, &g.device);
    tables.defer_powers = emit.is_some();
    let (mat_runs, tri_order) = (std::mem::take(&mut prep.mat_runs), std::mem::take(&mut prep.tri_order));
    let fixed_features = (!sc.textures.is_empty(), !sc.shaders.is_empty(), prep.groups);
    let mut up = g.upload(&sc, w, h, &prep.tri_verts, prep.groups, bytemuck::cast_slice(&prep.nodes), &prep.emitters, &prep.env, &prep.env_cdf, &prep.energy, &mut timings)?;
    // The merged layout: the kernel written for this view's group-1 buffers (merged.rs).
    if let (Some(caps), Some((_, live))) = (&mut up.kernel.caps, &targets.kernel.merged) {
        caps.live = *live;
    }
    let _ = g.pipeline(&up.kernel, true); // compile the scene's kernel variant now, not in the first frame
    let sky_inputs = ((prep.env.width, prep.env.height), sc.camera.position.y.max(0.0), prep.env_nee_floor);
    let mut base = prep.params;
    // The live viewport resamples fewer light-tree candidates at camera hits than offline renders (ltree.rs).
    if base.ltree[3] > 0.5 {
        base.ltree[3] = crate::ltree::live_candidates() as f32;
    }
    base.cam_fwd[3] = w as f32 / h as f32;
    let bgs = targets.bind_groups(&g.device, passes, &up.accum);
    let exposure_settings = ExposureSettings::new(&sc.camera.exposure, &sc.preset.post);
    // Manual: 2^-EV100. Auto: a first guess for the firefly clamp (the GPU histogram takes over from frame 1).
    let exposure = match &sc.camera.exposure {
        crate::desc::Exposure::Manual { ev100 } => 2f32.powf(-ev100),
        crate::desc::Exposure::Auto { compensation, max_ev100, .. } => 2f32.powf(*compensation) / 2f32.powf(*max_ev100),
    };
    let c = &sc.camera;
    let scene_camera = Camera { pos: c.position, fwd: c.forward, up: c.up, right: c.right, tan_half_fov: c.tan_half_fov };
    let (tris, tri_nodes, tri_instances) = (std::mem::take(&mut sc.tris), std::mem::take(&mut sc.tri_nodes), std::mem::take(&mut sc.tri_instances));
    let instanced = !sc.gpu_instances.is_empty() || !sc.procs.is_empty();
    let instance_features = gpu::instance_features(&sc);
    let (pool_tris, meshes, instances) = (std::mem::take(&mut sc.pool_tris), std::mem::take(&mut sc.meshes), std::mem::take(&mut sc.instances));
    let mut gpu_instances = std::mem::take(&mut sc.gpu_instances);
    let proc_first = gpu_instances.len();
    gpu_instances.extend_from_slice(&sc.proc_records);
    let procs = std::mem::take(&mut sc.procs);
    let proc_of = procs.iter().enumerate().filter(|(_, p)| p.sub.is_none()).map(|(j, p)| (p.node, j)).collect();
    let triangles = sc.stats.get("triangles").map_or(tris.len(), |t| *t as usize);
    let first = prep.groups[..4].iter().sum::<usize>();
    // The kernel's first dynamic triangle (motion vectors, prev_verts) is group 4's first.
    debug_assert_eq!(prep.params.group_first[1][0] as usize, first);
    // The dynamic tail: groups 4..8 (the instanced pool follows them).
    let end = prep.groups.iter().sum::<usize>();
    let blas = prep.tri_verts[first * 3..end * 3].chunks(3).flat_map(|t| [0, 1, 2].map(|k| if k == 0 { [t[0][0], t[0][1], t[0][2]] } else { [t[0][0] + t[k][0], t[0][1] + t[k][1], t[0][2] + t[k][2]] })).collect();
    let dynamic = (first, prep.tri_verts[first * 3..end * 3].to_vec(), sc.attrs[first..end].to_vec(), blas);
    let blobs = sc.proc_layout.as_ref().map_or_else(Default::default, |l| l.blobs.iter().map(|(b, off)| (b.hash, l.head.len() + off)).collect());
    let slot = up.accel.as_ref().map_or(0, |a| a.proc_base + procs.len());
    let room_base = (prep.tri_verts.len() / 3, gpu_instances.len(), tables.objects.len() / tables.stride, up.region_words, slot);
    // A record's TLAS instance is `inst_base` + it (model instances, then procedural objects, then the room's).
    debug_assert!(up.accel.as_ref().is_none_or(|a| room_base.4 == a.inst_base + room_base.1));
    let room = up.room;
    let mut users: std::collections::HashMap<u32, u32> = Default::default();
    for i in &instances {
        *users.entry(i.mesh).or_default() += 1;
    }
    let mesh_of = tables.retained.own_instance.iter().filter_map(|(&id, &k)| instances.get(k as usize).map(|i| (id, i.mesh))).collect();
    let managed = tables.retained.own_mesh.values().chain(tables.retained.emitter_mesh.values()).copied().collect();
    let edit = EditState {
        tris: Holes::new(room_base.0, room_base.0 + room.tris),
        records: Holes::new(room_base.1, room_base.1 + room.records),
        objects: Holes::new(room_base.2, room_base.2 + room.objects),
        word: room_base.3,
        word_cap: room_base.3 + room.proc_words,
        mesh_of,
        users,
        managed,
        dead_meshes: vec![],
        grows: 0,
        rebind: false,
        uniform_said: false,
        base: room_base,
        pool_first: prep.groups.iter().sum(),
        blobs,
        placed: Default::default(),
        instanced: Default::default(),
        inst_at_load: instances.len(),
        added: Default::default(),
    };
    Ok((LoadedScene { timings, triangles, tris, tri_nodes, tri_instances, pool_tris, meshes, instances, gpu_instances, procs, proc_of, proc_first, instanced, instance_features, name: sc.name.clone(), up, base, bgs, exposure, exposure_settings, tan_half_fov: c.tan_half_fov, scene_camera, dof: c.dof.clone(), tables, fixed_features, dynamic, edit, sky: None, emit, sky_inputs, mat_runs, tri_order }, sc.preset.post))
}

//! Headless wgpu backend: uploads the flattened scene once, accumulates path-traced samples, reads back HDR.

use crate::scene::Scene;
use crate::GROUPS;
use anyhow::{anyhow, Result};
use web_time::Instant;
use wgpu::util::DeviceExt;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    /// GPU timestamp queries are available (used for adaptive samples per frame in the viewer).
    pub timestamps: bool,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    /// The kernel's bounded tables are uniform buffers (`GpuOptions::uniform_tables`; merged.rs `uniform_tables`).
    pub uniform_tables: bool,
    /// The kernel's switches this device was opened with (`KernelSwitches`): they are in `source`.
    pub switches: KernelSwitches,
    /// Compiled pipelines kept between runs (`GpuOptions::pipeline_cache`), where the backend has such a cache.
    pipeline_cache: Option<crate::pipeline_cache::PipelineCache>,
    /// Kernel source (hardware or software traversal spliced in); scenes with material functions get their own module.
    source: String,
    /// The kernel as written (none in the merged layout: its sources are written for their arrays' lengths).
    module: Option<wgpu::ShaderModule>,
    /// The merged binding layout (merged.rs): a browser's, where a stage has eight storage buffers. Natively with
    /// THREERT_MERGED=1 (software traversal, the device held to eight too).
    pub merged: bool,
    /// By (material-function code hash, the features that change the source: `kernel_flags`, the merged layout's lengths).
    modules: std::sync::Mutex<std::collections::HashMap<(u64, u32, Option<crate::merged::Caps>), wgpu::ShaderModule>>,
    pipeline_layout: wgpu::PipelineLayout,
    /// Kernel variants by (feature mask, material-function code hash, LIVE): `FEAT` compiles out the features a scene
    /// does not use (they cost registers even when idle); `LIVE` = one fresh sample per dispatch for the viewport.
    pipelines: std::sync::Mutex<std::collections::HashMap<(u32, u64, bool, Option<crate::merged::Caps>), wgpu::ComputePipeline>>,
    layout: wgpu::BindGroupLayout,
    /// Group 1, the live viewport's: primary visibility (texture_2d<u32>), the radiance cache (`CacheBuffers`) and
    /// what the live passes read of a trace (`KernelBufs`). Offline renders bind `no_vis` (1x1 texture `no_vis_view`,
    /// one-entry cache).
    pub vis_layout: wgpu::BindGroupLayout,
    pub no_vis: wgpu::BindGroup,
    /// `no_vis`'s buffers (the merged layout: an offline kernel's source is written for their lengths).
    no_vis_bufs: KernelBufs,
    pub no_vis_view: wgpu::TextureView,
    /// Binding 19 of scenes without density grids (one cell), and the grid pass's bind group layout (grid_sim.wgsl).
    no_grid_view: wgpu::TextureView,
    grid_layout: wgpu::BindGroupLayout,
    /// Hardware ray queries (Metal / Vulkan RT). Software BVH traversal otherwise.
    pub hwrt: bool,
    /// Last uploaded texture pages, reused while the scene's texture set is unchanged (live edits re-upload often).
    pages: std::sync::Mutex<Option<(u64, wgpu::TextureView, wgpu::Buffer)>>,
    /// The translucency pass (hardware RT): group 2's layout (translucent.wgsl), the kernel layout with it, and its
    /// pipelines (`translucent`, `volume_train`) by the kernel's variant key.
    tl_layout: Option<wgpu::BindGroupLayout>,
    tl_pipeline_layout: Option<wgpu::PipelineLayout>,
    tl_pipelines: std::sync::Mutex<std::collections::HashMap<(u32, u64, bool), TranslucentPipelines>>,
    /// The live media passes (pathtrace.wgsl `media_train`, `media_composite`) by kernel variant and entry point.
    media_passes: std::sync::Mutex<std::collections::HashMap<(u32, u64, &'static str, Option<crate::merged::Caps>), wgpu::ComputePipeline>>,
    /// The procedural blobs of the last upload (`upload_procs`).
    proc_cache: std::sync::Mutex<Option<ProcCache>>,
    /// The last upload's procedural BLASes by geometry hash (read-only once built; another upload may share them), with
    /// the bytes Metal allocated for each (0 where unknown).
    proc_blases: std::sync::Mutex<std::collections::HashMap<[u8; 32], (wgpu::Blas, u64)>>,
}

/// Where the last upload's procedural blobs are on the GPU: a rebuild with the same geometries (`ProcLayout::key`)
/// copies them from there, GPU to GPU, instead of sending them again. Read-only: every upload owns its buffer (a scene
/// being replaced, or another room's on a render node, may still be reading the one it came from).
struct ProcCache {
    key: Vec<[u8; 32]>,
    buf: wgpu::Buffer,
    offset: u64,
    bytes: u64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub struct Params {
    pub cam_pos: [f32; 4],
    pub cam_fwd: [f32; 4],
    pub cam_right: [f32; 4],
    pub cam_up: [f32; 4],
    pub size: [u32; 4],
    pub counts: [u32; 4],
    pub env: [f32; 4],
    pub misc: [f32; 4],
    pub sky_color: [f32; 4],
    pub counts2: [u32; 4],   // directional lights (first in the light list), -, -, -
    pub time: [f32; 4],      // scene time (s), wetness, rain, -
    pub fog: [f32; 4],       // density at fog height (1/m), height falloff (1/m), fog height (m), start distance (m)
    pub fog_color: [f32; 4], // ambient inscattering radiance rgb, max opacity
    pub vol: [f32; 4],       // volumetric scattering albedo (0 = off), phase anisotropy, extinction scale in range, -
    pub vol2: [f32; 4],      // volumetric albedo tint rgb, range (m, 0 = unlimited)
    pub raster: [f32; 4],    // live primary visibility from the raster pass (1 = on), sub-pixel jitter x, y; ray tracing off (1)
    pub cache: [f32; 4],     // live radiance cache (1 = on), history cap in samples, history cap in frames, -
    pub motion: [f32; 4],    // live motion vectors: 1 = on, -, -, - (first dynamic triangle: group_first[1][0])
    /// First global triangle of each traversal group (`Prepared::groups`; group 4 = the first dynamic one). Hardware
    /// BLAS instances carry their group index, not this offset: instance custom data has only 24 bits (16.7 M).
    pub group_first: [[u32; 4]; 2],
    /// Light LOD (live; `emitter_lod`): reference position, d0 (0 = off).
    pub light_lod: [f32; 4],
    /// x: the emitter CDF's total without light LOD (camera-hit MIS weighs by power alone; = misc[1] when it is off);
    /// y: live emitter cull radius (lib.rs emitter_lod; 0 = off); z: the cull's reach inside the view cone (0 = none).
    pub emit_power: [f32; 4],
    /// Live emitter cull's view cone (lib.rs `EmitterCull`): the forward direction it was built for (xyz), the cosine
    /// of its half-angle (w).
    pub lod_view: [f32; 4],
    /// Light tree (`ltree`): its first node in `emitters` (bits), its entry count (bits: the first moving emitter's
    /// entry), the probability of picking a moving emitter (their share of power), candidates at camera hits (0 = off).
    pub ltree: [f32; 4],
    /// Procedural geometry (F_PROC; scene::proc_data): its first word in the `env_cdf` binding (after the environment's
    /// CDF), object count, first record in `instances`, -.
    pub procs: [u32; 4],
    /// The sky's aerial perspective (F_AERIAL; `scene::aerial_params`): density multiple, Rayleigh scale, Mie scale, -.
    pub aerial: [f32; 4],
    /// three.js `scene.background` (F_BACKGROUND): its radiance, 1. Last, and read only by the kernel patched for it
    /// (`kernel_flags`): every other kernel's `Params` ends before it.
    pub background: [f32; 4],
}

/// Room the live view's in-place geometry edits grow into (live/edits.rs `edit_geometry`), past the scene's own data:
/// triangles (verts, attrs, BLAS vertices), kernel records and TLAS instances, object-table entries, procedural words.
/// Only scenes with editable geometry (procedural meshes, `own_blas` meshes) under hardware RT get any: a quarter of
/// what they have, at least a minimum; `THREERT_EDIT_RESERVE` scales it (0: none). Past it an edit rebuilds the scene.
#[derive(Clone, Copy, Default, Debug)]
pub struct EditRoom {
    pub tris: usize,
    pub records: usize,
    pub objects: usize,
    pub proc_words: usize,
}

impl EditRoom {
    fn reserve() -> f64 {
        std::env::var("THREERT_EDIT_RESERVE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
    }

    /// Whether the scene gets room (`for_scene`): editable geometry under hardware RT, or under software traversal
    /// with units (`hwrt` true for them too: `lib.rs prepare`).
    pub fn editable(scene: &Scene, hwrt: bool) -> bool {
        hwrt && Self::reserve() > 0.0 && (!scene.procs.is_empty() || !scene.retained.own_instance.is_empty())
    }

    /// `units`: software traversal with units, whose room does not grow (hardware traversal's does, from nothing): a
    /// scene of theirs without editable geometry still gets records and object entries, for instances of the meshes
    /// it has (a mesh it has not rebuilds the scene: no triangles).
    pub(crate) fn for_scene(scene: &Scene, tris: usize, records: usize, hwrt: bool, units: bool) -> EditRoom {
        let k = Self::reserve();
        let (procs, meshes) = (!scene.procs.is_empty(), !scene.retained.own_instance.is_empty());
        let part = |n: usize, min: usize| ((n as f64 * 0.25 * k) as usize).max((min as f64 * k) as usize);
        if !Self::editable(scene, hwrt || units) {
            return if units && k > 0.0 {
                EditRoom { records: part(records, 1024), objects: part(scene.objects.len() / scene.object_stride(), 1024), ..EditRoom::default() }
            } else {
                EditRoom::default()
            };
        }
        let blob_words = scene.proc_layout.as_ref().map_or(0, |l| l.head.len() + l.blob_words);
        EditRoom {
            tris: if meshes { part(tris, 1 << 16) } else { 0 },
            records: part(records, 1024),
            objects: part(scene.objects.len() / scene.object_stride(), 1024),
            proc_words: if procs { part(blob_words, 1 << 20) } else { 0 },
        }
    }
}

/// A procedural BLAS an edit made, to be built with the edit's TLAS (`Gpu::proc_blas`, `build_edit`).
pub struct PendingProc {
    blas: wgpu::Blas,
    boxes: wgpu::Buffer,
    size: wgpu::BlasAABBGeometrySizeDescriptor,
}

/// The kernel's bindings of the seven tables the merged layout holds in one (merged.rs `TABLES`, in its order).
const MERGED_TABLE_BINDINGS: [u32; 7] = [4, 5, 6, 14, 15, 17, 18];

impl Uploaded {
    /// The seven tables, in merged.rs `TABLES`' order.
    fn table_buffers(&self) -> [&wgpu::Buffer; 7] {
        [&self.materials, &self.lights, &self.emitters, &self.bg_parts.texinfo, &self.objects, &self.instances, &self.media]
    }
}

/// The uniform tables' buffers among the seven (`table_buffers`), in merged.rs `UNIFORM_TABLES`' order.
fn uniform_sources(tables: [&wgpu::Buffer; 7]) -> [&wgpu::Buffer; 4] {
    crate::merged::UNIFORM_TABLES.map(|k| tables[k])
}

/// A note for each uniform table that a scene has more entries of (`used`, merged.rs `UNIFORM_TABLES`' order) than
/// its array holds: the kernel reads the last entry for the rest.
pub(crate) fn uniform_overflow(used: [u64; 4]) -> Vec<String> {
    use crate::merged::{TABLES, UNIFORM_LEN, UNIFORM_TABLES};
    (0..4)
        .filter(|&k| used[k] > UNIFORM_LEN[k] as u64)
        .map(|k| format!("uniform tables: the scene has {} entries of `{}`, a uniform buffer holds {} (the rest read as the last one; THREERT_UNIFORM_TABLES=0 binds storage buffers)", used[k], TABLES[UNIFORM_TABLES[k]].name, UNIFORM_LEN[k]))
        .collect()
}

/// The bytes of a table that its uniform copy takes: what both hold, in whole words (a copy's unit).
fn uniform_copy_bytes(table: u64, copy: u64) -> u64 {
    table.min(copy) & !3
}

/// A scene bind group's entries with the uniform tables' copies in their tables' places.
fn bind_uniform_tables<'a>(entries: &mut Vec<wgpu::BindGroupEntry<'a>>, copies: &'a [wgpu::Buffer; 4]) {
    let bindings = crate::merged::UNIFORM_TABLES.map(|k| MERGED_TABLE_BINDINGS[k]);
    entries.retain(|e| !bindings.contains(&e.binding));
    entries.extend(copies.iter().zip(bindings).map(|(b, binding)| wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }));
}

/// What the scene bind group holds besides `Uploaded`'s buffers (`Gpu::rebind_scene` makes it again).
pub struct BindParts {
    pub nodes: wgpu::Buffer,
    pub energy: wgpu::Buffer,
    pub texinfo: wgpu::Buffer,
    pub tex_view: wgpu::TextureView,
    pub env_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// The density grids' atlas (binding 19), or the one-cell texture of a scene without grids.
    pub grid_view: wgpu::TextureView,
}

pub struct Uploaded {
    pub bg: wgpu::BindGroup,
    pub bg_parts: BindParts,
    /// The merged layout (merged.rs): the buffer of the kernel's `MergedTables`, the seven tables below as its arrays
    /// (`Gpu::sync_tables` copies them there each frame; their lengths: `kernel.caps`).
    pub tables: Option<wgpu::Buffer>,
    /// The uniform tables (`Gpu::uniform_tables`): the uniform buffers the kernel reads `bg_parts.texinfo`, `objects`,
    /// `instances` and `media` from (merged.rs `UNIFORM_TABLES`' order; `Gpu::sync_tables` copies them there each frame).
    pub uniform: Option<[wgpu::Buffer; 4]>,
    pub accum: wgpu::Buffer,
    pub params: wgpu::Buffer,
    pub width: u32,
    pub height: u32,
    /// Kernel specialisation for this scene (`Gpu::pipeline`).
    pub kernel: Kernel,
    /// Tables that parameter edits rewrite in place (`patch`).
    pub materials: wgpu::Buffer,
    pub lights: wgpu::Buffer,
    pub objects: wgpu::Buffer,
    pub emitters: wgpu::Buffer,
    /// Triangle vertices / shading attributes (posed models rewrite their ranges).
    pub verts: wgpu::Buffer,
    pub attrs: wgpu::Buffer,
    /// The dynamic triangles' (posed models, movable objects: the tail from `dynamic_first`) vertices of the previous
    /// frame, kernel layout: live motion vectors.
    pub prev_verts: wgpu::Buffer,
    pub dynamic_first: usize,
    pub accel: Option<Accel>,
    /// Software traversal's units (units.rs), when the scene has any: what `accel` is to hardware traversal. Their
    /// trees are in the node buffer (`bg_parts.nodes`).
    pub units: Option<crate::units::Units>,
    /// The particle pass, when the scene has particle systems (hardware RT).
    pub particles: Option<ParticleGpu>,
    pub gpu_particles: Option<GpuParticlesGpu>,
    /// The density grids (the atlas the kernel reads, the grid programs' pass), when the scene has any.
    pub grids: Option<GridsGpu>,
    /// The media table (`MediumGpu`) and each medium's node.
    pub media: wgpu::Buffer,
    /// Vertex colours (F_VCOLOR): their byte offset in `prev_verts` (three words per triangle, room included).
    pub vcolor_base: Option<u64>,
    pub media_nodes: Vec<u32>,
    /// Per medium: its node is hidden (`set_medium_hidden`). The live view runs no media pass while all are.
    pub media_hidden: std::cell::RefCell<Vec<bool>>,
    /// The model instances' kernel records (`GpuInstance`): movable ones are rewritten when they move.
    pub instances: wgpu::Buffer,
    /// Room for in-place geometry edits (`EditRoom`), and the binding that holds the environment CDF and the
    /// procedural region, with the CDF's length and the region's words in use (the room follows them).
    pub room: EditRoom,
    pub cdf: wgpu::Buffer,
    pub cdf_words: usize,
    pub region_words: usize,
    /// The environment map (`write_env` replaces it in place when a dynamic sky's sun moves).
    pub env_tex: wgpu::Texture,
    /// Translucent particles' acceleration structures and pass resources (hardware RT).
    pub translucent: Option<Translucent>,
}

/// The particle pass of a scene (shaders/particles.wgsl with its particle functions): pipeline, bind group, the
/// per-frame parameters and the system table's size.
pub struct ParticleGpu {
    pipeline: wgpu::ComputePipeline,
    /// The BLAS build's timestamps (`STAMP_WGSL`).
    stamp: (wgpu::ComputePipeline, wgpu::BindGroup),
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    /// The system table (`ParticleSystemGpu`) and each system's node (pivot updates when it moves).
    table: wgpu::Buffer,
    nodes: Vec<u32>,
    systems: u32,
    max_count: u32,
    /// Not run yet: the first run's motion vectors are zero (no last frame).
    fresh: std::cell::Cell<bool>,
    /// Some systems are opaque (in the scene's dynamic BLASes, rebuilt every frame).
    opaque: bool,
    /// Per system: its particle count, whether the translucency pass draws it, and its state when hidden or shown in
    /// place (`set_particles_hidden`): `Shown`; `Collapse` (this frame's pass collapses its quads), `Collapsed` (zero
    /// its count next frame), `Idle`; `Showing` (this frame's pass is its first: no motion vectors), `ShowRan` (clear
    /// that next frame).
    counts: Vec<u32>,
    translucent: Vec<bool>,
    hidden: std::cell::RefCell<Vec<Hidden>>,
}

/// A particle system's state as hidden in place (`ParticleGpu::hidden`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hidden {
    Shown,
    Collapse,
    Collapsed,
    Idle,
    Showing,
    ShowRan,
}

/// Volume training points per frame at most (translucent.wgsl `volume_train`).
pub const VOLUME_TRAIN_CAP: u32 = 8192;

/// Translucent particles (`transparent` materials; hardware RT): their own BLAS + TLAS over the BLAS input the
/// particle pass writes (`verts`), from triangle `first` in the kernel's tables; the translucency pass's parameters and
/// volume training points (translucent.wgsl).
pub struct Translucent {
    blas: wgpu::Blas,
    pub tlas: wgpu::Tlas,
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    verts: wgpu::Buffer,
    pub first: u32,
    pub params: wgpu::Buffer,
    pub points: wgpu::Buffer,
    pub count: wgpu::Buffer,
    /// Live below full resolution: (colour, transmittance) per traced pixel (translucent.wgsl `tlow`).
    pub low: wgpu::Buffer,
}

impl Translucent {
    /// Rebuilds the BLAS from the particle pass's vertices, and the TLAS.
    fn build(&self, e: &mut wgpu::CommandEncoder) {
        let entry = wgpu::BlasBuildEntry {
            blas: &self.blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry { size: &self.size, vertex_buffer: &self.verts, first_vertex: 0, vertex_stride: 12, index_buffer: None, first_index: None, transform_buffer: None, transform_buffer_offset: None }]),
        };
        e.build_acceleration_structures([entry].iter(), [&self.tlas]);
    }
}

/// translucent.wgsl's entry points: the pass, volume training (live).
#[derive(Clone)]
pub struct TranslucentPipelines {
    pub draw: wgpu::ComputePipeline,
    pub train: wgpu::ComputePipeline,
    /// Live below full resolution: the traced-resolution particles laid over the output image.
    pub up: wgpu::ComputePipeline,
}

/// translucent.wgsl `TP`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TranslucentParams {
    pub size: [u32; 4],
    pub view: [u32; 4],
    pub misc: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleSystemGpu {
    a: [u32; 4],
    b: [u32; 4],
    c: [f32; 4],
    p: [f32; 32],
    d: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuParticleSystemGpu {
    a: [u32; 4],
    b: [u32; 4],
    c: [f32; 4],
    d: [f32; 4],
    e: [u32; 4],
    p: [f32; 32],
}

/// Frames an offline render simulates at most before it renders (stateful particles, grid programs): 10 minutes at 60
/// a second, whatever the scene's time.
pub const MAX_OFFLINE_STEPS: u32 = 36_000;

/// The density grids' texel: four half floats (filterable, and a storage texture's format in WebGPU's core).
pub const GRID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

fn grid_texture(d: &wgpu::Device, label: &str, dims: [u32; 3], written: bool) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: dims[0], height: dims[1], depth_or_array_layers: dims[2] },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: GRID_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | if written { wgpu::TextureUsages::STORAGE_BINDING } else { wgpu::TextureUsages::empty() },
        view_formats: &[],
    })
}

/// grid_sim.wgsl `SP`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GridStepGpu {
    l2w: [[f32; 4]; 3],
    w2l: [[f32; 4]; 3],
    dims: [u32; 4],
    at: [u32; 4],
    t: [f32; 4],
    cell: [f32; 4],
    custom: [f32; 4],
    p: [f32; 32],
}

/// A density grid on the GPU: where it lies in the atlas; with a program, its two half steps (A, B -> A', B' -> A, B:
/// their uniforms and bind groups), its sources, and the owning volume's placement and customData as they are now.
struct GridGpu {
    id: u32,
    node: u32,
    dims: [u32; 3],
    z0: u32,
    /// `world::DensityGrid::made`: the grid it was built from.
    made: u32,
    sim: Option<GridSim>,
}

struct GridSim {
    /// Field B, the program's own (its state with field A in the atlas; A' and B' are each frame's scratch).
    b: wgpu::Texture,
    program: u32,
    params: [f32; 32],
    steps: [(wgpu::Buffer, wgpu::BindGroup); 2],
    sources: wgpu::Buffer,
    placed: std::cell::Cell<([[f32; 4]; 3], [f32; 4])>,
}

/// Density grids (ROADMAP § 6 step 5b): field A of every grid in one 3D texture, stacked along z (the kernel's binding
/// 19, `vgrid`); the grid programs' pass (grid_sim.wgsl), when a grid has one.
pub struct GridsGpu {
    atlas: wgpu::Texture,
    pipeline: Option<wgpu::ComputePipeline>,
    grids: Vec<GridGpu>,
    step: std::cell::Cell<u32>,
}

/// grid_sim.wgsl with the scene's grid programs (`fn gp_<name>`) and their dispatch.
fn grid_sim_source(scene: &Scene) -> String {
    let base = include_str!("../shaders/grid_sim.wgsl");
    let mut f = String::new();
    let mut seen = std::collections::HashSet::new();
    for (_, _, helpers) in &scene.grid_programs {
        if !helpers.is_empty() && seen.insert(helpers.clone()) {
            f.push_str(helpers);
            f.push('\n');
        }
    }
    for (_, code, _) in &scene.grid_programs {
        f.push_str(code);
        f.push('\n');
    }
    f.push_str("fn grid_program(f: u32, s: Cell, i: CIn, p: array<vec4<f32>, 8>) -> Cell {\n    switch f {\n");
    for (k, (id, _, _)) in scene.grid_programs.iter().enumerate() {
        f.push_str(&format!("        case {k}u {{ return gp_{id}(s, i, p); }}\n"));
    }
    f.push_str("        default { return s; }\n    }\n}\n");
    let (head, rest) = base.split_once("// @grid-programs-begin").unwrap();
    let (_, tail) = rest.split_once("// @grid-programs-end").unwrap();
    format!("{head}{f}{tail}")
}

/// A grid's data (four f32 a cell) as the atlas's half floats.
fn grid_halves(data: &[f32]) -> Vec<half::f16> {
    data.iter().map(|&v| half::f16::from_f32(v.clamp(-65504.0, 65504.0))).collect()
}

/// Spawns one frame takes at most (gpu_particles.wgsl `spawns`: two vec4 each).
pub const MAX_SPAWNS: usize = 1 << 16;

/// Stateful GPU particles (gpu_particles.wgsl; ROADMAP § 6 step 5a): the pass, its tables and buffers, and per system
/// what the CPU keeps between frames: where the next emission starts in its ring of slots, the rate's fraction carried.
pub struct GpuParticlesGpu {
    pipeline: wgpu::ComputePipeline,
    /// Timestamps around the dynamic BLAS rebuild (`STAMP_WGSL`, as `ParticleGpu`'s).
    stamp: (wgpu::ComputePipeline, wgpu::BindGroup),
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    table: wgpu::Buffer,
    spawns: wgpu::Buffer,
    pub grid: wgpu::Buffer,
    systems: std::cell::RefCell<Vec<GpuParticleSystemGpu>>,
    host: std::cell::RefCell<Vec<(u32, f32, f32)>>,
    nodes: Vec<u32>,
    max_cap: u32,
    frame: std::cell::Cell<u32>,
    fresh: std::cell::Cell<bool>,
    grid_o: [f32; 4],
    grid_n: [u32; 4],
}

/// gpu_particles.wgsl with the scene's particle programs (`fn pe_` / `pu_<name>`) and their dispatch.
fn gpu_particle_source(scene: &Scene) -> String {
    let stride = "const OBJECT_STRIDE: u32 = 3u;";
    let base = include_str!("../shaders/gpu_particles.wgsl");
    assert_eq!(base.matches(stride).count(), 1, "gpu_particles.wgsl: {stride}");
    let base = if scene.tinted { base.replace(stride, "const OBJECT_STRIDE: u32 = 4u;") } else { base.to_string() };
    let mut f = String::new();
    let mut seen = std::collections::HashSet::new();
    for (_, _, _, helpers) in &scene.particle_programs {
        if !helpers.is_empty() && seen.insert(helpers.clone()) {
            f.push_str(helpers);
            f.push('\n');
        }
    }
    for (_, emit, update, _) in &scene.particle_programs {
        f.push_str(emit);
        f.push('\n');
        f.push_str(update);
        f.push('\n');
    }
    f.push_str("fn particle_emit(f: u32, i: EIn, p: array<vec4<f32>, 8>) -> PState {\n    switch f {\n");
    for (k, (id, ..)) in scene.particle_programs.iter().enumerate() {
        f.push_str(&format!("        case {k}u {{ return pe_{id}(i, p); }}\n"));
    }
    f.push_str("        default { var s: PState; s.pos = i.at; s.life = 0.0; return s; }\n    }\n}\n");
    f.push_str("fn particle_update(f: u32, s: PState, i: UIn, p: array<vec4<f32>, 8>) -> PState {\n    switch f {\n");
    for (k, (id, ..)) in scene.particle_programs.iter().enumerate() {
        f.push_str(&format!("        case {k}u {{ return pu_{id}(s, i, p); }}\n"));
    }
    f.push_str("        default { return s; }\n    }\n}\n");
    let (head, rest) = base.split_once("// @particle-programs-begin").unwrap();
    let (_, tail) = rest.split_once("// @particle-programs-end").unwrap();
    format!("{head}{f}{tail}")
}

/// particles.wgsl with the scene's particle functions (`fn pf_<name>`) and their dispatch (object entries of 4 vec4s
/// with instance colours: `scene::object_stride`).
fn particle_source(scene: &Scene) -> String {
    let stride = "const OBJECT_STRIDE: u32 = 3u;";
    let base = include_str!("../shaders/particles.wgsl");
    assert_eq!(base.matches(stride).count(), 1, "particles.wgsl: {stride}");
    let base = if scene.tinted { base.replace(stride, "const OBJECT_STRIDE: u32 = 4u;") } else { base.to_string() };
    let mut functions = String::new();
    let mut seen = std::collections::HashSet::new();
    for (_, _, helpers) in &scene.particle_fns {
        if !helpers.is_empty() && seen.insert(helpers.clone()) {
            functions.push_str(helpers);
            functions.push('\n');
        }
    }
    for (_, code, _) in &scene.particle_fns {
        functions.push_str(code);
        functions.push('\n');
    }
    functions.push_str("fn particle_function(f: u32, i: PIn, p: array<vec4<f32>, 8>) -> POut {\n    switch f {\n");
    for (k, (id, _, _)) in scene.particle_fns.iter().enumerate() {
        functions.push_str(&format!("        case {k}u {{ return pf_{id}(i, p); }}\n"));
    }
    functions.push_str("        default { return POut(i.pivot, vec3<f32>(0.0), vec3<f32>(0.0), false, vec4<f32>(0.0)); }\n    }\n}\n");
    let (head, rest) = base.split_once("// @particle-functions-begin").unwrap();
    let (_, tail) = rest.split_once("// @particle-functions-end").unwrap();
    format!("{head}{functions}{tail}")
}

/// Hardware acceleration structures: one BLAS per non-empty traversal group, one TLAS over them.
pub struct Accel {
    pub tlas: wgpu::Tlas,
    /// Triangle vertices the BLASes are built from (12 B per vertex, 3 per triangle).
    pub verts: wgpu::Buffer,
    /// (BLAS, group, first triangle, size).
    blases: Vec<(wgpu::Blas, usize, usize, wgpu::BlasTriangleGeometrySizeDescriptor)>,
    /// One per instanced mesh (model space), referenced by the TLAS's model instances.
    meshes: Vec<wgpu::Blas>,
    /// Per mesh: its (first pool triangle, count, opaque) geometries and their sizes (`rebuild_meshes`).
    mesh_geo: Vec<(Vec<(u32, u32, bool)>, Vec<wgpu::BlasTriangleGeometrySizeDescriptor>)>,
    /// The first pool triangle in the GPU order.
    pool_first: usize,
    /// The TLAS index of model instance 0 (the traversal groups' instances come first).
    pub inst_base: usize,
    /// The build's triangle BLAS bytes per triangle (Metal's sizes): edits' memory estimates (live/mod.rs `room_stats`).
    pub blas_per_tri: f64,
    /// The TLAS index of the first procedural object's instance (after the model instances; each has its own AABB
    /// BLAS, which its TLAS instance keeps).
    pub proc_base: usize,
}

/// What is above the triangles for an edit in place: hardware traversal's TLAS and BLASes, or software traversal's
/// units (units.rs). An instance's slot is its TLAS index; its kernel record is the slot less `inst_base` (0 for
/// units: a unit's slot is its record).
pub enum Top<'a> {
    Hw(&'a mut Accel),
    Sw(&'a mut crate::units::Units),
}

impl<'a> Top<'a> {
    /// The scene's, or None under software traversal without units (edits rebuild then).
    pub fn of(accel: &'a mut Option<Accel>, units: &'a mut Option<crate::units::Units>) -> Option<Top<'a>> {
        match (accel.as_mut(), units.as_mut()) {
            (Some(a), _) => Some(Top::Hw(a)),
            (None, Some(u)) => Some(Top::Sw(u)),
            _ => None,
        }
    }

    pub fn inst_base(&self) -> usize {
        match self {
            Top::Hw(a) => a.inst_base,
            Top::Sw(_) => 0,
        }
    }

    /// The slot of the first procedural object (`proc_first`: its record).
    pub fn proc_base(&self, proc_first: usize) -> usize {
        match self {
            Top::Hw(a) => a.proc_base,
            Top::Sw(_) => proc_first,
        }
    }

    pub fn sw(&self) -> bool {
        matches!(self, Top::Sw(_))
    }

    /// No instance at `slot`.
    pub fn clear(&mut self, slot: usize) {
        match self {
            Top::Hw(a) => a.tlas[slot] = None,
            Top::Sw(u) => u.set(slot, None),
        }
    }

    /// An instance of instanced mesh `k` at `slot`, placed by `world`. (Its bounds are the mesh's BLAS's, or its
    /// tree's: units.rs `Unit`.)
    pub fn place(&mut self, slot: usize, k: u32, world: glam::Mat4, custom: u32, cast_shadow: bool, hidden: bool) {
        match self {
            Top::Hw(a) => a.tlas[slot] = Some(wgpu::TlasInstance::new(&a.meshes[k as usize], tlas_rows(world), custom, instance_mask(cast_shadow, hidden))),
            Top::Sw(u) => u.set(slot, u.unit(k, world, hidden)),
        }
    }

    /// A procedural object's instance at `slot` (its AABB BLAS). Hardware traversal only: software traversal finds
    /// procedural objects by their records (`trace_procs`), and its edits of them rebuild the scene.
    pub fn place_blas(&mut self, slot: usize, blas: &wgpu::Blas, world: glam::Mat4, custom: u32, mask: u8) {
        if let Top::Hw(a) = self {
            a.tlas[slot] = Some(wgpu::TlasInstance::new(blas, tlas_rows(world), custom, mask));
        }
    }

    /// Whether instanced mesh `k` can be placed: always under hardware traversal (its BLAS); under software traversal
    /// if it has a tree (a mesh that deforms has none: its instances are flattened).
    pub fn has_mesh(&self, k: u32) -> bool {
        match self {
            Top::Hw(_) => true,
            Top::Sw(u) => u.root(k).is_some(),
        }
    }

    /// The instance at `slot`, if there is one, placed by `world` now; with `shown`, its mask too (hidden or not,
    /// casting shadows or not).
    pub fn moved(&mut self, slot: usize, world: glam::Mat4, shown: Option<(bool, bool)>) {
        match self {
            Top::Hw(a) => {
                if let Some(t) = a.tlas[slot].as_mut() {
                    t.transform = tlas_rows(world);
                    if let Some((cast_shadow, hidden)) = shown {
                        t.mask = instance_mask(cast_shadow, hidden);
                    }
                }
            }
            Top::Sw(u) => u.moved(slot, world, shown.map(|(_, hidden)| hidden)),
        }
    }

    /// An instance's kernel record as this traversal reads it: a unit's carries its mesh's tree's root and whether it
    /// casts shadows (hardware traversal has them in the TLAS).
    pub fn record(&self, r: crate::scene::GpuInstance, k: u32, cast_shadow: bool) -> crate::scene::GpuInstance {
        match self {
            Top::Hw(_) => r,
            Top::Sw(u) => crate::units::unit_record(r, u.root(k).unwrap_or(0), cast_shadow),
        }
    }
}

/// A TLAS instance transform: the world matrix's first three rows.
/// TLAS mask of an instance: 1 casts shadows, 2 does not, 0 a hidden instance of an instanced object (no ray sees it).
pub fn instance_mask(cast_shadow: bool, hidden: bool) -> u8 {
    if hidden {
        0
    } else if cast_shadow {
        1
    } else {
        2
    }
}

/// The kernel's files, one module in this order (pathtrace.wgsl's head says what each holds and names the `@if` flags).
const KERNEL_FILES: [&str; 4] = [
    include_str!("../shaders/pathtrace.wgsl"),
    include_str!("../shaders/trace_units.wgsl"),
    include_str!("../shaders/trace_hw.wgsl"),
    include_str!("../shaders/translucent.wgsl"),
];

/// The kernel's source: its files as one WESL module, every device's and scene's variant in it under `@if`.
pub fn kernel_template() -> String {
    KERNEL_FILES.join("\n")
}

/// The flags of the kernel's conditional translation (pathtrace.wgsl's head): a device's traversal and switches, and
/// a scene's features that change the kernel's declarations (`F_SOURCE`; the others switch code under `override FEAT`).
///  - vcolor (F_VCOLOR), three.js `vertexColors` (`scene::tri_colors`): three sRGB 8-bit colours per triangle after the
///    previous vertices in `prev_verts` (from vec4 `Params::procs.w`; the kernel has no binding to spare), interpolated
///    in linear at the hit, multiply the base colour of surfaces and glass (white for triangles without them).
///  - grid (F_GRID), density grids (`scene::GridBuild`): the grid atlas at binding 19 (a texture: the buffers are at
///    Metal's limit), `VIn::grid` and `vgrid` for volume functions.
///  - tint (F_TINT), instance colours (`scene::object_stride`): object entries of 4 vec4s, the 4th the instance colour,
///    which multiplies the base colour of surfaces, procedural surfaces and glass (three.js `instanceColor`).
///  - unlit (F_UNLIT), three.js MeshBasicMaterial (`scene::MAT_UNLIT`): its colour (x map, x tint) is emission seen by
///    camera rays and mirror-like paths only, its albedo 0: lit by nothing, lighting nothing (in no emitter list).
///  - units (F_UNITS), software traversal's second level (trace_units.wgsl): the still scene's walk hands its hit to
///    the units' and has no stack of its own (the module's, set to zero once an invocation and not once a ray).
///  - background (F_BACKGROUND), three.js `scene.background`: camera rays that miss, and paths bent straight on through
///    glass (the rays `sky_mis` gets with pdf 0), see `Params::background`, which `Params` ends with in this kernel only.
pub fn kernel_flags(hwrt: bool, k: &KernelSwitches, features: u32) -> [(&'static str, bool); 12] {
    let f = |bit: u32| features & bit != 0;
    [
        ("hwrt", hwrt),
        ("strict", k.strict),
        ("cutout_lite", k.cutout_lite >= 1),
        ("cutout_lite2", k.cutout_lite >= 2),
        ("last_direct", k.last_direct),
        ("light_pick", k.light_pick),
        ("vcolor", f(F_VCOLOR)),
        ("grid", f(F_GRID)),
        ("tint", f(F_TINT)),
        ("unlit", f(F_UNLIT)),
        ("units", f(F_UNITS)),
        ("background", f(F_BACKGROUND)),
    ]
}

/// A kernel's WGSL: `src` (the template, with what is put into its text first: the scene's functions, the merged
/// layout, the uniform tables, the loop guard) through WESL's conditional translation for `flags` (wesl-rs, WESL's
/// reference compiler in Rust). Every flag the source names must be given: one that is not is an error, not off.
pub fn kernel_wgsl(src: &str, flags: &[(&str, bool)]) -> String {
    use wesl::{resolver::VirtualResolver, syntax::ModulePath, CompileOptions, Feature, Features};
    let path: ModulePath = "package::kernel".parse().expect("a module path");
    let mut resolver = VirtualResolver::new();
    resolver.add_module(path.clone(), std::borrow::Cow::Borrowed(src));
    let mut features = Features { default: Feature::Error, ..Default::default() };
    for &(name, on) in flags {
        features.set(name, on);
    }
    let options = CompileOptions { imports: false, condcomp: true, strip: false, validate: false, sourcemap: false, mangler: wesl::ManglerKind::None, features, ..Default::default() };
    wesl::compile(&path, &options, &resolver).unwrap_or_else(|e| panic!("the kernel's WESL: {e}")).to_string()
}

/// The loop guard's limit for every kernel, if one is set (merged.rs `guard`: every loop of a kernel bounded, so a
/// dispatch ends whatever the data): THREERT_LOOP_GUARD=<iterations an invocation> natively, where a kernel is
/// otherwise as written; always in a browser, where a dispatch that does not end is the player's whole GPU.
pub fn loop_guard() -> Option<u32> {
    loop_guard_of(false)
}

/// The loop guard's limit for a kernel with units or without (`F_UNITS`). A kernel with units has the guard natively
/// too, at the browser's limit, unless THREERT_LOOP_GUARD=0: its second level is the traversal that has run least,
/// and what it walks is rewritten by edits while it runs. A kernel without units is as `loop_guard` says: natively,
/// with nothing set, the source as written.
pub fn loop_guard_of(units: bool) -> Option<u32> {
    let page = LOOP_GUARD.load(std::sync::atomic::Ordering::Relaxed);
    let var = std::env::var("THREERT_LOOP_GUARD").ok().and_then(|v| v.parse::<u32>().ok());
    guard_limit(page, var, units, cfg!(target_arch = "wasm32"))
}

/// `loop_guard_of`, of what was set: by a page (0: nothing), by the environment (Some(0): off), and where.
fn guard_limit(page: u32, var: Option<u32>, units: bool, browser: bool) -> Option<u32> {
    match (page, var) {
        (n, _) if n > 0 => Some(n),
        (_, Some(0)) => None,
        (_, Some(n)) => Some(n),
        (_, None) => (units || browser).then_some(1 << 16),
    }
}

/// The loop guard's limit as a page set it (0: not set), before the GPU is opened: a browser has no environment.
static LOOP_GUARD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Sets the loop guard's limit (a page's first runs of a new kernel take a low one); 0 leaves it as it is. Before
/// `Gpu::new_async`: the limit is in the kernels' source.
pub fn set_loop_guard(limit: u32) {
    if limit > 0 {
        LOOP_GUARD.store(limit, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Whether a device's layout as written has the guard's count (group 1, `merged::GUARD_BINDING`; `KernelBufs::guard`):
/// when a kernel of it may have the guard. Under software traversal that is unless THREERT_LOOP_GUARD=0, a scene's
/// first unit bringing the guard with it; under hardware traversal, when a limit is set.
fn guard_room(hwrt: bool) -> bool {
    loop_guard_of(!hwrt).is_some()
}

/// What a host decides about a device before it is opened (`Gpu::new_with`). `Gpu::new` takes it from the environment
/// (`from_env`), where every platform but Android gets `default()`: the device, its layout and its kernels as they
/// are without any of this. What a phone's GPU needed and why: docs/RUNTIME.md "Android (Adreno)".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuOptions {
    /// The kernel's bounded tables (texture pages, objects, instances, media) as uniform buffers instead of storage
    /// buffers (merged.rs `uniform_tables`). None: where a driver needs it, which is Android. Never with the merged
    /// layout, whose tables are one buffer of its own.
    pub uniform_tables: Option<bool>,
    /// A directory to keep compiled pipelines in between runs (pipeline_cache.rs). None: nothing is kept.
    pub pipeline_cache: Option<std::path::PathBuf>,
    pub kernel: KernelSwitches,
}

impl GpuOptions {
    /// THREERT_UNIFORM_TABLES=1|0, THREERT_PIPELINE_CACHE=<directory>, and the kernel's switches
    /// (`KernelSwitches::from_env`).
    pub fn from_env() -> GpuOptions {
        Self::from_vars(&|name| std::env::var(name).ok(), cfg!(target_os = "android"))
    }

    /// `from_env`, of these variables on this platform.
    fn from_vars(var: &dyn Fn(&str) -> Option<String>, android: bool) -> GpuOptions {
        GpuOptions {
            uniform_tables: var("THREERT_UNIFORM_TABLES").map(|v| v == "1"),
            pipeline_cache: var("THREERT_PIPELINE_CACHE").filter(|d| !d.is_empty()).map(std::path::PathBuf::from),
            kernel: KernelSwitches::from_vars(var, android),
        }
    }

    /// Whether the device has the uniform tables: as asked, else on Android; not in the merged layout.
    fn uniform_tables_on(&self, merged: bool, android: bool) -> bool {
        !merged && self.uniform_tables.unwrap_or(android)
    }
}

/// The kernel's switches: changes to its source for a GPU that the source as written is wrong or too slow on. Each
/// is a flag of the kernel's conditional translation (`kernel_flags`; pathtrace.wgsl, trace_hw.wgsl and
/// translucent.wgsl say where, under `@if`); with none set the kernel is as written, every platform's default but
/// Android's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KernelSwitches {
    /// THREERT_KERNEL_STRICT=1|0 (Android: 1). The kernel gives the driver nothing undefined to act on:
    ///  - a ray query whose ray is not finite (origin, direction) or whose length is negative or NaN is not made: the
    ///    ray misses. Such a description is undefined behaviour; on Adreno 740 the odd one faulted the GPU's MMU
    ///    thousands of times a second and doubled the trace. (naga's `ray_query_initialization_tracking` would do
    ///    it with bookkeeping around every ray-query call.)
    ///  - `env_pdf` no longer forms the index before the first entry (`select` evaluates both sides: `y - 1` at
    ///    y = 0 is 0xffffffff, read and then not used).
    pub strict: bool,
    /// THREERT_CUTOUT_LITE=0|1|2 (Android: 1). Hardware traversal: 1 = shadow rays and the other any-hit rays take
    /// alpha cutouts as opaque; 2 = camera rays also step through at most 4 cutouts (16 as written). Alpha-tested
    /// traversal is what costs most on Adreno 740 (a map with cutout foliage: the trace 235 ms, 41 ms without).
    pub cutout_lite: u32,
    /// THREERT_LAST_DIRECT=1|0 (Android: 1). The path's last vertex (past the camera hit, at the bounce limit) still
    /// takes its direct light. A device that affords one bounce and no radiance cache otherwise lights that bounce
    /// by sky and emitters alone: sunlit ground lit nothing, interiors were black.
    pub last_direct: bool,
    /// THREERT_LIGHT_PICK=1|0 (off everywhere unless set). Past the camera hit, `direct_light` takes the directional
    /// lights and one of the other kinds a scene has (local lights, emitters, the sky), weighted by their number,
    /// instead of all of them: two shadow rays a vertex instead of four. Unbiased; the indirect light is noisier.
    pub light_pick: bool,
}

impl KernelSwitches {
    pub fn from_env() -> KernelSwitches {
        Self::from_vars(&|name| std::env::var(name).ok(), cfg!(target_os = "android"))
    }

    fn from_vars(var: &dyn Fn(&str) -> Option<String>, android: bool) -> KernelSwitches {
        let num = |name: &str, default: u32| var(name).and_then(|v| v.parse::<u32>().ok()).unwrap_or(default);
        let on = android as u32;
        KernelSwitches { strict: num("THREERT_KERNEL_STRICT", on) != 0, cutout_lite: num("THREERT_CUTOUT_LITE", on), last_direct: num("THREERT_LAST_DIRECT", on) != 0, light_pick: num("THREERT_LIGHT_PICK", 0) != 0 }
    }
}

/// The kernel's runtime checks: none (all indices come from our own BVH / scene builder). THREERT_CHECKED turns naga's
/// on to diagnose a driver: 1 = every check, else a mask (2 bounds, 4 loop bounding, 8 ray-query initialization
/// tracking).
fn kernel_checks() -> wgpu::ShaderRuntimeChecks {
    let v: u32 = std::env::var("THREERT_CHECKED").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    if v == 1 {
        return wgpu::ShaderRuntimeChecks::checked();
    }
    let mut c = wgpu::ShaderRuntimeChecks::unchecked();
    c.bounds_checks = v & 2 != 0;
    c.force_loop_bounding = v & 4 != 0;
    c.ray_query_initialization_tracking = v & 8 != 0;
    c
}

/// A kernel source as it is compiled: with the loop guard if it has one (`loop_guard_of`: `units`, whether the source
/// has the unit walk), its count in `MergedLive` (the merged layout) or in a buffer of its own (group 1,
/// `merged::GUARD_BINDING`).
fn guarded(src: &str, merged: bool, units: bool) -> String {
    match loop_guard_of(units) {
        None => src.to_string(),
        Some(n) if merged => crate::merged::guard(src, n, "ML.loop_guard_trips"),
        Some(n) => crate::merged::guard(&format!("{src}\n{}", crate::merged::guard_decl()), n, crate::merged::GUARD_TRIPS.name),
    }
}

pub fn tlas_rows(w: glam::Mat4) -> [f32; 12] {
    let row = |r: usize| [w.x_axis[r], w.y_axis[r], w.z_axis[r], w.w_axis[r]];
    let [r0, r1, r2] = [row(0), row(1), row(2)];
    [r0[0], r0[1], r0[2], r0[3], r1[0], r1[1], r1[2], r1[3], r2[0], r2[1], r2[2], r2[3]]
}

/// Entries of the live radiance cache (pathtrace.wgsl CACHE_SIZE).
pub const CACHE_SIZE: u64 = 1 << 18;

/// A buffer, or a range of one: in the merged layout (merged.rs) several of the engine's buffers are ranges of the one
/// buffer a binding holds.
#[derive(Clone)]
pub struct Buf {
    pub buffer: wgpu::Buffer,
    pub offset: u64,
    pub size: u64,
}

impl Buf {
    pub fn whole(buffer: wgpu::Buffer) -> Buf {
        let size = buffer.size();
        Buf { buffer, offset: 0, size }
    }

    /// As a binding (a whole buffer: `as_entire_binding`).
    pub fn binding(&self) -> wgpu::BindingResource<'_> {
        if self.offset == 0 && self.size == self.buffer.size() {
            self.buffer.as_entire_binding()
        } else {
            wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.buffer, offset: self.offset, size: std::num::NonZeroU64::new(self.size) })
        }
    }

    pub fn clear(&self, e: &mut wgpu::CommandEncoder) {
        e.clear_buffer(&self.buffer, self.offset, Some(self.size));
    }
}

/// The radiance cache's storage (pathtrace.wgsl): voxel keys, this frame's training sums, resolved values.
#[derive(Clone)]
pub struct CacheBuffers {
    pub keys: Buf,
    pub acc: Buf,
    pub val: Buf,
}

impl CacheBuffers {
    pub fn new(d: &wgpu::Device, entries: u64) -> CacheBuffers {
        let b = |label, size| Buf::whole(d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        CacheBuffers { keys: b("cache keys", entries * 4), acc: b("cache sums", entries * 32), val: b("cache values", entries * 32) }
    }

    /// Empties the cache (a different world).
    pub fn clear(&self, e: &mut wgpu::CommandEncoder) {
        for b in [&self.keys, &self.acc, &self.val] {
            b.clear(e);
        }
    }
}

/// The kernel's group-1 buffers: the radiance cache, and what the live passes read of a trace
/// (live/targets.rs `Targets`): the specular-lobe rays, the motion vectors, the directional lights' direct light. In the
/// merged layout they are ranges of one buffer, the arrays of the kernel's `MergedLive` (merged.rs `KERNEL_LIVE`).
pub struct KernelBufs {
    pub cache: CacheBuffers,
    pub refl: Buf,
    pub motion: Buf,
    pub direct: Buf,
    /// The loop guard's count (merged.rs `guard`: its first word, the invocations a loop of which was stopped): with
    /// the guard, and always in the merged layout (the last array of `MergedLive`).
    pub guard: Option<Buf>,
    /// The merged layout: the one buffer, and its arrays' lengths.
    pub merged: Option<(wgpu::Buffer, [u32; 6])>,
}

impl KernelBufs {
    /// For a cache of `entries` and the live passes' three of these bytes.
    /// `guard`: the layout as written has the guard's count (`Gpu::guard_room`).
    pub fn new(d: &wgpu::Device, merged: bool, guard: bool, entries: u64, refl: u64, motion: u64, direct: u64) -> KernelBufs {
        if !merged {
            let sb = |label: &str, size: u64| Buf::whole(d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }));
            let guard = guard.then(|| sb("loop guard", crate::merged::GUARD_WORDS as u64 * 4));
            return KernelBufs { cache: CacheBuffers::new(d, entries), refl: sb("refl", refl), motion: sb("motion", motion), direct: sb("direct", direct), guard, merged: None };
        }
        let bytes = [entries * 4, entries * 32, entries * 32, refl, motion, direct, crate::merged::GUARD_WORDS as u64 * 4];
        let caps: [u32; 6] = std::array::from_fn(|k| crate::merged::live_cap(k, bytes[k]));
        let (at, size) = crate::merged::live_layout(&caps);
        let buffer = d.create_buffer(&wgpu::BufferDescriptor { label: Some("merged live"), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let b = |k: usize| Buf { buffer: buffer.clone(), offset: at[k], size: bytes[k] };
        KernelBufs { cache: CacheBuffers { keys: b(0), acc: b(1), val: b(2) }, refl: b(3), motion: b(4), direct: b(5), guard: Some(b(6)), merged: Some((buffer, caps)) }
    }
}

/// Group-1 bind group of the kernel: a primary-visibility texture, the radiance cache and the
/// reflection resolve's inputs (`KernelBufs`); in the merged layout, the texture and the one buffer.
pub fn vis_bind_group(d: &wgpu::Device, layout: &wgpu::BindGroupLayout, view: &wgpu::TextureView, b: &KernelBufs) -> wgpu::BindGroup {
    fn e(binding: u32, b: &Buf) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry { binding, resource: b.binding() }
    }
    let tex = wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) };
    match &b.merged {
        Some((buffer, _)) => d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vis"), layout, entries: &[tex, wgpu::BindGroupEntry { binding: crate::merged::KERNEL_LIVE_BINDING, resource: buffer.as_entire_binding() }] }),
        None => {
            let mut entries = vec![tex, e(1, &b.cache.keys), e(2, &b.cache.acc), e(3, &b.cache.val), e(5, &b.refl), e(6, &b.motion), e(7, &b.direct)];
            entries.extend(b.guard.as_ref().map(|g| e(crate::merged::GUARD_BINDING, g)));
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vis"), layout, entries: &entries })
        }
    }
}

/// Whether a material function's WGSL (`fn mf_<name>(...) -> MatOut { var o = base; ... return o; }`) can change its
/// result's `field`: it names `o.<field>`, or uses `o` otherwise than as `o.<another field>` (a whole assignment, a
/// pointer passed on) beyond its declaration and its return.
fn writes_field(code: &str, field: &str) -> bool {
    if code.contains(&format!("o.{field}")) {
        return true;
    }
    let body = code.replacen("var o = base;", "", 1).replace("return o;", "");
    let b = body.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    (0..b.len()).any(|k| b[k] == b'o' && (k == 0 || !ident(b[k - 1])) && b.get(k + 1).is_none_or(|&c| !ident(c) && c != b'.'))
}

/// What the path-tracing kernel is specialised on: the scene's feature mask and its material functions (WGSL).
#[derive(Clone)]
pub struct Kernel {
    pub features: u32,
    pub functions: String,
    /// The scene's volume functions and their dispatch (pathtrace.wgsl `volume_function`), spliced like `functions`.
    pub volume_functions: String,
    /// The scene's geometry functions and their dispatch (pathtrace.wgsl `geometry_function`), spliced like `functions`.
    pub geometry_functions: String,
    pub hash: u64,
    /// The merged layout (merged.rs): the lengths of the merged structs' arrays this kernel's source is written for
    /// (the tables': `Gpu::upload`; group 1's: an offline render's `no_vis`, a live view's `KernelBufs`).
    pub caps: Option<crate::merged::Caps>,
}

impl Kernel {
    pub fn new(scene: &Scene, groups: [usize; GROUPS]) -> Kernel {
        let mut functions = String::new();
        if !scene.shaders.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for (_, _, helpers) in &scene.shaders {
                if !helpers.is_empty() && seen.insert(helpers.clone()) {
                    functions.push_str(helpers);
                    functions.push('\n');
                }
            }
            for (_, code, _) in &scene.shaders {
                functions.push_str(code);
                functions.push('\n');
            }
            functions.push_str("fn material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut {\n    switch f {\n");
            for (k, (id, _, _)) in scene.shaders.iter().enumerate() {
                functions.push_str(&format!("        case {k} {{ return mf_{id}(i, m, base); }}\n"));
            }
            functions.push_str("        default { return base; }\n    }\n}\n");
            // The translucency pass's (translucent.wgsl): only the functions of its particles' materials. With every
            // function in it, the Metal compiler failed on Tokyo's kernel once the city's ground had one (wetLayer).
            let used: std::collections::BTreeSet<i32> = scene.trans_attrs.iter().filter_map(|a| scene.materials.get(a.uv[3].to_bits() as usize)).map(|m| m.tex1[3] - 1).filter(|&f| f >= 0).collect();
            functions.push_str("fn translucent_material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut {\n    switch f {\n");
            for k in used {
                if let Some((id, _, _)) = scene.shaders.get(k as usize) {
                    functions.push_str(&format!("        case {k} {{ return mf_{id}(i, m, base); }}\n"));
                }
            }
            functions.push_str("        default { return base; }\n    }\n}\n");
            // The sites that read one field of the result (alpha tests: `opacity`; emitter sampling: `emissive`): only
            // the functions that can change it; for the others the result's field is `base`'s. Each site inlines its
            // dispatch, and a function's code costs the kernel there even where it never runs (wetLayer's puddles:
            // ~0.8 ms on Tokyo, dry).
            for (name, field) in [("alpha_material_function", "opacity"), ("emissive_material_function", "emissive")] {
                functions.push_str(&format!("fn {name}(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut {{\n    switch f {{\n"));
                for (k, (id, code, _)) in scene.shaders.iter().enumerate() {
                    if writes_field(code, field) {
                        functions.push_str(&format!("        case {k} {{ return mf_{id}(i, m, base); }}\n"));
                    }
                }
                functions.push_str("        default { return base; }\n    }\n}\n");
            }
        }
        let mut volume_functions = String::new();
        if !scene.volume_fns.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for (_, _, helpers) in &scene.volume_fns {
                if !helpers.is_empty() && seen.insert(helpers.clone()) {
                    volume_functions.push_str(helpers);
                    volume_functions.push('\n');
                }
            }
            for (_, code, _) in &scene.volume_fns {
                volume_functions.push_str(code);
                volume_functions.push('\n');
            }
            volume_functions.push_str("fn volume_function(f: u32, i: VIn, p: array<vec4<f32>, 8>) -> VOut {\n    switch f {\n");
            for (k, (id, _, _)) in scene.volume_fns.iter().enumerate() {
                volume_functions.push_str(&format!("        case {k}u {{ return vf_{id}(i, p); }}\n"));
            }
            volume_functions.push_str("        default { return VOut(0.0, vec3<f32>(1.0), vec3<f32>(0.0)); }\n    }\n}\n");
        }
        let mut geometry_functions = String::new();
        if !scene.geometry_fns.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for (_, _, helpers) in &scene.geometry_fns {
                if !helpers.is_empty() && seen.insert(helpers.clone()) {
                    geometry_functions.push_str(helpers);
                    geometry_functions.push('\n');
                }
            }
            for (_, code, _) in &scene.geometry_fns {
                geometry_functions.push_str(code);
                geometry_functions.push('\n');
            }
            geometry_functions.push_str("fn geometry_function(f: u32, i: GIn) -> GOut {\n    switch f {\n");
            for (k, (id, _, _)) in scene.geometry_fns.iter().enumerate() {
                geometry_functions.push_str(&format!("        case {k}u {{ return gf_{id}(i); }}\n"));
            }
            geometry_functions.push_str("        default { return GOut(-1.0, vec3<f32>(0.0, 1.0, 0.0), 0u, vec2<f32>(0.0)); }\n    }\n}\n");
        }
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        functions.hash(&mut h);
        volume_functions.hash(&mut h);
        geometry_functions.hash(&mut h);
        Kernel { features: features(scene, groups), functions, volume_functions, geometry_functions, hash: h.finish(), caps: None }
    }

    /// The kernel as written (no feature, no function of a scene's), in the merged layout for arrays of `caps`'
    /// lengths: the radiance cache's resolve.
    fn plain(caps: crate::merged::Caps) -> Kernel {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for _ in 0..3 {
            String::new().hash(&mut h);
        }
        Kernel { features: 0, functions: String::new(), volume_functions: String::new(), geometry_functions: String::new(), hash: h.finish(), caps: Some(caps) }
    }
}

// Kernel features (`override FEAT` in pathtrace.wgsl).
pub const F_MASKED: u32 = 1;
pub const F_FOG: u32 = 64;
pub const F_VOLUME: u32 = 128;
pub const F_TEX: u32 = 256;
pub const F_FUNC: u32 = 512;
// (1024: the emitter-reservoir experiment, removed 2026-11-13.)
pub const F_GLASS: u32 = 2048;
pub const F_COAT: u32 = 4096;
/// A traversal group starts beyond 2^24 triangles: hardware instances carry their group index instead of their first
/// triangle (24-bit custom data), and hits look the offset up in `Params::group_first` (~0.2 ms on Kabukicho, so only
/// when needed).
pub const F_HUGE: u32 = 8192;
/// Instanced glTF meshes (scene::Instance): TLAS instances with custom data INSTANCE_BIT | record index, shaded from
/// model-space triangles through their `instances` record.
pub const F_INST: u32 = 16384;
/// Bounded participating media (`VolumeMesh`; pathtrace.wgsl `media_track`): compiled out where no scene has one.
pub const F_MEDIA: u32 = 32768;
/// Opaque particles: the kernel's emitter handling for them (nominal areas, emission not light-sampled), compiled out
/// elsewhere.
pub const F_PEMIT: u32 = 65536;
/// Homogeneous media (`VolumeMesh` `homogeneous`, water): the live kernel's analytic attenuation of paths and shadow
/// rays through them (pathtrace.wgsl `media_homogeneous`), compiled out elsewhere (it cost a scene with other media
/// 0.6 ms).
pub const F_HMEDIA: u32 = 131072;
/// Procedural geometry (`ProceduralMesh`; pathtrace.wgsl `proc_*`): AABB candidates in every ray query, its hits'
/// own surface path. Compiled out where no scene has any (`THREERT_FEATURES_ON` forces it in, to measure that cost).
pub const F_PROC: u32 = 262144;
/// Instance colours (`InstancedMesh.setColorAt`): object entries of 4 vec4s, the base colour times the 4th (`kernel_flags`;
/// `scene::object_stride`).
pub const F_TINT: u32 = 524288;
/// three.js MeshBasicMaterial (`scene::MAT_UNLIT`): seen by camera rays, lit by nothing, lighting nothing (`kernel_flags`).
pub const F_UNLIT: u32 = 1 << 20;
/// three.js `scene.background`: camera rays that miss see `Params::background` (`kernel_flags`).
pub const F_BACKGROUND: u32 = 1 << 21;
/// The features that change the kernel's declarations (`kernel_flags`, `@if` in its source) rather than switch code in it.
/// three.js `vertexColors` (`scene::tri_colors`): a colour per triangle vertex multiplies the base colour (F_SOURCE).
pub const F_VCOLOR: u32 = 1 << 22;
/// Density grids (`scene::GridBuild`): the grid atlas (binding 19) and `vgrid` for volume functions (F_SOURCE). (Bit
/// 24 is F_AERIAL's.)
pub const F_GRID: u32 = 1 << 25;
pub const F_SOURCE: u32 = F_TINT | F_UNLIT | F_BACKGROUND | F_VCOLOR | F_GRID | F_UNITS;
/// Software traversal's units (units.rs; shaders/trace_units.wgsl): the second level's walk, spliced into the
/// software traversal after the still scene's (F_SOURCE). Never with hardware traversal.
pub const F_UNITS: u32 = 1 << 26;
/// The sky's aerial perspective (SkyAtmosphere `aerialPerspective`; pathtrace.wgsl `aerial_tr`) on every path segment
/// that ends on a surface: compiled out where no scene has it. (Bit 23 is left to INSTANCE_BIT below.)
pub const F_AERIAL: u32 = 1 << 24;
/// Marks a model instance's custom data (bit 23); group instances then stay below it.
pub const INSTANCE_BIT: u32 = 1 << 23;

/// Whether some traversal group starts beyond what instance custom data holds (see F_HUGE): 24 bits, or 23 when
/// model instances take the top bit.
pub fn huge(groups: [usize; GROUPS], instanced: bool) -> bool {
    let limit = if instanced { 1 << 23 } else { 1 << 24 };
    let mut first = 0;
    groups.iter().any(|&count| {
        let past = count > 0 && first >= limit;
        first += count;
        past
    })
}

/// Features a scene needs; `THREERT_FEATURES` (bit mask) restricts them for profiling.
pub fn features(scene: &Scene, groups: [usize; GROUPS]) -> u32 {
    features_of(&scene.materials, scene.fog.as_ref(), !scene.textures.is_empty(), !scene.shaders.is_empty(), groups, instance_features(scene)) | if scene.aerial[0] > 0.0 { F_AERIAL } else { 0 }
}

/// Kernel features of the scene's structure, which parameter edits do not change: F_INST and F_MASKED for the
/// instanced meshes (cutout triangles among them), F_MEDIA for participating media (F_HMEDIA for homogeneous ones),
/// F_PEMIT for opaque particles, F_PROC for procedural geometry, F_TINT for instance colours.
pub fn instance_features(scene: &Scene) -> u32 {
    // (F_GRID also for a volume function that calls `vgrid` on a node without a grid: it reads zeros.)
    let grid = !scene.grids.is_empty() || (!scene.media.is_empty() && scene.volume_fns.iter().any(|(_, code, helpers)| code.contains("vgrid(") || helpers.contains("vgrid(")));
    let media = if scene.media.is_empty() { 0 } else { F_MEDIA } | if scene.media.iter().any(|m| m.a[2] != 0) { F_HMEDIA } else { 0 } | if grid { F_GRID } else { 0 };
    let particles = if scene.particles.iter().any(|p| !p.translucent) || !scene.gpu_particles.is_empty() { F_PEMIT } else { 0 } | if scene.procs.is_empty() { 0 } else { F_PROC } | if scene.tinted { F_TINT } else { 0 } | if scene.background.is_some() { F_BACKGROUND } else { 0 } | if scene.gpu_vcolors.is_empty() { 0 } else { F_VCOLOR };
    if scene.gpu_instances.is_empty() {
        return media | particles;
    }
    media | particles | F_INST | if scene.meshes.iter().any(|m| m.opaque < m.count) { F_MASKED } else { 0 } | if scene.units.is_some() { F_UNITS } else { 0 }
}

/// `features` from the parts parameter edits change (materials, fog), and the instanced meshes' (`instance_features`).
pub fn features_of(ms: &[crate::scene::GpuMaterial], fog: Option<&crate::desc::Fog>, textures: bool, functions: bool, groups: [usize; GROUPS], instances: u32) -> u32 {
    // `instances` carries the structural features (instance_features): instancing is its F_INST bit.
    let instanced = instances & F_INST != 0;
    let mut f = instances;
    if groups[1] + groups[3] + groups[5] + groups[7] > 0 {
        f |= F_MASKED;
    }
    if let Some(fog) = fog {
        if fog.density > 0.0 {
            f |= F_FOG;
            if fog.volumetric.as_ref().is_some_and(|v| v.scattering > 0.0) {
                f |= F_VOLUME;
            }
        }
    }
    if textures {
        f |= F_TEX;
    }
    if functions {
        f |= F_FUNC;
    }
    if ms.iter().any(|m| m.layer[0] > 0.0) {
        f |= F_GLASS;
    }
    if ms.iter().any(|m| m.layer[2] > 0.0) {
        f |= F_COAT;
    }
    if ms.iter().any(|m| m.tex1[2] == crate::scene::MAT_UNLIT) {
        f |= F_UNLIT;
    }
    if huge(groups, instanced) {
        f |= F_HUGE;
    }

    if let Some(mask) = std::env::var("THREERT_FEATURES").ok().and_then(|v| v.parse::<u32>().ok()) {
        f &= mask | F_HUGE | F_INST | F_PROC | F_TINT; // not optional: they say what the instances' custom data / the object table holds
    }
    // Features compiled in although the scene does not use them (the perf smoke's cost of a contract compiled in but off).
    if let Some(on) = std::env::var("THREERT_FEATURES_ON").ok().and_then(|v| v.parse::<u32>().ok()) {
        f |= on;
    }
    f
}

pub struct EnvMap {
    pub width: u32,
    pub height: u32,
    pub texels: Vec<[f32; 4]>,
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub color: Vec<[f32; 3]>,
    pub albedo: Vec<[f32; 3]>,
    pub normal: Vec<[f32; 3]>,
    /// With `Params::motion.y` (`render --signals`): what the camera saw directly, and the specular share at the camera
    /// hit (the diffuse signal: color less both). Zero otherwise.
    pub emission: Vec<[f32; 3]>,
    pub specular: Vec<[f32; 3]>,
    /// Variance of the per-pixel mean luminance.
    pub variance: Vec<f32>,
    pub samples: u32,
    /// Translucent particles to lay over the (denoised) image: per pixel colour (premultiplied) and transmittance;
    /// empty without them.
    pub overlay: Vec<[f32; 4]>,
}

/// Timestamps around the BLAS build (`run_particles`): a one-thread dispatch that reads the particle pass's output and
/// the TLAS, so Metal orders it after the particle pass and around the build (passes that share no resource overlap).
const STAMP_WGSL: &str = "enable wgpu_ray_query;
@group(0) @binding(0) var<storage, read> v: array<f32>;
@group(0) @binding(1) var acc: acceleration_structure;
@compute @workgroup_size(1, 1, 1)
fn stamp() {
    var rq: ray_query;
    rayQueryInitialize(&rq, acc, RayDesc(0u, 0xffu, 0.0, v[0] * 0.0, vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0)));
    rayQueryProceed(&rq);
}
";

impl Gpu {
    /// The GPU, waited for (the native shell; a browser has `new_async`: nothing may block there).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Result<Gpu> {
        pollster::block_on(Self::new_async())
    }

    /// `new`, with a host's own options instead of the environment's.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new_with(options: GpuOptions) -> Result<Gpu> {
        pollster::block_on(Self::new_async_with(options))
    }

    /// The adapter and the device come from futures: a browser resolves them from its event loop.
    pub async fn new_async() -> Result<Gpu> {
        Self::new_async_with(GpuOptions::from_env()).await
    }

    /// `new_async`, with a host's own options instead of the environment's.
    pub async fn new_async_with(options: GpuOptions) -> Result<Gpu> {
        let backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY);
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = backends;
        let instance = wgpu::Instance::new(desc);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, force_fallback_adapter: false, compatible_surface: None, ..Default::default() })
            .await
            .map_err(|e| anyhow!("no GPU adapter: {e}"))?;
        let adapter_info = adapter.get_info();
        let al = adapter.limits();
        // The defaults, lowered where the adapter cannot meet them (the iOS Simulator's GPU allows 15 inter-stage
        // variables, not 16); on desktop GPUs this changes nothing.
        let mut limits = wgpu::Limits::default().or_worse_values_from(&al);
        limits.max_storage_buffer_binding_size = al.max_storage_buffer_binding_size;
        limits.max_buffer_size = al.max_buffer_size;
        limits.max_texture_array_layers = al.max_texture_array_layers;
        limits.max_texture_dimension_2d = al.max_texture_dimension_2d;
        // The merged binding layout (merged.rs): a browser's, where a stage has eight storage buffers; natively with
        // THREERT_MERGED=1, on software traversal (what a browser has) and the device held to the same eight (a
        // layout with more does not validate).
        let merged = cfg!(target_arch = "wasm32") || std::env::var("THREERT_MERGED").is_ok_and(|v| v == "1");
        limits.max_storage_buffers_per_shader_stage = if merged { crate::merged::STORAGE_BUFFERS } else { al.max_storage_buffers_per_shader_stage.min(31) };
        // Storage + uniform buffers together (the kernel binds 19 storage buffers, a live pass at most 8, each beside its
        // uniforms; Metal allows 31).
        limits.max_buffers_and_acceleration_structures_per_shader_stage = al.max_buffers_and_acceleration_structures_per_shader_stage.min(31);
        limits.max_acceleration_structures_per_shader_stage = al.max_acceleration_structures_per_shader_stage;
        limits.max_blas_geometry_count = al.max_blas_geometry_count;
        limits.max_blas_primitive_count = al.max_blas_primitive_count;
        limits.max_tlas_instance_count = al.max_tlas_instance_count;
        let hwrt = !merged && adapter.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) && std::env::var("THREERT_HWRT").map(|v| v != "0").unwrap_or(true);
        let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let mut features = if hwrt { wgpu::Features::EXPERIMENTAL_RAY_QUERY } else { wgpu::Features::empty() };
        if timestamps {
            features |= wgpu::Features::TIMESTAMP_QUERY;
        }
        // Compiled pipelines kept between runs, where a host gave a directory and the backend has a cache.
        let cache_feature = options.pipeline_cache.as_ref().and_then(|_| crate::pipeline_cache::PipelineCache::feature(&adapter, &adapter_info));
        features |= cache_feature.unwrap_or(wgpu::Features::empty());
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("threert"),
            required_features: features,
            required_limits: limits,
            // Ray queries are still flagged experimental in wgpu 30.
            experimental_features: if hwrt { unsafe { wgpu::ExperimentalFeatures::enabled() } } else { wgpu::ExperimentalFeatures::disabled() },
            ..Default::default()
        }).await?;
        let pipeline_cache = cache_feature.and_then(|_| crate::pipeline_cache::PipelineCache::open(&device, options.pipeline_cache.as_ref()?, &adapter_info));
        let uniform_tables = options.uniform_tables_on(merged, cfg!(target_os = "android"));
        let switches = options.kernel;
        let src = kernel_template();
        // Bounds checks are not needed: all indices come from our own BVH / scene builder.
        let module = (!merged).then(|| unsafe {
            let src = if uniform_tables { crate::merged::uniform_tables(&src) } else { src.clone() };
            let wgsl = kernel_wgsl(&guarded(&src, false, false), &kernel_flags(hwrt, &switches, 0));
            device.create_shader_module_trusted(wgpu::ShaderModuleDescriptor { label: Some("pathtrace"), source: wgpu::ShaderSource::Wgsl(wgsl.into()) }, kernel_checks())
        });
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        // (The uniform tables: these four bindings hold uniform buffers; merged.rs `UNIFORM_TABLES`.)
        let table = |binding: u32| if uniform_tables { uniform(binding) } else { storage(binding, true) };
        let mut entries = vec![
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, true),
                storage(4, true),
                storage(5, true),
                storage(6, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 8, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                storage(10, true),
                storage(11, false),
                uniform(12),
                table(14),
                table(15),
                storage(16, true),
                table(17),
                table(18),
                // The density grids' atlas (F_GRID; a 1-cell texture in scenes without grids, whose kernel does not read it).
                wgpu::BindGroupLayoutEntry {
                    binding: 19,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
        ];
        if hwrt {
            entries.push(wgpu::BindGroupLayoutEntry { binding: 13, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::AccelerationStructure { vertex_return: false }, count: None });
        }
        // The merged layout: the seven tables are one binding, the first of theirs (merged.rs `TABLES`).
        if merged {
            entries.retain(|e| !MERGED_TABLE_BINDINGS[1..].contains(&e.binding));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries });
        let mut vis_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            storage(1, false),
            storage(2, false),
            storage(3, false),
            storage(5, false),
            storage(6, false),
            storage(7, false),
        ];
        // The merged layout: group 1's six buffers are one (merged.rs `KERNEL_LIVE`), the loop guard's count in it.
        // As written, a kernel with the guard has the count in a buffer of its own: the binding is there whenever a
        // kernel of this device may have the guard (`guard_room`; a kernel without it does not name the binding).
        if merged {
            vis_entries.truncate(2);
        } else if guard_room(hwrt) {
            vis_entries.push(storage(crate::merged::GUARD_BINDING, false));
        }
        let vis_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("vis"), entries: &vis_entries });
        let one = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("no vis"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let no_vis_view = one.create_view(&Default::default());
        let no_grid_view = grid_texture(&device, "no grid", [1, 1, 1], false).create_view(&Default::default());
        let grid_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid sim"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: GRID_FORMAT, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: GRID_FORMAT, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
                storage(6, true),
            ],
        });
        let no_vis_bufs = KernelBufs::new(&device, merged, guard_room(hwrt), 1, 64, 64, 64);
        let no_vis = vis_bind_group(&device, &vis_layout, &no_vis_view, &no_vis_bufs);
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout), Some(&vis_layout)], immediate_size: 0 });
        let (tl_layout, tl_pipeline_layout) = if hwrt {
            let st = |b: u32| wgpu::BindGroupLayoutEntry { binding: b, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None };
            let l = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("translucent"),
                entries: &[
                    wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::AccelerationStructure { vertex_return: false }, count: None },
                    wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                    st(2),
                    st(3),
                    st(4),
                    st(5),
                ],
            });
            let tpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("translucent"), bind_group_layouts: &[Some(&layout), Some(&vis_layout), Some(&l)], immediate_size: 0 });
            (Some(l), Some(tpl))
        } else {
            (None, None)
        };
        Ok(Gpu { instance, adapter, timestamps, device, queue, adapter_info, uniform_tables, switches, pipeline_cache, source: src, module, merged, modules: Default::default(), pipeline_layout: pl, pipelines: Default::default(), layout, vis_layout, no_vis, no_vis_bufs, no_vis_view, no_grid_view, grid_layout, hwrt, pages: Default::default(), tl_layout, tl_pipeline_layout, tl_pipelines: Default::default(), media_passes: Default::default(), proc_cache: Default::default(), proc_blases: Default::default() })
    }

    /// Whether this device's kernel buffers have the loop guard's count (`guard_room`; always in the merged layout).
    pub fn guard_room(&self) -> bool {
        guard_room(self.hwrt)
    }

    /// The pipeline cache for the descriptor of a pipeline about to be made (None: nothing is kept between runs).
    pub fn cache(&self) -> Option<&wgpu::PipelineCache> {
        self.pipeline_cache.as_ref().map(|c| c.for_pipeline())
    }

    /// The pipelines made since the last call, to the cache's file (pipeline_cache.rs: nothing when none was made, or
    /// the cache already had them). The renderer calls it after it compiles a kernel variant, a scene's passes and
    /// the live passes; a host that makes pipelines of its own with `cache` calls it after them.
    pub fn save_pipeline_cache(&self) {
        if let Some(c) = &self.pipeline_cache {
            c.save();
        }
    }

    /// The path-tracing kernel specialised for a scene (feature mask + material functions), compiled on first use.
    pub fn pipeline(&self, k: &Kernel, live: bool) -> wgpu::ComputePipeline {
        let mut cache = self.pipelines.lock().unwrap();
        cache
            .entry((k.features, k.hash, live, k.caps))
            .or_insert_with(|| {
                let module = self.kernel_module(k);
                let constants = [("LIVE", if live { 1.0 } else { 0.0 }), ("FEAT", k.features as f64)];
                let p = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(if live { "pathtrace-live" } else { "pathtrace" }),
                    layout: Some(&self.pipeline_layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                    cache: self.cache(),
                });
                self.save_pipeline_cache();
                p
            })
            .clone()
    }

    /// The kernel module for a scene: the shared one, or with its material functions spliced in.
    fn kernel_module(&self, k: &Kernel) -> wgpu::ShaderModule {
        let patches = k.features & F_SOURCE;
        assert_eq!(self.merged, k.caps.is_some(), "a kernel of the merged layout has its arrays' lengths");
        if let (Some(m), true) = (&self.module, patches == 0 && k.functions.is_empty() && k.volume_functions.is_empty() && k.geometry_functions.is_empty()) {
            return m.clone();
        }
        let mut mods = self.modules.lock().unwrap();
        mods.entry((k.hash, patches, k.caps))
            .or_insert_with(|| {
                let splice = |src: &str, marker: &str, code: &str| -> String {
                    if code.is_empty() {
                        return src.to_string();
                    }
                    let (head, rest) = src.split_once(&format!("// @{marker}-begin")).unwrap();
                    let (_, tail) = rest.split_once(&format!("// @{marker}-end")).unwrap();
                    format!("{head}{code}{tail}")
                };
                let src = splice(&splice(&splice(&self.source, "material-functions", &k.functions), "volume-functions", &k.volume_functions), "geometry-functions", &k.geometry_functions);
                // The merged layout: the same source, its buffers merged for this kernel's arrays (merged.rs).
                let src = k.caps.as_ref().map_or(src.clone(), |c| crate::merged::kernel(&src, c));
                // The uniform tables (never with the merged layout): the same source, four tables' declarations and
                // indices rewritten.
                let src = if self.uniform_tables { crate::merged::uniform_tables(&src) } else { src };
                // The loop guard, then the variant: this device's traversal and switches, this scene's features.
                let wgsl = kernel_wgsl(&guarded(&src, self.merged, patches & F_UNITS != 0), &kernel_flags(self.hwrt, &self.switches, patches));
                unsafe { self.device.create_shader_module_trusted(wgpu::ShaderModuleDescriptor { label: Some("pathtrace+functions"), source: wgpu::ShaderSource::Wgsl(wgsl.into()) }, kernel_checks()) }
            })
            .clone()
    }

    /// A live media pass of a scene's kernel variant: `media_train` (the volume voxels) or `media_composite` (camera
    /// rays through media, after the trace).
    pub fn media_pipeline(&self, k: &Kernel, entry: &'static str) -> wgpu::ComputePipeline {
        let mut cache = self.media_passes.lock().unwrap();
        cache
            .entry((k.features, k.hash, entry, k.caps))
            .or_insert_with(|| {
                let module = self.kernel_module(k);
                let constants = [("LIVE", 1.0), ("FEAT", k.features as f64)];
                let p = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&self.pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                    cache: self.cache(),
                });
                self.save_pipeline_cache();
                p
            })
            .clone()
    }

    /// The translucency pass's pipelines for a scene's kernel variant.
    pub fn translucent_pipelines(&self, k: &Kernel, live: bool) -> TranslucentPipelines {
        let mut cache = self.tl_pipelines.lock().unwrap();
        cache
            .entry((k.features, k.hash, live))
            .or_insert_with(|| {
                let module = self.kernel_module(k);
                let constants = [("LIVE", if live { 1.0 } else { 0.0 }), ("FEAT", k.features as f64)];
                let make = |entry: &str| {
                    self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: self.tl_pipeline_layout.as_ref(),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                        cache: self.cache(),
                    })
                };
                let made = TranslucentPipelines { draw: make("translucent"), train: make("volume_train"), up: make("translucent_up") };
                self.save_pipeline_cache();
                made
            })
            .clone()
    }

    /// The translucency pass's bind group (group 2) drawing into `target` (live: the composed image; offline: sums);
    pub fn translucent_bind_group(&self, t: &Translucent, target: &wgpu::Buffer) -> wgpu::BindGroup {
        fn e(b: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding: b, resource: buf.as_entire_binding() }
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("translucent"),
            layout: self.tl_layout.as_ref().unwrap(),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: t.tlas.as_binding() }, e(1, &t.params), e(2, target), e(3, &t.points), e(4, &t.count), e(5, &t.low)],
        })
    }

    /// The radiance cache's per-frame resolve (pathtrace.wgsl `cache_resolve`; kernel bind groups).
    pub fn cache_resolve_pipeline(&self, bufs: &KernelBufs) -> wgpu::ComputePipeline {
        // (The merged layout: the kernel written for these buffers' lengths, and the least tables.)
        let module = match (&self.module, &bufs.merged) {
            (Some(m), _) => m.clone(),
            (None, Some((_, live))) => self.kernel_module(&Kernel::plain(crate::merged::Caps { tables: crate::merged::TABLE_MIN, live: *live })),
            (None, None) => unreachable!("the merged layout's group 1 is one buffer"),
        };
        let p = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cache resolve"),
            layout: Some(&self.pipeline_layout),
            module: &module,
            entry_point: Some("cache_resolve"),
            compilation_options: Default::default(),
            cache: self.cache(),
        });
        self.save_pipeline_cache();
        p
    }

    /// Texture pages for the scene's texture set (see `texpack`), cached across uploads of the same set.
    fn texture_pages(&self, scene: &Scene, timings: &mut Vec<(String, f64)>) -> (wgpu::TextureView, wgpu::Buffer) {
        let key = crate::texpack::set_key(&scene.textures);
        let mut cache = self.pages.lock().unwrap();
        if let Some((k, view, info)) = cache.as_ref() {
            if *k == key {
                return (view.clone(), info.clone());
            }
        }
        let t = Instant::now();
        let d = &self.device;
        let packed = crate::texpack::pack(&scene.textures);
        let side = packed.side;
        let tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture pages"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: packed.layers.len() as u32 },
            mip_level_count: crate::texpack::MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (z, chain) in packed.layers.iter().enumerate() {
            for (lvl, img) in chain.iter().enumerate() {
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: lvl as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: z as u32 }, aspect: wgpu::TextureAspect::All },
                    img.as_raw(),
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.width() * 4), rows_per_image: Some(img.height()) },
                    wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
                );
            }
        }
        let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        // (The merged layout copies it into the tables' buffer, the uniform tables into its uniform copy: `sync_tables`.)
        let info = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("tex_info"), contents: bytemuck::cast_slice(&packed.info), usage: if self.merged || self.uniform_tables { wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC } else { wgpu::BufferUsages::STORAGE } });
        timings.push(("texture_pages_ms".into(), t.elapsed().as_secs_f64() * 1000.0));
        *cache = Some((key, view.clone(), info.clone()));
        (view, info)
    }

    /// Uploads a prepared scene (geometry, materials, textures, environment, acceleration structures) and returns the
    /// bind group used by both the offline and the live kernels. `accum` holds one 80-byte Pixel per output pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn upload(&self, scene: &Scene, w: u32, h: u32, tri_verts: &[[f32; 4]], groups: [usize; GROUPS], nodes: &[u8], emitters: &[[f32; 4]], env: &EnvMap, env_cdf: &[f32], energy: &[[f32; 4]], timings: &mut Vec<(String, f64)>) -> Result<Uploaded> {
        let d = &self.device;
        let t0 = Instant::now();
        // Empty tables still bind one zeroed element (the largest is a material, 368 B).
        // (The vertices too: the kernel's tree over no triangle is a leaf of triangle 0, which these zeros make a
        // triangle without area; bvh.rs `EMPTY`.)
        let buf = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            let data: &[u8] = if bytes.is_empty() { &[0u8; 512] } else { bytes };
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: data, usage })
        };
        // The same with `extra` zeroed bytes after the data (the edit room; none: as `buf`).
        let buf_room = |label: &str, bytes: &[u8], extra: usize, usage: wgpu::BufferUsages| {
            if extra == 0 {
                return buf(label, bytes, usage);
            }
            let b = d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: ((bytes.len() + extra) as u64).next_multiple_of(4), usage, mapped_at_creation: true });
            if !bytes.is_empty() {
                b.slice(..bytes.len() as u64).get_mapped_range_mut().expect("mapped at creation").copy_from_slice(bytes);
            }
            b.unmap();
            b
        };
        let records: Vec<crate::scene::GpuInstance> = scene.gpu_instances.iter().chain(&scene.proc_records).copied().collect();
        // (Software traversal's units have room as hardware traversal's instances do: `prepare` sized their trees'
        // by the same figures.)
        let room = EditRoom::for_scene(scene, tri_verts.len() / 3, records.len(), self.hwrt, scene.units.is_some());
        let st = wgpu::BufferUsages::STORAGE;
        // (COPY_SRC: the live view's in-place edits grow these past their room by a copy, live/edits.rs `edit_geometry`.)
        let edit = st | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let b_verts = buf_room("verts", bytemuck::cast_slice(tri_verts), room.tris * 48, edit | wgpu::BufferUsages::COPY_SRC);
        let dynamic_first: usize = groups[..4].iter().sum();
        // The dynamic tail (groups 4..8; the instanced pool follows it, its deforming meshes first: theirs too).
        let dynamic_end: usize = groups.iter().sum::<usize>() + crate::scene::deforming_pool(&scene.meshes);
        let prev = &tri_verts[(dynamic_first * 3).min(tri_verts.len())..(dynamic_end * 3).min(tri_verts.len())];
        // Vertex colours (F_VCOLOR) ride after the previous vertices (the kernel has no binding to spare), from vec4
        // `Params::procs.w` on, the edit room's triangles' too (the edits write theirs). Without them: as before.
        let b_prev = if scene.gpu_vcolors.is_empty() {
            buf("prev verts", bytemuck::cast_slice(prev), edit)
        } else {
            debug_assert_eq!(prev.len(), scene.vcolor_base as usize);
            buf_room("prev verts + vertex colours", &[bytemuck::cast_slice(prev), bytemuck::cast_slice(&scene.gpu_vcolors)].concat(), (room.tris * 12).next_multiple_of(16), edit)
        };
        // The vertex colours' bytes on the GPU (the edit room's included).
        timings.push(("vcolor_mb".into(), if scene.gpu_vcolors.is_empty() { 0.0 } else { (scene.gpu_vcolors.len() * 4 + room.tris * 12) as f64 / (1 << 20) as f64 }));
        let b_attrs = buf_room("attrs", bytemuck::cast_slice(&scene.attrs), room.tris * std::mem::size_of::<crate::scene::GpuTriAttr>(), edit);
        // (With units, edits write trees into it: units.rs.)
        let b_nodes = buf("nodes", nodes, if scene.units.is_some() { st | wgpu::BufferUsages::COPY_DST } else { st });
        // What the triangles hold on the GPU besides their BLASes: vertices (a, e1, e2), attributes, the software BVH's nodes
        // (ROADMAP § 6 step 2's table: a meshed representation against voxel data).
        timings.push(("tri_data_mb".into(), (std::mem::size_of_val(tri_verts) + std::mem::size_of_val(&scene.attrs[..]) + nodes.len()) as f64 / (1 << 20) as f64));
        let b_mats = buf("materials", bytemuck::cast_slice(&scene.materials), edit);
        let b_lights = buf("lights", bytemuck::cast_slice(&crate::lights_with_groups(&scene.lights)), edit);
        let b_emit = buf("emitters", bytemuck::cast_slice(emitters), edit);
        let (b_cdf, region_words, proc_room) = match &scene.proc_layout {
            None => (buf("env_cdf", bytemuck::cast_slice(env_cdf), st | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC), 0, 0),
            Some(l) => self.upload_procs(env_cdf, l, room.proc_words, timings)?,
        };
        let room = EditRoom { proc_words: proc_room, ..room };
        let b_energy = buf("energy", bytemuck::cast_slice(energy), wgpu::BufferUsages::UNIFORM);
        let b_objects = buf_room("objects", bytemuck::cast_slice(&scene.objects), room.objects * scene.object_stride() * 16, edit);
        let none = [crate::scene::GpuInstance::default()];
        // COPY_DST: live pose / customData edits (`set_medium`).
        let b_media = buf("media", bytemuck::cast_slice(&scene.media), edit);
        // Density grids: field A of each in the atlas (stacked along z), their data; for a grid with a program, its
        // other textures (A', B, B'), its two half steps and its sources.
        let grids = (!scene.grids.is_empty()).then(|| {
            let dims = [scene.grids.iter().map(|g| g.dims[0]).max().unwrap(), scene.grids.iter().map(|g| g.dims[1]).max().unwrap(), scene.grids.iter().map(|g| g.dims[2]).sum()];
            let atlas = grid_texture(d, "grid atlas", dims, true);
            let atlas_view = atlas.create_view(&Default::default());
            let smp = d.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
            let grids = scene
                .grids
                .iter()
                .map(|g| {
                    if let Some(data) = &g.data {
                        self.write_grid(&atlas, g.z0, g.dims, data);
                    }
                    let sim = g.program.map(|program| {
                        let tex = |label: &str| grid_texture(d, label, g.dims, true).create_view(&Default::default());
                        let field_b = grid_texture(d, "grid B", g.dims, true);
                        let (a2, b, b2) = (tex("grid A'"), field_b.create_view(&Default::default()), tex("grid B'"));
                        let sources = d.create_buffer(&wgpu::BufferDescriptor { label: Some("grid sources"), size: (crate::world::GRID_MAX_SOURCES * 48) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                        let step = |src_a: &wgpu::TextureView, src_b: &wgpu::TextureView, dst_a: &wgpu::TextureView, dst_b: &wgpu::TextureView| {
                            let u = d.create_buffer(&wgpu::BufferDescriptor { label: Some("grid step"), size: std::mem::size_of::<GridStepGpu>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                            fn tv(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
                                wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) }
                            }
                            let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("grid step"),
                                layout: &self.grid_layout,
                                entries: &[
                                    wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                                    tv(1, src_a),
                                    tv(2, src_b),
                                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&smp) },
                                    tv(4, dst_a),
                                    tv(5, dst_b),
                                    wgpu::BindGroupEntry { binding: 6, resource: sources.as_entire_binding() },
                                ],
                            });
                            (u, bg)
                        };
                        let steps = [step(&atlas_view, &b, &a2, &b2), step(&a2, &b2, &atlas_view, &b)];
                        let m = &scene.media[g.medium as usize];
                        GridSim { b: field_b, program, params: g.params, steps, sources, placed: std::cell::Cell::new((m.w, m.c)) }
                    });
                    GridGpu { id: g.id, node: g.node, dims: g.dims, z0: g.z0, made: g.made, sim }
                })
                .collect::<Vec<_>>();
            let pipeline = grids.iter().any(|g| g.sim.is_some()).then(|| {
                let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("grid sim"), source: wgpu::ShaderSource::Wgsl(grid_sim_source(scene).into()) });
                let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("grid sim"), bind_group_layouts: &[Some(&self.grid_layout)], immediate_size: 0 });
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("grid sim"), layout: Some(&layout), module: &module, entry_point: Some("grid_sim"), compilation_options: Default::default(), cache: self.cache() })
            });
            GridsGpu { atlas, pipeline, grids, step: std::cell::Cell::new(0) }
        });
        let grid_view = grids.as_ref().map_or_else(|| self.no_grid_view.clone(), |g| g.atlas.create_view(&Default::default()));
        // COPY_DST: movable instances' transforms (live moves).
        // The procedural objects' records follow the model instances' (moved like them); records edits add follow both.
        let b_instances = buf_room("instances", bytemuck::cast_slice(if records.is_empty() { &none[..] } else { &records[..] }), room.records * std::mem::size_of::<crate::scene::GpuInstance>(), edit);
        let accum_size = (w as u64) * (h as u64) * 80;
        let b_accum = d.create_buffer(&wgpu::BufferDescriptor { label: Some("accum"), size: accum_size, usage: st | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let b_params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("params"), size: std::mem::size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        let (tex_view, b_texinfo) = self.texture_pages(scene, timings);

        let env_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("env"),
            size: wgpu::Extent3d { width: env.width, height: env.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let half: Vec<u16> = env.texels.iter().flat_map(|t| t.iter().map(|v| half::f16::from_f32(v.min(65000.0)).to_bits())).collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &env_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&half),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(env.width * 8), rows_per_image: Some(env.height) },
            wgpu::Extent3d { width: env.width, height: env.height, depth_or_array_layers: 1 },
        );
        let env_view = env_tex.create_view(&Default::default());
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        // Hardware acceleration structures, one BLAS per traversal group (see `Prepared::groups`): shadow casters have
        // mask 1, the rest mask 2; alpha-cutout groups are non-opaque so the ray query reports candidates to test.
        let mut accel = None;
        if self.hwrt {
            let tb = Instant::now();
            let verts: Vec<[f32; 3]> = tri_verts
                .chunks(3)
                .flat_map(|t| {
                    let v0 = [t[0][0], t[0][1], t[0][2]];
                    [v0, [v0[0] + t[1][0], v0[1] + t[1][1], v0[2] + t[1][2]], [v0[0] + t[2][0], v0[1] + t[2][1], v0[2] + t[2][2]]]
                })
                .collect();
            let vbuf = buf_room("blas_verts", bytemuck::cast_slice(&verts), room.tris * 36, wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::STORAGE);
            let huge_scene = huge(groups, !scene.gpu_instances.is_empty());
            let mut first = 0usize;
            let groups: Vec<(usize, usize, u8, bool)> = groups
                .iter()
                .enumerate()
                .map(|(g, &count)| {
                    let r = (first, count, if g % 4 < 2 { 1u8 } else { 2u8 }, g % 2 == 0);
                    first += count;
                    r
                })
                .collect();
            let pool_first = first;
            let mut t = d.create_tlas(&wgpu::CreateTlasDescriptor { label: None, max_instances: (GROUPS + scene.instances.len() + scene.procs.len() + room.records) as u32, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });
            let sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor> = groups
                .iter()
                .map(|&(_, count, _, opaque)| wgpu::BlasTriangleGeometrySizeDescriptor {
                    vertex_format: wgpu::VertexFormat::Float32x3,
                    vertex_count: (count * 3) as u32,
                    index_format: None,
                    index_count: None,
                    flags: if opaque { wgpu::AccelerationStructureGeometryFlags::OPAQUE } else { wgpu::AccelerationStructureGeometryFlags::empty() },
                })
                .collect();
            let mut blases = vec![];
            for (i, (first, count, mask, _)) in groups.iter().enumerate() {
                if *count == 0 {
                    continue;
                }
                // Dynamic groups (4..8) are rebuilt whenever something in them moves: built fast rather than traced fast
                // (the C slice's ~100 k moving triangles: rebuild 4.14 -> 3.1 ms a frame, trace unchanged).
                let flags = if i >= 4 { wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD } else { wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE };
                let blas = d.create_blas(
                    &wgpu::CreateBlasDescriptor { label: None, flags, update_mode: wgpu::AccelerationStructureUpdateMode::Build },
                    wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![sizes[i].clone()] },
                );
                // Custom data = the first triangle, or past 2^24 (F_HUGE) the group: `Params::group_first` has its first.
                let custom = if huge_scene { i as u32 } else { *first as u32 };
                t[blases.len()] = Some(wgpu::TlasInstance::new(&blas, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], custom, *mask));
                blases.push((blas, i, *first, sizes[i].clone()));
            }
            // Instanced meshes: one opaque model-space BLAS each, a TLAS instance per placement (custom data
            // INSTANCE_BIT | record; shadow casters mask 1).
            // Per mesh: (first triangle, count, opaque) geometries: the opaque part, then the cutout part (any-hit).
            let mesh_parts: Vec<Vec<(u32, u32, bool)>> = scene.meshes.iter().map(|m| [(m.first, m.opaque, true), (m.first + m.opaque, m.count - m.opaque, false)].into_iter().filter(|p| p.1 > 0).collect()).collect();
            let mesh_sizes: Vec<Vec<wgpu::BlasTriangleGeometrySizeDescriptor>> = mesh_parts
                .iter()
                .map(|parts| {
                    parts
                        .iter()
                        .map(|&(_, count, opaque)| wgpu::BlasTriangleGeometrySizeDescriptor {
                            vertex_format: wgpu::VertexFormat::Float32x3,
                            vertex_count: count * 3,
                            index_format: None,
                            index_count: None,
                            flags: if opaque { wgpu::AccelerationStructureGeometryFlags::OPAQUE } else { wgpu::AccelerationStructureGeometryFlags::empty() },
                        })
                        .collect()
                })
                .collect();
            // Deforming meshes (shared poses) are rebuilt every new pose: built fast rather than traced fast, as the dynamic
            // groups are.
            let meshes: Vec<wgpu::Blas> = mesh_sizes
                .iter()
                .zip(&scene.meshes)
                .map(|(s, m)| {
                    let flags = if m.deforms { wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD } else { wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE };
                    d.create_blas(&wgpu::CreateBlasDescriptor { label: None, flags, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: s.clone() })
                })
                .collect();
            // What the triangle BLASes hold (Metal's own sizes; each instanced mesh once) and their vertex buffer.
            let group_parts = groups.iter().filter(|g| g.1 > 0).map(|&(first, count, _, opaque)| vec![((first * 3) as u32, (count * 3) as u32, opaque)]);
            let mesh_vertex_parts = mesh_parts.iter().map(|ps| ps.iter().map(|&(first, count, opaque)| ((pool_first as u32 + first) * 3, count * 3, opaque)).collect::<Vec<_>>());
            let mut blas_per_tri = 0.0;
            let parts: Vec<Vec<(u32, u32, bool)>> = group_parts.chain(mesh_vertex_parts).collect();
            if let Some(bytes) = parts.iter().map(|ps| self.tri_blas_bytes(&vbuf, ps)).sum::<Option<u64>>() {
                timings.push(("tri_blas_mb".into(), (bytes + vbuf.size()) as f64 / (1 << 20) as f64));
                let tris: u64 = parts.iter().flatten().map(|p| p.1 as u64 / 3).sum();
                blas_per_tri = bytes as f64 / tris.max(1) as f64;
            }
            let base = blases.len();
            for (k, inst) in scene.instances.iter().enumerate() {
                let w = inst.world;
                let row = |r: usize| [w.x_axis[r], w.y_axis[r], w.z_axis[r], w.w_axis[r]];
                let [r0, r1, r2] = [row(0), row(1), row(2)];
                let xf = [r0[0], r0[1], r0[2], r0[3], r1[0], r1[1], r1[2], r1[3], r2[0], r2[1], r2[2], r2[3]];
                t[base + k] = Some(wgpu::TlasInstance::new(&meshes[inst.mesh as usize], xf, INSTANCE_BIT | k as u32, instance_mask(inst.cast_shadow, inst.hidden)));
            }
            // Procedural geometry: one AABB BLAS per distinct geometry (object space; kept across uploads by the
            // geometry's hash, so a rebuild builds only new ones), and per mesh a TLAS instance of it after the model
            // instances, whose custom data is the mesh's record; its hits are told apart by their kind.
            let proc_base = base + scene.instances.len();
            let mut built = self.proc_blases.lock().unwrap();
            let mut kept: std::collections::HashMap<[u8; 32], (wgpu::Blas, u64)> = Default::default();
            let mut proc_new: Vec<(usize, wgpu::Buffer, wgpu::BlasAABBGeometrySizeDescriptor)> = vec![];
            let geo_blas: Vec<wgpu::Blas> = scene.proc_layout.as_ref().map_or(vec![], |l| {
                l.blobs
                    .iter()
                    .enumerate()
                    .map(|(k, (b, _))| {
                        let (blas, bytes) = built.get(&b.hash).cloned().unwrap_or_else(|| {
                            let size = wgpu::BlasAABBGeometrySizeDescriptor { primitive_count: (b.boxes.len() / 6) as u32, flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE };
                            let blas = d.create_blas(&wgpu::CreateBlasDescriptor { label: Some("proc"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::AABBs { descriptors: vec![size.clone()] });
                            let boxes = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("proc boxes"), contents: bytemuck::cast_slice(&b.boxes), usage: wgpu::BufferUsages::BLAS_INPUT });
                            let bytes = self.aabb_blas_bytes(&boxes, size.primitive_count).unwrap_or(0);
                            proc_new.push((k, boxes, size));
                            (blas, bytes)
                        });
                        kept.insert(b.hash, (blas.clone(), bytes));
                        blas
                    })
                    .collect()
            });
            // What the procedural BLASes hold on the GPU (each distinct geometry once; ROADMAP § 6 step 2's table).
            timings.push(("proc_blas_mb".into(), kept.values().map(|(_, b)| *b).sum::<u64>() as f64 / (1 << 20) as f64));
            *built = kept;
            drop(built);
            for (j, p) in scene.procs.iter().enumerate() {
                let record = (scene.gpu_instances.len() + j) as u32;
                anyhow::ensure!(record < INSTANCE_BIT, "too many instances for 23-bit custom data");
                let k = scene.proc_layout.as_ref().map_or(j, |l| l.blob_of[j]);
                t[proc_base + j] = Some(wgpu::TlasInstance::new(&geo_blas[k], tlas_rows(p.world), record, instance_mask(p.cast_shadow, p.hidden)));
            }
            let entries: Vec<wgpu::BlasBuildEntry> = blases
                .iter()
                .map(|(blas, _, first, size)| wgpu::BlasBuildEntry {
                    blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                        size,
                        vertex_buffer: &vbuf,
                        first_vertex: (*first * 3) as u32,
                        vertex_stride: 12,
                        index_buffer: None,
                        first_index: None,
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    }]),
                })
                .chain(meshes.iter().zip(&mesh_sizes).zip(&mesh_parts).map(|((blas, sizes), parts)| wgpu::BlasBuildEntry {
                    blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(
                        sizes
                            .iter()
                            .zip(parts)
                            .map(|(size, &(first, _, _))| wgpu::BlasTriangleGeometry {
                                size,
                                vertex_buffer: &vbuf,
                                first_vertex: ((pool_first + first as usize) * 3) as u32,
                                vertex_stride: 12,
                                index_buffer: None,
                                first_index: None,
                                transform_buffer: None,
                                transform_buffer_offset: None,
                            })
                            .collect(),
                    ),
                }))
                .chain(proc_new.iter().map(|(k, boxes, size)| wgpu::BlasBuildEntry {
                    blas: &geo_blas[*k],
                    geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry { size, stride: 24, aabb_buffer: boxes, primitive_offset: 0 }]),
                }))
                .collect();
            let mut enc = d.create_command_encoder(&Default::default());
            enc.build_acceleration_structures(entries.iter(), [&t]);
            self.queue.submit([enc.finish()]);
            self.device.poll(wgpu::PollType::wait_indefinitely())?;
            drop(entries);
            let mesh_geo = mesh_parts.into_iter().zip(mesh_sizes).collect();
            accel = Some(Accel { tlas: t, verts: vbuf, blases, meshes, mesh_geo, pool_first, inst_base: base, proc_base, blas_per_tri });
            timings.push(("accel_build_ms".into(), tb.elapsed().as_secs_f64() * 1000.0));
        }
        fn e(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
        }
        let mut bg_entries = vec![
                e(0, &b_params),
                e(1, &b_verts),
                e(2, &b_attrs),
                e(3, &b_nodes),
                e(4, &b_mats),
                e(5, &b_lights),
                e(6, &b_emit),
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&tex_view) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&env_view) },
                e(10, &b_cdf),
                e(11, &b_accum),
                e(12, &b_energy),
                e(14, &b_texinfo),
                e(15, &b_objects),
                e(16, &b_prev),
                e(17, &b_instances),
                e(18, &b_media),
                wgpu::BindGroupEntry { binding: 19, resource: wgpu::BindingResource::TextureView(&grid_view) },
        ];
        if let Some(a) = &accel {
            bg_entries.push(wgpu::BindGroupEntry { binding: 13, resource: a.tlas.as_binding() });
        }
        // The merged layout: the tables' one buffer in their place (merged.rs).
        let all_tables = [&b_mats, &b_lights, &b_emit, &b_texinfo, &b_objects, &b_instances, &b_media];
        let tables = self.merged.then(|| self.tables_buffer(all_tables));
        if let Some((t, _)) = &tables {
            bg_entries.retain(|e| !MERGED_TABLE_BINDINGS.contains(&e.binding));
            bg_entries.push(e(crate::merged::TABLES_BINDING, t));
        }
        // The uniform tables: four tables' uniform copies in their place (merged.rs `uniform_tables`).
        let uniform = self.uniform_tables.then(|| {
            // What the scene has of each, against what its array holds (the buffers have the edits' room after it).
            let used = [b_texinfo.size() / 16, scene.objects.len() as u64, records.len() as u64, scene.media.len() as u64];
            for note in uniform_overflow(used) {
                eprintln!("threert: {note}");
            }
            self.uniform_copies(uniform_sources(all_tables))
        });
        if let Some(u) = &uniform {
            bind_uniform_tables(&mut bg_entries, u);
        }
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.layout, entries: &bg_entries });
        let mut kernel = Kernel::new(scene, groups);
        kernel.caps = tables.as_ref().map(|t| crate::merged::Caps { tables: t.1, live: self.no_vis_bufs.merged.as_ref().expect("merged").1 });
        let tables = tables.map(|t| t.0);
        // Translucent particles (hardware RT): their own BLAS + TLAS, rebuilt after the particle pass.
        let translucent = match &accel {
            Some(_) if !scene.trans_tris.is_empty() => {
                let n = scene.trans_tris.len() as u32;
                let size = wgpu::BlasTriangleGeometrySizeDescriptor { vertex_format: wgpu::VertexFormat::Float32x3, vertex_count: n * 3, index_format: None, index_count: None, flags: wgpu::AccelerationStructureGeometryFlags::empty() };
                let blas = d.create_blas(
                    &wgpu::CreateBlasDescriptor { label: Some("translucent"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build },
                    wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![size.clone()] },
                );
                let mut tlas = d.create_tlas(&wgpu::CreateTlasDescriptor { label: Some("translucent"), max_instances: 1, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });
                tlas[0] = Some(wgpu::TlasInstance::new(&blas, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], 0, 0xff));
                let verts = d.create_buffer(&wgpu::BufferDescriptor { label: Some("translucent blas verts"), size: n as u64 * 36, usage: wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("translucent params"), size: 48, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let points = d.create_buffer(&wgpu::BufferDescriptor { label: Some("volume points"), size: VOLUME_TRAIN_CAP as u64 * 16, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let count = d.create_buffer(&wgpu::BufferDescriptor { label: Some("volume point count"), size: 16, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let low = d.create_buffer(&wgpu::BufferDescriptor { label: Some("translucent low"), size: (w as u64 * h as u64).max(1) * 16, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let t = Translucent { blas, tlas, size, verts, first: scene.trans_first, params, points, count, low };
                // Built once now (collapsed quads): bind groups may use the TLAS before the first particle pass.
                let mut enc = d.create_command_encoder(&Default::default());
                t.build(&mut enc);
                self.queue.submit([enc.finish()]);
                Some(t)
            }
            _ => None,
        };
        // Particle systems (hardware RT: the pass writes the dynamic BLASes' input).
        let particles = match &accel {
            Some(a) if !scene.particles.is_empty() => {
                let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("particles"), source: wgpu::ShaderSource::Wgsl(particle_source(scene).into()) });
                let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("particles"), layout: None, module: &module, entry_point: Some("particles"), compilation_options: Default::default(), cache: self.cache() });
                let sm = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("stamp"), source: wgpu::ShaderSource::Wgsl(STAMP_WGSL.into()) });
                let sp = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("stamp"), layout: None, module: &sm, entry_point: Some("stamp"), compilation_options: Default::default(), cache: self.cache() });
                // The stamps depend on what is rebuilt last: the translucent TLAS when there is one.
                let (sv, stl) = translucent.as_ref().map_or((&a.verts, &a.tlas), |t| (&t.verts, &t.tlas));
                let sbg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("stamp"), layout: &sp.get_bind_group_layout(0), entries: &[e(0, sv), wgpu::BindGroupEntry { binding: 1, resource: stl.as_binding() }] });
                let stamp = (sp, sbg);
                let systems: Vec<ParticleSystemGpu> = scene
                    .particles
                    .iter()
                    .map(|ps| {
                        let o = &scene.objects[ps.object as usize * scene.object_stride()];
                        let mut p = [0.0f32; 32];
                        p.copy_from_slice(&ps.params);
                        // (A system hidden at build runs nothing: its quads stay collapsed at the node.)
                        ParticleSystemGpu { a: [ps.final_first, if ps.hidden { 0 } else { ps.count }, ps.object, ps.func], b: [ps.seed, dynamic_first as u32, ps.translucent as u32, scene.trans_first], c: [o[0], o[1], o[2], 0.0], p, d: ps.custom }
                    })
                    .collect();
                // COPY_DST: pivot updates (`set_particle_pivot`).
                let b_sys = buf("particle systems", bytemuck::cast_slice(&systems), edit);
                let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("particle params"), size: 48, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let last = d.create_buffer(&wgpu::BufferDescriptor { label: Some("particles last"), size: b_prev.size(), usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let no_trans = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: 64, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let pbg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("particles"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[e(0, &params), e(1, &b_sys), e(2, &b_verts), e(3, &b_prev), e(4, &a.verts), e(5, &b_objects), e(6, &b_attrs), e(7, &last), e(8, translucent.as_ref().map_or(&no_trans, |t| &t.verts))],
                });
                let nodes = scene.particles.iter().map(|p| p.seed).collect();
                let opaque = scene.particles.iter().any(|p| !p.translucent);
                let counts = scene.particles.iter().map(|p| p.count).collect();
                let translucent_sys = scene.particles.iter().map(|p| p.translucent).collect();
                let hidden = std::cell::RefCell::new(scene.particles.iter().map(|p| if p.hidden { Hidden::Idle } else { Hidden::Shown }).collect());
                Some(ParticleGpu { pipeline, stamp, bg: pbg, params, table: b_sys, nodes, systems: systems.len() as u32, max_count: scene.particles.iter().map(|p| p.count).max().unwrap_or(0), fresh: std::cell::Cell::new(true), opaque, counts, translucent: translucent_sys, hidden })
            }
            _ => None,
        };
        // Stateful GPU particles (hardware RT, as `Particles`): their states (zero: none alive), the game's spawns, the
        // voxel occupancy grid they collide with (a word without one).
        let gpu_particles = match &accel {
            Some(a) if !scene.gpu_particles.is_empty() => {
                let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gpu particles"), source: wgpu::ShaderSource::Wgsl(gpu_particle_source(scene).into()) });
                let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("gpu particles"), layout: None, module: &module, entry_point: Some("gpu_particles"), compilation_options: Default::default(), cache: self.cache() });
                let systems: Vec<GpuParticleSystemGpu> = scene
                    .gpu_particles
                    .iter()
                    .map(|ps| {
                        let c = &ps.collide;
                        let flags = u32::from(c.plane.is_some()) | if c.voxels { 2 } else { 0 };
                        GpuParticleSystemGpu { a: [ps.final_first, ps.capacity, ps.object, ps.program], b: [ps.seed, dynamic_first as u32, 0, 0], c: [ps.pivot[0], ps.pivot[1], ps.pivot[2], flags as f32], d: [c.plane.unwrap_or(0.0), c.restitution, c.friction, 0.0], e: [0; 4], p: ps.params }
                    })
                    .collect();
                let table = buf("gpu particle systems", bytemuck::cast_slice(&systems), edit);
                let states = scene.gpu_particles.iter().map(|ps| (ps.object + ps.capacity) as u64).max().unwrap_or(1) * 64;
                let state = d.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particle states"), size: states, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let spawns = d.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particle spawns"), size: (MAX_SPAWNS * 32) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particle params"), size: 80, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let last = d.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particles last"), size: b_prev.size(), usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let occ = scene.voxel_occupancy.as_ref().filter(|_| scene.gpu_particles.iter().any(|p| p.collide.voxels)).map(|o| o.read().unwrap().clone());
                let (grid, grid_o, grid_n) = match &occ {
                    Some(o) => (buf("voxel occupancy", bytemuck::cast_slice(&o.words), edit), [o.origin[0], o.origin[1], o.origin[2], o.cell], [o.dims[0], o.dims[1], o.dims[2], o.row_words() as u32]),
                    None => (buf("voxel occupancy", &[0u8; 16], edit), [0.0; 4], [0; 4]),
                };
                if let Some(o) = &scene.voxel_occupancy {
                    o.write().unwrap().dirty.clear();
                }
                let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gpu particles"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[e(0, &params), e(1, &table), e(2, &b_verts), e(3, &b_prev), e(4, &a.verts), e(5, &b_objects), e(6, &b_attrs), e(7, &last), e(8, &state), e(9, &spawns), e(10, &grid)],
                });
                let host = scene.gpu_particles.iter().map(|ps| (0u32, 0.0f32, ps.rate)).collect();
                let sm = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("stamp"), source: wgpu::ShaderSource::Wgsl(STAMP_WGSL.into()) });
                let sp = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("stamp"), layout: None, module: &sm, entry_point: Some("stamp"), compilation_options: Default::default(), cache: self.cache() });
                let sbg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("stamp"), layout: &sp.get_bind_group_layout(0), entries: &[e(0, &a.verts), wgpu::BindGroupEntry { binding: 1, resource: a.tlas.as_binding() }] });
                Some(GpuParticlesGpu { pipeline, stamp: (sp, sbg), bg, params, table, spawns, grid, systems: std::cell::RefCell::new(systems), host: std::cell::RefCell::new(host), nodes: scene.gpu_particles.iter().map(|p| p.node).collect(), max_cap: scene.gpu_particles.iter().map(|p| p.capacity).max().unwrap_or(0), frame: std::cell::Cell::new(0), fresh: std::cell::Cell::new(true), grid_o, grid_n })
            }
            _ => None,
        };
        self.queue.submit([]);
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        timings.push(("upload_ms".into(), t0.elapsed().as_secs_f64() * 1000.0));
        let bg_parts = BindParts { nodes: b_nodes, energy: b_energy, texinfo: b_texinfo, tex_view, env_view, sampler, grid_view };
        // (The scene's own pipelines, made above: particles, grid programs.)
        self.save_pipeline_cache();
        Ok(Uploaded { bg, bg_parts, tables, uniform, accum: b_accum, params: b_params, width: w, height: h, kernel, materials: b_mats, lights: b_lights, objects: b_objects, emitters: b_emit, verts: b_verts, attrs: b_attrs, prev_verts: b_prev, dynamic_first, accel, units: scene.units.clone(), particles, gpu_particles, grids, translucent, media: b_media, vcolor_base: (!scene.gpu_vcolors.is_empty()).then_some(scene.vcolor_base as u64 * 16), media_nodes: scene.media_nodes.clone(), media_hidden: std::cell::RefCell::new(scene.media.iter().map(|m| m.a[1] == crate::scene::MEDIUM_HIDDEN).collect()), instances: b_instances, room, cdf: b_cdf, cdf_words: env_cdf.len(), region_words, env_tex })
    }

    /// Bytes Metal allocates for an AABB BLAS over `count` boxes in `boxes` (its own size query through wgpu-hal; None
    /// on other backends).
    fn aabb_blas_bytes(&self, boxes: &wgpu::Buffer, count: u32) -> Option<u64> {
        #[cfg(target_vendor = "apple")]
        {
            use wgpu::hal::{api::Metal, Device as _};
            let d = unsafe { self.device.as_hal::<Metal>() }?;
            let b = unsafe { boxes.as_hal::<Metal>() }?;
            let entries = wgpu::hal::AccelerationStructureEntries::AABBs(vec![wgpu::hal::AccelerationStructureAABBs { buffer: Some(&*b), offset: 0, count, stride: 24, flags: wgpu::hal::AccelerationStructureGeometryFlags::OPAQUE }]);
            let s = unsafe { d.get_acceleration_structure_build_sizes(&wgpu::hal::GetAccelerationStructureBuildSizesDescriptor { entries: &entries, flags: wgpu::hal::AccelerationStructureBuildFlags::PREFER_FAST_TRACE }) };
            return Some(s.acceleration_structure_size);
        }
        #[allow(unreachable_code)]
        {
            let _ = (boxes, count);
            None
        }
    }

    /// Bytes Metal allocates for a triangle BLAS over the (first vertex, vertex count, opaque) geometries of `verts`
    /// (non-indexed Float32x3, as every triangle BLAS here; None on other backends).
    fn tri_blas_bytes(&self, verts: &wgpu::Buffer, parts: &[(u32, u32, bool)]) -> Option<u64> {
        #[cfg(target_vendor = "apple")]
        {
            use wgpu::hal::{api::Metal, Device as _};
            let d = unsafe { self.device.as_hal::<Metal>() }?;
            let b = unsafe { verts.as_hal::<Metal>() }?;
            let entries = wgpu::hal::AccelerationStructureEntries::Triangles(
                parts
                    .iter()
                    .map(|&(first_vertex, vertex_count, opaque)| wgpu::hal::AccelerationStructureTriangles {
                        vertex_buffer: Some(&*b),
                        vertex_format: wgpu::VertexFormat::Float32x3,
                        first_vertex,
                        vertex_count,
                        vertex_stride: 12,
                        indices: None,
                        transform: None,
                        flags: if opaque { wgpu::hal::AccelerationStructureGeometryFlags::OPAQUE } else { wgpu::hal::AccelerationStructureGeometryFlags::empty() },
                    })
                    .collect(),
            );
            let s = unsafe { d.get_acceleration_structure_build_sizes(&wgpu::hal::GetAccelerationStructureBuildSizesDescriptor { entries: &entries, flags: wgpu::hal::AccelerationStructureBuildFlags::PREFER_FAST_TRACE }) };
            return Some(s.acceleration_structure_size);
        }
        #[allow(unreachable_code)]
        {
            let _ = (verts, parts);
            None
        }
    }

    /// The `env_cdf` binding of a scene with procedural geometry: the environment CDF, then the procedural region
    /// (scene::ProcLayout: headers and materials, then the geometries' blobs). Written straight into the new buffer's
    /// mapping; the blobs are copied on the GPU from the last upload when its geometries are the same.
    /// `room`: words of edit room wanted after the region (`EditRoom`); returns the buffer, the region's words in use
    /// and the room it got (as much as the binding limit leaves).
    fn upload_procs(&self, env_cdf: &[f32], l: &crate::scene::ProcLayout, room: usize, timings: &mut Vec<(String, f64)>) -> Result<(wgpu::Buffer, usize, usize)> {
        let t = Instant::now();
        let (cdf, head) = (env_cdf.len(), l.head.len());
        let used = ((cdf + head + l.blob_words).max(1) * 4) as u64;
        let limit = self.device.limits().max_storage_buffer_binding_size as u64;
        if used > limit {
            return Err(anyhow!("procedural geometry: {} MB of data, more than this device binds in one buffer ({} MB)", used >> 20, limit >> 20));
        }
        let room = room.min(((limit - used) / 4) as usize);
        let bytes = used + room as u64 * 4;
        let key = l.key();
        let blob_at = ((cdf + head) * 4) as u64;
        let blob_bytes = (l.blob_words * 4) as u64;
        let mut cache = self.proc_cache.lock().unwrap();
        let reuse = cache.as_ref().filter(|c| c.key == key && c.bytes == blob_bytes);
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        // Reused blobs: the small parts written through the queue, the blobs copied on the GPU (a mapped buffer of the
        // whole size would be zero-filled on the CPU first: 17 ms for 256 MB). New ones: written into the mapping.
        let b = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("env_cdf + procedural"), size: bytes, usage, mapped_at_creation: reuse.is_none() });
        if let Some(c) = reuse {
            self.queue.write_buffer(&b, 0, bytemuck::cast_slice(env_cdf));
            self.queue.write_buffer(&b, (cdf * 4) as u64, bytemuck::cast_slice(&l.head));
            if c.bytes > 0 {
                let mut e = self.device.create_command_encoder(&Default::default());
                e.copy_buffer_to_buffer(&c.buf, c.offset, &b, blob_at, c.bytes);
                self.queue.submit([e.finish()]);
            }
        } else {
            {
                let mut m = b.slice(..).get_mapped_range_mut().map_err(|e| anyhow!("procedural data: {e:?}"))?;
                // (Mapped memory is write-only here: byte ranges written whole.)
                let mut put = |word: usize, bytes: &[u8]| m.slice(word * 4..word * 4 + bytes.len()).copy_from_slice(bytes);
                put(0, bytemuck::cast_slice(env_cdf));
                put(cdf, bytemuck::cast_slice(&l.head));
                for (blob, off) in &l.blobs {
                    let o = cdf + head + off;
                    put(o, bytemuck::cast_slice(&blob.boxes));
                    put(o + blob.boxes.len(), bytemuck::cast_slice(&blob.data));
                    if l.sw {
                        let (nodes, order) = blob.sw_bvh();
                        let o = o + blob.boxes.len() + blob.data.len();
                        put(o, bytemuck::cast_slice(nodes));
                        put(o + nodes.len(), bytemuck::cast_slice(order));
                    }
                }
            }
            b.unmap();
        }
        timings.push((if reuse.is_some() { "proc_copy_ms" } else { "proc_upload_ms" }.into(), t.elapsed().as_secs_f64() * 1000.0));
        // The procedural region's GPU bytes (headers, materials, blobs: boxes and data, once per distinct geometry).
        timings.push(("proc_data_mb".into(), ((head + l.blob_words) * 4) as f64 / (1 << 20) as f64));
        *cache = Some(ProcCache { key, buf: b.clone(), offset: blob_at, bytes: blob_bytes });
        Ok((b, head + l.blob_words, room))
    }

    /// The particle pass (scene time, camera, frame time, rain, wetness), then the dynamic BLASes and the TLAS rebuilt.
    /// `pass_ts`: timestamps (q, q + 1) around the particle pass; `blas_ts`: (q, q + 1) around the BLAS build (the end of
    /// an empty pass before it, the beginning of one after it).
    #[allow(clippy::too_many_arguments)]
    pub fn run_particles(&self, up: &Uploaded, cam: [f32; 3], fwd: [f32; 3], time: f32, dt: f32, rain: f32, wetness: f32, e: &mut wgpu::CommandEncoder, pass_ts: Option<(&wgpu::QuerySet, u32)>, blas_ts: Option<(&wgpu::QuerySet, u32)>) {
        let (Some(p), Some(accel)) = (&up.particles, &up.accel) else { return };
        // Systems hidden in place: collapsed by last frame's pass, now idle (count 0); every system idle: no pass, no
        // BLAS builds (their quads stay collapsed).
        {
            let mut hidden = p.hidden.borrow_mut();
            for (k, h) in hidden.iter_mut().enumerate() {
                match *h {
                    Hidden::Collapse => *h = Hidden::Collapsed,
                    Hidden::Collapsed => {
                        self.write_particle_count(p, k, 0, 0.0);
                        *h = Hidden::Idle;
                    }
                    Hidden::Showing => *h = Hidden::ShowRan,
                    Hidden::ShowRan => {
                        self.write_particle_count(p, k, p.counts[k], 0.0);
                        *h = Hidden::Shown;
                    }
                    _ => {}
                }
            }
            if hidden.iter().all(|h| *h == Hidden::Idle) {
                // (The stateful GPU particles leave the dynamic BLAS rebuild to this pass when there are stateless ones.)
                if up.gpu_particles.is_some() {
                    self.rebuild_dynamic(accel, e);
                }
                return;
            }
        }
        let first = if p.fresh.replace(false) { 1.0 } else { 0.0 };
        let v = [cam[0], cam[1], cam[2], time, fwd[0], fwd[1], fwd[2], dt, rain, wetness, first, 0.0];
        self.queue.write_buffer(&p.params, 0, bytemuck::cast_slice(&v));
        {
            let tw = pass_ts.map(|(q, i)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) });
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("particles"), timestamp_writes: tw });
            pass.set_pipeline(&p.pipeline);
            pass.set_bind_group(0, &p.bg, &[]);
            pass.dispatch_workgroups(p.max_count.div_ceil(64), p.systems, 1);
        }
        let stamp = |e: &mut wgpu::CommandEncoder, tw: wgpu::ComputePassTimestampWrites| {
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("stamp"), timestamp_writes: Some(tw) });
            pass.set_pipeline(&p.stamp.0);
            pass.set_bind_group(0, &p.stamp.1, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        };
        if let Some((q, i)) = blas_ts {
            stamp(e, wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(i) });
        }
        if p.opaque {
            self.rebuild_dynamic(accel, e);
        }
        if let Some(t) = &up.translucent {
            t.build(e);
        }
        if let Some((q, i)) = blas_ts {
            stamp(e, wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i + 1), end_of_pass_write_index: None });
        }
    }

    /// One frame of the stateful GPU particles (gpu_particles.wgsl): each system's emissions (its rate over `dt`, and the
    /// game's spawns, `spawns` per node, 8 floats each), the update, collision and retirement of the rest, their quads;
    /// with `rebuild`, the dynamic BLASes after (else the stateless pass that follows rebuilds them).
    #[allow(clippy::too_many_arguments)]
    pub fn run_gpu_particles(&self, up: &Uploaded, cam: [f32; 3], fwd: [f32; 3], time: f32, dt: f32, spawns: &std::collections::BTreeMap<u32, Vec<f32>>, rebuild: bool, e: &mut wgpu::CommandEncoder, ts: Option<(&wgpu::QuerySet, u32)>, blas_ts: Option<(&wgpu::QuerySet, u32)>) {
        let (Some(p), Some(accel)) = (&up.gpu_particles, &up.accel) else { return };
        let first = if p.fresh.replace(false) { 1.0 } else { 0.0 };
        let frame = p.frame.get().wrapping_add(1);
        p.frame.set(frame);
        let mut pp = [0u32; 20];
        let f = |v: f32| v.to_bits();
        pp[..12].copy_from_slice(&[f(cam[0]), f(cam[1]), f(cam[2]), f(time), f(fwd[0]), f(fwd[1]), f(fwd[2]), f(dt), f(first), 0, 0, 0]);
        pp[12..16].copy_from_slice(&p.grid_o.map(f));
        pp[16..20].copy_from_slice(&p.grid_n);
        self.queue.write_buffer(&p.params, 0, bytemuck::cast_slice(&pp));
        let mut systems = p.systems.borrow_mut();
        let mut host = p.host.borrow_mut();
        let mut data: Vec<f32> = vec![];
        for (k, s) in systems.iter_mut().enumerate() {
            let cap = s.a[1];
            let (cursor, carry, rate) = &mut host[k];
            let want = *carry + *rate * dt.max(0.0);
            let from_rate = want.floor();
            *carry = want - from_rate;
            let room = MAX_SPAWNS - data.len() / 8;
            let given = spawns.get(&p.nodes[k]).map_or(&[][..], |v| v.as_slice());
            let spawned = (given.len() / 8).min(room).min(cap as usize);
            let first_spawn = (data.len() / 8) as u32;
            data.extend_from_slice(&given[..spawned * 8]);
            let emitted = (spawned as u32 + from_rate as u32).min(cap);
            s.b[2] = *cursor;
            s.b[3] = emitted;
            s.e = [first_spawn, spawned as u32, frame, 0];
            *cursor = (*cursor + emitted) % cap.max(1);
        }
        if !data.is_empty() {
            self.queue.write_buffer(&p.spawns, 0, bytemuck::cast_slice(&data));
        }
        self.queue.write_buffer(&p.table, 0, bytemuck::cast_slice(&systems));
        {
            let tw = ts.map(|(q, i)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) });
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gpu particles"), timestamp_writes: tw });
            pass.set_pipeline(&p.pipeline);
            pass.set_bind_group(0, &p.bg, &[]);
            pass.dispatch_workgroups(p.max_cap.div_ceil(64), systems.len() as u32, 1);
        }
        if rebuild {
            let stamp = |e: &mut wgpu::CommandEncoder, tw: wgpu::ComputePassTimestampWrites| {
                let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("stamp"), timestamp_writes: Some(tw) });
                pass.set_pipeline(&p.stamp.0);
                pass.set_bind_group(0, &p.stamp.1, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            };
            if let Some((q, i)) = blas_ts {
                stamp(e, wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(i) });
            }
            self.rebuild_dynamic(accel, e);
            if let Some((q, i)) = blas_ts {
                stamp(e, wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i + 1), end_of_pass_write_index: None });
            }
        }
    }

    /// A grid's data (four f32 a cell) into the atlas, from layer `z0`.
    fn write_grid(&self, atlas: &wgpu::Texture, z0: u32, dims: [u32; 3], data: &[f32]) {
        let halves = grid_halves(data);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: atlas, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z: z0 }, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&halves),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(dims[0] * 8), rows_per_image: Some(dims[1]) },
            wgpu::Extent3d { width: dims[0], height: dims[1], depth_or_array_layers: dims[2] },
        );
    }

    /// What the grid programs have simulated, from the scene before a rebuild into the one after: for each grid with a
    /// program that is the same grid still (`world::DensityGrid::made`, its size), its field A (the atlas's layers)
    /// and field B copied on the GPU, and the programs' step count. Without it a rebuild started every simulated grid
    /// again from its data: any structural change in a game (an object hidden, a texture) wiped its smoke.
    pub fn carry_grids(&self, old: &Uploaded, new: &Uploaded) {
        let (Some(from), Some(to)) = (&old.grids, &new.grids) else { return };
        let mut e = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("carry grids") });
        let mut any = false;
        for g in &to.grids {
            let Some(sim) = &g.sim else { continue };
            let Some((o, osim)) = from.grids.iter().find(|o| o.id == g.id && o.made == g.made && o.dims == g.dims).and_then(|o| o.sim.as_ref().map(|s| (o, s))) else { continue };
            let size = wgpu::Extent3d { width: g.dims[0], height: g.dims[1], depth_or_array_layers: g.dims[2] };
            let at = |texture, z| wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z }, aspect: wgpu::TextureAspect::All };
            e.copy_texture_to_texture(at(&from.atlas, o.z0), at(&to.atlas, g.z0), size);
            e.copy_texture_to_texture(at(&osim.b, 0), at(&sim.b, 0), size);
            any = true;
        }
        if any {
            to.step.set(from.step.get());
            self.queue.submit([e.finish()]);
        }
    }

    /// A density grid's new data (the game's: `World::set_grid`), uploaded in place.
    pub fn upload_grid(&self, up: &Uploaded, id: u32, data: &[f32]) {
        let Some(gs) = &up.grids else { return };
        for g in gs.grids.iter().filter(|g| g.id == id && data.len() == (g.dims[0] * g.dims[1] * g.dims[2] * 4) as usize) {
            self.write_grid(&gs.atlas, g.z0, g.dims, data);
        }
    }

    /// One frame of the grid programs (grid_sim.wgsl): for each grid with one, two half steps of `dt` / 2, the first
    /// from the atlas (field A) and its B into A' and B', the second back; `sources` per volume node (12 floats each)
    /// for the first. `ts`: four timestamps from that query on, each pass's beginning and end (Metal writes a pass's in pairs).
    pub fn run_grids(&self, up: &Uploaded, time: f32, dt: f32, sources: &std::collections::BTreeMap<u32, Vec<f32>>, e: &mut wgpu::CommandEncoder, ts: Option<(&wgpu::QuerySet, u32)>) {
        let Some(gs) = &up.grids else { return };
        let Some(pipeline) = &gs.pipeline else { return };
        let n = gs.step.get();
        gs.step.set(n.wrapping_add(2));
        for g in &gs.grids {
            let Some(sim) = &g.sim else { continue };
            let given = sources.get(&g.node).map_or(&[][..], |v| v.as_slice());
            let count = (given.len() / 12).min(crate::world::GRID_MAX_SOURCES);
            if count > 0 {
                self.queue.write_buffer(&sim.sources, 0, bytemuck::cast_slice(&given[..count * 12]));
            }
            let (w2l, custom) = sim.placed.get();
            let world = glam::Mat4::from_cols_array_2d(&[w2l[0], w2l[1], w2l[2], [0.0, 0.0, 0.0, 1.0]]).transpose().inverse();
            let l2w = [world.row(0).to_array(), world.row(1).to_array(), world.row(2).to_array()];
            let cell = [world.x_axis.truncate().length() / g.dims[0] as f32, world.y_axis.truncate().length() / g.dims[1] as f32, world.z_axis.truncate().length() / g.dims[2] as f32, 0.0];
            for (k, (u, _)) in sim.steps.iter().enumerate() {
                let half = dt.max(0.0) * 0.5;
                let (src, dst) = if k == 0 { (g.z0, 0) } else { (0, g.z0) };
                // (The sources go to the first half step alone: a program that adds what a source gives adds it once.)
                let step = GridStepGpu { l2w, w2l, dims: [g.dims[0], g.dims[1], g.dims[2], if k == 0 { count as u32 } else { 0 }], at: [src, dst, sim.program, n.wrapping_add(k as u32)], t: [time + half * k as f32, half, 0.0, 0.0], cell, custom, p: sim.params };
                self.queue.write_buffer(u, 0, bytemuck::bytes_of(&step));
            }
        }
        for k in 0..2 {
            let tw = ts.map(|(q, i)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i + 2 * k as u32), end_of_pass_write_index: Some(i + 2 * k as u32 + 1) });
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("grid sim"), timestamp_writes: tw });
            pass.set_pipeline(pipeline);
            for g in &gs.grids {
                let Some(sim) = &g.sim else { continue };
                pass.set_bind_group(0, &sim.steps[k].1, &[]);
                pass.dispatch_workgroups(g.dims[0].div_ceil(4), g.dims[1].div_ceil(4), g.dims[2].div_ceil(4));
            }
        }
    }

    /// Whether the scene has a grid program to run every frame.
    pub fn has_grid_sim(up: &Uploaded) -> bool {
        up.grids.as_ref().is_some_and(|g| g.pipeline.is_some())
    }

    /// The voxel occupancy grid's changed words (`Occupancy::dirty`), uploaded.
    pub fn upload_occupancy(&self, up: &Uploaded, occ: &mut crate::world::Occupancy) {
        let Some(p) = &up.gpu_particles else { return };
        if p.grid_n[0] == 0 {
            occ.dirty.clear();
            return;
        }
        for (a, b) in std::mem::take(&mut occ.dirty) {
            let b = b.min(occ.words.len());
            if a < b {
                self.queue.write_buffer(&p.grid, (a * 4) as u64, bytemuck::cast_slice(&occ.words[a..b]));
            }
        }
    }

    /// A stateful particle node moved: its pivot (`i.pivot`; where its rate's particles are born), from the next pass.
    pub fn set_gpu_particle_pivot(&self, up: &Uploaded, node: u32, pivot: [f32; 3]) {
        let Some(p) = &up.gpu_particles else { return };
        let mut systems = p.systems.borrow_mut();
        for (k, _) in p.nodes.iter().enumerate().filter(|(_, n)| **n == node) {
            let w = systems[k].c[3];
            systems[k].c = [pivot[0], pivot[1], pivot[2], w];
        }
    }

    /// A particle node moved: its systems' pivot (`i.pivot`; c.w, the collapse flag, kept).
    pub fn set_particle_pivot(&self, up: &Uploaded, node: u32, pivot: [f32; 3]) {
        let Some(p) = &up.particles else { return };
        for (k, _) in p.nodes.iter().enumerate().filter(|(_, n)| **n == node) {
            let at = (k * std::mem::size_of::<ParticleSystemGpu>() + 32) as u64;
            self.queue.write_buffer(&p.table, at, bytemuck::cast_slice(&pivot));
        }
    }

    /// System `k`'s particle count and flag in the table (a.y; c.w: 1 collapse, 2 first run, particles.wgsl).
    fn write_particle_count(&self, p: &ParticleGpu, k: usize, count: u32, flag: f32) {
        let at = (k * std::mem::size_of::<ParticleSystemGpu>()) as u64;
        self.queue.write_buffer(&p.table, at + 4, bytemuck::bytes_of(&count));
        self.queue.write_buffer(&p.table, at + 44, bytemuck::bytes_of(&flag));
    }

    /// A particle node shown or hidden by its own `visible`, in place: hidden, its systems' next pass collapses their
    /// quads and then they run nothing (`run_particles`); shown, they run again. False when no system of the build is
    /// that node's (the build left it out: a rebuild).
    pub fn set_particles_hidden(&self, up: &Uploaded, node: u32, hide: bool) -> bool {
        let Some(p) = &up.particles else { return false };
        let mut hidden = p.hidden.borrow_mut();
        let mut found = false;
        for (k, _) in p.nodes.iter().enumerate().filter(|(_, n)| **n == node) {
            found = true;
            let shown = matches!(hidden[k], Hidden::Shown | Hidden::Showing | Hidden::ShowRan);
            if hide && shown {
                self.write_particle_count(p, k, p.counts[k], 1.0);
                hidden[k] = Hidden::Collapse;
            } else if !hide && !shown {
                // (Its first run since: no last frame for its motion vectors.)
                self.write_particle_count(p, k, p.counts[k], 2.0);
                hidden[k] = Hidden::Showing;
            }
        }
        found
    }

    /// Whether the translucency pass has anything to draw: a translucent system shown (or collapsing).
    pub fn translucent_particles_live(&self, up: &Uploaded) -> bool {
        up.particles.as_ref().is_none_or(|p| p.translucent.iter().zip(p.hidden.borrow().iter()).any(|(t, h)| *t && *h != Hidden::Idle))
    }

    /// A particle node's customData changed: its systems' `i.custom`.
    pub fn set_particle_custom(&self, up: &Uploaded, node: u32, custom: [f32; 4]) {
        let Some(p) = &up.particles else { return };
        for (k, _) in p.nodes.iter().enumerate().filter(|(_, n)| **n == node) {
            let at = (k * std::mem::size_of::<ParticleSystemGpu>() + std::mem::offset_of!(ParticleSystemGpu, d)) as u64;
            self.queue.write_buffer(&p.table, at, bytemuck::cast_slice(&custom));
        }
    }

    /// A volume node moved or changed its customData: its medium's world -> local rows / `custom` (None: unchanged).
    /// Replaces the environment map and its sampling CDF in place (a dynamic sky's new bake). Both keep their sizes:
    /// the CDF heads the binding it shares with the procedural region, which follows it.
    pub fn write_env(&self, up: &Uploaded, env: &EnvMap, cdf: &[f32]) {
        let size = up.env_tex.size();
        if (env.width, env.height) != (size.width, size.height) || cdf.len() != up.cdf_words {
            return;
        }
        let half: Vec<u16> = env.texels.iter().flat_map(|t| t.iter().map(|v| half::f16::from_f32(v.min(65000.0)).to_bits())).collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &up.env_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&half),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(env.width * 8), rows_per_image: Some(env.height) },
            size,
        );
        self.queue.write_buffer(&up.cdf, 0, bytemuck::cast_slice(cdf));
    }

    /// A volume node shown or hidden by its own `visible`, in place: its media keep their place in the table with
    /// their shape (`shape`: the node's) or none (pathtrace.wgsl `medium_span`: no ray crosses it). False when no
    /// medium of the build is that node's (the build left it out: a rebuild).
    pub fn set_medium_hidden(&self, up: &Uploaded, node: u32, shape: u32, hide: bool) -> bool {
        let size = std::mem::size_of::<crate::scene::MediumGpu>();
        let mut hidden = up.media_hidden.borrow_mut();
        let mut found = false;
        for (k, _) in up.media_nodes.iter().enumerate().filter(|(_, n)| **n == node) {
            found = true;
            hidden[k] = hide;
            let at = k * size + std::mem::offset_of!(crate::scene::MediumGpu, a) + 4;
            self.queue.write_buffer(&up.media, at as u64, bytemuck::bytes_of(&if hide { crate::scene::MEDIUM_HIDDEN } else { shape }));
        }
        found
    }

    pub fn set_medium(&self, up: &Uploaded, node: u32, rows: Option<[[f32; 4]; 3]>, custom: Option<[f32; 4]>) {
        // (Its grid program runs in the node's box as it is now.)
        for sim in up.grids.iter().flat_map(|g| g.grids.iter()).filter(|g| g.node == node).filter_map(|g| g.sim.as_ref()) {
            let (r, c) = sim.placed.get();
            sim.placed.set((rows.unwrap_or(r), custom.unwrap_or(c)));
        }
        let size = std::mem::size_of::<crate::scene::MediumGpu>();
        for (k, _) in up.media_nodes.iter().enumerate().filter(|(_, n)| **n == node) {
            if let Some(r) = rows {
                self.queue.write_buffer(&up.media, (k * size) as u64, bytemuck::cast_slice(&r));
            }
            if let Some(c) = custom {
                self.queue.write_buffer(&up.media, (k * size + std::mem::offset_of!(crate::scene::MediumGpu, c)) as u64, bytemuck::cast_slice(&c));
            }
        }
    }

    /// Rebuilds the BLASes of the dynamic groups (posed models, after their vertices changed) and the TLAS.
    /// Rebuilds the TLAS alone (instance transforms changed, no triangles).
    /// `old`'s contents in a new buffer of `size` bytes (its usage and label), copied on the GPU and submitted now, so
    /// writes queued after this land on the copy (live/edits.rs `edit_geometry` growing past its room).
    pub fn grown(&self, old: &wgpu::Buffer, size: u64, label: &str) -> wgpu::Buffer {
        let b = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: old.usage(), mapped_at_creation: false });
        let mut e = self.device.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(old, 0, &b, 0, old.size().min(size));
        self.queue.submit([e.finish()]);
        b
    }

    /// The TLAS again with room for `max` instances, its instances as they are (built with the edit's next build).
    pub fn grow_tlas(&self, accel: &mut Accel, max: usize) {
        let mut t = self.device.create_tlas(&wgpu::CreateTlasDescriptor { label: None, max_instances: max as u32, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });
        for (i, inst) in accel.tlas.get().iter().enumerate() {
            if let Some(inst) = inst {
                t[i] = Some(inst.clone());
            }
        }
        accel.tlas = t;
    }

    /// The BLAS vertex buffer grown to `bytes` (`grown`).
    pub fn grow_blas_verts(&self, accel: &mut Accel, bytes: u64) {
        accel.verts = self.grown(&accel.verts, bytes, "blas_verts");
    }

    /// Mesh `k` (one an edit no longer draws) made again over `count` triangles from `first`: a new BLAS in its slot.
    pub fn set_mesh(&self, accel: &mut Accel, k: u32, first: usize, count: u32, opaque: u32) {
        let (blas, geo) = self.mesh_blas_for(accel, first, count, opaque);
        accel.meshes[k as usize] = blas;
        accel.mesh_geo[k as usize] = geo;
    }

    fn mesh_blas_for(&self, accel: &Accel, first: usize, count: u32, opaque: u32) -> (wgpu::Blas, (Vec<(u32, u32, bool)>, Vec<wgpu::BlasTriangleGeometrySizeDescriptor>)) {
        let rel = (first - accel.pool_first) as u32;
        let parts: Vec<(u32, u32, bool)> = [(rel, opaque, true), (rel + opaque, count - opaque, false)].into_iter().filter(|p| p.1 > 0).collect();
        let sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor> = parts
            .iter()
            .map(|&(_, c, opaque)| wgpu::BlasTriangleGeometrySizeDescriptor {
                vertex_format: wgpu::VertexFormat::Float32x3,
                vertex_count: c * 3,
                index_format: None,
                index_count: None,
                flags: if opaque { wgpu::AccelerationStructureGeometryFlags::OPAQUE } else { wgpu::AccelerationStructureGeometryFlags::empty() },
            })
            .collect();
        let flags = if std::env::var("THREERT_EDIT_BLAS").is_ok_and(|v| v == "trace") { wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE } else { wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD };
        let blas = self.device.create_blas(&wgpu::CreateBlasDescriptor { label: Some("edit mesh"), flags, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: sizes.clone() });
        (blas, (parts, sizes))
    }

    /// A new instanced mesh over `count` triangles from GPU-order index `first` (in the edit room; live/edits.rs
    /// `edit_geometry`), its first `opaque` ones opaque: its BLAS, which `rebuild_meshes` builds. Returns its index.
    pub fn add_mesh(&self, accel: &mut Accel, first: usize, count: u32, opaque: u32) -> u32 {
        // Built fast rather than traced fast (edits come every frame under destruction; the batches of pieces are rebuilt
        // every frame); THREERT_EDIT_BLAS=trace for the A/B (`mesh_blas_for`).
        let (blas, geo) = self.mesh_blas_for(accel, first, count, opaque);
        accel.meshes.push(blas);
        accel.mesh_geo.push(geo);
        accel.meshes.len() as u32 - 1
    }

    /// Instanced mesh `k`'s BLAS (a TLAS instance of it).
    pub fn mesh_blas(accel: &Accel, k: u32) -> &wgpu::Blas {
        &accel.meshes[k as usize]
    }

    /// The AABB BLAS of a procedural geometry's boxes (live/edits.rs `edit_geometry`): the one kept by its hash or built
    /// earlier in this edit (`fresh`), or a new one whose build goes into `e` and into `fresh`. The edit keeps `fresh`
    /// (`keep_proc_blases`) only once `e` is submitted: an edit that falls back never submits, and a BLAS never built
    /// must not be reused.
    /// Its BLAS (kept by hash, or made now: its build added to `pending`, which `build_meshes` builds with the TLAS).
    pub fn proc_blas(&self, blob: &crate::procasset::ProcBlob, pending: &mut Vec<PendingProc>, fresh: &mut Vec<([u8; 32], wgpu::Blas, u64)>) -> wgpu::Blas {
        if let Some((blas, _)) = self.proc_blases.lock().unwrap().get(&blob.hash) {
            return blas.clone();
        }
        if let Some((_, blas, _)) = fresh.iter().find(|f| f.0 == blob.hash) {
            return blas.clone();
        }
        let d = &self.device;
        let size = wgpu::BlasAABBGeometrySizeDescriptor { primitive_count: (blob.boxes.len() / 6) as u32, flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE };
        let blas = d.create_blas(&wgpu::CreateBlasDescriptor { label: Some("proc"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::AABBs { descriptors: vec![size.clone()] });
        let boxes = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("proc boxes"), contents: bytemuck::cast_slice(&blob.boxes), usage: wgpu::BufferUsages::BLAS_INPUT });
        let bytes = self.aabb_blas_bytes(&boxes, size.primitive_count).unwrap_or(0);
        fresh.push((blob.hash, blas.clone(), bytes));
        pending.push(PendingProc { blas: blas.clone(), boxes, size });
        blas
    }

    /// Keeps the procedural BLASes an edit built (`proc_blas`), once its builds are submitted.
    pub fn keep_proc_blases(&self, fresh: Vec<([u8; 32], wgpu::Blas, u64)>) {
        let mut built = self.proc_blases.lock().unwrap();
        for (hash, blas, bytes) in fresh {
            built.insert(hash, (blas, bytes));
        }
    }

    /// The scene bind group again, from `up`'s buffers as they are now (a buffer that grew: live/edits.rs `write_emitters`).
    pub fn rebind_scene(&self, up: &mut Uploaded) {
        fn e(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
        }
        let p = &up.bg_parts;
        let mut entries = vec![
            e(0, &up.params),
            e(1, &up.verts),
            e(2, &up.attrs),
            e(3, &p.nodes),
            e(4, &up.materials),
            e(5, &up.lights),
            e(6, &up.emitters),
            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&p.tex_view) },
            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&p.sampler) },
            wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&p.env_view) },
            e(10, &up.cdf),
            e(11, &up.accum),
            e(12, &p.energy),
            e(14, &p.texinfo),
            e(15, &up.objects),
            e(16, &up.prev_verts),
            e(17, &up.instances),
            e(18, &up.media),
            wgpu::BindGroupEntry { binding: 19, resource: wgpu::BindingResource::TextureView(&p.grid_view) },
        ];
        if let Some(a) = &up.accel {
            entries.push(wgpu::BindGroupEntry { binding: 13, resource: a.tlas.as_binding() });
        }
        // The merged layout: the tables' buffer, a larger one if a table outgrew its array (the kernel's source then
        // has the new lengths: compiled on its next use).
        let now: [u32; 7] = std::array::from_fn(|k| crate::merged::table_cap(k, up.table_buffers()[k].size()));
        let tables = match (&up.tables, &up.kernel.caps) {
            (Some(t), Some(caps)) if now == caps.tables => Some(t.clone()),
            (Some(_), Some(_)) => Some(self.tables_buffer(up.table_buffers()).0),
            _ => None,
        };
        if let Some(t) = &tables {
            entries.retain(|e| !MERGED_TABLE_BINDINGS.contains(&e.binding));
            entries.push(e(crate::merged::TABLES_BINDING, t));
        }
        // The uniform tables: the same copies (their size does not follow a table's: merged.rs `UNIFORM_LEN`).
        if let Some(u) = &up.uniform {
            bind_uniform_tables(&mut entries, u);
        }
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.layout, entries: &entries });
        drop(entries);
        up.bg = bg;
        if let (Some(_), Some(caps)) = (&tables, &mut up.kernel.caps) {
            caps.tables = now;
        }
        up.tables = tables;
    }

    /// The loop guard's count as the GPU has it now (None: `kernel` has no guard): the invocations it stopped since
    /// the buffers were made. Waits for the GPU.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn guard_trips(&self, bufs: &KernelBufs, kernel: &Kernel) -> Result<Option<u32>> {
        let Some(g) = bufs.guard.as_ref().filter(|_| loop_guard_of(kernel.features & F_UNITS != 0).is_some()) else { return Ok(None) };
        let read = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("loop guard read"), size: 4, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut e = self.device.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(&g.buffer, g.offset, &read, 0, 4);
        self.queue.submit([e.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        let n = u32::from_le_bytes(read.slice(..).get_mapped_range()?[..4].try_into().expect("four bytes"));
        Ok(Some(n))
    }

    /// The buffer of the merged layout's `MergedTables` for these seven tables (merged.rs `TABLES`' order), and its
    /// arrays' lengths.
    fn tables_buffer(&self, tables: [&wgpu::Buffer; 7]) -> (wgpu::Buffer, [u32; 7]) {
        let caps: [u32; 7] = std::array::from_fn(|k| crate::merged::table_cap(k, tables[k].size()));
        let (_, size) = crate::merged::offsets(&crate::merged::TABLES, &caps);
        (self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("merged tables"), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }), caps)
    }

    /// The uniform tables' buffers for these four tables (merged.rs `UNIFORM_TABLES`' order), filled from them as they
    /// are now.
    fn uniform_copies(&self, tables: [&wgpu::Buffer; 4]) -> [wgpu::Buffer; 4] {
        use crate::merged::{TABLES, UNIFORM_LEN, UNIFORM_TABLES};
        let copies: [wgpu::Buffer; 4] = std::array::from_fn(|k| {
            let m = &TABLES[UNIFORM_TABLES[k]];
            // (COPY_SRC: read back by the test that holds them to the tables, and by whoever diagnoses a driver.)
            self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(m.name), size: m.stride * UNIFORM_LEN[k] as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false })
        });
        let mut e = self.device.create_command_encoder(&Default::default());
        Self::copy_uniform(&mut e, tables, &copies);
        self.queue.submit([e.finish()]);
        copies
    }

    fn copy_uniform(e: &mut wgpu::CommandEncoder, tables: [&wgpu::Buffer; 4], copies: &[wgpu::Buffer; 4]) {
        for (src, dst) in tables.into_iter().zip(copies) {
            let bytes = uniform_copy_bytes(src.size(), dst.size());
            if bytes > 0 {
                e.copy_buffer_to_buffer(src, 0, dst, 0, bytes);
            }
        }
    }

    /// The merged layout: the tables as they are now, copied into the struct's buffer. The renderer writes the tables
    /// where it always did; the passes that read the scene read this copy, so it is made in their encoder, before
    /// them, each frame (the tables are small: a few copies of kilobytes). The uniform tables: the same, into the
    /// four uniform buffers. After whatever writes a table on the GPU in that encoder (the particle passes write
    /// their objects' entries).
    pub fn sync_tables(&self, up: &Uploaded, e: &mut wgpu::CommandEncoder) {
        if let Some(u) = &up.uniform {
            Self::copy_uniform(e, uniform_sources(up.table_buffers()), u);
        }
        let (Some(t), Some(caps)) = (&up.tables, &up.kernel.caps) else { return };
        let (at, _) = crate::merged::offsets(&crate::merged::TABLES, &caps.tables);
        for (k, src) in up.table_buffers().into_iter().enumerate() {
            let room = crate::merged::TABLES[k].stride * caps.tables[k] as u64;
            e.copy_buffer_to_buffer(src, 0, t, at[k], src.size().min(room));
        }
    }

    /// Software traversal's top tree again, over the units as they are now, written to its range of the node buffer
    /// (what `rebuild_tlas` is to hardware traversal). Err: no tree can be made (more units than its range holds, a
    /// unit that is nowhere); nothing was written, and the caller rebuilds the scene.
    pub fn retop(&self, units: &mut crate::units::Units, nodes: &wgpu::Buffer) -> std::result::Result<(), String> {
        let top = units.top()?;
        if !top.is_empty() {
            self.queue.write_buffer(nodes, units.layout.top_first as u64 * 32, bytemuck::cast_slice(&top));
        }
        Ok(())
    }

    /// A mesh tree's nodes into the node buffer at its root (`Units::add_tree` placed it).
    pub fn write_tree(&self, nodes: &wgpu::Buffer, root: u32, tree: &crate::units::MeshTree) {
        self.queue.write_buffer(nodes, root as u64 * 32, bytemuck::cast_slice(&tree.nodes));
    }

    /// A freed mesh tree's nodes (`Units::drop_tree`: its root and length) are room again: leaves that end a walk
    /// (`units::FILL`), as the room was laid out. (No unit names them: `Units::top` holds that. This is so that the
    /// room never holds a part of an old tree beside a part of a new one.)
    pub fn fill_tree(&self, nodes: &wgpu::Buffer, (root, len): (u32, u32)) {
        self.queue.write_buffer(nodes, root as u64 * 32, bytemuck::cast_slice(&vec![crate::units::FILL; len as usize]));
    }

    pub fn rebuild_tlas(&self, accel: &Accel, e: &mut wgpu::CommandEncoder) {
        e.build_acceleration_structures(std::iter::empty::<&wgpu::BlasBuildEntry>(), [&accel.tlas]);
    }

    /// Rebuilds the BLASes of instanced meshes whose triangles changed (a new shared pose, `scene::PoseGroup`), not the
    /// TLAS: the caller rebuilds it after (it holds their bounds).
    pub fn rebuild_meshes(&self, accel: &Accel, meshes: &[u32], e: &mut wgpu::CommandEncoder) {
        self.build_meshes(accel, meshes, e, false);
    }

    /// The BLASes of instanced meshes `meshes` and, with `tlas`, the TLAS in the same build (the TLAS reads the BLASes'
    /// bounds through their resource ids, which Metal does not order a later build after: a TLAS built apart from a
    /// BLAS it references, in the same encoder, could see it unbuilt and lose its instance; live/edits.rs `edit_geometry`).
    pub fn build_meshes(&self, accel: &Accel, meshes: &[u32], e: &mut wgpu::CommandEncoder, tlas: bool) {
        self.build_edit(accel, meshes, &[], e, tlas);
    }

    /// `build_meshes` with the procedural BLASes `proc_blas` left pending, all in the one build.
    pub fn build_edit(&self, accel: &Accel, meshes: &[u32], procs: &[PendingProc], e: &mut wgpu::CommandEncoder, tlas: bool) {
        let mut entries: Vec<wgpu::BlasBuildEntry> = meshes
            .iter()
            .map(|&m| {
                let (parts, sizes) = &accel.mesh_geo[m as usize];
                wgpu::BlasBuildEntry {
                    blas: &accel.meshes[m as usize],
                    geometry: wgpu::BlasGeometries::TriangleGeometries(
                        sizes
                            .iter()
                            .zip(parts)
                            .map(|(size, &(first, _, _))| wgpu::BlasTriangleGeometry {
                                size,
                                vertex_buffer: &accel.verts,
                                first_vertex: ((accel.pool_first + first as usize) * 3) as u32,
                                vertex_stride: 12,
                                index_buffer: None,
                                first_index: None,
                                transform_buffer: None,
                                transform_buffer_offset: None,
                            })
                            .collect(),
                    ),
                }
            })
            .collect();
        entries.extend(procs.iter().map(|p| wgpu::BlasBuildEntry { blas: &p.blas, geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry { size: &p.size, stride: 24, aabb_buffer: &p.boxes, primitive_offset: 0 }]) }));
        e.build_acceleration_structures(entries.iter(), tlas.then_some(&accel.tlas));
    }

    pub fn rebuild_dynamic(&self, accel: &Accel, e: &mut wgpu::CommandEncoder) {
        let entries: Vec<wgpu::BlasBuildEntry> = accel
            .blases
            .iter()
            .filter(|b| b.1 >= 4)
            .map(|(blas, _, first, size)| wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size,
                    vertex_buffer: &accel.verts,
                    first_vertex: (*first * 3) as u32,
                    vertex_stride: 12,
                    index_buffer: None,
                    first_index: None,
                    transform_buffer: None,
                    transform_buffer_offset: None,
                }]),
            })
            .collect();
        e.build_acceleration_structures(entries.iter(), [&accel.tlas]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Offline translucency: the pass (translucent.wgsl) `spp` times at random points in each pixel, each with a fresh
    /// lighting estimate per hit; the mean (colour, transmittance) per pixel. Empty without translucent particles.
    fn render_translucent(&self, up: &Uploaded, base: Params, spp: u32) -> Result<Vec<[f32; 4]>> {
        let Some(t) = &up.translucent else { return Ok(vec![]) };
        let d = &self.device;
        let (w, h) = (up.width, up.height);
        let pipeline = self.translucent_pipelines(&up.kernel, false).draw;
        let size = w as u64 * h as u64 * 16;
        let sums = d.create_buffer(&wgpu::BufferDescriptor { label: Some("translucent sums"), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let tbg = self.translucent_bind_group(t, &sums);
        let tp = TranslucentParams { size: [w, h, t.first, 0], view: [w, h, 0, 0], misc: [0.0; 4] };
        self.queue.write_buffer(&t.params, 0, bytemuck::bytes_of(&tp));
        let n = spp.max(1);
        for s in 0..n {
            let mut p = base;
            p.size[2] = s;
            self.queue.write_buffer(&up.params, 0, bytemuck::bytes_of(&p));
            let mut enc = d.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("translucent"), timestamp_writes: None });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &up.bg, &[]);
                pass.set_bind_group(1, &self.no_vis, &[]);
                pass.set_bind_group(2, &tbg, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            self.queue.submit([enc.finish()]);
            if s % 4 == 3 {
                self.device.poll(wgpu::PollType::wait_indefinitely())?;
            }
        }
        let readback = d.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = d.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&sums, 0, &readback, 0, size);
        self.queue.submit([enc.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        let data = readback.slice(..).get_mapped_range()?;
        let k = 1.0 / n as f32;
        let out = bytemuck::cast_slice::<u8, [f32; 4]>(&data).iter().map(|v| v.map(|x| if x.is_finite() { x * k } else { 0.0 })).collect();
        drop(data);
        readback.unmap();
        Ok(out)
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Render `spp` samples. `base` holds everything except the per-dispatch sample index.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&self, scene: &Scene, tri_verts: &[[f32; 4]], groups: [usize; GROUPS], nodes: &[u8], emitters: &[[f32; 4]], env: &EnvMap, env_cdf: &[f32], energy: &[[f32; 4]], base: Params, spp: u32, timings: &mut Vec<(String, f64)>) -> Result<Frame> {
        let d = &self.device;
        let (w, h) = (scene.width, scene.height);
        let up = self.upload(scene, w, h, tri_verts, groups, nodes, emitters, env, env_cdf, energy, timings)?;
        let pipeline = self.pipeline(&up.kernel, false);
        // Particles: placed once, at the scene time and camera.
        if up.particles.is_some() {
            let mut e = d.create_command_encoder(&Default::default());
            let (c, f) = (base.cam_pos, base.cam_fwd);
            self.run_particles(&up, [c[0], c[1], c[2]], [f[0], f[1], f[2]], base.time[0], 1.0 / 60.0, base.time[2], base.time[1], &mut e, None, None);
            self.queue.submit([e.finish()]);
        }
        // Stateful particles: simulated from the scene's start to its time at 60 steps a second (the spawns of the game's
        // setup at the first), then rendered.
        if up.gpu_particles.is_some() {
            let steps = ((base.time[0].max(0.0) * 60.0).round() as u32).min(MAX_OFFLINE_STEPS);
            let (c, f) = (base.cam_pos, base.cam_fwd);
            let none = std::collections::BTreeMap::new();
            for k in 0..steps.max(1) {
                let mut e = d.create_command_encoder(&Default::default());
                self.run_gpu_particles(&up, [c[0], c[1], c[2]], [f[0], f[1], f[2]], k as f32 / 60.0, 1.0 / 60.0, if k == 0 { &scene.particle_spawns } else { &none }, k + 1 == steps.max(1) && up.particles.is_none(), &mut e, None, None);
                self.queue.submit([e.finish()]);
            }
            if up.particles.is_some() {
                let mut e = d.create_command_encoder(&Default::default());
                if let Some(a) = &up.accel {
                    self.rebuild_dynamic(a, &mut e);
                }
                self.queue.submit([e.finish()]);
            }
        }
        // Grid programs: run from the scene's start to its time at 60 frames a second (the sources of the game's setup at
        // the first), then rendered.
        if Self::has_grid_sim(&up) {
            let steps = ((base.time[0].max(0.0) * 60.0).round() as u32).min(MAX_OFFLINE_STEPS);
            let none = std::collections::BTreeMap::new();
            for k in 0..steps {
                let mut e = d.create_command_encoder(&Default::default());
                self.run_grids(&up, k as f32 / 60.0, 1.0 / 60.0, if k == 0 { &scene.grid_sources } else { &none }, &mut e, None);
                self.queue.submit([e.finish()]);
            }
        }
        let (b_accum, b_params, bg) = (&up.accum, &up.params, &up.bg);
        let accum_size = b_accum.size();
        let stopped_before = self.guard_trips(&self.no_vis_bufs, &up.kernel)?;


        let t1 = Instant::now();
        let mut enc = d.create_command_encoder(&Default::default());
        enc.clear_buffer(b_accum, 0, None);
        self.sync_tables(&up, &mut enc);
        self.queue.submit([enc.finish()]);
        // One dispatch per sample; params are rewritten between submissions. Batch a few per submit.
        let batch = 4u32;
        let mut s = 0u32;
        while s < spp {
            for k in 0..batch.min(spp - s) {
                let mut p = base;
                p.size[2] = s + k;
                self.queue.write_buffer(b_params, 0, bytemuck::bytes_of(&p));
                let mut enc = d.create_command_encoder(&Default::default());
                {
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, bg, &[]);
                    pass.set_bind_group(1, &self.no_vis, &[]);
                    pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                }
                self.queue.submit([enc.finish()]);
            }
            s += batch.min(spp - s);
            self.device.poll(wgpu::PollType::wait_indefinitely())?;
        }
        timings.push(("trace_ms".into(), t1.elapsed().as_secs_f64() * 1000.0));
        // The loop guard, if the kernel has it: a render in which it stopped a loop has wrong pixels, and is an error.
        if let (Some(before), Some(now)) = (stopped_before, self.guard_trips(&self.no_vis_bufs, &up.kernel)?) {
            if now != before {
                return Err(anyhow!("the loop guard stopped {} of the kernel's invocations at {} iterations (THREERT_LOOP_GUARD): a traversal that does not end on this scene's data, or a limit too low for it", now.wrapping_sub(before), loop_guard_of(up.kernel.features & F_UNITS != 0).unwrap_or(0)));
            }
        }
        let t_tl = Instant::now();
        let overlay = self.render_translucent(&up, base, spp)?;
        if up.translucent.is_some() {
            timings.push(("translucent_ms".into(), t_tl.elapsed().as_secs_f64() * 1000.0));
        }

        let t2 = Instant::now();
        let readback = d.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: accum_size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = d.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(b_accum, 0, &readback, 0, accum_size);
        self.queue.submit([enc.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        let data = readback.slice(..).get_mapped_range()?;
        let px: &[[f32; 20]] = bytemuck::cast_slice(&data); // color, albedo, normal, live-only emission + specular
        let n = (w * h) as usize;
        let mut frame = Frame { width: w, height: h, color: Vec::with_capacity(n), albedo: Vec::with_capacity(n), normal: Vec::with_capacity(n), emission: Vec::with_capacity(n), specular: Vec::with_capacity(n), variance: Vec::with_capacity(n), samples: spp, overlay };
        for p in px {
            let c = p[3].max(1.0);
            let p: [f32; 20] = p.map(|v| if v.is_finite() { v } else { 0.0 });
            frame.color.push([p[0] / c, p[1] / c, p[2] / c]);
            frame.albedo.push([p[4] / c, p[5] / c, p[6] / c]);
            frame.normal.push([p[8] / c, p[9] / c, p[10] / c]);
            frame.emission.push([p[12] / c, p[13] / c, p[14] / c]);
            frame.specular.push([p[16] / c, p[17] / c, p[18] / c]);
            let mean = p[7] / c;
            frame.variance.push(((p[11] / c - mean * mean).max(0.0)) / c);
        }
        drop(data);
        readback.unmap();
        timings.push(("readback_ms".into(), t2.elapsed().as_secs_f64() * 1000.0));
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn material_functions_by_the_fields_they_write() {
        let f = |body: &str| format!("fn mf_x(i: MatIn, m: Material, base: MatOut) -> MatOut {{\n    var o = base;\n{body}\n    return o;\n}}\n");
        let wet = f("    o.base = base.base * 0.5;\n    o.roughness = 0.1;\n    o.normal = i.ng;");
        assert!(!writes_field(&wet, "emissive") && !writes_field(&wet, "opacity"));
        assert!(writes_field(&f("    o.emissive = vec3<f32>(1.0);"), "emissive"));
        assert!(writes_field(&f("    o.opacity *= 0.5;"), "opacity"));
        // A whole assignment or a pointer passed on may change any field.
        assert!(writes_field(&f("    o = base;"), "emissive"));
        assert!(writes_field(&f("    helper(&o);"), "opacity"));
        assert!(!writes_field(&f("    let no = 1.0;\n    o.base = vec3<f32>(no);"), "opacity"));
    }

    use super::*;

    /// The kernel features are one bit each, no two the same (`override FEAT` gets them all, the source-patching ones
    /// too), and pathtrace.wgsl's constants for the ones it switches on are the same bits.
    #[test]
    fn feature_bits_are_distinct() {
        let all = [
            ("F_MASKED", F_MASKED), ("F_FOG", F_FOG), ("F_VOLUME", F_VOLUME), ("F_TEX", F_TEX), ("F_FUNC", F_FUNC),
            ("F_GLASS", F_GLASS), ("F_COAT", F_COAT), ("F_HUGE", F_HUGE), ("F_INST", F_INST), ("F_MEDIA", F_MEDIA), ("F_PEMIT", F_PEMIT),
            ("F_HMEDIA", F_HMEDIA), ("F_PROC", F_PROC), ("F_TINT", F_TINT), ("F_UNLIT", F_UNLIT), ("F_BACKGROUND", F_BACKGROUND),
            ("F_VCOLOR", F_VCOLOR), ("F_AERIAL", F_AERIAL), ("F_GRID", F_GRID),
        ];
        let mut seen = 0u32;
        for (name, bit) in all {
            assert!(bit.is_power_of_two() && seen & bit == 0, "{name} = {bit:#x} is not a bit of its own");
            seen |= bit;
        }
        let wgsl = include_str!("../shaders/pathtrace.wgsl");
        for line in wgsl.lines().filter(|l| l.starts_with("const F_")) {
            let name = &line[6..line.find(':').unwrap()];
            let value: u32 = line[line.find('=').unwrap() + 1..line.find("u;").unwrap()].trim().parse().unwrap();
            let rust = all.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name}: not in gpu.rs")).1;
            assert_eq!(value, rust, "{name}: pathtrace.wgsl and gpu.rs differ");
        }
    }

    fn naga_valid(src: &str) {
        let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    }

    /// The native default (no merged layout, no loop guard): `guarded` gives the kernel back untouched and the live
    /// passes are their file. Every variant the kernel's flags make is valid WGSL (naga, wgpu's front end): each
    /// traversal with no switch, each alone and all of them, by no scene feature, each alone and all of them (units are
    /// software traversal's), and with the uniform tables. Prints the default variants' SHA-256 (`--nocapture`).
    #[test]
    fn every_kernel_variant_is_valid() {
        use sha2::{Digest, Sha256};
        if std::env::var("THREERT_LOOP_GUARD").is_ok() {
            return;
        }
        assert_eq!(loop_guard(), None);
        let sha = |s: &str| Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(crate::live::live_source(), include_str!("../shaders/live.wgsl"));
        let template = kernel_template();
        assert_eq!(guarded(&template, false, false), template);
        let all = KernelSwitches { strict: true, cutout_lite: 2, last_direct: true, light_pick: true };
        let switches = [
            KernelSwitches::default(),
            KernelSwitches { strict: true, ..Default::default() },
            KernelSwitches { cutout_lite: 1, ..Default::default() },
            KernelSwitches { cutout_lite: 2, ..Default::default() },
            KernelSwitches { last_direct: true, ..Default::default() },
            KernelSwitches { light_pick: true, ..Default::default() },
            all,
        ];
        for hwrt in [false, true] {
            println!("source kernel hwrt={hwrt} {}", sha(&kernel_wgsl(&template, &kernel_flags(hwrt, &KernelSwitches::default(), 0))));
            for k in &switches {
                for f in [0, F_VCOLOR, F_GRID, F_TINT, F_UNLIT, F_UNITS, F_BACKGROUND, F_SOURCE] {
                    naga_valid(&kernel_wgsl(&template, &kernel_flags(hwrt, k, if hwrt { f & !F_UNITS } else { f })));
                }
                naga_valid(&kernel_wgsl(&crate::merged::uniform_tables(&template), &kernel_flags(hwrt, k, F_SOURCE & !F_UNITS)));
            }
        }
    }

    /// A kernel with units has the loop guard natively with nothing set, at the browser's limit; THREERT_LOOP_GUARD=0
    /// takes it out, a limit set is the limit, a page's limit comes first. A kernel without units has it only where
    /// one is set, or in a browser.
    #[test]
    fn a_kernel_with_units_has_the_loop_guard() {
        assert_eq!(guard_limit(0, None, true, false), Some(65536));
        assert_eq!(guard_limit(0, None, false, false), None);
        assert_eq!(guard_limit(0, Some(0), true, false), None);
        assert_eq!(guard_limit(0, Some(0), true, true), None);
        assert_eq!(guard_limit(0, Some(16), true, false), Some(16));
        assert_eq!(guard_limit(0, Some(16), false, false), Some(16));
        assert_eq!(guard_limit(0, None, false, true), Some(65536));
        assert_eq!(guard_limit(8192, Some(0), false, false), Some(8192));
        if std::env::var("THREERT_LOOP_GUARD").is_ok() {
            return;
        }
        // As compiled: the source with units has every loop bounded and the count's buffer; hardware traversal, which
        // never has units, has room for the count only when a limit is set.
        let src = kernel_template();
        let g = guarded(&src, false, true);
        assert!(g.contains("return LOOP_GUARD_N > 65536u;") && g.contains(&crate::merged::guard_decl()), "the guard and its count");
        assert_eq!(guarded(&src, false, false), src, "units told to have no guard: the source as written");
        // What a native session with units compiles with nothing set, to naga (wgpu's front end); the unit walk's loops
        // are bounded too (that every loop is: merged.rs `the_guard_bounds_every_loop`).
        let wgsl = kernel_wgsl(&g, &kernel_flags(false, &KernelSwitches::default(), F_UNITS));
        naga_valid(&wgsl);
        let walk = &wgsl[wgsl.find("fn trace_units(").expect("the unit walk")..];
        assert!(walk[..walk[1..].find("\nfn ").unwrap_or(walk.len() - 1)].contains("loop_guard()"));
        assert!(guard_room(false) && !guard_room(true));
    }

    /// Software traversal's units are in a kernel only when the scene has them (F_UNITS, `units`): without, the still
    /// walk keeps its stack and there is no unit walk; with, the still walk hands its hit to `trace_units` and its stack
    /// is the module's (the procedural objects' walk keeps its own). Hardware traversal has neither software walk.
    #[test]
    fn units_are_software_traversals_only() {
        let t = kernel_template();
        let sw = kernel_wgsl(&t, &kernel_flags(false, &KernelSwitches::default(), 0));
        let u = kernel_wgsl(&t, &kernel_flags(false, &KernelSwitches::default(), F_UNITS));
        let hw = kernel_wgsl(&t, &kernel_flags(true, &KernelSwitches::default(), 0));
        assert!(!sw.contains("trace_units") && !hw.contains("trace_units") && !hw.contains("fn trace_procs("));
        assert_eq!((u.matches("fn trace_units(").count(), u.matches("trace_units(o, d, inv_d, hit, mode)").count()), (1, 1));
        let stacks = |s: &str| (s.matches("var<private> stack: array<u32, 64>").count(), s.matches("var stack: array<u32, 48>").count());
        assert_eq!((stacks(&sw), stacks(&u)), ((0, 2), (1, 1)));
    }

    /// With nothing set, every platform but Android opens the device as it was before the options existed: no
    /// uniform tables, no pipeline cache, the kernel's source the files (`the_default_sources_are_untouched` has its
    /// hashes). Android's defaults are the uniform tables and the switches its GPUs needed; the environment overrides
    /// either way, and the merged layout never has the uniform tables.
    #[test]
    fn the_options_default_to_the_device_as_written() {
        let vars = |set: &'static [(&'static str, &'static str)]| move |name: &str| set.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
        let desktop = GpuOptions::from_vars(&vars(&[]), false);
        assert_eq!(desktop, GpuOptions::default());
        assert_eq!(desktop.kernel, KernelSwitches { strict: false, cutout_lite: 0, last_direct: false, light_pick: false });
        assert!(!desktop.uniform_tables_on(false, false));
        for hwrt in [false, true] {
            assert!(kernel_flags(hwrt, &desktop.kernel, 0).iter().all(|&(name, on)| on == (name == "hwrt" && hwrt)));
        }
        let android = GpuOptions::from_vars(&vars(&[]), true);
        assert_eq!(android.kernel, KernelSwitches { strict: true, cutout_lite: 1, last_direct: true, light_pick: false });
        assert_eq!((android.uniform_tables, android.pipeline_cache.clone()), (None, None));
        assert!(android.uniform_tables_on(false, true) && !android.uniform_tables_on(true, true));
        // Set: what is asked, on either platform.
        let off = GpuOptions::from_vars(&vars(&[("THREERT_UNIFORM_TABLES", "0"), ("THREERT_KERNEL_STRICT", "0"), ("THREERT_CUTOUT_LITE", "0"), ("THREERT_LAST_DIRECT", "0")]), true);
        assert_eq!((off.kernel, off.uniform_tables_on(false, true)), (KernelSwitches::default(), false));
        let on = GpuOptions::from_vars(&vars(&[("THREERT_UNIFORM_TABLES", "1"), ("THREERT_CUTOUT_LITE", "2"), ("THREERT_LIGHT_PICK", "1"), ("THREERT_PIPELINE_CACHE", "/data/cache/pipelines")]), false);
        assert!(on.uniform_tables_on(false, false) && !on.uniform_tables_on(true, false));
        assert_eq!(on.kernel, KernelSwitches { strict: false, cutout_lite: 2, last_direct: false, light_pick: true });
        assert_eq!(on.pipeline_cache.as_deref(), Some(std::path::Path::new("/data/cache/pipelines")));
        assert_eq!(GpuOptions::from_vars(&vars(&[("THREERT_PIPELINE_CACHE", "")]), true).pipeline_cache, None);
        // (And the environment map's light sampling, which is the scene's preparation's: lib.rs `env_nee_on`.)
        assert_eq!([crate::env_nee_of(None, false), crate::env_nee_of(None, true), crate::env_nee_of(Some("1"), true), crate::env_nee_of(Some("0"), false)], [true, false, true, false]);
    }

    /// The other shader files are WGSL as the specification writes it: wesl's parser is as strict as a browser's
    /// (Chrome's Tint) where naga is not. naga took `a * b ^ c`, which WGSL's grammar forbids (a bitwise operator's
    /// operands are unary expressions): the scene library's surface materials and both particle passes would not
    /// compile in a browser until 2026-11-14. (The kernel's files: `every_kernel_variant_is_valid`.)
    #[test]
    fn the_other_shaders_are_wgsl_as_the_specification_writes_it() {
        for src in [include_str!("../shaders/live.wgsl"), include_str!("../shaders/particles.wgsl"), include_str!("../shaders/gpu_particles.wgsl"), include_str!("../shaders/grid_sim.wgsl")] {
            kernel_wgsl(src, &[]);
        }
    }

    /// Each switch changes what it says, under both traversals (that every combination is valid WGSL:
    /// `every_kernel_variant_is_valid`). Strict guards every ray query the hardware kernel makes; software traversal
    /// has none and takes only `env_pdf`'s index; the cutout switches are hardware traversal's.
    #[test]
    fn the_kernel_switches_change_what_they_say() {
        let t = kernel_template();
        let v = |hwrt: bool, k: KernelSwitches| kernel_wgsl(&t, &kernel_flags(hwrt, &k, 0));
        for hwrt in [false, true] {
            let plain = v(hwrt, KernelSwitches::default());
            let strict = v(hwrt, KernelSwitches { strict: true, ..Default::default() });
            let queries = strict.matches("rayQueryInitialize(").count();
            assert_eq!(queries, if hwrt { 3 } else { 0 });
            assert_eq!(strict.matches("ray_bad(o, d, tmax").count(), queries, "every ray query is guarded");
            assert_eq!(strict.matches("fn ray_bad(").count(), hwrt as usize);
            assert!(!strict.contains("env_cdf[y - 1u]") && !strict.contains("env_cdf[base + x - 1u]") && plain.contains("env_cdf[y - 1u]"));
            let lit = v(hwrt, KernelSwitches { strict: true, cutout_lite: 2, last_direct: true, light_pick: true });
            assert_eq!(lit.matches("radiance += dz;").count(), 1);
            assert_eq!(lit.matches("env_on = take_env && has_env;").count(), 1);
            assert_eq!(lit.contains("let steps = 4u;"), hwrt);
            assert_eq!(v(hwrt, KernelSwitches { cutout_lite: 2, ..Default::default() }) == plain, !hwrt);
        }
    }

    /// The uniform tables' buffers: each array fits a uniform binding of 64 KiB, the media's holds every medium a
    /// scene may have, and the four are at their tables' bindings. A copy takes what the table and its uniform buffer
    /// both hold, in whole words; a scene with more entries than an array is said, by name.
    #[test]
    fn the_uniform_tables_fit_their_bindings() {
        use crate::merged::{TABLES, UNIFORM_LEN, UNIFORM_TABLES};
        let bindings = UNIFORM_TABLES.map(|k| MERGED_TABLE_BINDINGS[k]);
        assert_eq!(bindings, [14, 15, 17, 18]);
        for (k, &t) in UNIFORM_TABLES.iter().enumerate() {
            let m = &TABLES[t];
            assert!(m.decl.starts_with(&format!("@group(0) @binding({}) var<storage, read> {}:", bindings[k], m.name)), "{}", m.decl);
            let bytes = m.stride * UNIFORM_LEN[k] as u64;
            assert!(bytes <= 65536 && bytes % 16 == 0, "{}: {bytes} bytes", m.name);
        }
        assert_eq!(UNIFORM_TABLES.map(|t| TABLES[t].name), ["tex_info", "objects", "instances", "media"]);
        assert_eq!(UNIFORM_LEN[3] as usize, crate::scene::MAX_MEDIA);
        assert_eq!(UNIFORM_LEN[2] as u64, 65536 / TABLES[5].stride);
        // A table shorter than its array, one with the edits' room past it, one whose size is not the array's multiple.
        assert_eq!([uniform_copy_bytes(512, 65536), uniform_copy_bytes(1 << 20, 65536), uniform_copy_bytes(65600, 65440), uniform_copy_bytes(0, 7168), uniform_copy_bytes(7, 64)], [512, 65536, 65440, 0, 4]);
        assert!(uniform_overflow([4096, 4096, 409, 32]).is_empty());
        let notes = uniform_overflow([4096, 4099, 1024, 32]);
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("4099 entries of `objects`") && notes[0].contains("holds 4096") && notes[1].contains("1024 entries of `instances`"), "{notes:?}");
    }

    /// The order of a layout's entries and of a bind group's is not the device's to see: wgpu puts both in binding
    /// order before its backend gets them (wgpu-core 30: `bgl::EntryMap::from_entries` sorts a layout's, and
    /// `create_bind_group` its entries), and takes two layouts of the same entries as one. So the kernel's layout,
    /// whose acceleration structure (binding 13) is pushed last, needs no sorting here for a driver that minds the
    /// order. Held on this machine's GPU: a bind group made of one order runs with a pipeline laid out in another,
    /// without a validation error, and each binding reads its own buffer. (Skipped without a GPU.)
    #[test]
    fn wgpu_takes_bind_group_entries_in_any_order() {
        let g = match Gpu::new_with(GpuOptions::default()) {
            Ok(g) => g,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        let d = &g.device;
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let (uni, read, write) = (entry(0, wgpu::BufferBindingType::Uniform), entry(5, wgpu::BufferBindingType::Storage { read_only: true }), entry(9, wgpu::BufferBindingType::Storage { read_only: false }));
        let scope = d.push_error_scope(wgpu::ErrorFilter::Validation);
        let ordered = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[uni, read, write] });
        let shuffled = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[write, uni, read] });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl("@group(0) @binding(0) var<uniform> a: vec4<u32>;\n@group(0) @binding(5) var<storage, read> b: array<u32>;\n@group(0) @binding(9) var<storage, read_write> c: array<u32>;\n@compute @workgroup_size(1) fn main() { c[0] = a.x; c[1] = b[0]; }\n".into()),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&ordered)], immediate_size: 0 });
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: Some(&layout), module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let buf = |words: [u32; 4], usage| d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&words), usage });
        let (a, b, c) = (buf([11, 0, 0, 0], wgpu::BufferUsages::UNIFORM), buf([22, 0, 0, 0], wgpu::BufferUsages::STORAGE), buf([0; 4], wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC));
        fn e(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
        }
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &shuffled, entries: &[e(9, &c), e(5, &b), e(0, &a)] });
        let out = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = d.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        enc.copy_buffer_to_buffer(&c, 0, &out, 0, 16);
        g.queue.submit([enc.finish()]);
        assert!(pollster::block_on(scope.pop()).is_none(), "a layout's entries in another order are another layout to wgpu");
        out.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let words: Vec<u32> = bytemuck::cast_slice(&out.slice(..).get_mapped_range().unwrap()).to_vec();
        assert_eq!(words[..2], [11, 22]);
    }

    /// Each feature that changes the kernel's declarations (`kernel_flags`) changes what it says, and nothing of it
    /// is in a kernel without it.
    #[test]
    fn the_source_features_change_what_they_say() {
        let t = kernel_template();
        let v = |f: u32| kernel_wgsl(&t, &kernel_flags(false, &KernelSwitches::default(), f));
        let base = v(0);
        let tint = v(F_TINT);
        assert!(tint.contains("const OBJECT_STRIDE: u32 = 4u;") && !tint.contains("const OBJECT_STRIDE: u32 = 3u;"));
        assert_eq!(tint.matches("*= obj_tint(").count(), 3);
        assert!(base.contains("const OBJECT_STRIDE: u32 = 3u;") && !base.contains("obj_tint"));
        assert!(v(F_UNLIT).contains("MAT_UNLIT") && !base.contains("MAT_UNLIT"));
        assert!(v(F_BACKGROUND).contains("P.background.rgb") && !base.contains("background"));
        let vc = v(F_VCOLOR);
        assert!(vc.contains("fn vcolor(") && vc.matches("*= vcolor(").count() == 2 && !base.contains("vcolor"));
        let gr = v(F_GRID);
        assert!(gr.contains("var grid_tex: texture_3d<f32>") && gr.contains("fn vgrid(") && gr.contains("i.grid = ") && !base.contains("grid_tex") && !base.contains("vgrid"));
    }

    /// Structural features other than F_INST (opaque particles, media) do not make a scene count as instanced: a group
    /// starting between 2^23 and 2^24 stays within the non-instanced custom-data limit (no F_HUGE).
    #[test]
    fn structural_features_are_not_instancing() {
        const M: usize = 1 << 23;
        let groups = [M + 5, 3, 0, 0, 0, 0, 0, 0];
        assert_eq!(features_of(&[], None, false, false, groups, F_PEMIT) & F_HUGE, 0);
        assert_eq!(features_of(&[], None, false, false, groups, F_MEDIA) & F_HUGE, 0);
        assert_ne!(features_of(&[], None, false, false, groups, F_INST) & F_HUGE, 0);
    }

    #[test]
    fn huge_when_a_group_starts_past_24_bits() {
        const M: usize = 1 << 24;
        // One group of any size: its first triangle is 0.
        assert!(!huge([M * 3, 0, 0, 0, 0, 0, 0, 0], false));
        // The second group starts at 2^24 - 1: still fits.
        assert!(!huge([M - 1, 5, 0, 0, 0, 0, 0, 0], false));
        // ... at 2^24: does not.
        assert!(huge([M, 5, 0, 0, 0, 0, 0, 0], false));
        // Empty groups past 2^24 have no instance.
        assert!(!huge([M + 7, 0, 0, 0, 0, 0, 0, 0], false));
        // The 3 x 3 Kabukicho district (37.75 M triangles, opaque shadow casters first).
        assert!(huge([36_000_000, 900_000, 500_000, 352_799, 0, 0, 0, 0], false));
        // With model instances, group custom data must stay below the instance bit (2^23).
        assert!(!huge([M - 1, 5, 0, 0, 0, 0, 0, 0], false));
        assert!(huge([M / 2, 5, 0, 0, 0, 0, 0, 0], true));
        assert!(!huge([M / 2 - 1, 5, 0, 0, 0, 0, 0, 0], true));
    }
}

//! Headless wgpu backend: uploads the scene once, accumulates path-traced samples, reads back HDR.

use crate::heap::{Heap, Live, Section, Stream, Table};
use crate::scene::Scene;
use anyhow::{anyhow, Result};
use web_time::Instant;
use wgpu::util::DeviceExt;

/// The uploaded texture pages of a texture set (texpack.rs).
struct Pages {
    /// `texpack::set_key` of the set.
    key: u64,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    info: std::sync::Arc<Vec<[f32; 4]>>,
    /// Stream textures by name: where their pixels go (`Gpu::write_stream`).
    streams: Vec<(String, crate::texpack::Place)>,
    /// Where each texture is and what the pages have free (`Gpu::place_texture`).
    atlas: crate::texpack::Atlas,
    /// By slot: the texture's key, its size and whether it wraps (what `texpack::set_key` is made of).
    keys: Vec<(String, u32, u32, bool)>,
}

/// What `Gpu::place_texture` did to the pages.
pub struct PagesEdit {
    /// The texture's slot: its index among the scene's textures, its rows in `tex_info`.
    pub slot: i32,
    /// Every texture's rows as they are now, where a texture was placed.
    pub info: Option<std::sync::Arc<Vec<[f32; 4]>>>,
    /// The pages' view, where they are another texture now (they grew): bind groups hold the old one's.
    pub view: Option<wgpu::TextureView>,
}

#[cfg(dlss)]
pub(crate) type DlssSdk = std::sync::Arc<std::sync::Mutex<dlss_wgpu::DlssSdk>>;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    /// GPU timestamp queries are available (used for adaptive samples per frame in the viewer).
    pub timestamps: bool,
    #[cfg(dlss)]
    pub(crate) dlss_sdk: Option<DlssSdk>,
    /// NGX reported DLSS Ray Reconstruction on this device (with `dlss_sdk`).
    #[cfg(dlss)]
    pub(crate) dlss_rr: bool,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    /// The kernel reads its seven typed tables from the tables' buffer whole, not from views of their sections
    /// (`GpuOptions::table_views`; merged.rs `TABLE_VIEWS`).
    pub whole_tables: bool,
    /// The kernel's switches this device was opened with (`KernelSwitches`): they are in `source`.
    pub switches: KernelSwitches,
    /// The approximations a live view's kernel takes (`A_*`): the options' (`GpuOptions::approx`) unless a view asked
    /// for others (`set_approx`).
    pub approx: u32,
    approx_default: u32,
    /// Compiled pipelines kept between runs (`GpuOptions::pipeline_cache`), where the backend has such a cache.
    pipeline_cache: Option<crate::pipeline_cache::PipelineCache>,
    /// Kernel source (hardware or software traversal spliced in); scenes with material functions get their own module.
    source: std::sync::Arc<String>,
    /// The kernel as written: a scene without features that change the source or functions of its own.
    module: wgpu::ShaderModule,
    /// The merged binding layout (merged.rs): a browser's, where a stage has eight storage buffers. Natively with
    /// THREERT_MERGED=1 (software traversal, the device held to eight too).
    pub merged: bool,
    /// By (material-function code hash, the features that change the source: `KernelVariant::flags`).
    modules: ModuleCache,
    pipeline_layout: wgpu::PipelineLayout,
    /// Kernel variants by (feature mask, material-function code hash, LIVE): `FEAT` compiles out the features a scene
    /// does not use (they cost registers even when idle); `LIVE` = one fresh sample per dispatch for the viewport.
    pipelines: PipelineMap<(u32, u64, bool, u32)>,
    /// The kernels being made on another thread (`warm_kernel`): by (features, functions' hash, approximations).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    warming: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<(u32, u64, u32)>>>,
    layout: wgpu::BindGroupLayout,
    /// Group 1, the live viewport's: the radiance cache (`CacheBuffers`) and what the live passes read of a trace
    /// (`KernelBufs`). Offline renders bind `no_live` (a one-entry cache).
    pub live_layout: wgpu::BindGroupLayout,
    pub no_live: wgpu::BindGroup,
    /// `no_live`'s buffers (the merged layout: an offline kernel's source is written for their lengths).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    no_live_bufs: KernelBufs,
    /// Binding 19 of scenes without density grids (one cell), and the grid pass's bind group layout (grid_sim.wgsl).
    no_grid_view: wgpu::TextureView,
    grid_layout: wgpu::BindGroupLayout,
    /// The pass that fills runs of the light list from their triangles (emitter_fill.wgsl), made when first needed.
    fill: std::sync::OnceLock<wgpu::ComputePipeline>,
    /// Hardware ray queries (Metal / Vulkan RT). Software BVH traversal otherwise.
    pub hwrt: bool,
    /// Last uploaded texture pages, reused while the scene's texture set is unchanged (live edits re-upload often).
    pages: std::sync::Mutex<Option<Pages>>,
    /// The last picture of each stream texture (`write_stream`), kept for pages that are packed again.
    streams: std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<image::RgbaImage>>>,
    /// The translucency pass (hardware RT): group 2's layout (translucent.wgsl), the kernel layout with it, and its
    /// pipelines (`translucent`, `volume_train`) by the kernel's variant key.
    tl_layout: Option<wgpu::BindGroupLayout>,
    tl_pipeline_layout: Option<wgpu::PipelineLayout>,
    /// Its group 1: `live_layout` without `finish`'s records, which the pass does not read (with them its three groups
    /// are one buffer past Metal's 29 a stage), and `no_live` made for it.
    pub tl_live_layout: Option<wgpu::BindGroupLayout>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    no_live_tl: Option<wgpu::BindGroup>,
    tl_pipelines: PipelineMap<(u32, u64, bool, u32), TranslucentPipelines>,
    /// The kernel's passes beside `main` (`kernel_pass`) by kernel variant and entry point.
    kernel_passes: PipelineMap<(u32, u64, &'static str, u32)>,
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
    pub jitter: [f32; 4],    // live: this frame's sub-pixel jitter x, y (every camera ray's); ray tracing off (1); -
    pub cache: [f32; 4],     // live radiance cache (1 = on), history cap in samples, history cap in frames, -
    pub motion: [f32; 4],    // live motion vectors: 1 = on; `render --signals`; software traversal's units: the top tree's root, their count (bits)
    /// Light LOD (live; `emitter_lod`): reference position, d0 (0 = off).
    pub light_lod: [f32; 4],
    /// x: the emitter CDF's total without light LOD (camera-hit MIS weighs by power alone; = misc[1] when it is off);
    /// y: live emitter cull radius (lib.rs emitter_lod; 0 = off); z: the cull's reach inside the view cone (0 = none).
    pub emit_power: [f32; 4],
    /// Live emitter cull's view cone (lib.rs `EmitterCull`): the forward direction it was built for (xyz), the cosine
    /// of its half-angle (w).
    pub lod_view: [f32; 4],
    /// Light tree (`ltree`): -, its entry count (bits: the first moving emitter's entry), the probability of picking
    /// a moving emitter (their share of power), candidates at camera hits (0 = off).
    pub ltree: [f32; 4],
    /// Procedural geometry (F_PROC; scene::proc_data): -, object count, first record in `instances`, -.
    pub procs: [u32; 4],
    /// The sky's aerial perspective (F_AERIAL; `scene::aerial_params`): density multiple, Rayleigh scale, Mie scale, -.
    pub aerial: [f32; 4],
    /// three.js `scene.background` (F_BACKGROUND): its radiance, 1.
    pub background: [f32; 4],
    /// The live light tiles (pathtrace.wgsl "Light tiles"; live/frame.rs): 1 with the last frame's lists (0: none
    /// this frame), tiles a row, the grid's offset when they were built (x | y << 16), candidates a
    /// vertex the camera sees takes from its list beside the uniform ones (`live::LIST_CANDIDATES`; half past it).
    pub ltile: [u32; 4],
    /// The traced size the lists were built at, -, -.
    pub ltile2: [u32; 4],
    /// The camera the lists were built under (the last frame's): position (w = tan(fov / 2)), forward (w = aspect),
    /// right, up. A camera hit reads the list of the tile it was seen in then.
    pub lt_cam: [[f32; 4]; 4],
    /// Each table's first vec4 in the tables' heap (`heap::Table`'s order; `Uploaded::place`).
    pub table_first: [[u32; 4]; 3],
    /// Each stream's first word in the words' heap (`heap::Stream`'s order): the environment's CDF, the procedural
    /// data, the vertex colours, the instances' rows; the triangles' indices, their rows. Then the first vertex that
    /// moves (the previous positions count from it), -.
    pub word_first: [[u32; 4]; 2],
    /// The merged layout (`KernelBufs::place`): each of group 1's vec4 sections' first vec4 in the tail of its one
    /// buffer (the cache's values, the specular rays, the motion vectors, the direct light; the light lists), and
    /// the last four's lengths. As written: zeros (the kernel asks the buffers).
    pub live_first: [[u32; 4]; 2],
    pub live_len: [u32; 4],
}

/// A vertex's bytes (`indexed::Vertex`): the stride the acceleration structures read positions by.
const VERTEX_STRIDE: u64 = crate::indexed::VERTEX_BYTES as u64;

/// Room the live view's in-place geometry edits grow into (live/edits.rs `edit_geometry`), past the scene's own data:
/// triangles (verts, attrs, BLAS vertices), kernel records and TLAS instances (and a row a record: what an edit
/// places has one section), object-table entries, procedural words.
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
        hwrt && Self::reserve() > 0.0 && (!scene.procs.is_empty() || scene.retained.has_own_instances())
    }

    /// `units`: software traversal with units, whose room does not grow (hardware traversal's does, from nothing): a
    /// scene of theirs without editable geometry still gets records and object entries, for instances of the meshes
    /// it has (a mesh it has not rebuilds the scene: no triangles).
    pub(crate) fn for_scene(scene: &Scene, tris: usize, records: usize, hwrt: bool, units: bool) -> EditRoom {
        let k = Self::reserve();
        let (procs, meshes) = (!scene.procs.is_empty(), scene.retained.has_own_instances());
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

/// A procedural BLAS an edit made, to be built with its commit's TLAS (`Gpu::proc_blas`, `TopBuild`): kept by its
/// blob's hash for reuse once built (`bytes`: its size).
pub struct PendingProc {
    hash: [u8; 32],
    blas: wgpu::Blas,
    boxes: wgpu::Buffer,
    size: wgpu::BlasAABBGeometrySizeDescriptor,
    bytes: u64,
}

/// What a commit's edits leave to build above the triangles (live/edits.rs `commit`): each writes its data and notes
/// here what is to build, and the commit builds once at its end (`Gpu::build_top`).
#[derive(Default)]
pub struct TopBuild {
    /// Instances (hardware traversal's TLAS instances, software traversal's units) moved, came or went.
    pub instances: bool,
    /// Instanced meshes' BLASes: new meshes, a skinned model's new pose. (Software traversal: their trees are written
    /// by then; the top tree is made over their bounds.)
    pub meshes: Vec<u32>,
    /// Procedural BLASes an edit made.
    pub procs: Vec<PendingProc>,
}

impl TopBuild {
    pub fn any(&self) -> bool {
        self.instances || !self.meshes.is_empty() || !self.procs.is_empty()
    }
}

/// The build entries of instanced meshes' BLASes over their pool triangles as they are now.
fn mesh_entries<'a>(accel: &'a Accel, meshes: &[u32]) -> Vec<wgpu::BlasBuildEntry<'a>> {
    meshes
        .iter()
        .map(|&m| {
            let (parts, sizes) = &accel.mesh_geo[m as usize];
            wgpu::BlasBuildEntry {
                blas: &accel.meshes[m as usize],
                geometry: wgpu::BlasGeometries::TriangleGeometries(
                    sizes
                        .iter()
                        .zip(parts)
                        .map(|(size, &(first, _))| wgpu::BlasTriangleGeometry {
                            size,
                            vertex_buffer: &accel.vertices,
                            first_vertex: accel.mesh_verts[m as usize].0,
                            vertex_stride: VERTEX_STRIDE,
                            index_buffer: Some(&accel.indices),
                            first_index: Some(accel.index_first + first * 3),
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        })
                        .collect(),
                ),
            }
        })
        .collect()
}

/// The acceleration structure's binding in the scene's group (trace_hw.wgsl `acc`): the last, after every buffer.
const ACC_BINDING: u32 = 19;

/// What the kernel's bind group layouts follow: the device's traversal and binding layout, and whether group 1 has
/// the loop guard's count (`guard_room`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayoutShape {
    pub hwrt: bool,
    pub merged: bool,
    /// No views of the tables' sections (`Gpu::whole_tables`).
    pub whole_tables: bool,
    pub guard: bool,
}

fn layout_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }
}
fn storage_buffer(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }
}
const UNIFORM_BUFFER: wgpu::BindingType = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
fn float_texture(view_dimension: wgpu::TextureViewDimension) -> wgpu::BindingType {
    wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension, multisampled: false }
}
const ACCELERATION_STRUCTURE: wgpu::BindingType = wgpu::BindingType::AccelerationStructure { vertex_return: false };

/// The entries of the kernel's bind group layouts, the one place they are written: group 0 (the scene), group 1
/// (what the kernel writes), group 2 (the translucency pass's; hardware traversal). A test holds them to what every
/// variant of the kernel's source declares (`the_kernels_layouts_are_its_declarations`).
pub(crate) fn kernel_layout_entries(shape: LayoutShape) -> [Vec<wgpu::BindGroupLayoutEntry>; 3] {
    use crate::merged::{DEFERRED_BINDING, FIRST_BINDING, GUARD_BINDING, LISTS_BINDING, LIVE_BINDING, TABLE_VIEWS};
    let LayoutShape { hwrt, merged, whole_tables, guard } = shape;
    let (read, write) = (storage_buffer(true), storage_buffer(false));
    // Group 0: the parameters; the vertices; the tables' heap, the words' heap; the previous frame's vertices of
    // those that move; the texture pages and their sampler, the environment, the density grids' atlas
    // (a 1-cell texture in scenes without grids, whose kernel does not read it); the accumulation buffer.
    let mut scene = vec![
        layout_entry(0, UNIFORM_BUFFER),
        layout_entry(1, read),
        layout_entry(4, read),
        layout_entry(5, read),
        layout_entry(6, read),
        layout_entry(7, float_texture(wgpu::TextureViewDimension::D2Array)),
        layout_entry(8, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
        layout_entry(9, float_texture(wgpu::TextureViewDimension::D2)),
        layout_entry(10, float_texture(wgpu::TextureViewDimension::D3)),
        layout_entry(11, write),
    ];
    // Software traversal's trees: their leaves' triangles, their nodes.
    if !hwrt {
        scene.push(layout_entry(2, read));
        scene.push(layout_entry(3, read));
    }
    // The tables the kernel reads at every hit or light, bound again as typed views of their sections (the merged
    // layout has no binding for them, nor has a device that reads them from the buffer whole).
    if !merged && !whole_tables {
        scene.extend(TABLE_VIEWS.iter().map(|&(_, b)| layout_entry(b, read)));
    }
    if hwrt {
        scene.push(layout_entry(ACC_BINDING, ACCELERATION_STRUCTURE));
    }
    // Group 1: merged, the one buffer (`heap::Live`, the loop guard's count in it); as written, the radiance cache's three, the light tiles' lists, the specular rays, the motion vectors, the
    // direct light, the pixels left for `finish`, a frame's camera hits, and the loop guard's count where a kernel of
    // this device may have the guard (one without it does not name the binding).
    let mut live = vec![];
    if merged {
        live.push(layout_entry(LIVE_BINDING, write));
    } else {
        live.extend([1, 2, 3, 5, 6, 7].map(|b| layout_entry(b, write)));
        live.push(layout_entry(LISTS_BINDING, read));
        live.push(layout_entry(DEFERRED_BINDING, write));
        live.push(layout_entry(FIRST_BINDING, write));
        if guard {
            live.push(layout_entry(GUARD_BINDING, write));
        }
    }
    // Group 2: the translucent particles' acceleration structure, the pass's parameters, its image, the volume
    // training points and their count, the image at reduced scale.
    let translucent = if hwrt { vec![layout_entry(0, ACCELERATION_STRUCTURE), layout_entry(1, UNIFORM_BUFFER), layout_entry(2, write), layout_entry(3, write), layout_entry(4, write), layout_entry(5, write)] } else { vec![] };
    [scene, live, translucent]
}


/// What the scene bind group holds besides `Uploaded`'s buffers (`Gpu::rebind_scene` makes it again).
pub struct BindParts {
    pub nodes: wgpu::Buffer,
    /// Software traversal: the trees' leaf triangles (`indexed::Leaf`: pathtrace.wgsl `leaves`), with room for an
    /// edit's. (Hardware traversal: unbound, of no triangle.)
    pub leaves: wgpu::Buffer,
    pub tex_view: wgpu::TextureView,
    pub env_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// The density grids' atlas (binding 19), or the one-cell texture of a scene without grids.
    pub grid_view: wgpu::TextureView,
}

/// What the scene's bind group is made of (`Gpu::scene_bind_group`).
struct SceneBuffers<'a> {
    params: &'a wgpu::Buffer,
    vertices: &'a wgpu::Buffer,
    heap: &'a Heap<Table>,
    words: &'a Heap<Stream>,
    prev_vertices: &'a wgpu::Buffer,
    accum: &'a wgpu::Buffer,
    parts: &'a BindParts,
    accel: Option<&'a Accel>,
}

/// The scene's buffers a particle pass writes (particles.wgsl, gpu_particles.wgsl: bindings 2 to 6; the objects'
/// section of the tables as a buffer of its own). The pass's bind group holds them: one replaced, the group is made
/// again (`Gpu::rebind_scene`).
struct ParticleTargets<'a> {
    vertices: &'a wgpu::Buffer,
    prev_vertices: &'a wgpu::Buffer,
    heap: &'a Heap<Table>,
}

fn buffer_entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
}

/// A particle pass's bind group: its `own` buffers at their bindings, and the scene's it writes.
fn particle_bind_group(d: &wgpu::Device, pipeline: &wgpu::ComputePipeline, label: &str, own: &[(u32, &wgpu::Buffer)], t: &ParticleTargets) -> wgpu::BindGroup {
    let mut entries: Vec<wgpu::BindGroupEntry> = own.iter().map(|(binding, b)| buffer_entry(*binding, b)).collect();
    entries.extend([buffer_entry(2, t.vertices), buffer_entry(3, t.prev_vertices), wgpu::BindGroupEntry { binding: 5, resource: t.heap.binding(Table::Objects) }]);
    d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &pipeline.get_bind_group_layout(0), entries: &entries })
}

pub struct Uploaded {
    pub bg: wgpu::BindGroup,
    pub bg_parts: BindParts,
    /// The scene's tables (heap.rs): materials, lights, emitters, objects, instances, media and the rest, each a
    /// section that edits write in place (`Heap::write`) and grow (`Heap::grow`, then `Gpu::rebind_scene`).
    pub heap: Heap<Table>,
    /// The trace's samples, a `Pixel` a pixel: the view's buffer (a live view's targets', an offline render's own:
    /// `accum_buffer`). A handle of it here, for the scene's bind group, where the kernel's layout has it;
    /// `Gpu::set_accum` gives it another (a view of another size).
    pub accum: wgpu::Buffer,
    pub params: wgpu::Buffer,
    /// Kernel specialisation for this scene (`Gpu::pipeline`).
    pub kernel: Kernel,
    /// The vertices (indexed.rs `Vertex`): a record of 48 bytes each; and those that move as they were the previous
    /// frame, whole records too (a copy on the GPU has no stride). What the kernel reads (bindings 1 and 6).
    pub vertices: wgpu::Buffer,
    pub prev_vertices: wgpu::Buffer,
    pub accel: Option<Accel>,
    /// Software traversal's units (units.rs), when the scene has any: what `accel` is to hardware traversal. Their
    /// trees are in the node buffer (`bg_parts.nodes`).
    pub units: Option<crate::units::Units>,
    /// The particle pass, when the scene has particle systems (hardware RT).
    pub particles: Option<ParticleGpu>,
    pub gpu_particles: Option<GpuParticlesGpu>,
    /// The skinning pass, when meshes of the scene deform (hardware RT).
    pub skin: Option<SkinGpu>,
    pub fills: EmitterFills,
    /// The density grids (the atlas the kernel reads, the grid programs' pass), when the scene has any.
    pub grids: Option<GridsGpu>,
    /// Each medium's node (the media table's order).
    /// The scene has vertex colours (F_VCOLOR): three words per triangle in `words`, the edit room's included.
    pub vcolors: bool,
    pub media_nodes: Vec<u32>,
    /// Per medium: its node is hidden (`set_medium_hidden`). The live view runs no media pass while all are.
    pub media_hidden: std::cell::RefCell<Vec<bool>>,
    /// Room for in-place geometry edits (`EditRoom`), and the scene's 32-bit arrays (heap.rs `Stream`: the
    /// environment's CDF, the procedural region, the vertex colours), with the CDF's length and the region's words
    /// in use (the room follows them).
    pub room: EditRoom,
    pub words: Heap<Stream>,
    pub cdf_words: usize,
    pub region_words: usize,
    /// The environment map (`put_env` replaces it: a dynamic sky's sun moved, another sky).
    pub env_tex: wgpu::Texture,
    /// Translucent particles' acceleration structures and pass resources (hardware RT).
    pub translucent: Option<Translucent>,
}

/// Runs of the light list the GPU takes again from their triangles (emitter_fill.wgsl; lib.rs `EmitterRuns`): the
/// runs, the pass's bind group, and its dispatch (the runs' count, the longest's length).
pub struct EmitterFill {
    runs: wgpu::Buffer,
    bg: wgpu::BindGroup,
    count: u32,
    longest: u32,
}

/// A scene's fill a frame: of the particles' emitters, after their passes. None where the scene has no such
/// emitters.
#[derive(Default)]
pub struct EmitterFills {
    pub particles: Option<EmitterFill>,
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
    /// The pass's own buffer besides: the last frame's vertices.
    last: wgpu::Buffer,
    nodes: Vec<u32>,
    systems: u32,
    max_count: u32,
    /// Not run yet: the first run's motion vectors are zero (no last frame).
    fresh: std::cell::Cell<bool>,
    /// The systems' own meshes (`scene::ParticleSystem::mesh`): each one's BLAS is built again after the pass.
    meshes: Vec<u32>,
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

/// Translucent particles (`transparent` materials; hardware RT): their own BLAS + TLAS over their vertices, which the
/// particle pass writes (`vertices` from `first_vertex`, three a triangle), from triangle `first` in the kernel's
/// tables; the translucency pass's parameters and volume training points (translucent.wgsl).
pub struct Translucent {
    pub tlas: wgpu::Tlas,
    /// The systems' meshes (`scene::ParticleSystem::mesh`), an instance of the TLAS each: their BLASes are built with
    /// the other particle meshes' after the pass (`Gpu::build_particles`).
    meshes: Vec<u32>,
    pub params: wgpu::Buffer,
    pub points: wgpu::Buffer,
    pub count: wgpu::Buffer,
}

impl Uploaded {
    /// The light table as it is now, and its groups' spheres (lib.rs `light_groups`).
    pub fn put_lights(&self, q: &wgpu::Queue, lights: &[crate::scene::GpuLight]) {
        self.heap.write(q, Table::Lights, 0, bytemuck::cast_slice(lights));
        self.heap.write(q, Table::LightGroups, 0, bytemuck::cast_slice(&crate::light_groups(lights)));
    }

    /// Where the kernel finds each table and stream (`Params::table_first`, `word_first`): written with the
    /// parameters, every time (a section that grew moves those after it).
    pub fn place(&self, p: &mut Params) {
        let first = self.heap.firsts();
        p.table_first = std::array::from_fn(|k| std::array::from_fn(|j| first.get(k * 4 + j).copied().unwrap_or(0)));
        let first = self.words.firsts();
        for (k, at) in first.iter().enumerate().take(6) {
            p.word_first[k / 4][k % 4] = *at;
        }
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
    /// The grid programs the pipeline was made from (none without one).
    programs: Vec<(String, String, String)>,
    grids: Vec<GridGpu>,
    step: std::cell::Cell<u32>,
}

/// grid_sim.wgsl with the scene's grid programs (`fn gp_<name>`) and their dispatch.
fn grid_sim_source(programs: &[(String, String, String)]) -> String {
    let base = include_str!("../shaders/grid_sim.wgsl");
    let mut f = String::new();
    let mut seen = std::collections::HashSet::new();
    for (_, _, helpers) in programs {
        if !helpers.is_empty() && seen.insert(helpers.clone()) {
            f.push_str(helpers);
            f.push('\n');
        }
    }
    for (_, code, _) in programs {
        f.push_str(code);
        f.push('\n');
    }
    f.push_str("fn grid_program(f: u32, s: Cell, i: CIn, p: array<vec4<f32>, 8>) -> Cell {\n    switch f {\n");
    for (k, (id, _, _)) in programs.iter().enumerate() {
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

/// Triangles a workgroup of the skinning pass, and workgroups a row of its dispatch (skin.wgsl).
const SKIN_BLOCK: u32 = 64;
const SKIN_ROW: u32 = 32768;

/// skin.wgsl `Job`: a mesh that deforms: its first triangle in the scene's and how many, its first triangle among the
/// corners, its palette's first matrix.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SkinJob {
    first: u32,
    count: u32,
    source: u32,
    palette: u32,
}

/// leaf_gather.wgsl `Job`: a mesh: its first triangle in the scene's and how many, its first vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct LeafJob {
    first: u32,
    count: u32,
    vertex: u32,
    spare: u32,
}

/// The leaf-gather pass (shaders/leaf_gather.wgsl; software traversal): after the skinning pass has written a
/// mesh's vertices, its tree's leaves take their triangles again from them. A job each skinned model, the skinning
/// pass's order.
struct LeafGather {
    pipeline: wgpu::ComputePipeline,
    bg: Option<wgpu::BindGroup>,
    params: wgpu::Buffer,
    /// A frame's work: (job, its first triangle of the block), `SKIN_BLOCK` triangles each.
    blocks: wgpu::Buffer,
    table: wgpu::Buffer,
    jobs: Vec<LeafJob>,
}

fn leaf_gather_bind_group(d: &wgpu::Device, l: &LeafGather, vertices: &wgpu::Buffer, words: &wgpu::Buffer, leaves: &wgpu::Buffer) -> wgpu::BindGroup {
    let entries: Vec<wgpu::BindGroupEntry> = [&l.params, &l.blocks, &l.table, vertices, words, leaves].iter().enumerate().map(|(k, b)| buffer_entry(k as u32, b)).collect();
    d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("leaf gather"), layout: &l.pipeline.get_bind_group_layout(0), entries: &entries })
}

/// The skinning pass of a scene (shaders/skin.wgsl): the meshes that deform (a job each, `scene::Skinned`'s index)
/// are skinned from their models' corners and their poses' palettes into the scene's triangles, attributes and BLAS
/// input. Hardware RT.
pub struct SkinGpu {
    pipeline: wgpu::ComputePipeline,
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    /// A frame's work: (job, its first triangle of the block), `SKIN_BLOCK` triangles each.
    blocks: wgpu::Buffer,
    table: wgpu::Buffer,
    corners: wgpu::Buffer,
    /// Every job's palette, three rows a matrix (`skin::PaletteRows`).
    palette: wgpu::Buffer,
    /// Per job, and how many matrices its palette is.
    jobs: Vec<(SkinJob, u32)>,
    /// Each source's first triangle among the corners (`Scene::skin_sources`), and the palette's matrices in use.
    source_first: Vec<u32>,
    palette_end: u32,
    /// Software traversal: the pass that takes the trees' leaves again from the vertices this one wrote.
    leaves: Option<LeafGather>,
}

impl SkinGpu {
    /// (Tests: the bytes of the jobs' table, the palettes' and the blocks' buffers.)
    #[cfg(test)]
    pub fn rooms(&self) -> [u64; 3] {
        [self.table.size(), self.palette.size(), self.blocks.size()]
    }

    /// The job of a mesh an edit placed, at `slot` (a new one, or one whose mesh is gone): its triangles from `first`
    /// in the scene's, skinned from `source`'s corners by a palette of `palette_len` matrices. The pass's buffers
    /// grow with it (the table written again whole; the palette's and the blocks' contents are a dispatch's, and
    /// need no copy). True where one was made again: the pass's bind group is then to be (`Gpu::rebind_scene`).
    pub fn set_job(&mut self, d: &wgpu::Device, q: &wgpu::Queue, slot: usize, first: u32, count: u32, source: u32, palette_len: u32) -> bool {
        let room = |label: &str, size: u64| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut grew = false;
        // Its palette: the slot's own where that is as long, else new room at the end.
        let palette = match self.jobs.get(slot) {
            Some((j, len)) if *len == palette_len => j.palette,
            _ => {
                self.palette_end += palette_len;
                self.palette_end - palette_len
            }
        };
        let job = SkinJob { first, count, source: self.source_first[source as usize], palette };
        if slot < self.jobs.len() {
            self.jobs[slot] = (job, palette_len);
        } else {
            self.jobs.push((job, palette_len));
        }
        if self.jobs.len() as u64 * 16 > self.table.size() {
            self.table = room("skin jobs", self.jobs.len() as u64 * 32);
            let table: Vec<SkinJob> = self.jobs.iter().map(|j| j.0).collect();
            q.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
            grew = true;
        } else {
            q.write_buffer(&self.table, slot as u64 * 16, bytemuck::bytes_of(&job));
        }
        if self.palette_end as u64 * 48 > self.palette.size() {
            self.palette = room("skin palette", self.palette_end as u64 * 96);
            grew = true;
        }
        let blocks: u64 = self.jobs.iter().map(|j| j.0.count.div_ceil(SKIN_BLOCK) as u64).sum();
        if blocks * 8 > self.blocks.size() {
            self.blocks = room("skin blocks", blocks * 16);
            grew = true;
        }
        grew
    }
}

fn skin_bind_group(d: &wgpu::Device, pipeline: &wgpu::ComputePipeline, own: [&wgpu::Buffer; 5], vertices: &wgpu::Buffer) -> wgpu::BindGroup {
    let mut entries: Vec<wgpu::BindGroupEntry> = own.iter().enumerate().map(|(k, b)| buffer_entry(k as u32, b)).collect();
    entries.push(buffer_entry(5, vertices));
    d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("skin"), layout: &pipeline.get_bind_group_layout(0), entries: &entries })
}

/// Stateful GPU particles (gpu_particles.wgsl; ROADMAP § 6 step 5a): the pass, its tables and buffers, and per system
/// what the CPU keeps between frames: where the next emission starts in its ring of slots, the rate's fraction carried.
pub struct GpuParticlesGpu {
    pipeline: wgpu::ComputePipeline,
    /// Timestamps around the systems' BLAS rebuild (`STAMP_WGSL`, as `ParticleGpu`'s).
    stamp: (wgpu::ComputePipeline, wgpu::BindGroup),
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    table: wgpu::Buffer,
    last: wgpu::Buffer,
    state: wgpu::Buffer,
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
    /// The systems' own meshes (as `ParticleGpu`'s).
    meshes: Vec<u32>,
}

/// gpu_particles.wgsl with the scene's particle programs (`fn pe_` / `pu_<name>`) and their dispatch.
fn gpu_particle_source(scene: &Scene) -> String {
    let base = include_str!("../shaders/gpu_particles.wgsl");
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
    format!("{RECORDS}{head}{f}{tail}")
}

/// particles.wgsl with the scene's particle functions (`fn pf_<name>`) and their dispatch.
fn particle_source(scene: &Scene) -> String {
    let base = include_str!("../shaders/particles.wgsl");
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
    format!("{RECORDS}{head}{functions}{tail}")
}

/// Hardware acceleration structures: a BLAS a mesh, one TLAS over their instances and the procedural objects'.
pub struct Accel {
    pub tlas: wgpu::Tlas,
    /// What the BLASes are built from: the vertices (`Uploaded::vertices`: a record's first 12 bytes its position) and
    /// the triangles' indices (the words' heap, from word `index_first` on). Handles of the scene's buffers as they
    /// are now (`Gpu::rebind_scene` takes them again when one grew).
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub index_first: u32,
    /// By mesh: its first vertex and how many (`indexed::Indexed::meshes`).
    pub mesh_verts: Vec<(u32, u32)>,
    /// One per instanced mesh (model space), referenced by the TLAS's model instances.
    meshes: Vec<wgpu::Blas>,
    /// Per mesh: its (first pool triangle, count) geometries and their sizes (`mesh_entries`).
    mesh_geo: Vec<(Vec<(u32, u32)>, Vec<wgpu::BlasTriangleGeometrySizeDescriptor>)>,
    /// The build's triangle BLAS bytes per triangle (Metal's sizes): edits' memory estimates (live/mod.rs `room_stats`).
    pub blas_per_tri: f64,
}

/// What is above the triangles for an edit in place: hardware traversal's TLAS and BLASes, or software traversal's
/// units (units.rs). An instance's slot (its TLAS index, its unit's) is its kernel record.
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
    pub fn place(&mut self, slot: usize, k: u32, world: glam::Mat4, cast_shadow: bool, hidden: bool) {
        match self {
            Top::Hw(a) => a.tlas[slot] = Some(wgpu::TlasInstance::new(&a.meshes[k as usize], tlas_rows(world), slot as u32, instance_mask(cast_shadow, hidden))),
            Top::Sw(u) => u.set(slot, u.unit(k, world, hidden)),
        }
    }

    /// A procedural object's instance at `slot` (its AABB BLAS). Hardware traversal only: software traversal finds
    /// procedural objects by their records (`trace_procs`), and its edits of them rebuild the scene.
    pub fn place_blas(&mut self, slot: usize, blas: &wgpu::Blas, world: glam::Mat4, mask: u8) {
        if let Top::Hw(a) = self {
            a.tlas[slot] = Some(wgpu::TlasInstance::new(blas, tlas_rows(world), slot as u32, mask));
        }
    }

    /// Whether instanced mesh `k` can be placed: always under hardware traversal (its BLAS); under software traversal
    /// if it has a tree (one without is no unit, and is not drawn).
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
/// Triangles a batch of the scene's first BLAS build holds at most (`Gpu::upload`): the build's scratch buffer is the sum
/// of its BLASes' (on an RTX GPU about as large as the BLASes themselves).
const BLAS_BATCH_TRIS: u64 = 4_000_000;

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
/// What more than one shader reads or writes, declared once: a file of the kernel, of the particle passes and of
/// the live passes (live/targets.rs `live_source`).
const RECORDS: &str = include_str!("../shaders/records.wgsl");

/// (pathtrace.wgsl first: a module's imports and directives come before its declarations.)
const KERNEL_FILES: [&str; 6] = [
    include_str!("../shaders/pathtrace.wgsl"),
    RECORDS,
    include_str!("../shaders/sample.wgsl"),
    include_str!("../shaders/trace_units.wgsl"),
    include_str!("../shaders/trace_hw.wgsl"),
    include_str!("../shaders/translucent.wgsl"),
];

/// The kernel's source: its files as one WESL module, every device's and scene's variant in it under `@if`.
pub fn kernel_template() -> String {
    KERNEL_FILES.join("\n")
}

/// What a kernel is compiled for (pathtrace.wgsl's head names the flags): the device's traversal, switches, binding
/// layout and loop guard, and the scene's features that change the kernel's declarations (`F_SOURCE`; the others
/// switch code under `override FEAT`).
#[derive(Clone, Copy, Debug, Default)]
pub struct KernelVariant {
    pub hwrt: bool,
    pub switches: KernelSwitches,
    /// The scene's features: `F_SOURCE`'s are flags.
    pub features: u32,
    /// The merged layout (merged.rs).
    pub merged: bool,
    /// The seven typed tables read from the tables' buffer whole, as the merged layout reads them, with the layout as
    /// written otherwise (`GpuOptions::table_views`).
    pub whole_tables: bool,
    /// The loop guard's limit, in a kernel with the guard (`loop_guard_of`, `guard_loops`).
    pub guard: Option<u32>,
}

impl KernelVariant {
    /// A device's traversal and switches, a scene's features; the layout as written, no guard.
    pub fn new(hwrt: bool, switches: KernelSwitches, features: u32) -> KernelVariant {
        KernelVariant { hwrt, switches, features, ..Default::default() }
    }

    /// The flags of the kernel's conditional translation (pathtrace.wgsl's head).
    pub fn flags(&self) -> [(&'static str, bool); 11] {
        let (k, f) = (&self.switches, |bit: u32| self.features & bit != 0);
        [
            ("hwrt", self.hwrt),
            ("strict", k.strict),
            ("cutout_lite", k.cutout_lite >= 1),
            ("cutout_lite2", k.cutout_lite >= 2),
            ("last_direct", k.last_direct),
            ("last_sun", k.last_sun),
            ("light_pick", k.light_pick),
            ("units", f(F_UNITS)),
            ("merged", self.merged),
            ("whole_tables", self.whole_tables),
            ("guard", self.guard.is_some()),
        ]
    }

    /// The numbers the variant is compiled for, WESL constants (`import constants::`): the loop guard's count's
    /// words and its limit.
    pub fn constants(&self) -> Vec<(String, u32)> {
        use crate::merged::GUARD_WORDS;
        let mut out = vec![];
        if self.merged || self.guard.is_some() {
            out.push(("GUARD_WORDS".into(), GUARD_WORDS));
        }
        if let Some(n) = self.guard {
            out.push(("LOOP_GUARD_LIMIT".into(), n));
        }
        out
    }
}

/// A kernel's WGSL: `src` (the template, the scene's functions spliced in) for a variant: WESL's conditional
/// translation for its flags, with its constants, then the loop guard if it has one (`guard_loops`).
pub fn kernel_wgsl(src: &str, v: &KernelVariant) -> String {
    try_kernel_wgsl(src, v).unwrap_or_else(|e| panic!("{e}"))
}

/// `kernel_wgsl`, saying what did not compile instead of panicking (a scene's own functions are a game's code).
pub fn try_kernel_wgsl(src: &str, v: &KernelVariant) -> Result<String, String> {
    let mut out = try_wesl_wgsl(src, &v.flags(), &v.constants())?;
    if v.guard.is_some() {
        guard_loops(&mut out);
    }
    Ok(out.to_string())
}

/// A scene's functions as packages built before the kernel was WESL (#70) carry them: the scene library's hash
/// (packages/scene/src/materials.ts `std_hash`, `std_hash3`) was `a * b ^ c * d`, which naga took and WESL, by the
/// specification's grammar (a bitwise operator's operands are unary), does not. Those packages keep the old library
/// in their bundles, so the two expressions are parenthesised here as the library now writes them, in the order naga
/// read them: the same hash, the same picture. Anything else is left as it is.
pub fn legacy_wgsl(code: String) -> String {
    const OLD: [(&str, &str); 2] = [
        (
            "std_pcg(bitcast<u32>(x) * 73856093u ^ bitcast<u32>(y) * 19349663u ^ bitcast<u32>(z) * 83492791u)",
            "std_pcg((bitcast<u32>(x) * 73856093u) ^ (bitcast<u32>(y) * 19349663u) ^ (bitcast<u32>(z) * 83492791u))",
        ),
        (
            "let v = bitcast<u32>(x) * 73856093u ^ bitcast<u32>(y) * 19349663u ^ bitcast<u32>(z) * 83492791u;",
            "let v = (bitcast<u32>(x) * 73856093u) ^ (bitcast<u32>(y) * 19349663u) ^ (bitcast<u32>(z) * 83492791u);",
        ),
    ];
    if !OLD.iter().any(|(old, _)| code.contains(old)) {
        return code;
    }
    OLD.iter().fold(code, |c, (old, new)| c.replace(old, new))
}

/// `src` through wesl-rs (WESL's reference compiler in Rust): conditional translation for `flags`, imports of the
/// `constants` package from `constants`. Every flag the source names must be given (one that is not is an error, not
/// off), and every constant it imports.
pub fn wesl_wgsl(src: &str, flags: &[(&str, bool)], constants: &[(String, u32)]) -> wesl::syntax::TranslationUnit {
    try_wesl_wgsl(src, flags, constants).unwrap_or_else(|e| panic!("{e}"))
}

/// `wesl_wgsl`, with what did not compile as an error.
pub fn try_wesl_wgsl(src: &str, flags: &[(&str, bool)], constants: &[(String, u32)]) -> Result<wesl::syntax::TranslationUnit, String> {
    use wesl::{syntax::ModulePath, CompileOptions, Feature, Features};
    let mut features = Features { default: Feature::Error, ..Default::default() };
    for &(name, on) in flags {
        features.set(name, on);
    }
    let constants = constants.iter().map(|(name, n)| format!("public const {name} = {n}u;\n")).collect();
    let resolver = KernelResolver { src, constants };
    let options = CompileOptions { imports: true, condcomp: true, strip: false, validate: false, sourcemap: false, mangler: wesl::ManglerKind::None, features, ..Default::default() };
    let path: ModulePath = "package::kernel".parse().expect("a module path");
    wesl::compile(&path, &options, &resolver).map(|r| r.syntax).map_err(|e| format!("the kernel's WESL: {e}"))
}

/// The modules a kernel's WESL sees: itself, and WESL's `constants` package (`wesl_wgsl`).
struct KernelResolver<'a> {
    src: &'a str,
    constants: String,
}

impl wesl::resolver::Resolver for KernelResolver<'_> {
    fn resolve_source<'a>(&'a self, path: &wesl::syntax::ModulePath) -> Result<std::borrow::Cow<'a, str>, wesl::error::ResolveError> {
        use wesl::syntax::PathOrigin;
        match &path.origin {
            PathOrigin::Package(p) if p == "constants" => Ok(std::borrow::Cow::Borrowed(&self.constants)),
            PathOrigin::Absolute if path.components == ["kernel"] => Ok(std::borrow::Cow::Borrowed(self.src)),
            _ => Err(wesl::error::ResolveError::ModuleNotFound(path.clone(), "the kernel imports nothing but `constants`".into())),
        }
    }
}

/// The loop guard (docs/RUNTIME.md "The engine in a browser"): every `loop`, `for` and `while` of a variant, the
/// scene's functions' too, begins with `if (loop_guard()) { break; }` (pathtrace.wgsl declares `loop_guard`, under
/// `@if(guard)`). A pass over the variant's syntax tree, as a compiler instruments code: the tree knows what a loop
/// and its body are, so nothing the text says (comments, strings, a loop without braces) can mislead it.
/// A `for` loop that ends by its own count: `for (var i = N; i < M; i++)` (or `<=`), N and M integer literals, whose
/// body does not assign `i`. The guard leaves it out, as Tint's PreventInfiniteLoops leaves loops it can prove finite.
fn counted(f: &wesl::syntax::ForStatement) -> bool {
    let (Some(init), Some(cond), Some(update)) = (&f.initializer, &f.condition, &f.update) else { return false };
    let lit = |t: &str| {
        let t = t.trim().trim_end_matches(['u', 'i']);
        !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit())
    };
    let init = init.to_string();
    let Some((name, start)) = init.trim().trim_end_matches(';').strip_prefix("var ").and_then(|r| r.split_once('=')) else { return false };
    let name = name.split(':').next().unwrap_or("").trim();
    let cond = cond.to_string();
    let bound = cond.trim().trim_start_matches('(').trim_end_matches(')').strip_prefix(name).map(str::trim_start).and_then(|r| r.strip_prefix("<=").or_else(|| r.strip_prefix('<')));
    if name.is_empty() || !lit(start) || !bound.is_some_and(lit) || update.to_string().trim().trim_end_matches(';') != format!("{name}++") {
        return false;
    }
    // Not assigned in the body: `name` as an identifier followed by `=` (not `==`), `op=`, `++` or `--`.
    let b = f.body.to_string();
    let bytes = b.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    !b.match_indices(name).any(|(k, _)| {
        let before = k.checked_sub(1).map(|p| bytes[p]);
        let after = b[k + name.len()..].trim_start();
        before.is_none_or(|c| !ident(c) && c != b'.') && !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            && (after.starts_with("++") || after.starts_with("--") || (after.starts_with('=') && !after.starts_with("==")) || ["+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>="].iter().any(|op| after.starts_with(op)))
    })
}

pub fn guard_loops(tu: &mut wesl::syntax::TranslationUnit) {
    use wesl::syntax::{CompoundStatement, GlobalDeclaration, Statement, StatementNode};
    fn body(b: &mut CompoundStatement, guard: &StatementNode) {
        for s in b.statements.iter_mut() {
            statement(s, guard);
        }
    }
    fn opened(b: &mut CompoundStatement, guard: &StatementNode) {
        body(b, guard);
        b.statements.insert(0, guard.clone());
    }
    fn statement(s: &mut StatementNode, guard: &StatementNode) {
        match s.node_mut() {
            Statement::Compound(c) => body(c, guard),
            Statement::If(i) => {
                body(&mut i.if_clause.body, guard);
                for e in i.else_if_clauses.iter_mut() {
                    body(&mut e.body, guard);
                }
                if let Some(e) = i.else_clause.as_mut() {
                    body(&mut e.body, guard);
                }
            }
            Statement::Switch(sw) => sw.clauses.iter_mut().for_each(|c| body(&mut c.body, guard)),
            Statement::Loop(l) => {
                opened(&mut l.body, guard);
                if let Some(c) = l.continuing.as_mut() {
                    body(&mut c.body, guard);
                }
            }
            Statement::For(f) if counted(f) => body(&mut f.body, guard),
            Statement::For(f) => opened(&mut f.body, guard),
            Statement::While(w) => opened(&mut w.body, guard),
            _ => {}
        }
    }
    let guard: Statement = "if (loop_guard()) { break; }".parse().expect("the guard's statement");
    let guard = StatementNode::from(guard);
    for d in tu.global_declarations.iter_mut() {
        if let GlobalDeclaration::Function(f) = d.node_mut() {
            body(&mut f.body, &guard);
        }
    }
}

/// The loop guard's limit for every kernel, if one is set (`guard_loops`: every loop of a kernel bounded, so a
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
    /// The seven tables the kernel reads at every hit or light (materials, lights, emitters, texture rects, objects,
    /// instances, media) as views of their sections, each a binding of its own (merged.rs `TABLE_VIEWS`), or read from
    /// the tables' buffer whole by the offsets in the parameters, as the merged layout reads them. None: views, but on
    /// Android: Adreno 740's driver read the views wrong (docs/RUNTIME.md "Android").
    pub table_views: Option<bool>,
    /// A directory to keep compiled pipelines in between runs (pipeline_cache.rs). None: nothing is kept.
    pub pipeline_cache: Option<std::path::PathBuf>,
    pub kernel: KernelSwitches,
    /// The live renderer's approximations (`A_*`). None: all of them (`A_LIVE`). 0: a live view whose kernel is the
    /// offline one's estimator, slower, for holding its picture to the offline render's.
    pub approx: Option<u32>,
}

impl GpuOptions {
    /// THREERT_TABLE_VIEWS=1|0, THREERT_PIPELINE_CACHE=<directory>, THREERT_APPROX=<bits of `A_*`>, and the
    /// kernel's switches (`KernelSwitches::from_env`).
    pub fn from_env() -> GpuOptions {
        Self::from_vars(&|name| std::env::var(name).ok(), cfg!(target_os = "android"))
    }

    /// `from_env`, of these variables on this platform.
    fn from_vars(var: &dyn Fn(&str) -> Option<String>, android: bool) -> GpuOptions {
        GpuOptions {
            table_views: var("THREERT_TABLE_VIEWS").map(|v| v == "1"),
            pipeline_cache: var("THREERT_PIPELINE_CACHE").filter(|d| !d.is_empty()).map(std::path::PathBuf::from),
            kernel: KernelSwitches::from_vars(var, android),
            approx: var("THREERT_APPROX").and_then(|v| v.trim().parse().ok()),
        }
    }

    /// Whether the kernel reads its tables from views of their sections: as asked, else everywhere but Android; never
    /// in the merged layout.
    fn table_views_on(&self, merged: bool, android: bool) -> bool {
        !merged && self.table_views.unwrap_or(!android)
    }
}

/// The kernel's switches: changes to its source for a GPU that the source as written is wrong or too slow on. Each
/// is a flag of the kernel's conditional translation (`KernelVariant::flags`; pathtrace.wgsl, trace_hw.wgsl and
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
    /// THREERT_LAST_DIRECT=sun (off everywhere unless set; the ThreeRT clients set it for a budget of one bounce on
    /// Linux). The path's last vertex takes the directional lights' direct light alone (one shadow ray a light): what
    /// one bounce indoors lacks most (sunlit floor lighting the ceiling: Daylight Home's ceiling was black on a Steam
    /// Deck), at a fraction of `last_direct`'s cost where a scene has many local lights (OpenStrikeRT's Foundry on the
    /// Deck: +1 ms a frame, `last_direct` +8). `last_direct` includes it.
    pub last_sun: bool,
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
        let sun = var("THREERT_LAST_DIRECT").is_some_and(|v| v.trim() == "sun");
        KernelSwitches { strict: num("THREERT_KERNEL_STRICT", on) != 0, cutout_lite: num("THREERT_CUTOUT_LITE", on), last_direct: !sun && num("THREERT_LAST_DIRECT", on) != 0, last_sun: sun, light_pick: num("THREERT_LIGHT_PICK", 0) != 0 }
    }
}

type ModuleCache = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<(u64, u32), wgpu::ShaderModule>>>;
type PipelineMap<K, V = wgpu::ComputePipeline> = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<K, V>>>;

/// What a kernel variant's module and pipelines are made from and kept in: a device's, cloned to the thread that
/// makes a variant the frame does not wait for (`Gpu::warm_kernel`). A variant is looked up, made without a lock
/// held (a compile takes long, and the frame looks its own up meanwhile), then kept; made twice at once, the first
/// kept stays.
#[derive(Clone)]
struct KernelMaker {
    device: wgpu::Device,
    source: std::sync::Arc<String>,
    /// The kernel as written (`Gpu::module`).
    module: wgpu::ShaderModule,
    /// The device's part of a variant (its features 0, its guard a kernel's own).
    variant: KernelVariant,
    layout: wgpu::PipelineLayout,
    tl_layout: Option<wgpu::PipelineLayout>,
    modules: ModuleCache,
    pipelines: PipelineMap<(u32, u64, bool, u32)>,
    passes: PipelineMap<(u32, u64, &'static str, u32)>,
    tl: PipelineMap<(u32, u64, bool, u32), TranslucentPipelines>,
}

impl KernelMaker {
    /// The kernel module for a scene: the shared one, or with its material functions spliced in.
    fn module(&self, k: &Kernel) -> wgpu::ShaderModule {
        let patches = k.features & F_SOURCE;
        if patches == 0 && k.functions.is_empty() && k.volume_functions.is_empty() && k.geometry_functions.is_empty() {
            return self.module.clone();
        }
        if let Some(m) = self.modules.lock().unwrap().get(&(k.hash, patches)) {
            return m.clone();
        }
        let splice = |src: &str, marker: &str, code: &str| -> String {
            if code.is_empty() {
                return src.to_string();
            }
            let (head, rest) = src.split_once(&format!("// @{marker}-begin")).unwrap();
            let (_, tail) = rest.split_once(&format!("// @{marker}-end")).unwrap();
            format!("{head}{code}{tail}")
        };
        let src = splice(&splice(&splice(&self.source, "material-functions", &k.functions), "volume-functions", &k.volume_functions), "geometry-functions", &k.geometry_functions);
        // The variant: this device's traversal, switches, layout and loop guard, this scene's features.
        let v = KernelVariant { features: patches, guard: loop_guard_of(patches & F_UNITS != 0), ..self.variant };
        // A scene's functions that do not compile (a game's own WGSL) cost the game its functions, not the
        // app its renderer: the kernel is made without them, and the game's log says why. (A panic here took
        // the engine down, and ThreeRT for Android then took the device for one without a renderer.)
        let wgsl = try_kernel_wgsl(&src, &v).unwrap_or_else(|e| {
            let msg = format!("this scene's material, volume or geometry functions do not compile, so it is drawn without them: {e}");
            #[cfg(not(target_arch = "wasm32"))]
            crate::script::engine_log("error", &msg);
            #[cfg(target_arch = "wasm32")]
            eprintln!("{msg}");
            kernel_wgsl(&self.source, &v)
        });
        let m = unsafe { self.device.create_shader_module_trusted(wgpu::ShaderModuleDescriptor { label: Some("pathtrace+functions"), source: wgpu::ShaderSource::Wgsl(wgsl.into()) }, kernel_checks()) };
        self.modules.lock().unwrap().entry((k.hash, patches)).or_insert(m).clone()
    }

    fn make(&self, k: &Kernel, live: bool, entry: &str, label: &str, layout: Option<&wgpu::PipelineLayout>, cache: Option<&wgpu::PipelineCache>) -> wgpu::ComputePipeline {
        let (module, constants) = (self.module(k), k.constants(live));
        self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout,
            module: &module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
            cache,
        })
    }

    /// The kernel's `main` for `k` (`live`: the viewport's), and whether it was made now.
    fn pipeline(&self, k: &Kernel, live: bool, cache: Option<&wgpu::PipelineCache>) -> (wgpu::ComputePipeline, bool) {
        let key = (k.features, k.hash, live, k.approx_of(live));
        if let Some(p) = self.pipelines.lock().unwrap().get(&key) {
            return (p.clone(), false);
        }
        let p = self.make(k, live, "main", if live { "pathtrace-live" } else { "pathtrace" }, Some(&self.layout), cache);
        (self.pipelines.lock().unwrap().entry(key).or_insert(p).clone(), true)
    }

    /// A live pass of `k` beside `main`, and whether it was made now.
    fn pass(&self, k: &Kernel, entry: &'static str, cache: Option<&wgpu::PipelineCache>) -> (wgpu::ComputePipeline, bool) {
        let key = (k.features, k.hash, entry, k.approx);
        if let Some(p) = self.passes.lock().unwrap().get(&key) {
            return (p.clone(), false);
        }
        let p = self.make(k, true, entry, entry, Some(&self.layout), cache);
        (self.passes.lock().unwrap().entry(key).or_insert(p).clone(), true)
    }

    /// The translucency pass's pipelines for `k`, and whether they were made now.
    fn translucent(&self, k: &Kernel, live: bool, cache: Option<&wgpu::PipelineCache>) -> (TranslucentPipelines, bool) {
        let key = (k.features, k.hash, live, k.approx_of(live));
        if let Some(p) = self.tl.lock().unwrap().get(&key) {
            return (p.clone(), false);
        }
        let make = |entry: &str| self.make(k, live, entry, entry, self.tl_layout.as_ref(), cache);
        let made = TranslucentPipelines { draw: make("translucent"), train: make("volume_train"), up: make("translucent_up") };
        (self.tl.lock().unwrap().entry(key).or_insert(made).clone(), true)
    }
}

/// The kernel's runtime checks: loop bounding alone, wgpu's default for it (every loop has a count its compiler can
/// see the end of). The kernel's loops end on sound data, which does not promise a compiler that they end; without
/// the bound, kernels Metal's compiler made lost the state of whole SIMD groups on a scene with cutouts, one build and
/// not the next. No bounds checks: all indices come from our own BVH / scene builder. THREERT_CHECKED, to diagnose a
/// driver: 0 = none, 1 = every check, else a mask (2 bounds, 4 loop bounding, 8 ray-query initialization tracking).
fn kernel_checks() -> wgpu::ShaderRuntimeChecks {
    kernel_checks_of(std::env::var("THREERT_CHECKED").ok().and_then(|v| v.parse().ok()))
}

fn kernel_checks_of(asked: Option<u32>) -> wgpu::ShaderRuntimeChecks {
    let v = asked.unwrap_or(4);
    if v == 1 {
        return wgpu::ShaderRuntimeChecks::checked();
    }
    let mut c = wgpu::ShaderRuntimeChecks::unchecked();
    c.bounds_checks = v & 2 != 0;
    c.force_loop_bounding = v & 4 != 0;
    c.ray_query_initialization_tracking = v & 8 != 0;
    c
}

pub fn tlas_rows(w: glam::Mat4) -> [f32; 12] {
    let row = |r: usize| [w.x_axis[r], w.y_axis[r], w.z_axis[r], w.w_axis[r]];
    let [r0, r1, r2] = [row(0), row(1), row(2)];
    [r0[0], r0[1], r0[2], r0[3], r1[0], r1[1], r1[2], r1[3], r2[0], r2[1], r2[2], r2[3]]
}

/// Entries of the live radiance cache (pathtrace.wgsl CACHE_SIZE).
pub const CACHE_SIZE: u64 = 1 << 18;
/// The kernel's `finish` pass (pathtrace.wgsl): a record's bytes (DEFERRED_VECS vec4s) after the 16 of their count,
/// and the invocations of a dispatch row (DEFERRED_ROW).
pub const DEFERRED_BYTES: u64 = 96;
pub const DEFERRED_ROW: u32 = 16384;
/// The bytes of a traced pixel's samples (records.wgsl `Pixel`), in a view's buffer of them (`Gpu::accum_buffer`).
pub const SAMPLE_BYTES: u64 = 80;

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

/// The kernel's group-1 buffers: the radiance cache, what the live passes read of a trace (live/targets.rs
/// `Targets`: the specular-lobe rays, the motion vectors, the directional lights' direct light) and the light tiles'
/// lists. In the merged layout they are the sections of one buffer (`heap::Live`).
pub struct KernelBufs {
    pub cache: CacheBuffers,
    pub refl: Buf,
    pub motion: Buf,
    pub direct: Buf,
    pub lists: Buf,
    /// A live frame's camera hits, 32 bytes a traced pixel (pathtrace.wgsl `first_store`), and in a scene with a
    /// mirror 96 more for a mirror chain (`Chain`): its first pass writes them, its paths start from them. Of the
    /// view's size at 32 bytes until a frame makes it for what it traces (`with_first`).
    pub first: Buf,
    /// The pixels a sample's trace leaves for `finish` (their count, then `DEFERRED_BYTES` each): the layout as
    /// written only. Of one record until a live view makes it for its traced size (`deferred_buffer`).
    pub deferred: Option<Buf>,
    /// The loop guard's count (`guard_loops`: its first word, the invocations a loop of which was stopped): with
    /// the guard, and always in the merged layout.
    pub guard: Option<Buf>,
    /// The merged layout: the one buffer.
    pub merged: Option<Heap<Live>>,
}

impl KernelBufs {
    /// Each in a buffer of its own: a view's five of these bytes (`VIEW_BUFFERS`), made (and `deferred` of one
    /// record: its view makes it for what it traces), with this radiance cache and loop guard's count.
    fn separate(d: &wgpu::Device, cache: CacheBuffers, guard: Option<Buf>, view: [u64; 5]) -> KernelBufs {
        let sb = |k: usize| Buf::whole(d.create_buffer(&wgpu::BufferDescriptor { label: Some(VIEW_BUFFERS[k]), size: view[k], usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }));
        KernelBufs { cache, refl: sb(0), motion: sb(1), direct: sb(2), lists: sb(3), first: sb(4), deferred: Some(deferred_buffer(d, 1)), guard, merged: None }
    }

    /// These for a view of another size: the five that follow the size made again, the radiance cache and the loop
    /// guard's count kept. The merged layout's are one buffer: all of it made again, the cache (as long) empty.
    pub fn resized(&self, d: &wgpu::Device, view: [u64; 5]) -> KernelBufs {
        match self.merged {
            Some(_) => KernelBufs::new(d, true, true, self.cache.val.size / 32, view),
            None => KernelBufs::separate(d, self.cache.clone(), self.guard.clone(), view),
        }
    }

    /// These with the camera hits' buffer of `bytes`. As written that buffer alone is another; the merged layout's
    /// are one buffer, all of it made again (the cache, as long, empty).
    pub fn with_first(&self, d: &wgpu::Device, bytes: u64) -> KernelBufs {
        let view = [self.refl.size, self.motion.size, self.direct.size, self.lists.size, bytes];
        match self.merged {
            Some(_) => KernelBufs::new(d, true, true, self.cache.val.size / 32, view),
            None => KernelBufs {
                cache: self.cache.clone(),
                refl: self.refl.clone(),
                motion: self.motion.clone(),
                direct: self.direct.clone(),
                lists: self.lists.clone(),
                first: Buf::whole(d.create_buffer(&wgpu::BufferDescriptor { label: Some(VIEW_BUFFERS[4]), size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false })),
                deferred: self.deferred.clone(),
                guard: self.guard.clone(),
                merged: None,
            },
        }
    }

    /// For a cache of `entries` and a view's five of these bytes (`VIEW_BUFFERS`).
    /// `guard`: the layout as written has the guard's count (`Gpu::guard_room`).
    pub fn new(d: &wgpu::Device, merged: bool, guard: bool, entries: u64, view: [u64; 5]) -> KernelBufs {
        if !merged {
            let guard = guard.then(|| Buf::whole(d.create_buffer(&wgpu::BufferDescriptor { label: Some("loop guard"), size: crate::merged::GUARD_WORDS as u64 * 4, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false })));
            return KernelBufs::separate(d, CacheBuffers::new(d, entries), guard, view);
        }
        // (The cache's keys and sums are as long as the kernel's struct says, whatever `entries`: `live_sizes`. A
        // section's range is the bytes asked for: a pass that binds it sees an array of that length.)
        let bytes = live_sizes(entries, view);
        let heap = Heap::<Live>::new(d, &bytes);
        let b = |s: Live| Buf { buffer: heap.buffer.clone(), offset: heap.at(s), size: bytes[s.index()] };
        KernelBufs { cache: CacheBuffers { keys: b(Live::CacheKeys), acc: b(Live::CacheAcc), val: b(Live::CacheVal) }, refl: b(Live::Refl), motion: b(Live::Motion), direct: b(Live::Direct), lists: b(Live::Lists), first: b(Live::First), deferred: None, guard: Some(b(Live::Guard)), merged: Some(heap) }
    }

    /// Where the kernel finds the vec4 sections in the merged layout (`Params::live_first`, `live_len`): written with
    /// the parameters of whatever is traced into these buffers.
    pub fn place(&self, p: &mut Params) {
        let Some(heap) = &self.merged else { return };
        let first = |s: Live| ((heap.at(s) - heap.at(Live::CacheVal)) / 16) as u32;
        p.live_first = [[first(Live::CacheVal), first(Live::Refl), first(Live::Motion), first(Live::Direct)], [first(Live::Lists), first(Live::First), (self.first.size / 16) as u32, 0]];
        p.live_len = [&self.refl, &self.motion, &self.direct, &self.lists].map(|b| (b.size / 16) as u32);
    }
}

/// `finish`'s buffer for `records` pixels: their count (16 bytes, cleared before each sample), then the records.
pub fn deferred_buffer(d: &wgpu::Device, records: u64) -> Buf {
    Buf::whole(d.create_buffer(&wgpu::BufferDescriptor { label: Some("deferred"), size: 16 + records.max(1) * DEFERRED_BYTES, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }))
}

/// The kernel's buffers that follow a view's size, in the order their bytes are given: the specular-lobe rays, the
/// motion vectors, the direct light, the light tiles' lists, the camera hits.
pub const VIEW_BUFFERS: [&str; 5] = ["refl", "motion", "direct", "light lists", "first hits"];

/// The bytes of the merged layout's group-1 buffer's sections (`heap::Live`'s order) for a cache whose values are
/// `entries` long and a view's five of these bytes. The cache's keys and sums and the loop guard's count are
/// members of the kernel's struct, of the kernel's lengths (pathtrace.wgsl `MergedLive`).
pub(crate) fn live_sizes(entries: u64, view: [u64; 5]) -> [u64; 9] {
    [CACHE_SIZE * 4, CACHE_SIZE * 32, crate::merged::GUARD_WORDS as u64 * 4, entries * 32, view[0], view[1], view[2], view[3], view[4]]
}

/// Group-1 bind group of the kernel: the radiance cache and the reflection resolve's inputs (`KernelBufs`); in the
/// merged layout, the one buffer.
/// `translucent`: for the translucency pass's layout (`tl_live_layout`), which has what that pass reads alone: the
/// radiance cache, the light tiles' lists and the camera hits.
pub fn live_bind_group(d: &wgpu::Device, layout: &wgpu::BindGroupLayout, b: &KernelBufs, translucent: bool) -> wgpu::BindGroup {
    fn e(binding: u32, b: &Buf) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry { binding, resource: b.binding() }
    }
    match &b.merged {
        Some(heap) => d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("live"), layout, entries: &[wgpu::BindGroupEntry { binding: crate::merged::LIVE_BINDING, resource: heap.buffer.as_entire_binding() }] }),
        None => {
            let mut entries = vec![e(1, &b.cache.keys), e(2, &b.cache.acc), e(3, &b.cache.val), e(crate::merged::LISTS_BINDING, &b.lists), e(crate::merged::FIRST_BINDING, &b.first)];
            if !translucent {
                entries.extend([e(5, &b.refl), e(6, &b.motion), e(7, &b.direct)]);
                entries.extend(b.deferred.as_ref().map(|r| e(crate::merged::DEFERRED_BINDING, r)));
            }
            entries.extend(b.guard.as_ref().map(|g| e(crate::merged::GUARD_BINDING, g)));
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("live"), layout, entries: &entries })
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
    /// The approximations its live pipelines are compiled with (`A_*`; an offline pipeline has none): the device's
    /// (`Gpu::approx`) unless a view sets others.
    pub approx: u32,
}

impl Kernel {
    /// A pipeline's constants: `LIVE`, `FEAT`, `APPROX`.
    fn constants(&self, live: bool) -> [(&'static str, f64); 3] {
        [("LIVE", if live { 1.0 } else { 0.0 }), ("FEAT", self.features as f64), ("APPROX", self.approx_of(live) as f64)]
    }

    fn approx_of(&self, live: bool) -> u32 {
        if live { self.approx } else { 0 }
    }

    /// A scene's material functions and their dispatch (pathtrace.wgsl `material_function` and its kin), as they are
    /// spliced into the kernel: `shaders` (name, code, helpers) in the order of their numbers; `translucent`: the
    /// functions of translucent particles' materials.
    fn material_functions(shaders: &[(String, String, String)], translucent: &std::collections::BTreeSet<i32>) -> String {
        let mut functions = String::new();
        if !shaders.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for (_, _, helpers) in shaders {
                if !helpers.is_empty() && seen.insert(helpers.clone()) {
                    functions.push_str(helpers);
                    functions.push('\n');
                }
            }
            for (_, code, _) in shaders {
                functions.push_str(code);
                functions.push('\n');
            }
            functions.push_str("fn material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut {\n    switch f {\n");
            for (k, (id, _, _)) in shaders.iter().enumerate() {
                functions.push_str(&format!("        case {k} {{ return mf_{id}(i, m, base); }}\n"));
            }
            functions.push_str("        default { return base; }\n    }\n}\n");
            // The translucency pass's (translucent.wgsl): only the functions of its particles' materials. With every
            // function in it, the Metal compiler failed on Tokyo's kernel once the city's ground had one (wetLayer).
            functions.push_str("fn translucent_material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut {\n    switch f {\n");
            for &k in translucent {
                if let Some((id, _, _)) = shaders.get(k as usize) {
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
                for (k, (id, code, _)) in shaders.iter().enumerate() {
                    if writes_field(code, field) {
                        functions.push_str(&format!("        case {k} {{ return mf_{id}(i, m, base); }}\n"));
                    }
                }
                functions.push_str("        default { return base; }\n    }\n}\n");
            }
        }
        functions
    }

    /// A scene's volume functions and their dispatch (pathtrace.wgsl `volume_function`), as they are spliced into the
    /// kernel: (name, code, helpers) in the order of their numbers.
    fn volume_functions_of(fns: &[(String, String, String)]) -> String {
        let mut volume_functions = String::new();
        if !fns.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for (_, _, helpers) in fns {
                if !helpers.is_empty() && seen.insert(helpers.clone()) {
                    volume_functions.push_str(helpers);
                    volume_functions.push('\n');
                }
            }
            for (_, code, _) in fns {
                volume_functions.push_str(code);
                volume_functions.push('\n');
            }
            volume_functions.push_str("fn volume_function(f: u32, i: VIn, p: array<vec4<f32>, 8>) -> VOut {\n    switch f {\n");
            for (k, (id, _, _)) in fns.iter().enumerate() {
                volume_functions.push_str(&format!("        case {k}u {{ return vf_{id}(i, p); }}\n"));
            }
            volume_functions.push_str("        default { return VOut(0.0, vec3<f32>(1.0), vec3<f32>(0.0)); }\n    }\n}\n");
        }
        volume_functions
    }

    /// This kernel with other volume functions (one that came or changed while the scene runs): another module.
    pub fn with_volume_functions(&self, fns: &[(String, String, String)]) -> Kernel {
        use std::hash::{Hash, Hasher};
        let volume_functions = legacy_wgsl(Kernel::volume_functions_of(fns));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.functions.hash(&mut h);
        volume_functions.hash(&mut h);
        self.geometry_functions.hash(&mut h);
        Kernel { volume_functions, hash: h.finish(), ..self.clone() }
    }

    /// This kernel with other material functions (a function added or changed while the scene runs): another module
    /// (`hash`), the volume and geometry functions as they are.
    pub fn with_material_functions(&self, shaders: &[(String, String, String)], translucent: &std::collections::BTreeSet<i32>) -> Kernel {
        use std::hash::{Hash, Hasher};
        let functions = legacy_wgsl(Kernel::material_functions(shaders, translucent));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        functions.hash(&mut h);
        self.volume_functions.hash(&mut h);
        self.geometry_functions.hash(&mut h);
        Kernel { functions, hash: h.finish(), ..self.clone() }
    }

    pub fn new(scene: &Scene) -> Kernel {
        // (The translucency pass's functions: those of its particles' materials.)
        let used: std::collections::BTreeSet<i32> = scene.translucent_mats().iter().filter_map(|&m| scene.materials.get(m as usize)).map(|m| m.tex1[3] - 1).filter(|&f| f >= 0).collect();
        let functions = Kernel::material_functions(&scene.shaders, &used);
        let volume_functions = Kernel::volume_functions_of(&scene.volume_fns);
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
        // (Functions from a package built before the kernel was WESL, as WESL reads them: `legacy_wgsl`.)
        let (functions, volume_functions, geometry_functions) = (legacy_wgsl(functions), legacy_wgsl(volume_functions), legacy_wgsl(geometry_functions));
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        functions.hash(&mut h);
        volume_functions.hash(&mut h);
        geometry_functions.hash(&mut h);
        Kernel { features: features(scene), functions, volume_functions, geometry_functions, hash: h.finish(), approx: A_LIVE }
    }
}

// The live renderer's approximations (`override APPROX` in pathtrace.wgsl, which says what each is; docs/RUNTIME.md
// "How live differs from offline").
pub const A_GLASS_SHADOWS: u32 = 1;
pub const A_MEDIA_SHADOWS: u32 = 2;
pub const A_CUTOUT_PATHS: u32 = 4;
pub const A_CUTOUT_EMITTERS: u32 = 8;
pub const A_FAR_EMITTERS: u32 = 16;
/// All of them: what a live view takes unless it asks otherwise (`GpuOptions::approx`).
pub const A_LIVE: u32 = 31;

// Kernel features (`override FEAT` in pathtrace.wgsl).
pub const F_MASKED: u32 = 1;
/// A particle system's mesh among the instances (`scene::INST_PARTICLES`): a hit on one has its particle's object.
pub const F_PART: u32 = 1024;
pub const F_FOG: u32 = 64;
pub const F_VOLUME: u32 = 128;
pub const F_TEX: u32 = 256;
pub const F_FUNC: u32 = 512;
// (1024: the emitter-reservoir experiment, removed 2026-11-13.)
pub const F_GLASS: u32 = 2048;
pub const F_COAT: u32 = 4096;
/// A material that can be a mirror alone (`mirror_possible`; pathtrace.wgsl's of the name): a live frame's first
/// pass follows what it shows, and the kernel has the code for it.
pub const F_MIRROR: u32 = 8192;
/// The scene has instances (scene::Instance): TLAS instances with their record as custom data, shaded from
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
/// Instance colours (`InstancedMesh.setColorAt`): object entries of 4 vec4s, the base colour times the 4th
/// (`scene::object_stride`).
pub const F_TINT: u32 = 524288;
/// three.js MeshBasicMaterial (`scene::MAT_UNLIT`): seen by camera rays, lit by nothing, lighting nothing.
pub const F_UNLIT: u32 = 1 << 20;
/// three.js `scene.background`: camera rays that miss see `Params::background`.
pub const F_BACKGROUND: u32 = 1 << 21;
/// three.js `vertexColors` (`scene::tri_colors`): a colour per triangle vertex multiplies the base colour.
pub const F_VCOLOR: u32 = 1 << 22;
/// Density grids (`scene::GridBuild`): the grid atlas (binding 19) and `vgrid` for volume functions. (Bit 24 is
/// F_AERIAL's.)
pub const F_GRID: u32 = 1 << 25;
/// More shadow-casting local lights than the kernel sums exactly at a camera hit (`RIS_EXACT`): they are resampled,
/// guided live by the light tiles (pathtrace.wgsl "Light tiles"; the live view's `light_tiles` pass runs for such a
/// scene).
pub const F_LISTS: u32 = 1 << 27;
/// A material function on an instanced mesh: its inputs outside full shading (pathtrace.wgsl `mat_in_at`) place the
/// triangle by its instance. Compiled out where no scene has one: the kernel's compiler works the placement out for
/// every call otherwise, taken or not (Kabukicho's 64,224 emitters with a function, none on an instance: frame GPU
/// +2.7 %).
pub const F_IFUNC: u32 = 1 << 28;
/// Glass among an instance's other sections: a shadow ray steps through a hit whose row says glass (trace_hw.wgsl;
/// software traversal skips such a triangle of a unit). Compiled out where no instance mixes the two: one that is all
/// glass is out of a shadow ray's way by its mask (`scene::rows_cast`).
pub const F_IGLASS: u32 = 1 << 29;
/// A mesh that deforms among the instances (a posed skinned model's): the first hit's motion takes its previous
/// frame's vertices from where its instance's record says. Compiled out where no mesh deforms: with the read in every
/// kernel a scene without one paid for it (a flight over a city: frame GPU +0.6 %, five pairs of five).
pub const F_DEFORM: u32 = 1 << 30;
/// Whether a scene with more such lights has the light tiles (`F_LISTS`): `THREERT_LIGHT_TILES=1|0`, else everywhere
/// but Android. On Adreno 740 they took OpenStrikeRT's Foundry from 31 to 48 ms a frame (249 x 140, one bounce):
/// there the lights are resampled as in a scene of fewer, unguided (docs/RUNTIME.md "Android").
pub fn light_tiles_on() -> bool {
    light_tiles_of(std::env::var("THREERT_LIGHT_TILES").ok().as_deref(), cfg!(target_os = "android"))
}

fn light_tiles_of(var: Option<&str>, android: bool) -> bool {
    var.map_or(!android, |v| v == "1")
}
/// Local lights the kernel sums exactly (pathtrace.wgsl `RIS_EXACT`; the test below holds the two together).
pub const RIS_EXACT: usize = 8;
/// The features that change the kernel's source (an `@if` flag) rather than switch code in it (`FEAT`).
pub const F_SOURCE: u32 = F_UNITS;
/// Software traversal's units (units.rs; shaders/trace_units.wgsl): the walk of a scene's instances, spliced into
/// the software traversal (F_SOURCE). Never with hardware traversal.
pub const F_UNITS: u32 = 1 << 26;
/// The sky's aerial perspective (SkyAtmosphere `aerialPerspective`; pathtrace.wgsl `aerial_tr`) on every path segment
/// that ends on a surface: compiled out where no scene has it.
pub const F_AERIAL: u32 = 1 << 24;
/// Features a scene needs; `THREERT_FEATURES` (bit mask) restricts them for profiling.
pub fn features(scene: &Scene) -> u32 {
    features_of(&scene.materials, scene.environment.features(), !scene.textures.is_empty(), !scene.shaders.is_empty(), instance_features(scene))
}

impl Params {
    /// What the kernel takes from the render preset and the camera's exposure: the bounce limit, the depth Russian
    /// roulette starts at, the firefly clamp on indirect paths (relative to the exposure), energy compensation.
    pub fn set_preset(&mut self, native: &crate::desc::NativePreset, exposure: &crate::desc::Exposure) {
        self.counts[2] = native.max_bounces;
        self.counts[3] = native.russian_roulette_depth;
        self.misc[0] = native.firefly_clamp(exposure);
        self.misc[2] = native.energy_compensation;
    }
}

/// Kernel features of the environment, which edits change: fog (F_FOG, with lit scattering F_VOLUME), a background
/// (F_BACKGROUND), the sky's aerial perspective (F_AERIAL; `aerial`: `Params::aerial`).
pub fn environment_features(fog: Option<&crate::desc::Fog>, background: bool, aerial: [f32; 4]) -> u32 {
    let mut f = if background { F_BACKGROUND } else { 0 } | if aerial[0] > 0.0 { F_AERIAL } else { 0 };
    if let Some(fog) = fog.filter(|fog| fog.density > 0.0) {
        f |= F_FOG | if fog.volumetric.as_ref().is_some_and(|v| v.scattering > 0.0) { F_VOLUME } else { 0 };
    }
    f
}

/// Kernel features of the scene's structure, which parameter edits do not change: F_INST and F_MASKED for the
/// instanced meshes (cutout triangles among them), F_MEDIA for participating media (F_HMEDIA for homogeneous ones),
/// F_PEMIT for opaque particles, F_PROC for procedural geometry, F_TINT for instance colours, F_IFUNC for a material
/// function on an instanced mesh, F_IGLASS for glass on one, F_DEFORM for a mesh that deforms.
pub fn instance_features(scene: &Scene) -> u32 {
    // (F_GRID also for a volume function that calls `vgrid` on a node without a grid: it reads zeros.)
    let grid = !scene.grids.is_empty() || (!scene.media.is_empty() && scene.volume_fns.iter().any(|(_, code, helpers)| code.contains("vgrid(") || helpers.contains("vgrid(")));
    let media = if scene.media.is_empty() { 0 } else { F_MEDIA } | if scene.media.iter().any(|m| m.a[2] != 0) { F_HMEDIA } else { 0 } | if grid { F_GRID } else { 0 };
    let particles = if scene.particles.iter().any(|p| !p.translucent) || !scene.gpu_particles.is_empty() { F_PEMIT } else { 0 } | if scene.procs.is_empty() { 0 } else { F_PROC } | if scene.tinted { F_TINT } else { 0 } | if scene.gpu_vcolors.is_empty() { 0 } else { F_VCOLOR } | if light_tiles_on() && scene.lights.iter().filter(|l| l.pos[3] != 0.0 && l.ext[3] >= 0.5).count() > RIS_EXACT { F_LISTS } else { 0 };
    if scene.gpu_instances.is_empty() {
        return media | particles;
    }
    // (An instance's rows say its materials and their classes.)
    let function = |mat: u32| scene.materials.get(mat as usize).is_some_and(|m| m.tex1[3] != 0);
    let rows = |i: &crate::scene::Instance| &scene.rows[i.rows as usize..][..scene.meshes[i.mesh as usize].sect_count as usize];
    let any_row = |bit: u32| scene.instances.iter().any(|i| rows(i).iter().any(|r| r[0] & bit != 0));
    let mixed_glass = scene.instances.iter().any(|i| i.bits & crate::scene::INST_GLASS != 0 && i.cast_shadow);
    let ifunc = if scene.rows.iter().any(|r| function(crate::scene::row_material(r))) { F_IFUNC } else { 0 };
    let iglass = if mixed_glass { F_IGLASS } else { 0 };
    let masked = any_row(crate::scene::ROW_CUTOUT);
    let deform = if scene.meshes.iter().any(|m| m.deforms) { F_DEFORM } else { 0 };
    // (Hardware traversal's: the particle passes run with it alone, and software traversal has such a mesh flat.)
    let part = if scene.units.is_none() && !(scene.particles.is_empty() && scene.gpu_particles.is_empty()) { F_PART } else { 0 };
    media | particles | ifunc | iglass | deform | part | F_INST | if masked { F_MASKED } else { 0 } | if scene.units.is_some() { F_UNITS } else { 0 }
}

/// Whether a material's own numbers leave a mirror alone possible (pathtrace.wgsl `mirror_possible`, `mirror_alone`):
/// its roughness under the kernel's `DELTA_ROUGHNESS`, metal whole, no glass. (Its textures can only lower them.)
pub fn mirror_possible(m: &crate::scene::GpuMaterial) -> bool {
    m.params[1] < 0.025 && m.params[0] >= 1.0 && m.layer[0] <= 0.0
}

/// `features` from the parts parameter edits change (the materials; `environment`: `environment_features`), and the
/// scene's structure (`instances`: `instance_features`).
pub fn features_of(ms: &[crate::scene::GpuMaterial], environment: u32, textures: bool, functions: bool, instances: u32) -> u32 {
    let mut f = instances | environment;
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
    if ms.iter().any(mirror_possible) {
        f |= F_MIRROR;
    }
    if let Some(mask) = std::env::var("THREERT_FEATURES").ok().and_then(|v| v.parse::<u32>().ok()) {
        f &= mask | F_INST | F_PROC | F_TINT; // not optional: they say what the instances' records / the object table hold
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

/// Reopen the selected Vulkan adapter with NGX's instance and device extensions. Keep the original adapter
/// available for fallback, and never replace a non-Vulkan adapter (e.g. when WGPU_BACKEND requests DX12).
#[cfg(dlss)]
async fn open_dlss(
    info: &wgpu::AdapterInfo, desc: &wgpu::DeviceDescriptor<'_>,
) -> Result<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue, DlssSdk, bool)> {
    const PROJECT: uuid::Uuid = uuid::uuid!("7e2053f1-3c7a-4d3c-9a60-d28593bcfdb7");
    let mut support = dlss_wgpu::FeatureSupport::default();
    let instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
    let instance = dlss_wgpu::create_instance(PROJECT, &instance_desc, &mut support)?;
    anyhow::ensure!(support.super_resolution_supported, "missing NGX Vulkan instance extensions");
    let adapter = instance.enumerate_adapters(wgpu::Backends::VULKAN).await.into_iter()
        .find(|adapter| adapter.get_info() == *info)
        .ok_or_else(|| anyhow!("the selected Vulkan adapter is unavailable in the NGX instance"))?;
    let (device, queue) = dlss_wgpu::request_device(PROJECT, &adapter, desc, &mut support, Some(desc.required_limits.clone()))?;
    anyhow::ensure!(support.super_resolution_supported, "missing NGX Vulkan device extensions");
    let sdk = dlss_wgpu::DlssSdk::new(PROJECT, device.clone())?;
    Ok((instance, adapter, device, queue, sdk, support.ray_reconstruction_supported))
}

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
        // Storage + uniform buffers and acceleration structures together (the translucency pass binds 24 storage
        // buffers, 25 with the loop guard's, 2 uniform ones and 2 structures; Metal allows 29).
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
        let device_desc = wgpu::DeviceDescriptor {
            label: Some("threert"),
            required_features: features,
            required_limits: limits,
            // Ray queries are still flagged experimental in wgpu 30.
            experimental_features: if hwrt { unsafe { wgpu::ExperimentalFeatures::enabled() } } else { wgpu::ExperimentalFeatures::disabled() },
            ..Default::default()
        };
        #[cfg(dlss)]
        let (instance, adapter, device, queue, dlss_sdk, dlss_rr) = {
            let dlss = if adapter_info.backend == wgpu::Backend::Vulkan && adapter_info.vendor == 0x10de
                && std::env::var("THREERT_DLSS").is_ok_and(|v| v == "1")
            {
                match open_dlss(&adapter_info, &device_desc).await {
                    Ok(opened) => Some(opened),
                    Err(err) => {
                        eprintln!("DLSS unavailable: {err:#}");
                        None
                    }
                }
            } else {
                None
            };
            match dlss {
                Some((instance, adapter, device, queue, sdk, rr)) => (instance, adapter, device, queue, Some(sdk), rr),
                None => {
                    let (device, queue) = adapter.request_device(&device_desc).await?;
                    (instance, adapter, device, queue, None, false)
                }
            }
        };
        #[cfg(not(dlss))]
        let (device, queue) = adapter.request_device(&device_desc).await?;
        let pipeline_cache = cache_feature.and_then(|_| crate::pipeline_cache::PipelineCache::open(&device, options.pipeline_cache.as_ref()?, &adapter_info));
        let whole_tables = !merged && !options.table_views_on(merged, cfg!(target_os = "android"));
        let switches = options.kernel;
        let approx = options.approx.unwrap_or(A_LIVE) & A_LIVE;
        let src = kernel_template();
        // Bounds checks are not needed: all indices come from our own BVH / scene builder.
        let module = unsafe {
            let wgsl = kernel_wgsl(&src, &KernelVariant { merged, whole_tables, guard: loop_guard_of(false), ..KernelVariant::new(hwrt, switches, 0) });
            device.create_shader_module_trusted(wgpu::ShaderModuleDescriptor { label: Some("pathtrace"), source: wgpu::ShaderSource::Wgsl(wgsl.into()) }, kernel_checks())
        };
        let [entries, live_entries, tl_entries] = kernel_layout_entries(LayoutShape { hwrt, merged, whole_tables, guard: guard_room(hwrt) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries });
        let live_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("live"), entries: &live_entries });
        let no_grid_view = grid_texture(&device, "no grid", [1, 1, 1], false).create_view(&Default::default());
        let grid_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid sim"),
            entries: &[
                layout_entry(0, UNIFORM_BUFFER),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: GRID_FORMAT, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: GRID_FORMAT, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
                layout_entry(6, storage_buffer(true)),
            ],
        });
        let no_live_bufs = KernelBufs::new(&device, merged, guard_room(hwrt), 1, [64; 5]);
        let no_live = live_bind_group(&device, &live_layout, &no_live_bufs, false);
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout), Some(&live_layout)], immediate_size: 0 });
        let (tl_layout, tl_pipeline_layout, tl_live_layout, no_live_tl) = if hwrt {
            let l = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("translucent"), entries: &tl_entries });
            // (What the pass reads of group 1: the radiance cache, the light tiles' lists, the camera hits, the loop guard's count. A stage's buffers are counted against Metal's limit.)
            let tl_vis: Vec<_> = live_entries.iter().filter(|e| merged || [1, 2, 3, crate::merged::LISTS_BINDING, crate::merged::FIRST_BINDING, crate::merged::GUARD_BINDING].contains(&e.binding)).copied().collect();
            let vl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("live, translucent"), entries: &tl_vis });
            let tpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("translucent"), bind_group_layouts: &[Some(&layout), Some(&vl), Some(&l)], immediate_size: 0 });
            let nv = live_bind_group(&device, &vl, &no_live_bufs, true);
            (Some(l), Some(tpl), Some(vl), Some(nv))
        } else {
            (None, None, None, None)
        };
        Ok(Gpu {
            #[cfg(dlss)]
            dlss_sdk,
            #[cfg(dlss)]
            dlss_rr,
            instance, adapter, timestamps, device, queue, adapter_info, whole_tables, switches, approx, approx_default: approx, pipeline_cache, source: std::sync::Arc::new(src), module, merged, modules: Default::default(), pipeline_layout: pl, pipelines: Default::default(), warming: Default::default(), layout, live_layout, no_live, no_live_bufs, no_grid_view, grid_layout, fill: Default::default(), hwrt, pages: Default::default(), streams: Default::default(), tl_layout, tl_pipeline_layout, tl_live_layout, no_live_tl, tl_pipelines: Default::default(), kernel_passes: Default::default(), proc_cache: Default::default(), proc_blases: Default::default(),
        })
    }

    /// Whether this device successfully initialized NGX with DLSS Super Resolution support.
    pub fn dlss_supported(&self) -> bool {
        #[cfg(dlss)]
        { self.dlss_sdk.is_some() }
        #[cfg(not(dlss))]
        { false }
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

    /// The approximations of the live views made on this device from now on (`A_*`; None: the options').
    pub fn set_approx(&mut self, approx: Option<u32>) {
        self.approx = approx.unwrap_or(self.approx_default) & A_LIVE;
    }

    /// The path-tracing kernel specialised for a scene (feature mask + material functions), compiled on first use.
    pub fn pipeline(&self, k: &Kernel, live: bool) -> wgpu::ComputePipeline {
        let (p, made) = self.maker().pipeline(k, live, self.cache());
        if made {
            self.save_pipeline_cache();
        }
        p
    }

    /// The entries of the live passes made so far (`kernel_pass`), for tests: which were made, and when.
    #[cfg(test)]
    pub fn kernel_passes_made(&self) -> Vec<&'static str> {
        self.kernel_passes.lock().unwrap().keys().map(|k| k.2).collect()
    }

    /// A live pass of a scene's kernel variant beside `main`: `first_hit` (the frame's camera rays, before its paths),
    /// `finish` (the pixels a sample's trace left), `media_train` (the volume voxels) or `media_composite` (camera
    /// rays through media, after the trace).
    pub fn kernel_pass(&self, k: &Kernel, entry: &'static str) -> wgpu::ComputePipeline {
        let (p, made) = self.maker().pass(k, entry, self.cache());
        if made {
            self.save_pipeline_cache();
        }
        p
    }

    /// The translucency pass's pipelines for a scene's kernel variant.
    pub fn translucent_pipelines(&self, k: &Kernel, live: bool) -> TranslucentPipelines {
        let (p, made) = self.maker().translucent(k, live, self.cache());
        if made {
            self.save_pipeline_cache();
        }
        p
    }

    /// What makes this device's kernel modules and pipelines, for whichever thread makes them.
    fn maker(&self) -> KernelMaker {
        KernelMaker {
            device: self.device.clone(),
            source: self.source.clone(),
            module: self.module.clone(),
            variant: KernelVariant { merged: self.merged, whole_tables: self.whole_tables, ..KernelVariant::new(self.hwrt, self.switches, 0) },
            layout: self.pipeline_layout.clone(),
            tl_layout: self.tl_pipeline_layout.clone(),
            modules: self.modules.clone(),
            pipelines: self.pipelines.clone(),
            passes: self.kernel_passes.clone(),
            tl: self.tl_pipelines.clone(),
        }
    }

    /// Whether what a live frame asks of kernel `k` is made: its `main`, the passes named, and the translucency
    /// pass's pipelines if `translucent`.
    pub fn kernel_ready(&self, k: &Kernel, passes: &[&'static str], translucent: bool) -> bool {
        self.pipelines.lock().unwrap().contains_key(&(k.features, k.hash, true, k.approx_of(true)))
            && passes.iter().all(|e| self.kernel_passes.lock().unwrap().contains_key(&(k.features, k.hash, *e, k.approx)))
            && (!translucent || self.tl_pipelines.lock().unwrap().contains_key(&(k.features, k.hash, true, k.approx_of(true))))
    }

    /// Makes what `kernel_ready` asks of `k` on another thread, each pipeline on one of its own (a driver compiles a
    /// kernel variant for half a second to tens of seconds): the frame goes on with the kernel it has and takes this
    /// one when it is ready. Nothing if it is ready, or being made.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn warm_kernel(&self, k: &Kernel, passes: &[&'static str], translucent: bool) {
        if self.kernel_ready(k, passes, translucent) || !self.warming.lock().unwrap().insert((k.features, k.hash, k.approx)) {
            return;
        }
        let (maker, k, passes, warming) = (self.maker(), k.clone(), passes.to_vec(), self.warming.clone());
        let work = move || {
            // (The module first: every pipeline is made of it.)
            let _ = maker.module(&k);
            std::thread::scope(|s| {
                s.spawn(|| maker.pipeline(&k, true, None));
                for entry in &passes {
                    s.spawn(|| maker.pass(&k, entry, None));
                }
                if translucent {
                    s.spawn(|| maker.translucent(&k, true, None));
                }
            });
            warming.lock().unwrap().remove(&(k.features, k.hash, k.approx));
        };
        std::thread::spawn(work);
    }

    /// (A browser has no thread to give, and its device is one thread's: its frame makes its kernel itself,
    /// live/frame.rs `choose_kernel`.)
    #[cfg(target_arch = "wasm32")]
    pub fn warm_kernel(&self, _: &Kernel, _: &[&'static str], _: bool) {}

    /// The translucency pass's bind group (group 2) drawing into `target` (live: the composed image; offline: sums);
    /// `low`: (colour, transmittance) per traced pixel, where a live view draws below full resolution
    /// (`translucent_low`; translucent.wgsl `tlow`).
    pub fn translucent_bind_group(&self, t: &Translucent, target: &wgpu::Buffer, low: &wgpu::Buffer) -> wgpu::BindGroup {
        fn e(b: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding: b, resource: buf.as_entire_binding() }
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("translucent"),
            layout: self.tl_layout.as_ref().unwrap(),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: t.tlas.as_binding() }, e(1, &t.params), e(2, target), e(3, &t.points), e(4, &t.count), e(5, low)],
        })
    }

    /// A view's buffer of the trace's samples (`Uploaded::accum`): a `Pixel` a pixel, the largest buffer a view holds
    /// (`view_fits`).
    pub fn accum_buffer(&self, w: u32, h: u32) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("accum"), size: w as u64 * h as u64 * SAMPLE_BYTES, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    /// Whether this device holds a view of `w` x `h` (its samples in one buffer).
    pub fn view_fits(&self, w: u32, h: u32) -> bool {
        self.buffer_fits(w as u64 * h as u64 * SAMPLE_BYTES)
    }

    /// A view's buffer for translucent particles drawn below full resolution (`translucent_bind_group`'s `low`).
    pub fn translucent_low(&self, w: u32, h: u32) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("translucent low"), size: (w as u64 * h as u64).max(1) * 16, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false })
    }

    /// Another buffer for the trace's samples (a view of another size): the scene bound again with it.
    pub fn set_accum(&self, up: &mut Uploaded, accum: &wgpu::Buffer) {
        up.accum = accum.clone();
        self.rebind_scene(up);
    }

    /// The radiance cache's per-frame resolve (pathtrace.wgsl `cache_resolve`; kernel bind groups).
    pub fn cache_resolve_pipeline(&self) -> wgpu::ComputePipeline {
        let module = &self.module;
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
    fn texture_pages(&self, scene: &Scene, timings: &mut Vec<(String, f64)>) -> (wgpu::TextureView, std::sync::Arc<Vec<[f32; 4]>>) {
        let key = crate::texpack::set_key(&scene.textures);
        let mut cache = self.pages.lock().unwrap();
        if let Some(p) = cache.as_ref() {
            if p.key == key {
                return (p.view.clone(), p.info.clone());
            }
        }
        let t = Instant::now();
        let packed = crate::texpack::pack(&scene.textures);
        let tex = self.pages_texture(packed.side, packed.layers.len() as u32);
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
        let info = std::sync::Arc::new(packed.info);
        // Streams show their last picture, not the one the scene was described with.
        let last = self.streams.lock().unwrap();
        for (name, place) in &packed.streams {
            if let Some(img) = last.get(name) {
                self.write_place(&tex, place, img);
            }
        }
        timings.push(("texture_pages_ms".into(), t.elapsed().as_secs_f64() * 1000.0));
        let keys = scene.textures.iter().map(|t| (t.key.clone(), t.width, t.height, t.wrap)).collect();
        *cache = Some(Pages { key, texture: tex, view: view.clone(), info: info.clone(), streams: packed.streams, atlas: packed.atlas, keys });
        (view, info)
    }

    /// The pages' texture: `layers` square layers of `side`, `texpack::MIPS` levels. (A source of copies too: the
    /// pages grow by a copy on the GPU, `place_texture`.)
    fn pages_texture(&self, side: u32, layers: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture pages"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: layers },
            mip_level_count: crate::texpack::MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// The slot of texture `t` in the uploaded scene's pages; a texture they have not got is placed in them now
    /// (`texpack::Atlas::place`) and its pixels written there, every level. Nothing is packed again: where the pages
    /// have no room they become a texture with a layer more, or a longer side, and what they hold is copied there
    /// on the GPU. A slot and a rect that a freed texture left (`free_textures`) are taken first.
    pub fn place_texture(&self, t: &crate::scene::Texture) -> Result<PagesEdit> {
        use crate::texpack::{set_key_of, stream_name, MIPS};
        let mut cache = self.pages.lock().unwrap();
        let pages = cache.as_mut().ok_or_else(|| anyhow!("no scene's pages to place a texture in"))?;
        if let Some(slot) = pages.keys.iter().position(|k| !k.0.is_empty() && k.0 == t.key) {
            return Ok(PagesEdit { slot: slot as i32, info: None, view: None });
        }
        let (side, layers) = (pages.atlas.side, pages.atlas.layers());
        // (On a copy: one that finds no room leaves the pages' record as it was.)
        let mut atlas = pages.atlas.clone();
        let (slot, place) = atlas.place(t.width, t.height, t.wrap, self.device.limits().max_texture_array_layers).ok_or_else(|| anyhow!("no room in the texture pages"))?;
        pages.atlas = atlas;
        let mut view = None;
        if (pages.atlas.side, pages.atlas.layers()) != (side, layers) {
            let tex = self.pages_texture(pages.atlas.side, pages.atlas.layers());
            let mut e = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("texture pages grow") });
            for level in 0..MIPS {
                let at = |texture| wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All };
                let s = (side >> level).max(1);
                e.copy_texture_to_texture(at(&pages.texture), at(&tex), wgpu::Extent3d { width: s, height: s, depth_or_array_layers: layers });
            }
            self.queue.submit([e.finish()]);
            pages.view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
            pages.texture = tex;
            view = Some(pages.view.clone());
        }
        // (A stream shows its last picture, as in pages packed whole.)
        let last = stream_name(&t.key).and_then(|n| self.streams.lock().unwrap().get(n).cloned());
        self.write_place(&pages.texture, &place, last.as_deref().unwrap_or(&t.rgba));
        if let Some(n) = stream_name(&t.key) {
            pages.streams.push((n.to_string(), place));
        }
        let k = (t.key.clone(), t.width, t.height, t.wrap);
        if slot < pages.keys.len() {
            pages.keys[slot] = k;
        } else {
            pages.keys.push(k);
        }
        pages.key = set_key_of(pages.keys.iter().map(|k| (k.0.as_str(), k.1, k.2, k.3)));
        pages.info = std::sync::Arc::new(pages.atlas.info());
        Ok(PagesEdit { slot: slot as i32, info: Some(pages.info.clone()), view })
    }

    /// Frees the textures in `slots` (no material names them any more): their slots and rects are the next placed
    /// textures' (`texpack::Atlas::free`), their rows zeros. The texture table's rows as they are now, if any was
    /// freed. A stream's texture is never freed (its name keeps it).
    pub fn free_textures(&self, slots: &[i32]) -> Option<std::sync::Arc<Vec<[f32; 4]>>> {
        let mut cache = self.pages.lock().unwrap();
        let pages = cache.as_mut()?;
        let mut freed = false;
        for &slot in slots {
            let s = slot as usize;
            if slot < 0 || !pages.atlas.holds(s) || crate::texpack::stream_name(&pages.keys[s].0).is_some() {
                continue;
            }
            pages.atlas.free(s);
            pages.keys[s] = (String::new(), 0, 0, false);
            freed = true;
        }
        if !freed {
            return None;
        }
        pages.key = crate::texpack::set_key_of(pages.keys.iter().map(|k| (k.0.as_str(), k.1, k.2, k.3)));
        pages.info = std::sync::Arc::new(pages.atlas.info());
        Some(pages.info.clone())
    }

    /// The slots that hold a texture now.
    pub fn texture_slots(&self) -> Vec<i32> {
        self.pages.lock().unwrap().as_ref().map_or_else(Vec::new, |p| (0..p.atlas.places.len()).filter(|&s| p.atlas.holds(s)).map(|s| s as i32).collect())
    }

    /// The slot of the texture of `key` (`scene::Texture::key_of`) in the uploaded scene's pages, if they have it.
    pub fn texture_slot(&self, key: &str) -> Option<i32> {
        self.pages.lock().unwrap().as_ref()?.keys.iter().position(|k| !k.0.is_empty() && k.0 == key).map(|s| s as i32)
    }

    /// New pixels for the texture in `slot`, written over the old ones in the pages: where their size in the pages
    /// and their wrap are the same. False, and nothing written, where they are not (the texture is then placed as
    /// another: `place_texture`).
    pub fn rewrite_texture(&self, slot: i32, t: &crate::scene::Texture) -> bool {
        let mut cache = self.pages.lock().unwrap();
        let Some(pages) = cache.as_mut() else { return false };
        let Some(place) = pages.atlas.places.get(slot as usize).copied().filter(|_| pages.atlas.holds(slot as usize)) else { return false };
        if crate::texpack::content_size(t.width, t.height) != place.size || t.wrap != place.wrap || crate::texpack::stream_name(&t.key).is_some() {
            return false;
        }
        self.write_place(&pages.texture, &place, &t.rgba);
        pages.keys[slot as usize] = (t.key.clone(), t.width, t.height, t.wrap);
        pages.key = crate::texpack::set_key_of(pages.keys.iter().map(|k| (k.0.as_str(), k.1, k.2, k.3)));
        true
    }

    /// The pages' textures as they are now, by slot: (key, width, height, wrap), their side and their layers.
    #[cfg(test)]
    pub fn pages_state(&self) -> (Vec<(String, u32, u32, bool)>, u32, u32, Vec<crate::texpack::Place>) {
        let cache = self.pages.lock().unwrap();
        let p = cache.as_ref().expect("pages");
        (p.keys.clone(), p.atlas.side, p.atlas.layers(), p.atlas.places.clone())
    }

    fn write_place(&self, pages: &wgpu::Texture, place: &crate::texpack::Place, img: &image::RgbaImage) {
        for l in crate::texpack::patch(place, img) {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: pages, mip_level: l.level, origin: wgpu::Origin3d { x: l.x, y: l.y, z: place.layer }, aspect: wgpu::TextureAspect::All },
                &l.rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(l.width * 4), rows_per_image: Some(l.height) },
                wgpu::Extent3d { width: l.width, height: l.height, depth_or_array_layers: 1 },
            );
        }
    }

    /// New pixels for the stream texture `name` (world.rs `TextureSource::Stream`): written into its place in the
    /// texture pages, every mip level, before the next frame's passes. Nothing of the scene is rebuilt. True when the
    /// uploaded scene has the stream; the picture is kept either way, for the pages a later scene packs.
    pub fn write_stream(&self, name: &str, width: u32, height: u32, rgba: &[u8]) -> Result<bool> {
        let img = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or_else(|| anyhow::anyhow!("stream {name}: {} bytes are not {width}x{height} RGBA8 pixels", rgba.len()))?;
        let img = std::sync::Arc::new(img);
        self.streams.lock().unwrap().insert(name.to_string(), img.clone());
        let cache = self.pages.lock().unwrap();
        let Some(pages) = cache.as_ref() else { return Ok(false) };
        let mut found = false;
        for (n, place) in &pages.streams {
            if n == name {
                self.write_place(&pages.texture, place, &img);
                found = true;
            }
        }
        Ok(found)
    }

    /// Uploads a prepared scene (geometry, materials, textures, environment, acceleration structures) and returns the
    /// bind group used by both the offline and the live kernels. `accum` holds one 80-byte Pixel per output pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn upload(&self, scene: &Scene, accum: &wgpu::Buffer, nodes: &[u8], emitters: &[[f32; 4]], emitter_tris: &[crate::GpuEmitterTri], emitter_runs: &crate::EmitterRuns, light_tree: &[[f32; 4]], env: &EnvMap, env_cdf: &[f32], energy: &[[f32; 4]], timings: &mut Vec<(String, f64)>) -> Result<Uploaded> {
        let d = &self.device;
        let t0 = Instant::now();
        // (A buffer is never empty: zeros where the scene has nothing for it.)
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
        let room = EditRoom::for_scene(scene, scene.indexed.indices.len(), records.len(), self.hwrt, scene.units.is_some());
        let st = wgpu::BufferUsages::STORAGE;
        // (COPY_SRC: the live view's in-place edits grow these past their room by a copy, live/edits.rs `edit_geometry`.)
        let edit = st | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        // The vertices, with room for three a triangle of the edit room; and those that move, as they are (their
        // previous frame's).
        let ix = &scene.indexed;
        anyhow::ensure!(scene.meshes.iter().all(|m| m.sect_count as usize <= crate::scene::MESH_SECTIONS), "a mesh with more than 65,536 sections");
        let b_vertices = buf_room("vertices", bytemuck::cast_slice(&ix.vertices), room.tris * 3 * crate::indexed::VERTEX_BYTES, edit | if self.hwrt { wgpu::BufferUsages::BLAS_INPUT } else { wgpu::BufferUsages::empty() });
        let b_prev = buf("previous vertices", bytemuck::cast_slice(&ix.vertices[ix.dynamic.0 as usize..(ix.dynamic.0 + ix.dynamic.1) as usize]), edit);
        // (A particle pass's own last positions of the vertices that move: 16 bytes each.)
        let last_bytes = ix.dynamic.1.max(1) as u64 * 16;
        // The vertex colours' bytes on the GPU (the edit room's included).
        timings.push(("vcolor_mb".into(), if scene.gpu_vcolors.is_empty() { 0.0 } else { (scene.gpu_vcolors.len() * 4 + room.tris * 12) as f64 / (1 << 20) as f64 }));
        // (With units, edits write trees into it: units.rs.)
        // (COPY_SRC: read back by the test that holds a refit tree to its triangles.)
        let b_nodes = buf("nodes", nodes, if scene.units.is_some() { st | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC } else { st });
        // (Software traversal: the trees' leaves, with the edit room's.)
        let b_leaves = buf_room("leaves", bytemuck::cast_slice(&ix.leaves), if self.hwrt { 0 } else { room.tris * crate::indexed::LEAF_BYTES }, edit);
        // What the triangles hold on the GPU besides their BLASes: vertices (48 B each), indices (12 B a triangle), the
        // rows of those that have one (16 B), and the software BVH: its nodes and its leaves' triangles (48 B each)
        // (ROADMAP § 6 step 2's table: a meshed representation against voxel data).
        timings.push(("tri_data_mb".into(), (ix.vertices.len() * crate::indexed::VERTEX_BYTES + ix.indices.len() * 12 + ix.leaves.len() * crate::indexed::LEAF_BYTES + nodes.len()) as f64 / (1 << 20) as f64));
        let (words, region_words, proc_room) = self.upload_words(scene, env_cdf, room.proc_words, room.tris, room.records, timings)?;
        let room = EditRoom { proc_words: proc_room, ..room };
        let grids = self.make_grids(&scene.grids, &scene.media, &scene.grid_programs);
        let grid_view = grids.as_ref().map_or_else(|| self.no_grid_view.clone(), |g| g.atlas.create_view(&Default::default()));
        let b_accum = accum.clone();
        let b_params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("params"), size: std::mem::size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        let (tex_view, tex_info) = self.texture_pages(scene, timings);
        // The scene's tables, the sections of one buffer (heap.rs), written as it is made. The objects' and the
        // records' have the edits' room after them (the procedural objects' records follow the model instances').
        let none = [crate::scene::GpuInstance::default()];
        let spheres = crate::light_groups(&scene.lights);
        let parts: [(Table, &[u8], usize); Table::COUNT] = [
            (Table::LightTree, bytemuck::cast_slice(light_tree), 0),
            (Table::Materials, bytemuck::cast_slice(&scene.materials), 0),
            (Table::Lights, bytemuck::cast_slice(&scene.lights), 0),
            (Table::LightGroups, bytemuck::cast_slice(&spheres), 0),
            (Table::Emitters, bytemuck::cast_slice(emitters), 0),
            (Table::TexInfo, bytemuck::cast_slice(&tex_info[..]), 0),
            (Table::Objects, bytemuck::cast_slice(&scene.objects), room.objects * scene.object_stride() * 16),
            (Table::Instances, bytemuck::cast_slice(if records.is_empty() { &none[..] } else { &records[..] }), room.records * std::mem::size_of::<crate::scene::GpuInstance>()),
            (Table::Media, bytemuck::cast_slice(&scene.media), 0),
            (Table::Energy, bytemuck::cast_slice(energy), 0),
            (Table::GlassEnergy, bytemuck::cast_slice(crate::glass_energy_lut()), 0),
            (Table::EmitterTris, bytemuck::cast_slice(emitter_tris), 0),
        ];
        assert!(parts.iter().enumerate().all(|(k, p)| p.0.index() == k), "the tables in `Table`'s order");
        let heap = Heap::filled(d, &parts.map(|p| (p.1.len() + p.2) as u64), |put| parts.iter().for_each(|p| put(p.0, 0, p.1)));

        let env_tex = self.env_texture(env);
        let env_view = env_tex.create_view(&Default::default());
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        // Hardware acceleration structures: a BLAS a mesh (model space, a geometry a section), a TLAS instance a
        // placement (its custom data its record; shadow casters mask 1, the rest mask 2), then the procedural objects'.
        // Every geometry is flagged opaque, a cutout's too: every query forces opacity (trace_hw.wgsl).
        let mut accel = None;
        if self.hwrt {
            let tb = Instant::now();
            // (The indices' first word in the words' heap.)
            let index_first = (words.at(Stream::Indices) / 4) as u32;
            // (Room for one at least: a scene of the sky alone.)
            let mut t = d.create_tlas(&wgpu::CreateTlasDescriptor { label: None, max_instances: (scene.instances.len() + scene.procs.len() + room.records).max(1) as u32, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });
            // Per mesh: its geometries, one a section (`InstMesh::parts`): first triangle, count.
            let mesh_parts: Vec<Vec<(u32, u32)>> = scene.meshes.iter().map(|m| m.parts(&scene.sections).into_iter().map(|(at, count)| (m.first + at, count)).collect()).collect();
            // (A translucent particle system's is not flagged opaque: the translucency pass asks for every hit.)
            let layered: std::collections::HashSet<usize> = scene.particles.iter().filter(|p| p.translucent).map(|p| p.mesh as usize).collect();
            let mesh_sizes: Vec<Vec<wgpu::BlasTriangleGeometrySizeDescriptor>> = mesh_parts
                .iter()
                .zip(&ix.meshes)
                .enumerate()
                .map(|(mesh, (parts, &(_, vertices)))| {
                    parts
                        .iter()
                        .map(|&(_, count)| wgpu::BlasTriangleGeometrySizeDescriptor {
                            vertex_format: wgpu::VertexFormat::Float32x3,
                            vertex_count: vertices.max(1),
                            index_format: Some(wgpu::IndexFormat::Uint32),
                            index_count: Some(count * 3),
                            flags: if layered.contains(&mesh) { wgpu::AccelerationStructureGeometryFlags::empty() } else { wgpu::AccelerationStructureGeometryFlags::OPAQUE },
                        })
                        .collect()
                })
                .collect();
            // Deforming meshes (posed skinned models') are rebuilt every new pose: built fast rather than traced fast.
            let meshes: Vec<wgpu::Blas> = mesh_sizes
                .iter()
                .zip(&scene.meshes)
                .map(|(s, m)| {
                    let flags = if m.deforms { wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD } else { wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE };
                    d.create_blas(&wgpu::CreateBlasDescriptor { label: None, flags, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: s.clone() })
                })
                .collect();
            // What the triangle BLASes hold (Metal's own sizes; each instanced mesh once). A geometry: its first vertex
            // and how many, its first triangle and how many.
            let mut blas_per_tri = 0.0;
            let parts: Vec<Vec<(u32, u32, u32, u32)>> = mesh_parts.iter().zip(&ix.meshes).map(|(ps, &(vfirst, vcount))| ps.iter().map(|&(first, count)| (vfirst, vcount.max(1), first, count)).collect()).collect();
            if let Some(bytes) = parts.iter().map(|ps| self.tri_blas_bytes(&b_vertices, &words.buffer, index_first, ps)).sum::<Option<u64>>() {
                timings.push(("tri_blas_mb".into(), bytes as f64 / (1 << 20) as f64));
                let tris: u64 = parts.iter().flatten().map(|p| p.3 as u64).sum();
                blas_per_tri = bytes as f64 / tris.max(1) as f64;
            }
            anyhow::ensure!(scene.instances.len() + scene.procs.len() <= 1 << 24, "too many instances for 24-bit custom data");
            for (k, inst) in scene.instances.iter().enumerate() {
                let w = inst.world;
                let row = |r: usize| [w.x_axis[r], w.y_axis[r], w.z_axis[r], w.w_axis[r]];
                let [r0, r1, r2] = [row(0), row(1), row(2)];
                let xf = [r0[0], r0[1], r0[2], r0[3], r1[0], r1[1], r1[2], r1[3], r2[0], r2[1], r2[2], r2[3]];
                t[k] = Some(wgpu::TlasInstance::new(&meshes[inst.mesh as usize], xf, k as u32, instance_mask(inst.cast_shadow, inst.hidden)));
            }
            // Procedural geometry: one AABB BLAS per distinct geometry (object space; kept across uploads by the
            // geometry's hash, so a rebuild builds only new ones), and per mesh a TLAS instance of it after the model
            // instances, whose custom data is the mesh's record; its hits are told apart by their kind.
            let proc_base = scene.instances.len();
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
                let k = scene.proc_layout.as_ref().map_or(j, |l| l.blob_of[j]);
                t[proc_base + j] = Some(wgpu::TlasInstance::new(&geo_blas[k], tlas_rows(p.world), record, instance_mask(p.cast_shadow, p.hidden)));
            }
            let entries: Vec<wgpu::BlasBuildEntry> = meshes
                .iter()
                .zip(&mesh_sizes)
                .zip(&mesh_parts)
                .zip(&ix.meshes)
                .map(|(((blas, sizes), parts), &(first_vertex, _))| wgpu::BlasBuildEntry {
                    blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(
                        sizes
                            .iter()
                            .zip(parts)
                            .map(|(size, &(first, _))| wgpu::BlasTriangleGeometry {
                                size,
                                vertex_buffer: &b_vertices,
                                first_vertex,
                                vertex_stride: VERTEX_STRIDE,
                                index_buffer: Some(&words.buffer),
                                first_index: Some(index_first + first * 3),
                                transform_buffer: None,
                                transform_buffer_offset: None,
                            })
                            .collect(),
                    ),
                })
                .chain(proc_new.iter().map(|(k, boxes, size)| wgpu::BlasBuildEntry {
                    blas: &geo_blas[*k],
                    geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry { size, stride: 24, aabb_buffer: boxes, primitive_offset: 0 }]),
                }))
                .collect();
            // In batches of at most BLAS_BATCH_TRIS triangles (a BLAS larger alone), each submitted and waited for, the
            // TLAS with the last: wgpu-core gives one build a single scratch buffer for all its BLASes, the sum of theirs,
            // and on a GPU of 8 GB a city's (a few thousand meshes) did not fit beside the scene.
            let tris_of = |e: &wgpu::BlasBuildEntry<'_>| -> u64 {
                match &e.geometry {
                    wgpu::BlasGeometries::TriangleGeometries(g) => g.iter().map(|x| x.size.index_count.unwrap_or(x.size.vertex_count) as u64 / 3).sum(),
                    wgpu::BlasGeometries::AabbGeometries(g) => g.iter().map(|x| x.size.primitive_count as u64).sum(),
                }
            };
            let mut start = 0;
            loop {
                let mut end = start;
                let mut tris = 0;
                while end < entries.len() && (end == start || tris + tris_of(&entries[end]) <= BLAS_BATCH_TRIS) {
                    tris += tris_of(&entries[end]);
                    end += 1;
                }
                let mut enc = d.create_command_encoder(&Default::default());
                if end == entries.len() {
                    enc.build_acceleration_structures(entries[start..end].iter(), [&t]);
                } else {
                    enc.build_acceleration_structures(entries[start..end].iter(), std::iter::empty::<&wgpu::Tlas>());
                }
                self.queue.submit([enc.finish()]);
                self.device.poll(wgpu::PollType::wait_indefinitely())?;
                if end == entries.len() {
                    break;
                }
                start = end;
            }
            drop(entries);
            let mesh_geo = mesh_parts.into_iter().zip(mesh_sizes).collect();
            accel = Some(Accel { tlas: t, vertices: b_vertices.clone(), indices: words.buffer.clone(), index_first, mesh_verts: ix.meshes.clone(), meshes, mesh_geo, blas_per_tri });
            timings.push(("accel_build_ms".into(), tb.elapsed().as_secs_f64() * 1000.0));
        }
        fn e(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
        }
        let bg_parts = BindParts { nodes: b_nodes, leaves: b_leaves, tex_view, env_view, sampler, grid_view };
        let bg = self.scene_bind_group(&SceneBuffers { params: &b_params, vertices: &b_vertices, heap: &heap, words: &words, prev_vertices: &b_prev, accum: &b_accum, parts: &bg_parts, accel: accel.as_ref() });
        let kernel = Kernel { approx: self.approx, ..Kernel::new(scene) };
        // Translucent particles (hardware RT): the translucency pass's own TLAS, built again after the particle pass:
        // an instance of it a system's mesh, the instance's record in its custom data as in the scene's TLAS.
        let layered: Vec<(u32, u32)> = scene.particles.iter().filter(|p| p.translucent).filter_map(|p| scene.instances.iter().position(|i| i.mesh == p.mesh).map(|r| (p.mesh, r as u32))).collect();
        let translucent = match &accel {
            Some(a) if !layered.is_empty() => {
                let mut tlas = d.create_tlas(&wgpu::CreateTlasDescriptor { label: Some("translucent"), max_instances: layered.len() as u32, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });
                for (k, &(mesh, record)) in layered.iter().enumerate() {
                    tlas[k] = Some(wgpu::TlasInstance::new(&a.meshes[mesh as usize], [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], record, 0xff));
                }
                let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("translucent params"), size: 48, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let points = d.create_buffer(&wgpu::BufferDescriptor { label: Some("volume points"), size: VOLUME_TRAIN_CAP as u64 * 16, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let count = d.create_buffer(&wgpu::BufferDescriptor { label: Some("volume point count"), size: 16, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let t = Translucent { tlas, meshes: layered.iter().map(|l| l.0).collect(), params, points, count };
                // Built once now (collapsed quads): bind groups may use the TLAS before the first particle pass. (The
                // systems' meshes with their TLAS, in one build.)
                let mut enc = d.create_command_encoder(&Default::default());
                enc.build_acceleration_structures(mesh_entries(a, &t.meshes).iter(), [&t.tlas]);
                self.queue.submit([enc.finish()]);
                Some(t)
            }
            _ => None,
        };
        // (The particle passes' pipeline constant: vec4s an object entry, 4 with instance colours.)
        let object_stride = [("OBJECT_STRIDE", scene.object_stride() as f64)];
        // Particle systems (hardware RT: the pass writes their BLASes' input).
        let particles = match &accel {
            Some(a) if !scene.particles.is_empty() => {
                let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("particles"), source: wgpu::ShaderSource::Wgsl(particle_source(scene).into()) });
                let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("particles"), layout: None, module: &module, entry_point: Some("particles"), compilation_options: wgpu::PipelineCompilationOptions { constants: &object_stride, ..Default::default() }, cache: self.cache() });
                let sm = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("stamp"), source: wgpu::ShaderSource::Wgsl(STAMP_WGSL.into()) });
                let sp = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("stamp"), layout: None, module: &sm, entry_point: Some("stamp"), compilation_options: Default::default(), cache: self.cache() });
                // The stamps depend on what is rebuilt last: the translucent TLAS when there is one.
                let stl = translucent.as_ref().map_or(&a.tlas, |t| &t.tlas);
                let sbg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("stamp"), layout: &sp.get_bind_group_layout(0), entries: &[e(0, &b_vertices), wgpu::BindGroupEntry { binding: 1, resource: stl.as_binding() }] });
                let stamp = (sp, sbg);
                let systems: Vec<ParticleSystemGpu> = scene
                    .particles
                    .iter()
                    .map(|ps| {
                        let o = &scene.objects[ps.object as usize * scene.object_stride()];
                        let mut p = [0.0f32; 32];
                        p.copy_from_slice(&ps.params);
                        // (A system hidden at build runs nothing: its quads stay collapsed at the node.)
                        // (Its mesh's first vertex: four a particle.)
                        ParticleSystemGpu { a: [ix.meshes[ps.mesh as usize].0, if ps.hidden { 0 } else { ps.count }, ps.object, ps.func], b: [ps.seed, ix.meshes[ps.mesh as usize].0 - ix.dynamic.0, 0, 0], c: [o[0], o[1], o[2], 0.0], p, d: ps.custom }
                    })
                    .collect();
                // COPY_DST: pivot updates (`set_particle_pivot`).
                let b_sys = buf("particle systems", bytemuck::cast_slice(&systems), edit);
                let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("particle params"), size: 48, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let last = d.create_buffer(&wgpu::BufferDescriptor { label: Some("particles last"), size: last_bytes, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let written = ParticleTargets { vertices: &b_vertices, prev_vertices: &b_prev, heap: &heap };
                let pbg = particle_bind_group(d, &pipeline, "particles", &[(0, &params), (1, &b_sys), (7, &last)], &written);
                let nodes = scene.particles.iter().map(|p| p.seed).collect();
                let meshes = scene.particles.iter().map(|p| p.mesh).collect();
                let counts = scene.particles.iter().map(|p| p.count).collect();
                let translucent_sys = scene.particles.iter().map(|p| p.translucent).collect();
                let hidden = std::cell::RefCell::new(scene.particles.iter().map(|p| if p.hidden { Hidden::Idle } else { Hidden::Shown }).collect());
                Some(ParticleGpu { pipeline, stamp, bg: pbg, params, table: b_sys, last, nodes, systems: systems.len() as u32, max_count: scene.particles.iter().map(|p| p.count).max().unwrap_or(0), fresh: std::cell::Cell::new(true), meshes, counts, translucent: translucent_sys, hidden })
            }
            _ => None,
        };
        // Stateful GPU particles (hardware RT, as `Particles`): their states (zero: none alive), the game's spawns, the
        // voxel occupancy grid they collide with (a word without one).
        let gpu_particles = match &accel {
            Some(a) if !scene.gpu_particles.is_empty() => {
                let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gpu particles"), source: wgpu::ShaderSource::Wgsl(gpu_particle_source(scene).into()) });
                let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("gpu particles"), layout: None, module: &module, entry_point: Some("gpu_particles"), compilation_options: wgpu::PipelineCompilationOptions { constants: &object_stride, ..Default::default() }, cache: self.cache() });
                let systems: Vec<GpuParticleSystemGpu> = scene
                    .gpu_particles
                    .iter()
                    .map(|ps| {
                        let c = &ps.collide;
                        let flags = u32::from(c.plane.is_some()) | if c.voxels { 2 } else { 0 };
                        GpuParticleSystemGpu { a: [ix.meshes[ps.mesh as usize].0, ps.capacity, ps.object, ps.program], b: [ps.seed, ix.meshes[ps.mesh as usize].0 - ix.dynamic.0, 0, 0], c: [ps.pivot[0], ps.pivot[1], ps.pivot[2], flags as f32], d: [c.plane.unwrap_or(0.0), c.restitution, c.friction, 0.0], e: [0; 4], p: ps.params }
                    })
                    .collect();
                let table = buf("gpu particle systems", bytemuck::cast_slice(&systems), edit);
                let states = scene.gpu_particles.iter().map(|ps| (ps.object + ps.capacity) as u64).max().unwrap_or(1) * 64;
                let state = d.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particle states"), size: states, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let spawns = d.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particle spawns"), size: (MAX_SPAWNS * 32) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particle params"), size: 80, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let last = d.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particles last"), size: last_bytes, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
                let occ = scene.voxel_occupancy.as_ref().filter(|_| scene.gpu_particles.iter().any(|p| p.collide.voxels)).map(|o| o.read().unwrap().clone());
                let (grid, grid_o, grid_n) = match &occ {
                    Some(o) => (buf("voxel occupancy", bytemuck::cast_slice(&o.words), edit), [o.origin[0], o.origin[1], o.origin[2], o.cell], [o.dims[0], o.dims[1], o.dims[2], o.row_words() as u32]),
                    None => (buf("voxel occupancy", &[0u8; 16], edit), [0.0; 4], [0; 4]),
                };
                if let Some(o) = &scene.voxel_occupancy {
                    o.write().unwrap().dirty.clear();
                }
                let written = ParticleTargets { vertices: &b_vertices, prev_vertices: &b_prev, heap: &heap };
                let bg = particle_bind_group(d, &pipeline, "gpu particles", &[(0, &params), (1, &table), (7, &last), (8, &state), (9, &spawns), (10, &grid)], &written);
                let host = scene.gpu_particles.iter().map(|ps| (0u32, 0.0f32, ps.rate)).collect();
                let sm = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("stamp"), source: wgpu::ShaderSource::Wgsl(STAMP_WGSL.into()) });
                let sp = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("stamp"), layout: None, module: &sm, entry_point: Some("stamp"), compilation_options: Default::default(), cache: self.cache() });
                let sbg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("stamp"), layout: &sp.get_bind_group_layout(0), entries: &[e(0, &b_vertices), wgpu::BindGroupEntry { binding: 1, resource: a.tlas.as_binding() }] });
                Some(GpuParticlesGpu { pipeline, stamp: (sp, sbg), bg, params, table, last, state, spawns, grid, systems: std::cell::RefCell::new(systems), host: std::cell::RefCell::new(host), nodes: scene.gpu_particles.iter().map(|p| p.node).collect(), max_cap: scene.gpu_particles.iter().map(|p| p.capacity).max().unwrap_or(0), frame: std::cell::Cell::new(0), fresh: std::cell::Cell::new(true), grid_o, grid_n, meshes: scene.gpu_particles.iter().map(|p| p.mesh).collect() })
            }
            _ => None,
        };
        // The skinning pass, a job each mesh that deforms: under hardware traversal (it writes the BLASes' input too),
        // and under software traversal where such a mesh is a unit.
        let skin = (!scene.retained.skinned.is_empty() && (accel.is_some() || scene.units.is_some())).then(|| self.skin_pass(scene, &b_vertices, scene.units.is_some().then_some((&words.buffer, &bg_parts.leaves))));
        self.queue.submit([]);
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        timings.push(("upload_ms".into(), t0.elapsed().as_secs_f64() * 1000.0));
        // (The scene's own pipelines, made above: particles, grid programs.)
        self.save_pipeline_cache();
        // (What moves emitters after the load needs hardware ray tracing: triangles reposed in place, the particle passes.)
        let fills = match &accel {
            Some(_) => EmitterFills { particles: self.emitter_fill(&emitter_runs.particles, &b_vertices, &heap, &words) },
            None => EmitterFills::default(),
        };
        Ok(Uploaded { bg, bg_parts, heap, accum: b_accum, params: b_params, kernel, vertices: b_vertices, prev_vertices: b_prev, accel, units: scene.units.clone(), particles, gpu_particles, skin, fills, grids, translucent, vcolors: !scene.gpu_vcolors.is_empty(), media_nodes: scene.media_nodes.clone(), media_hidden: std::cell::RefCell::new(scene.media.iter().map(|m| m.a[1] == crate::scene::MEDIUM_HIDDEN).collect()), room, words, cdf_words: env_cdf.len(), region_words, env_tex })
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

    /// Bytes Metal allocates for a triangle BLAS over geometries (first vertex, vertices, first triangle, triangles) of
    /// the vertices and the indices from word `index_first` of `words` (flagged opaque, as every triangle
    /// BLAS here; None on other backends).
    fn tri_blas_bytes(&self, vertices: &wgpu::Buffer, words: &wgpu::Buffer, index_first: u32, parts: &[(u32, u32, u32, u32)]) -> Option<u64> {
        #[cfg(target_vendor = "apple")]
        {
            use wgpu::hal::{api::Metal, Device as _};
            let d = unsafe { self.device.as_hal::<Metal>() }?;
            let (b, i) = unsafe { (vertices.as_hal::<Metal>()?, words.as_hal::<Metal>()?) };
            let entries = wgpu::hal::AccelerationStructureEntries::Triangles(
                parts
                    .iter()
                    .map(|&(first_vertex, vertex_count, first, count)| wgpu::hal::AccelerationStructureTriangles {
                        vertex_buffer: Some(&*b),
                        vertex_format: wgpu::VertexFormat::Float32x3,
                        first_vertex,
                        vertex_count,
                        vertex_stride: VERTEX_STRIDE,
                        indices: Some(wgpu::hal::AccelerationStructureTriangleIndices { format: wgpu::IndexFormat::Uint32, buffer: Some(&*i), offset: (index_first + first * 3) * 4, count: count * 3 }),
                        transform: None,
                        flags: wgpu::hal::AccelerationStructureGeometryFlags::OPAQUE,
                    })
                    .collect(),
            );
            let s = unsafe { d.get_acceleration_structure_build_sizes(&wgpu::hal::GetAccelerationStructureBuildSizesDescriptor { entries: &entries, flags: wgpu::hal::AccelerationStructureBuildFlags::PREFER_FAST_TRACE }) };
            return Some(s.acceleration_structure_size);
        }
        #[allow(unreachable_code)]
        {
            let _ = (vertices, words, index_first, parts);
            None
        }
    }

    /// The scene's 32-bit arrays (heap.rs `Stream`): the environment's CDF, the procedural region (scene::ProcLayout:
    /// headers and materials, then the geometries' blobs) and the vertex colours, each with its edit room
    /// (`room_words` after the region, as much of it as the binding limit leaves; `room_tris` triangles' colours).
    /// Written straight into the new buffer's mapping; the blobs are copied on the GPU from the last upload when its
    /// geometries are the same. Returns the heap, the region's words in use and the room it got.
    fn upload_words(&self, scene: &Scene, env_cdf: &[f32], room_words: usize, room_tris: usize, room_rows: usize, timings: &mut Vec<(String, f64)>) -> Result<(Heap<Stream>, usize, usize)> {
        let t = Instant::now();
        let l = scene.proc_layout.as_ref();
        let (head, blob_words) = l.map_or((0, 0), |l| (l.head.len(), l.blob_words));
        let colors: &[u8] = bytemuck::cast_slice(&scene.gpu_vcolors);
        let color_bytes = if colors.is_empty() { 0 } else { colors.len() + room_tris * 12 };
        let rows: &[u8] = bytemuck::cast_slice(&scene.rows);
        let row_bytes = if rows.is_empty() && room_rows == 0 { 0 } else { rows.len() + room_rows * 16 };
        // The triangles' indices.
        let ix = &scene.indexed;
        let index_bytes = (ix.indices.len() + room_tris) * 12;
        let sizes = |room: usize| [(env_cdf.len() * 4) as u64, ((head + blob_words + room) * 4) as u64, color_bytes as u64, row_bytes as u64, index_bytes as u64];
        let (used, limit) = (Heap::<Stream>::size_for(&sizes(0)), self.device.limits().max_storage_buffer_binding_size as u64);
        if used > limit {
            return Err(anyhow!("procedural geometry: {} MB of data, more than this device binds in one buffer ({} MB)", used >> 20, limit >> 20));
        }
        // (A section's size is rounded up to where the next may start.)
        let room = room_words.min(((limit - used) / 4).saturating_sub(crate::heap::ALIGN / 4) as usize);
        let sizes = sizes(room);
        let blob_at = (head * 4) as u64;
        let blob_bytes = (blob_words * 4) as u64;
        let key = l.map(|l| l.key());
        // (The BLASes are built from the indices' section.)
        let usage = crate::heap::USAGE | if self.hwrt { wgpu::BufferUsages::BLAS_INPUT } else { wgpu::BufferUsages::empty() };
        let mut cache = self.proc_cache.lock().unwrap();
        let reuse = cache.as_ref().filter(|c| blob_bytes > 0 && Some(&c.key) == key.as_ref() && c.bytes == blob_bytes);
        // Reused blobs: the small parts written through the queue, the blobs copied on the GPU (a mapped buffer of the
        // whole size would be zero-filled on the CPU first: 17 ms for 256 MB). New ones: written into the mapping.
        let heap = match (reuse, l) {
            (Some(c), Some(l)) => {
                let heap = Heap::new_with(&self.device, &sizes, usage);
                heap.write(&self.queue, Stream::EnvCdf, 0, bytemuck::cast_slice(env_cdf));
                heap.write(&self.queue, Stream::Procs, 0, bytemuck::cast_slice(&l.head));
                heap.write(&self.queue, Stream::VertexColors, 0, colors);
                heap.write(&self.queue, Stream::Rows, 0, rows);
                heap.write(&self.queue, Stream::Indices, 0, bytemuck::cast_slice(&ix.indices));
                let mut e = self.device.create_command_encoder(&Default::default());
                e.copy_buffer_to_buffer(&c.buf, c.offset, &heap.buffer, heap.at(Stream::Procs) + blob_at, c.bytes);
                self.queue.submit([e.finish()]);
                heap
            }
            _ => Heap::filled_with(&self.device, &sizes, usage, |put| {
                put(Stream::EnvCdf, 0, bytemuck::cast_slice(env_cdf));
                put(Stream::VertexColors, 0, colors);
                put(Stream::Rows, 0, rows);
                put(Stream::Indices, 0, bytemuck::cast_slice(&ix.indices));
                let Some(l) = l else { return };
                put(Stream::Procs, 0, bytemuck::cast_slice(&l.head));
                for (blob, off) in &l.blobs {
                    let mut at = blob_at + (off * 4) as u64;
                    let mut parts: Vec<&[u8]> = vec![bytemuck::cast_slice(&blob.boxes), bytemuck::cast_slice(&blob.data)];
                    if l.sw {
                        let (nodes, order) = blob.sw_bvh();
                        parts.extend([bytemuck::cast_slice::<_, u8>(nodes), bytemuck::cast_slice(order)]);
                    }
                    for part in parts {
                        put(Stream::Procs, at, part);
                        at += part.len() as u64;
                    }
                }
            }),
        };
        if let Some(key) = key {
            timings.push((if reuse.is_some() { "proc_copy_ms" } else { "proc_upload_ms" }.into(), t.elapsed().as_secs_f64() * 1000.0));
            // The procedural region's GPU bytes (headers, materials, blobs: boxes and data, once per distinct geometry).
            timings.push(("proc_data_mb".into(), ((head + blob_words) * 4) as f64 / (1 << 20) as f64));
            *cache = Some(ProcCache { key, buf: heap.buffer.clone(), offset: heap.at(Stream::Procs) + blob_at, bytes: blob_bytes });
        }
        Ok((heap, head + blob_words, room))
    }

    /// The particle pass (scene time, camera, frame time, rain, wetness), then the systems' BLASes and the TLAS rebuilt.
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
                // (The stateful GPU particles leave what they placed to this pass when there are stateless ones.)
                if up.gpu_particles.is_some() {
                    self.build_particles(up, accel, false, e);
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
        // (The stateful particles leave what they placed to this pass.)
        self.build_particles(up, accel, true, e);
        if let Some((q, i)) = blas_ts {
            stamp(e, wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i + 1), end_of_pass_write_index: None });
        }
    }

    fn emitter_fill_bind_group(&self, runs: &wgpu::Buffer, vertices: &wgpu::Buffer, heap: &Heap<Table>, words: &Heap<Stream>) -> wgpu::BindGroup {
        let entries = [buffer_entry(0, runs), buffer_entry(1, vertices), wgpu::BindGroupEntry { binding: 2, resource: heap.binding(Table::EmitterTris) }, wgpu::BindGroupEntry { binding: 3, resource: heap.binding(Table::Instances) }, wgpu::BindGroupEntry { binding: 4, resource: words.binding(Stream::Indices) }];
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("emitter fill"), layout: &self.emitter_fill_pipeline().get_bind_group_layout(0), entries: &entries })
    }

    fn emitter_fill_pipeline(&self) -> &wgpu::ComputePipeline {
        self.fill.get_or_init(|| {
            let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("emitter fill"), source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/emitter_fill.wgsl").into()) });
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("emitter fill"), layout: None, module: &module, entry_point: Some("fill"), compilation_options: Default::default(), cache: self.cache() })
        })
    }

    /// The fill of `runs` of the light list (none: no fill).
    fn emitter_fill(&self, runs: &[[u32; 2]], vertices: &wgpu::Buffer, heap: &Heap<Table>, words: &Heap<Stream>) -> Option<EmitterFill> {
        let longest = runs.iter().map(|r| r[1]).max()?;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("emitter runs"), size: (runs.len() * 8) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        self.queue.write_buffer(&buffer, 0, bytemuck::cast_slice(runs));
        Some(EmitterFill { bg: self.emitter_fill_bind_group(&buffer, vertices, heap, words), runs: buffer, count: runs.len() as u32, longest })
    }

    /// `runs` of the light list taken again now (the instances an edit just moved: their records are queued).
    pub fn fill_emitter_runs(&self, up: &Uploaded, runs: &[[u32; 2]]) {
        let Some(f) = self.emitter_fill(runs, &up.vertices, &up.heap, &up.words) else { return };
        let mut e = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("emitter fill") });
        self.fill_emitters(&f, &mut e);
        self.queue.submit([e.finish()]);
    }

    /// The runs of `f` taken again from their triangles as the encoder's earlier commands, and the writes queued
    /// before it is submitted, leave them.
    pub fn fill_emitters(&self, f: &EmitterFill, e: &mut wgpu::CommandEncoder) {
        let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("emitter fill"), timestamp_writes: None });
        pass.set_pipeline(self.emitter_fill_pipeline());
        pass.set_bind_group(0, &f.bg, &[]);
        pass.dispatch_workgroups(f.longest.div_ceil(64), f.count, 1);
    }

    /// One frame of the stateful GPU particles (gpu_particles.wgsl): each system's emissions (its rate over `dt`, and the
    /// game's spawns, `spawns` per node, 8 floats each), the update, collision and retirement of the rest, their quads;
    /// with `rebuild`, the systems' BLASes after (else the stateless pass that follows rebuilds them).
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
            self.build_particles(up, accel, false, e);
            if let Some((q, i)) = blas_ts {
                stamp(e, wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i + 1), end_of_pass_write_index: None });
            }
        }
    }

    /// What the particle passes placed, above the triangles, in a frame's encoder: the BLASes of the systems' meshes
    /// and the TLAS (the translucency pass's too). `stateless`: the stateless systems' too (not when every one of
    /// them is idle).
    fn build_particles(&self, up: &Uploaded, accel: &Accel, stateless: bool, e: &mut wgpu::CommandEncoder) {
        let own = up.particles.as_ref().filter(|_| stateless);
        let meshes: Vec<u32> = own.iter().flat_map(|p| &p.meshes).chain(up.gpu_particles.iter().flat_map(|p| &p.meshes)).copied().collect();
        // (The translucent systems' meshes are among the stateless ones': their own TLAS with the scene's.)
        let layered = up.translucent.as_ref().filter(|_| stateless).map(|t| &t.tlas);
        if !meshes.is_empty() {
            e.build_acceleration_structures(mesh_entries(accel, &meshes).iter(), std::iter::once(&accel.tlas).chain(layered));
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
        self.carry(from, to);
    }

    /// `carry_grids`, of the grids themselves.
    fn carry(&self, from: &GridsGpu, to: &GridsGpu) {
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

    /// Whether a particle system's mesh is among the instances (they move every frame: motion vectors).
    pub fn particle_meshes(up: &Uploaded) -> bool {
        up.particles.is_some() || up.gpu_particles.is_some()
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

    /// An environment map's texture, with its texels.
    fn env_texture(&self, env: &EnvMap) -> wgpu::Texture {
        let t = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("env"),
            size: wgpu::Extent3d { width: env.width, height: env.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.write_env_texels(&t, env);
        t
    }

    fn write_env_texels(&self, t: &wgpu::Texture, env: &EnvMap) {
        let half: Vec<u16> = env.texels.iter().flat_map(|t| t.iter().map(|v| half::f16::from_f32(v.min(65000.0)).to_bits())).collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&half),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(env.width * 8), rows_per_image: Some(env.height) },
            wgpu::Extent3d { width: env.width, height: env.height, depth_or_array_layers: 1 },
        );
    }

    /// Whether a buffer of `bytes` can be made and bound (the device's buffer and storage binding limits).
    pub fn buffer_fits(&self, bytes: u64) -> bool {
        let l = self.device.limits();
        bytes <= (l.max_storage_buffer_binding_size as u64).min(l.max_buffer_size)
    }

    /// A sky's environment map and its sampling CDF (a dynamic sky's new bake, another sky): written in place where
    /// they are as large as the ones they replace, else a new texture, the CDF's section grown where it has to, and
    /// the scene bound again. False, with nothing changed, when the grown buffer would pass the device's limits (a
    /// build lays the scene's words out again, with less room for edits: the caller rebuilds).
    pub fn put_env(&self, up: &mut Uploaded, env: &EnvMap, cdf: &[f32]) -> bool {
        let size = up.env_tex.size();
        if (env.width, env.height) == (size.width, size.height) && cdf.len() == up.cdf_words {
            self.write_env_texels(&up.env_tex, env);
            up.words.write(&self.queue, Stream::EnvCdf, 0, bytemuck::cast_slice(cdf));
            return true;
        }
        let bytes = cdf.len() as u64 * 4;
        if bytes > up.words.cap(Stream::EnvCdf) {
            if !self.buffer_fits(up.words.size_with(Stream::EnvCdf, bytes)) {
                return false;
            }
            up.words.grow(&self.device, &self.queue, Stream::EnvCdf, bytes);
        }
        up.env_tex = self.env_texture(env);
        up.bg_parts.env_view = up.env_tex.create_view(&Default::default());
        up.words.write(&self.queue, Stream::EnvCdf, 0, bytemuck::cast_slice(cdf));
        up.cdf_words = cdf.len();
        self.rebind_scene(up);
        true
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
            up.heap.write(&self.queue, Table::Media, at as u64, bytemuck::bytes_of(&if hide { crate::scene::MEDIUM_HIDDEN } else { shape }));
        }
        found
    }

    /// The scene's density grids on the GPU: field A of each in the atlas (stacked along z), their data; for a grid
    /// with a program, its other textures (A', B, B'), its two half steps and its sources; the programs' pipeline.
    /// `media`: the media table (a grid's program runs in its volume's box); `programs`: every grid program, in the
    /// order of their numbers. None: no grid.
    fn make_grids(&self, grids: &[crate::scene::GridBuild], media: &[crate::scene::MediumGpu], programs: &[(String, String, String)]) -> Option<GridsGpu> {
        if grids.is_empty() {
            return None;
        }
        let d = &self.device;
        let dims = [grids.iter().map(|g| g.dims[0]).max().unwrap(), grids.iter().map(|g| g.dims[1]).max().unwrap(), grids.iter().map(|g| g.dims[2]).sum()];
        let atlas = grid_texture(d, "grid atlas", dims, true);
        let atlas_view = atlas.create_view(&Default::default());
        let smp = d.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let made = grids
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
                    let m = &media[g.medium as usize];
                    GridSim { b: field_b, program, params: g.params, steps, sources, placed: std::cell::Cell::new((m.w, m.c)) }
                });
                GridGpu { id: g.id, node: g.node, dims: g.dims, z0: g.z0, made: g.made, sim }
            })
            .collect::<Vec<_>>();
        let pipeline = made.iter().any(|g| g.sim.is_some()).then(|| {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("grid sim"), source: wgpu::ShaderSource::Wgsl(grid_sim_source(programs).into()) });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("grid sim"), bind_group_layouts: &[Some(&self.grid_layout)], immediate_size: 0 });
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("grid sim"), layout: Some(&layout), module: &module, entry_point: Some("grid_sim"), compilation_options: Default::default(), cache: self.cache() })
        });
        let programs = if pipeline.is_some() { programs.to_vec() } else { vec![] };
        Some(GridsGpu { atlas, pipeline, programs, grids: made, step: std::cell::Cell::new(0) })
    }

    /// The uploaded scene's density grids as `grids`, in place: a volume with a grid came or went, a grid has other
    /// cells or another program, a program's code changed. Grids that are the same still take their parameters,
    /// their volumes and the volumes' placement; else all are made again, and what the programs had simulated goes with the grids that are the same (`carry`).
    /// Whether they were made again (the atlas is another texture: the bind groups hold the old one).
    pub fn put_grids(&self, up: &mut Uploaded, grids: &[crate::scene::GridBuild], media: &[crate::scene::MediumGpu], programs: &[(String, String, String)]) -> bool {
        let same = match &mut up.grids {
            None => grids.is_empty(),
            Some(have) => {
                // (The same grids in the same order lie at the same places.)
                let same = have.grids.len() == grids.len()
                    && have.grids.iter().zip(grids).all(|(h, g)| (h.id, h.dims, h.made) == (g.id, g.dims, g.made) && h.sim.as_ref().map(|s| s.program) == g.program)
                    && (have.pipeline.is_none() || have.programs == programs);
                if same {
                    // (And its volume, with the volume's box and customData as the table has them now: its program
                    // runs there.)
                    for (h, g) in have.grids.iter_mut().zip(grids) {
                        h.node = g.node;
                        if let Some(sim) = &mut h.sim {
                            let m = &media[g.medium as usize];
                            (sim.params, sim.placed) = (g.params, std::cell::Cell::new((m.w, m.c)));
                        }
                    }
                }
                same
            }
        };
        if same {
            return false;
        }
        let made = self.make_grids(grids, media, programs);
        if let (Some(from), Some(to)) = (&up.grids, &made) {
            self.carry(from, to);
        }
        up.bg_parts.grid_view = made.as_ref().map_or_else(|| self.no_grid_view.clone(), |g| g.atlas.create_view(&Default::default()));
        up.grids = made;
        true
    }

    /// The scene's density grids as they are now: each one's id, volume node, cells, first layer in the atlas and
    /// program; the atlas's size.
    #[cfg(test)]
    pub fn grids_state(&self, up: &Uploaded) -> (Vec<(u32, u32, [u32; 3], u32, Option<u32>)>, [u32; 3]) {
        let Some(gs) = &up.grids else { return (vec![], [0; 3]) };
        let size = gs.atlas.size();
        (gs.grids.iter().map(|g| (g.id, g.node, g.dims, g.z0, g.sim.as_ref().map(|s| s.program))).collect(), [size.width, size.height, size.depth_or_array_layers])
    }

    /// Field A of one cell of density grid `id`, read back.
    #[cfg(test)]
    pub fn grid_cell(&self, up: &Uploaded, id: u32, cell: [u32; 3]) -> [f32; 4] {
        let gs = up.grids.as_ref().expect("grids");
        let g = gs.grids.iter().find(|g| g.id == id).expect("the grid");
        let to = self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut e = self.device.create_command_encoder(&Default::default());
        e.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &gs.atlas, mip_level: 0, origin: wgpu::Origin3d { x: cell[0], y: cell[1], z: g.z0 + cell[2] }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &to, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: None, rows_per_image: None } },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        self.queue.submit([e.finish()]);
        to.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let halves: [half::f16; 4] = bytemuck::pod_read_unaligned(&to.slice(..).get_mapped_range().unwrap());
        halves.map(|h| h.to_f32())
    }

    /// The media table as `media` (each one's node in `nodes`), in place: volumes that came, went or were set again
    /// while the scene runs. Whether the table is another buffer now (it grew: the bind groups hold the old one).
    pub fn put_media(&self, up: &mut Uploaded, media: &[crate::scene::MediumGpu], nodes: Vec<u32>) -> bool {
        let bytes: &[u8] = bytemuck::cast_slice(media);
        let grew = bytes.len() as u64 > up.heap.cap(Table::Media);
        if grew {
            up.heap.grow(&self.device, &self.queue, Table::Media, bytes.len() as u64);
        }
        if !bytes.is_empty() {
            up.heap.write(&self.queue, Table::Media, 0, bytes);
        }
        up.media_hidden = std::cell::RefCell::new(media.iter().map(|m| m.a[1] == crate::scene::MEDIUM_HIDDEN).collect());
        up.media_nodes = nodes;
        grew
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
                up.heap.write(&self.queue, Table::Media, (k * size) as u64, bytemuck::cast_slice(&r));
            }
            if let Some(c) = custom {
                up.heap.write(&self.queue, Table::Media, (k * size + std::mem::offset_of!(crate::scene::MediumGpu, c)) as u64, bytemuck::cast_slice(&c));
            }
        }
    }

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

    /// Mesh `k` (one an edit no longer draws) made again over `count` triangles from `first`: a new BLAS in its slot.
    pub fn set_mesh(&self, accel: &mut Accel, k: u32, first: usize, mesh: &crate::scene::InstMesh, sections: &[crate::scene::Section]) {
        let (blas, geo) = self.mesh_blas_for(first, mesh, sections);
        accel.meshes[k as usize] = blas;
        accel.mesh_geo[k as usize] = geo;
        accel.mesh_verts[k as usize] = (mesh.vfirst, mesh.vcount);
    }

    fn mesh_blas_for(&self, first: usize, mesh: &crate::scene::InstMesh, sections: &[crate::scene::Section]) -> (wgpu::Blas, (Vec<(u32, u32)>, Vec<wgpu::BlasTriangleGeometrySizeDescriptor>)) {
        let parts: Vec<(u32, u32)> = mesh.parts(sections).into_iter().map(|(at, count)| (first as u32 + at, count)).collect();
        let sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor> = parts
            .iter()
            .map(|&(_, c)| wgpu::BlasTriangleGeometrySizeDescriptor {
                vertex_format: wgpu::VertexFormat::Float32x3,
                vertex_count: mesh.vcount.max(1),
                index_format: Some(wgpu::IndexFormat::Uint32),
                index_count: Some(c * 3),
                flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
            })
            .collect();
        let blas = self.device.create_blas(&wgpu::CreateBlasDescriptor { label: Some("edit mesh"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: sizes.clone() });
        (blas, (parts, sizes))
    }

    /// A new instanced mesh over `count` triangles from GPU-order index `first` (in the edit room; live/edits.rs
    /// `edit_geometry`), its first `opaque` ones opaque: its BLAS, which the commit's build builds (`TopBuild::meshes`). Returns its index.
    pub fn add_mesh(&self, accel: &mut Accel, first: usize, mesh: &crate::scene::InstMesh, sections: &[crate::scene::Section]) -> u32 {
        // Built fast rather than traced fast (edits come every frame under destruction; the batches of pieces are rebuilt
        // every frame): `mesh_blas_for`.
        let (blas, geo) = self.mesh_blas_for(first, mesh, sections);
        accel.meshes.push(blas);
        accel.mesh_geo.push(geo);
        accel.mesh_verts.push((mesh.vfirst, mesh.vcount));
        accel.meshes.len() as u32 - 1
    }

    /// Instanced mesh `k`'s BLAS (a TLAS instance of it).
    pub fn mesh_blas(accel: &Accel, k: u32) -> &wgpu::Blas {
        &accel.meshes[k as usize]
    }

    /// The AABB BLAS of a procedural geometry's boxes (live/edits.rs `edit_geometry`): the one kept by its hash, the
    /// one this commit already made (`pending`), or a new one added to `pending` (`TopBuild::procs`).
    pub fn proc_blas(&self, blob: &crate::procasset::ProcBlob, pending: &mut Vec<PendingProc>) -> wgpu::Blas {
        if let Some((blas, _)) = self.proc_blases.lock().unwrap().get(&blob.hash) {
            return blas.clone();
        }
        if let Some(p) = pending.iter().find(|p| p.hash == blob.hash) {
            return p.blas.clone();
        }
        let d = &self.device;
        let size = wgpu::BlasAABBGeometrySizeDescriptor { primitive_count: (blob.boxes.len() / 6) as u32, flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE };
        let blas = d.create_blas(&wgpu::CreateBlasDescriptor { label: Some("proc"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::AABBs { descriptors: vec![size.clone()] });
        let boxes = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("proc boxes"), contents: bytemuck::cast_slice(&blob.boxes), usage: wgpu::BufferUsages::BLAS_INPUT });
        let bytes = self.aabb_blas_bytes(&boxes, size.primitive_count).unwrap_or(0);
        pending.push(PendingProc { hash: blob.hash, blas: blas.clone(), boxes, size, bytes });
        blas
    }

    /// The scene's bind group (group 0: pathtrace.wgsl's declarations, the layout `Gpu::new` makes).
    fn scene_bind_group(&self, b: &SceneBuffers) -> wgpu::BindGroup {
        let view = |binding: u32, v| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) };
        let p = b.parts;
        let mut entries = vec![
            buffer_entry(0, b.params),
            buffer_entry(1, b.vertices),
            buffer_entry(4, &b.heap.buffer),
            buffer_entry(5, &b.words.buffer),
            buffer_entry(6, b.prev_vertices),
            view(7, &p.tex_view),
            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&p.sampler) },
            view(9, &p.env_view),
            view(10, &p.grid_view),
            buffer_entry(11, b.accum),
        ];
        if !self.hwrt {
            entries.push(buffer_entry(2, &p.leaves));
            entries.push(buffer_entry(3, &p.nodes));
        }
        if !self.merged && !self.whole_tables {
            for &(table, binding) in &crate::merged::TABLE_VIEWS {
                entries.push(wgpu::BindGroupEntry { binding, resource: b.heap.binding(table) });
            }
        }
        if let Some(a) = b.accel {
            entries.push(wgpu::BindGroupEntry { binding: ACC_BINDING, resource: a.tlas.as_binding() });
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.layout, entries: &entries })
    }

    /// The bind groups that hold the scene's buffers again, from `up`'s as they are now (one that grew: the tables,
    /// live/edits.rs): the kernel's, the light list's fills', the skinning pass's and the particle passes'.
    pub fn rebind_scene(&self, up: &mut Uploaded) {
        if let Some(f) = &mut up.fills.particles {
            f.bg = self.emitter_fill_bind_group(&f.runs, &up.vertices, &up.heap, &up.words);
        }
        if let Some(k) = &mut up.skin {
            k.bg = skin_bind_group(&self.device, &k.pipeline, [&k.params, &k.blocks, &k.table, &k.corners, &k.palette], &up.vertices);
            if let Some(l) = &mut k.leaves {
                l.bg = Some(leaf_gather_bind_group(&self.device, l, &up.vertices, &up.words.buffer, &up.bg_parts.leaves));
            }
        }
        if let Some(a) = &mut up.accel {
            (a.vertices, a.indices, a.index_first) = (up.vertices.clone(), up.words.buffer.clone(), (up.words.at(Stream::Indices) / 4) as u32);
        }
        up.bg = self.scene_bind_group(&SceneBuffers { params: &up.params, vertices: &up.vertices, heap: &up.heap, words: &up.words, prev_vertices: &up.prev_vertices, accum: &up.accum, parts: &up.bg_parts, accel: up.accel.as_ref() });
        if up.accel.is_none() {
            return;
        }
        let written = ParticleTargets { vertices: &up.vertices, prev_vertices: &up.prev_vertices, heap: &up.heap };
        if let Some(p) = &mut up.particles {
            p.bg = particle_bind_group(&self.device, &p.pipeline, "particles", &[(0, &p.params), (1, &p.table), (7, &p.last)], &written);
        }
        if let Some(p) = &mut up.gpu_particles {
            p.bg = particle_bind_group(&self.device, &p.pipeline, "gpu particles", &[(0, &p.params), (1, &p.table), (7, &p.last), (8, &p.state), (9, &p.spawns), (10, &p.grid)], &written);
        }
    }

    /// The skinning pass of `scene`: its pipeline, the models' corners, a job each skinned model, and room for a frame's
    /// work and every job's palette. `tree`: software traversal's words' heap and leaves, for the pass that takes the
    /// leaves again.
    fn skin_pass(&self, scene: &Scene, vertices: &wgpu::Buffer, tree: Option<(&wgpu::Buffer, &wgpu::Buffer)>) -> SkinGpu {
        let d = &self.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("skin"), source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/skin.wgsl").into()) });
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("skin"), layout: None, module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: self.cache() });
        // The models' vertices one model after another (`indexed::Indexed::skins`).
        let ix = &scene.indexed;
        let mut source_first = vec![];
        let mut at = 0u32;
        for (vertices, _) in &ix.skins {
            source_first.push(at);
            at += vertices.len() as u32;
        }
        let corners: Vec<crate::skin::SkinCorner> = ix.skins.iter().flat_map(|s| s.0.iter().copied()).collect();
        let (mut jobs, mut matrices, mut blocks) = (vec![], 0u32, 0u32);
        for g in &scene.retained.skinned {
            let (first, count) = ix.meshes[g.mesh as usize];
            jobs.push((SkinJob { first, count, source: source_first[g.source as usize], palette: matrices }, g.palette_len));
            matrices += g.palette_len;
            blocks += count.div_ceil(SKIN_BLOCK);
        }
        let table: Vec<SkinJob> = jobs.iter().map(|j| j.0).collect();
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let init = |label: &str, bytes: &[u8]| d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytes, usage: storage });
        let room = |label: &str, size: u64, usage| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage, mapped_at_creation: false });
        let (b_corners, b_table) = (init("skin corners", bytemuck::cast_slice(&corners)), init("skin jobs", bytemuck::cast_slice(&table)));
        let params = room("skin params", 16, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let (b_blocks, palette) = (room("skin blocks", blocks as u64 * 8, storage), room("skin palette", matrices as u64 * 48, storage));
        let bg = skin_bind_group(d, &pipeline, [&params, &b_blocks, &b_table, &b_corners, &palette], vertices);
        let leaves = tree.map(|(words, leaves)| {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("leaf gather"), source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/leaf_gather.wgsl").into()) });
            let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("leaf gather"), layout: None, module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: self.cache() });
            let jobs: Vec<LeafJob> = scene.retained.skinned.iter().map(|g| { let m = &scene.meshes[g.mesh as usize]; LeafJob { first: m.first, count: m.count, vertex: ix.meshes[g.mesh as usize].0, spare: 0 } }).collect();
            let blocks: u64 = jobs.iter().map(|j| j.count.div_ceil(SKIN_BLOCK) as u64).sum();
            let (params, b_blocks, b_jobs) = (room("leaf gather params", 16, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST), room("leaf gather blocks", blocks * 8, storage), init("leaf gather jobs", bytemuck::cast_slice(&jobs)));
            let mut l = LeafGather { pipeline, bg: None, params, blocks: b_blocks, table: b_jobs, jobs };
            l.bg = Some(leaf_gather_bind_group(d, &l, vertices, words, leaves));
            l
        });
        SkinGpu { pipeline, bg, params, blocks: b_blocks, table: b_table, corners: b_corners, palette, jobs, source_first, palette_end: matrices, leaves }
    }

    /// Skins the meshes of `work` (a job and its pose's palette each) on the GPU: their triangles, shading attributes
    /// and BLAS input, in a submit of its own (before the commit builds their BLASes). False: the scene has no such
    /// pass, or a palette is not the job's size (nothing was written).
    pub fn skin(&self, up: &Uploaded, work: &[(u32, Vec<crate::skin::PaletteRows>)]) -> bool {
        let Some(s) = &up.skin else { return false };
        if work.iter().any(|(job, rows)| s.jobs.get(*job as usize).is_none_or(|j| j.1 as usize != rows.len())) {
            return false;
        }
        let mut blocks: Vec<[u32; 2]> = vec![];
        for (job, rows) in work {
            let j = s.jobs[*job as usize].0;
            self.queue.write_buffer(&s.palette, j.palette as u64 * 48, bytemuck::cast_slice(rows));
            blocks.extend((0..j.count.div_ceil(SKIN_BLOCK)).map(|b| [*job, b * SKIN_BLOCK]));
        }
        if blocks.is_empty() {
            return true;
        }
        let n = blocks.len() as u32;
        self.queue.write_buffer(&s.blocks, 0, bytemuck::cast_slice(&blocks));
        self.queue.write_buffer(&s.params, 0, bytemuck::cast_slice(&[n, 0, 0, 0]));
        let mut e = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("skin"), timestamp_writes: None });
            pass.set_pipeline(&s.pipeline);
            pass.set_bind_group(0, &s.bg, &[]);
            pass.dispatch_workgroups(n.min(SKIN_ROW), n.div_ceil(SKIN_ROW), 1);
        }
        // Software traversal: the same meshes' leaves, from the vertices just written.
        if let Some(l) = &s.leaves {
            let blocks: Vec<[u32; 2]> = work.iter().flat_map(|(job, _)| (0..l.jobs[*job as usize].count.div_ceil(SKIN_BLOCK)).map(|b| [*job, b * SKIN_BLOCK])).collect();
            if !blocks.is_empty() {
                let n = blocks.len() as u32;
                self.queue.write_buffer(&l.blocks, 0, bytemuck::cast_slice(&blocks));
                self.queue.write_buffer(&l.params, 0, bytemuck::cast_slice(&[n, (up.words.at(Stream::Indices) / 4) as u32, 0, 0]));
                let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("leaf gather"), timestamp_writes: None });
                pass.set_pipeline(&l.pipeline);
                pass.set_bind_group(0, l.bg.as_ref().expect("made with the pass"), &[]);
                pass.dispatch_workgroups(n.min(SKIN_ROW), n.div_ceil(SKIN_ROW), 1);
            }
        }
        self.queue.submit([e.finish()]);
        true
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

    /// Software traversal's top tree again, over the units as they are now, written to its range of the node buffer
    /// (what the TLAS's build is to hardware traversal). Err: no tree can be made (more units than its range holds, a
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
        self.write_nodes(nodes, root, &tree.nodes);
    }

    /// A tree's nodes written again where they lie (its boxes taken again, `units::refit`: as many, the same shape).
    pub fn write_nodes(&self, nodes: &wgpu::Buffer, root: u32, tree: &[crate::bvh::GpuNode]) {
        self.queue.write_buffer(nodes, root as u64 * 32, bytemuck::cast_slice(tree));
    }

    /// A freed mesh tree's nodes (`Units::drop_tree`: its root and length) are room again: leaves that end a walk
    /// (`units::FILL`), as the room was laid out. (No unit names them: `Units::top` holds that. This is so that the
    /// room never holds a part of an old tree beside a part of a new one.)
    pub fn fill_tree(&self, nodes: &wgpu::Buffer, (root, len): (u32, u32)) {
        self.queue.write_buffer(nodes, root as u64 * 32, bytemuck::cast_slice(&vec![crate::units::FILL; len as usize]));
    }

    /// What a commit left to build above the triangles, once. Hardware traversal: the BLASes named and the TLAS in one
    /// call: on Metal a TLAS built apart from a BLAS it references, in the next submit too, can see it unbuilt and
    /// lose its instance until the TLAS is next built (tests/render/skinning-shared-pose-live). Software traversal:
    /// the top tree over the units as they are now; Err: none can be made (`retop`), nothing was written, and the
    /// caller rebuilds the scene.
    pub fn build_top(&self, up: &mut Uploaded, b: TopBuild) -> std::result::Result<(), String> {
        match Top::of(&mut up.accel, &mut up.units) {
            Some(Top::Hw(accel)) => {
                let mut meshes = b.meshes;
                meshes.sort_unstable();
                meshes.dedup();
                let mut entries = mesh_entries(accel, &meshes);
                entries.extend(b.procs.iter().map(|p| wgpu::BlasBuildEntry { blas: &p.blas, geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry { size: &p.size, stride: 24, aabb_buffer: &p.boxes, primitive_offset: 0 }]) }));
                let mut e = self.device.create_command_encoder(&Default::default());
                e.build_acceleration_structures(entries.iter(), [&accel.tlas]);
                self.queue.submit([e.finish()]);
                drop(entries);
                // Built: kept for reuse (a commit that falls back never builds, and a BLAS never built must not be).
                self.proc_blases.lock().unwrap().extend(b.procs.into_iter().map(|p| (p.hash, (p.blas, p.bytes))));
                Ok(())
            }
            Some(Top::Sw(units)) => self.retop(units, &up.bg_parts.nodes),
            None => Ok(()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Offline translucency: the pass (translucent.wgsl) `spp` times at random points in each pixel, each with a fresh
    /// lighting estimate per hit; the mean (colour, transmittance) per pixel. Empty without translucent particles.
    fn render_translucent(&self, up: &Uploaded, base: Params, spp: u32, (w, h): (u32, u32)) -> Result<Vec<[f32; 4]>> {
        let Some(t) = &up.translucent else { return Ok(vec![]) };
        let d = &self.device;
        let pipeline = self.translucent_pipelines(&up.kernel, false).draw;
        let size = w as u64 * h as u64 * 16;
        let sums = d.create_buffer(&wgpu::BufferDescriptor { label: Some("translucent sums"), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        // (Drawn at full resolution: the reduced image's buffer is bound and not read.)
        let tbg = self.translucent_bind_group(t, &sums, &self.translucent_low(1, 1));
        let tp = TranslucentParams { size: [w, h, 0, 0], view: [w, h, 0, 0], misc: [0.0; 4] };
        self.queue.write_buffer(&t.params, 0, bytemuck::bytes_of(&tp));
        let n = spp.max(1);
        for s in 0..n {
            let mut p = base;
            p.size[2] = s;
            up.place(&mut p);
            self.no_live_bufs.place(&mut p);
            self.queue.write_buffer(&up.params, 0, bytemuck::bytes_of(&p));
            let mut enc = d.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("translucent"), timestamp_writes: None });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &up.bg, &[]);
                pass.set_bind_group(1, self.no_live_tl.as_ref().unwrap(), &[]);
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
    pub fn render(&self, scene: &Scene, nodes: &[u8], emitters: &[[f32; 4]], emitter_tris: &[crate::GpuEmitterTri], emitter_runs: &crate::EmitterRuns, light_tree: &[[f32; 4]], env: &EnvMap, env_cdf: &[f32], energy: &[[f32; 4]], base: Params, spp: u32, timings: &mut Vec<(String, f64)>) -> Result<Frame> {
        let d = &self.device;
        let (w, h) = (scene.width, scene.height);
        let up = self.upload(scene, &self.accum_buffer(w, h), nodes, emitters, emitter_tris, emitter_runs, light_tree, env, env_cdf, energy, timings)?;
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
                // (What the stateful systems placed: the stateless pass above built before they ran.)
                if let Some(a) = &up.accel {
                    self.build_particles(&up, a, false, &mut e);
                }
                self.queue.submit([e.finish()]);
            }
        }
        // The particles' emitters, where the passes above placed them.
        if let Some(f) = &up.fills.particles {
            let mut e = d.create_command_encoder(&Default::default());
            self.fill_emitters(f, &mut e);
            self.queue.submit([e.finish()]);
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
        let stopped_before = self.guard_trips(&self.no_live_bufs, &up.kernel)?;


        let t1 = Instant::now();
        let mut enc = d.create_command_encoder(&Default::default());
        enc.clear_buffer(b_accum, 0, None);
        self.queue.submit([enc.finish()]);
        // One dispatch per sample; params are rewritten between submissions. Batch a few per submit.
        let batch = 4u32;
        let mut s = 0u32;
        while s < spp {
            for k in 0..batch.min(spp - s) {
                let mut p = base;
                p.size[2] = s + k;
                up.place(&mut p);
                self.no_live_bufs.place(&mut p);
                self.queue.write_buffer(b_params, 0, bytemuck::bytes_of(&p));
                let mut enc = d.create_command_encoder(&Default::default());
                {
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, bg, &[]);
                    pass.set_bind_group(1, &self.no_live, &[]);
                    pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                }
                self.queue.submit([enc.finish()]);
            }
            s += batch.min(spp - s);
            self.device.poll(wgpu::PollType::wait_indefinitely())?;
        }
        timings.push(("trace_ms".into(), t1.elapsed().as_secs_f64() * 1000.0));
        // The loop guard, if the kernel has it: a render in which it stopped a loop has wrong pixels, and is an error.
        if let (Some(before), Some(now)) = (stopped_before, self.guard_trips(&self.no_live_bufs, &up.kernel)?) {
            if now != before {
                return Err(anyhow!("the loop guard stopped {} of the kernel's invocations at {} iterations (THREERT_LOOP_GUARD): a traversal that does not end on this scene's data, or a limit too low for it", now.wrapping_sub(before), loop_guard_of(up.kernel.features & F_UNITS != 0).unwrap_or(0)));
            }
        }
        let t_tl = Instant::now();
        let overlay = self.render_translucent(&up, base, spp, (w, h))?;
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

    /// A package built before the kernel was WESL carries the scene library's old hash (`a * b ^ c * d`): as it is,
    /// WESL refuses it (Daylight Home on ThreeRT 0.4.2: the engine panicked); through `legacy_wgsl` it compiles.
    #[test]
    fn a_package_with_the_old_scene_library_still_compiles() {
        let old = "fn std_pcg(v: u32) -> u32 {\n    let s = v * 747796405u + 2891336453u;\n    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;\n    return (w >> 22u) ^ w;\n}\nfn std_hashi(x: i32, y: i32, z: i32) -> f32 {\n    return f32(std_pcg(bitcast<u32>(x) * 73856093u ^ bitcast<u32>(y) * 19349663u ^ bitcast<u32>(z) * 83492791u) >> 8u) * (1.0 / 16777216.0);\n}\nfn std_hash3(x: f32, y: f32, z: f32) -> f32 {\n    let v = bitcast<u32>(x) * 73856093u ^ bitcast<u32>(y) * 19349663u ^ bitcast<u32>(z) * 83492791u;\n    return f32(std_pcg(v) >> 8u) * (1.0 / 16777216.0);\n}\n";
        let slot = "fn material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut { return base; }\nfn translucent_material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut { return base; }\nfn alpha_material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut { return base; }\nfn emissive_material_function(f: i32, i: MatIn, m: Material, base: MatOut) -> MatOut { return base; }\n";
        let template = kernel_template();
        let with = |functions: &str| {
            let (head, rest) = template.split_once("// @material-functions-begin").unwrap();
            let (_, tail) = rest.split_once("// @material-functions-end").unwrap();
            format!("{head}{functions}{slot}{tail}")
        };
        let v = KernelVariant::new(false, KernelSwitches::default(), 0);
        assert!(try_kernel_wgsl(&with(old), &v).is_err(), "WESL takes the old hash as written: the shim is no longer needed");
        let fixed = legacy_wgsl(old.to_string());
        assert!(!fixed.contains("73856093u ^ "), "{fixed}");
        let wgsl = try_kernel_wgsl(&with(&fixed), &v).expect("the old library, parenthesised, compiles");
        naga_valid(&wgsl);
        // What is not the old library is left alone.
        assert_eq!(legacy_wgsl("fn f() -> u32 { return 1u ^ 2u; }".into()), "fn f() -> u32 { return 1u ^ 2u; }");
    }
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
            ("F_GLASS", F_GLASS), ("F_COAT", F_COAT), ("F_INST", F_INST), ("F_MEDIA", F_MEDIA), ("F_PEMIT", F_PEMIT),
            ("F_HMEDIA", F_HMEDIA), ("F_PROC", F_PROC), ("F_TINT", F_TINT), ("F_UNLIT", F_UNLIT), ("F_BACKGROUND", F_BACKGROUND),
            ("F_VCOLOR", F_VCOLOR), ("F_AERIAL", F_AERIAL), ("F_GRID", F_GRID), ("F_LISTS", F_LISTS), ("F_IFUNC", F_IFUNC), ("F_IGLASS", F_IGLASS),
            ("F_DEFORM", F_DEFORM), ("F_PART", F_PART), ("F_MIRROR", F_MIRROR),
        ];
        let mut seen = 0u32;
        for (name, bit) in all {
            assert!(bit.is_power_of_two() && seen & bit == 0, "{name} = {bit:#x} is not a bit of its own");
            seen |= bit;
        }
        let wgsl = include_str!("../shaders/pathtrace.wgsl");
        // The live approximations: one bit each, the shader's the same, `A_LIVE` all of them.
        let approx = [("A_GLASS_SHADOWS", A_GLASS_SHADOWS), ("A_MEDIA_SHADOWS", A_MEDIA_SHADOWS), ("A_CUTOUT_PATHS", A_CUTOUT_PATHS), ("A_CUTOUT_EMITTERS", A_CUTOUT_EMITTERS), ("A_FAR_EMITTERS", A_FAR_EMITTERS)];
        assert_eq!(approx.iter().fold(0, |all, a| { assert!(a.1.is_power_of_two() && all & a.1 == 0, "{} = {}", a.0, a.1); all | a.1 }), A_LIVE);
        let declared: Vec<&str> = wgsl.lines().filter(|l| l.starts_with("const A_")).collect();
        assert_eq!(declared.len(), approx.len());
        for (name, bit) in approx {
            assert!(declared.iter().any(|l| l.starts_with(&format!("const {name}: u32 = {bit}u;"))), "{name}: pathtrace.wgsl and gpu.rs differ");
        }
        // (The render tests' names for them: tests/render/lib.ts `APPROX`.)
        let lib = include_str!("../../tests/render/lib.ts");
        for (name, bit) in [("glassShadows", A_GLASS_SHADOWS), ("mediaShadows", A_MEDIA_SHADOWS), ("cutoutPaths", A_CUTOUT_PATHS), ("cutoutEmitters", A_CUTOUT_EMITTERS), ("farEmitters", A_FAR_EMITTERS)] {
            assert!(lib.contains(&format!("{name}: {bit},")), "{name}: tests/render/lib.ts and gpu.rs differ");
        }
        assert!(lib.contains(&format!("all: {A_LIVE},")));
        assert!(wgsl.contains(&format!("const RIS_EXACT: u32 = {RIS_EXACT}u;")), "RIS_EXACT: pathtrace.wgsl and gpu.rs differ");
        for line in wgsl.lines().filter(|l| l.starts_with("const F_")) {
            let name = &line[6..line.find(':').unwrap()];
            let value: u32 = line[line.find('=').unwrap() + 1..line.find("u;").unwrap()].trim().parse().unwrap();
            let rust = all.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name}: not in gpu.rs")).1;
            assert_eq!(value, rust, "{name}: pathtrace.wgsl and gpu.rs differ");
        }
    }

    /// The level the kernel reads a texture at, less the texture's own size (`mesh_density` and `mesh_lod`, their own
    /// text, run on the GPU): half the log of a triangle's UV area times the material's repeat over its area, times a
    /// footprint's square, over what else divides; the repeat alone for the triangle where it or its UVs have no
    /// area. Triangles of every size and UV scale against the same in 64-bit floats.
    #[test]
    fn the_kernels_texel_density_is_the_triangles() {
        use wgpu::util::DeviceExt;
        let g = match super::Gpu::new() {
            Ok(g) => g,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        let part = |src: &'static str, from: &str, to: &str| {
            let at = src.find(from).unwrap_or_else(|| panic!("the kernel has `{from}`"));
            &src[at..at + src[at..].find(to).expect("its end") + to.len()]
        };
        let attr = part(include_str!("../shaders/records.wgsl"), "struct TriAttr {", "};");
        let kernel = include_str!("../shaders/pathtrace.wgsl");
        let density = part(kernel, "fn mesh_density(e1: vec3<f32>, e2: vec3<f32>, a: TriAttr, m: Material) -> vec2<f32> {", "\n}\n");
        let level = part(kernel, "fn mesh_lod(density: vec2<f32>, footprint2: f32, over: f32) -> f32 {", "}\n");
        // (A row: the two edges, with the footprint's square and the divisor in their fourth words; the attributes; the
        // material's `uv`.)
        let src = format!(
            "{attr}\nstruct Material {{ uv: vec4<f32> }};\nstruct In {{ e1: vec4<f32>, e2: vec4<f32>, a: TriAttr, m: vec4<f32> }};\n\
             @group(0) @binding(0) var<storage, read> input: array<In>;\n@group(0) @binding(1) var<storage, read_write> out: array<f32>;\n{density}\n{level}\n\
             @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {{\n    if (id.x >= arrayLength(&input)) {{ return; }}\n    let i = input[id.x];\n    out[id.x] = mesh_lod(mesh_density(i.e1.xyz, i.e2.xyz, i.a, Material(i.m)), i.e1.w, i.e2.w);\n}}\n"
        );
        // A deterministic stream in [0, 1).
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut f = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 40) as f32) / (1u64 << 24) as f32
        };
        let (mut rows, mut want): (Vec<[f32; 32]>, Vec<f64>) = (vec![], vec![]);
        for k in 0..512 {
            // Sizes from a millimetre to a hundred metres, UVs from a hundredth of the texture to a hundred repeats,
            // footprints from a tenth of a millimetre to ten metres, a divisor from a thousandth to one.
            let (size, uvs) = (10f32.powf(f() * 5.0 - 3.0), 10f32.powf(f() * 4.0 - 2.0));
            let (fp2, over) = (10f32.powf(f() * 10.0 - 8.0), 10f32.powf(-f() * 3.0));
            let mut v = |s: f32| [(f() - 0.5) * s, (f() - 0.5) * s, (f() - 0.5) * s];
            let (mut e1, mut e2) = (v(size), v(size));
            let (uv0, mut uv1, mut uv2) = ([f() * uvs, f() * uvs], [f() * uvs, f() * uvs], [f() * uvs, f() * uvs]);
            let rep = if k % 4 == 0 { [10f32.powf(f() * 2.0 - 1.0), -10f32.powf(f() * 2.0 - 1.0)] } else { [1.0, 1.0] };
            match k % 16 {
                // A triangle without area (its edges along one line, or none), UVs without area.
                5 => e2 = e1.map(|x| x * 2.0),
                6 => (e1, e2) = ([0.0; 3], [0.0; 3]),
                7 => (uv1, uv2) = (uv0, uv0),
                8 => uv2 = [uv0[0] + (uv1[0] - uv0[0]) * 3.0, uv0[1] + (uv1[1] - uv0[1]) * 3.0],
                _ => {}
            }
            let mut row = [0f32; 32];
            row[..3].copy_from_slice(&e1);
            row[4..7].copy_from_slice(&e2);
            (row[3], row[7]) = (fp2, over);
            // (The attributes' UV words: n0.w, n1.w = uv0; n2.w, uv.x = uv1; uv.y, uv.z = uv2.)
            (row[8 + 3], row[8 + 7], row[8 + 11], row[8 + 12], row[8 + 13], row[8 + 14]) = (uv0[0], uv0[1], uv1[0], uv1[1], uv2[0], uv2[1]);
            // (The attributes: five vec4s; then the material's.)
            (row[28], row[29]) = (rep[0], rep[1]);
            rows.push(row);
            let d = |a: [f32; 3]| a.map(|x| x as f64);
            let (a, b) = (d(e1), d(e2));
            let cross = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
            let area = 0.5 * (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
            let (d1, d2) = ([(uv1[0] - uv0[0]) as f64, (uv1[1] - uv0[1]) as f64], [(uv2[0] - uv0[0]) as f64, (uv2[1] - uv0[1]) as f64]);
            let uv_area = 0.5 * (d1[0] * d2[1] - d2[0] * d1[1]).abs();
            let ratio = if area > 1e-12 && uv_area > 1e-14 { uv_area / area } else { 1.0 };
            want.push(0.5 * (ratio * ((rep[0] * rep[1]).abs() as f64).max(1e-12) * fp2 as f64 / over as f64).max(1e-30).log2());
        }
        let d = &g.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("texel density"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let input = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&rows), usage: wgpu::BufferUsages::STORAGE });
        let bytes = (rows.len() * 4) as u64;
        let out = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let to = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: out.as_entire_binding() }] });
        let mut e = d.create_command_encoder(&Default::default());
        {
            let mut pass = e.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(rows.len().div_ceil(64) as u32, 1, 1);
        }
        e.copy_buffer_to_buffer(&out, 0, &to, 0, bytes);
        g.queue.submit([e.finish()]);
        to.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let got: Vec<f32> = bytemuck::pod_collect_to_vec(&to.slice(..).get_mapped_range().unwrap());
        let (mut flat, mut spread) = (0, (f64::MAX, f64::MIN));
        for (k, (g, w)) in got.iter().zip(&want).enumerate() {
            // (A triangle or UVs at the edge of having an area may fall on either side of it in 32 bits: not those.)
            let edge = matches!(k % 16, 5 | 8);
            assert!(edge || (*g as f64 - w).abs() <= 2e-4 + 2e-5 * w.abs(), "triangle {k}: the kernel's {g}, {w} in 64 bits");
            flat += (k % 16 == 6 || k % 16 == 7) as usize;
            spread = (spread.0.min(*w), spread.1.max(*w));
        }
        assert!(flat > 40 && spread.0 < -8.0 && spread.1 > 8.0, "the triangles span the levels ({spread:?}) and {flat} have no area");
    }

    fn naga_valid(src: &str) {
        let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    }

    /// The kernel's bind group layouts are what its source declares, in every layout and traversal: a variant's
    /// bindings are the layout's, each of its kind (a storage buffer's access too), but for the loop guard's count,
    /// which a layout as written has room for and a kernel without the guard does not name.
    #[test]
    fn the_kernels_layouts_are_its_declarations() {
        use naga::{AddressSpace, ImageClass, ImageDimension as Dim, ScalarKind, StorageAccess, TypeInner};
        use wgpu::TextureViewDimension as View;
        type Entry = (u32, u32, wgpu::BindingType);
        let declared = |v: &KernelVariant| -> Vec<Entry> {
            let src = kernel_wgsl(&kernel_template(), v);
            let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
            let mut out: Vec<Entry> = m
                .global_variables
                .iter()
                .filter_map(|(_, g)| {
                    let b = g.binding.as_ref()?;
                    let name = g.name.as_deref().unwrap_or("");
                    let ty = match (g.space, &m.types[g.ty].inner) {
                        (AddressSpace::Uniform, _) => UNIFORM_BUFFER,
                        (AddressSpace::Storage { access }, _) => storage_buffer(!access.contains(StorageAccess::STORE)),
                        (AddressSpace::Handle, TypeInner::Image { dim: Dim::D2, arrayed: false, class: ImageClass::Sampled { kind: ScalarKind::Uint, multi: false } }) => {
                            wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: View::D2, multisampled: false }
                        }
                        (AddressSpace::Handle, TypeInner::Image { dim, arrayed, class: ImageClass::Sampled { kind: ScalarKind::Float, multi: false } }) => float_texture(match (dim, arrayed) {
                            (Dim::D2, true) => View::D2Array,
                            (Dim::D2, false) => View::D2,
                            (Dim::D3, false) => View::D3,
                            other => panic!("{name}: a {other:?} texture"),
                        }),
                        (AddressSpace::Handle, TypeInner::Sampler { comparison: false }) => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        (AddressSpace::Handle, TypeInner::AccelerationStructure { .. }) => ACCELERATION_STRUCTURE,
                        (space, _) => panic!("{name}: a {space:?} binding the layouts do not know"),
                    };
                    Some((b.group, b.binding, ty))
                })
                .collect();
            out.sort_by_key(|e| (e.0, e.1));
            out
        };
        let mut checked = 0;
        for (hwrt, merged, whole_tables) in [(true, false, false), (true, false, true), (false, false, false), (false, false, true), (false, true, false)] {
            for room in [false, true] {
                let mut layout: Vec<Entry> = kernel_layout_entries(LayoutShape { hwrt, merged, whole_tables, guard: room }).iter().enumerate().flat_map(|(g, es)| es.iter().map(move |e| (g as u32, e.binding, e.ty))).collect();
                layout.sort_by_key(|e| (e.0, e.1));
                for guard in [None, Some(1 << 16)] {
                    // (As written, a kernel with the guard needs the room.)
                    if guard.is_some() && !room && !merged {
                        continue;
                    }
                    for features in if hwrt { vec![0] } else { vec![0, F_UNITS] } {
                        let mut want = layout.clone();
                        if guard.is_none() && !merged {
                            want.retain(|e| (e.0, e.1) != (1, crate::merged::GUARD_BINDING));
                        }
                        let v = KernelVariant { merged, whole_tables, guard, ..KernelVariant::new(hwrt, KernelSwitches::default(), features) };
                        assert_eq!(declared(&v), want, "{v:?} in a layout {} room for the guard", if room { "with" } else { "without" });
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 2 * 3 + 2 * 6 + 8);
    }

    /// The native default has no loop guard, and the live passes are their file. Every variant the kernel's flags
    /// make is valid WGSL (naga, wgpu's front end): each traversal with no switch, each alone and all of them, by no
    /// scene feature, each alone and all of them (units are software traversal's), with the tables read whole, and
    /// with the loop guard. (The merged layout's: merged.rs.) Prints the default variants'
    /// SHA-256 (`--nocapture`).
    #[test]
    fn every_kernel_variant_is_valid() {
        use sha2::{Digest, Sha256};
        if std::env::var("THREERT_LOOP_GUARD").is_ok() {
            return;
        }
        assert_eq!(loop_guard(), None);
        let sha = |s: &str| Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(crate::live::live_source(), [RECORDS, include_str!("../shaders/live.wgsl")].concat());
        let template = kernel_template();
        let all = KernelSwitches { strict: true, cutout_lite: 2, last_direct: true, last_sun: false, light_pick: true };
        let switches = [
            KernelSwitches::default(),
            KernelSwitches { strict: true, ..Default::default() },
            KernelSwitches { cutout_lite: 1, ..Default::default() },
            KernelSwitches { cutout_lite: 2, ..Default::default() },
            KernelSwitches { last_direct: true, ..Default::default() },
            KernelSwitches { last_sun: true, ..Default::default() },
            KernelSwitches { light_pick: true, ..Default::default() },
            all,
        ];
        for hwrt in [false, true] {
            println!("source kernel hwrt={hwrt} {}", sha(&kernel_wgsl(&template, &KernelVariant::new(hwrt, KernelSwitches::default(), 0))));
            for &k in &switches {
                for f in [0, F_UNITS] {
                    naga_valid(&kernel_wgsl(&template, &KernelVariant::new(hwrt, k, if hwrt { f & !F_UNITS } else { f })));
                }
                let all_features = KernelVariant::new(hwrt, k, if hwrt { F_SOURCE & !F_UNITS } else { F_SOURCE });
                naga_valid(&kernel_wgsl(&template, &KernelVariant { whole_tables: true, ..all_features }));
                naga_valid(&kernel_wgsl(&template, &KernelVariant { guard: Some(1 << 16), ..all_features }));
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
        let v = KernelVariant { guard: loop_guard_of(true), ..KernelVariant::new(false, KernelSwitches::default(), F_UNITS) };
        assert_eq!((v.guard, loop_guard_of(false)), (Some(65536), None), "units told to have no guard: the source as written");
        // What a native session with units compiles with nothing set, to naga (wgpu's front end); the unit walk's loops
        // are bounded too (that every loop is: merged.rs `the_guard_bounds_every_loop`).
        let wgsl = kernel_wgsl(&kernel_template(), &v);
        assert!(wgsl.contains("const LOOP_GUARD_LIMIT = 65536u;") && wgsl.contains("var<storage, read_write> loop_guard_trips: array<atomic<u32>, GUARD_WORDS>;"), "the guard and its count");
        naga_valid(&wgsl);
        let walk = &wgsl[wgsl.find("fn trace_units(").expect("the unit walk")..];
        assert!(walk[..walk[1..].find("\nfn ").unwrap_or(walk.len() - 1)].contains("loop_guard()"));
        assert!(guard_room(false) && !guard_room(true));
    }

    /// Software traversal's units are in a kernel only when the scene has them (F_UNITS, `units`): without, there is
    /// no unit walk; with, `trace` calls `trace_units`, whose stack is the module's (the procedural objects' walk
    /// keeps its own). Hardware traversal has neither software walk.
    #[test]
    fn units_are_software_traversals_only() {
        let t = kernel_template();
        let sw = kernel_wgsl(&t, &KernelVariant::new(false, KernelSwitches::default(), 0));
        let u = kernel_wgsl(&t, &KernelVariant::new(false, KernelSwitches::default(), F_UNITS));
        let hw = kernel_wgsl(&t, &KernelVariant::new(true, KernelSwitches::default(), 0));
        assert!(!sw.contains("trace_units") && !hw.contains("trace_units") && !hw.contains("fn trace_procs("));
        assert_eq!((u.matches("fn trace_units(").count(), u.matches("hit = trace_units(").count()), (1, 1));
        let stacks = |s: &str| (s.matches("var<private> stack: array<u32, 64>").count(), s.matches("var stack: array<u32, 48>").count());
        assert_eq!((stacks(&sw), stacks(&u)), ((0, 1), (1, 1)));
    }

    /// With nothing set, every platform but Android opens the device as it was before the options existed: no
    /// pipeline cache, the kernel's source the files (`every_kernel_variant_is_valid` prints the default variants'
    /// hashes). Android's defaults are the tables read whole and the switches its GPUs needed; the environment
    /// overrides either way, and the merged layout never has the views.
    #[test]
    fn the_options_default_to_the_device_as_written() {
        let vars = |set: &'static [(&'static str, &'static str)]| move |name: &str| set.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
        let desktop = GpuOptions::from_vars(&vars(&[]), false);
        assert_eq!(desktop, GpuOptions::default());
        assert_eq!(desktop.kernel, KernelSwitches { strict: false, cutout_lite: 0, last_direct: false, last_sun: false, light_pick: false });
        assert!(desktop.table_views_on(false, false));
        for hwrt in [false, true] {
            assert!(KernelVariant::new(hwrt, desktop.kernel, 0).flags().iter().all(|&(name, on)| on == (name == "hwrt" && hwrt)));
        }
        let android = GpuOptions::from_vars(&vars(&[]), true);
        assert_eq!(android.kernel, KernelSwitches { strict: true, cutout_lite: 1, last_direct: true, last_sun: false, light_pick: false });
        // `sun`: the directional lights alone at the last vertex, in place of all of them, on either platform.
        let sun = GpuOptions::from_vars(&vars(&[("THREERT_LAST_DIRECT", "sun")]), true);
        assert_eq!((sun.kernel.last_direct, sun.kernel.last_sun), (false, true));
        assert_eq!((android.table_views, android.pipeline_cache.clone()), (None, None));
        // Android reads the tables whole; with the views asked for, the views.
        assert!(!android.table_views_on(false, true));
        let views = GpuOptions::from_vars(&vars(&[("THREERT_TABLE_VIEWS", "1")]), true);
        assert!(views.table_views_on(false, true) && !views.table_views_on(true, true));
        // Set: what is asked, on either platform.
        let off = GpuOptions::from_vars(&vars(&[("THREERT_KERNEL_STRICT", "0"), ("THREERT_CUTOUT_LITE", "0"), ("THREERT_LAST_DIRECT", "0")]), true);
        assert_eq!(off.kernel, KernelSwitches::default());
        let on = GpuOptions::from_vars(&vars(&[("THREERT_CUTOUT_LITE", "2"), ("THREERT_LIGHT_PICK", "1"), ("THREERT_PIPELINE_CACHE", "/data/cache/pipelines")]), false);
        // Elsewhere the views unless asked otherwise.
        assert!(on.table_views_on(false, false) && !on.table_views_on(true, false));
        let whole = GpuOptions::from_vars(&vars(&[("THREERT_TABLE_VIEWS", "0")]), false);
        assert!(!whole.table_views_on(false, false));
        assert_eq!(on.kernel, KernelSwitches { strict: false, cutout_lite: 2, last_direct: false, last_sun: false, light_pick: true });
        assert_eq!(on.pipeline_cache.as_deref(), Some(std::path::Path::new("/data/cache/pipelines")));
        assert_eq!(GpuOptions::from_vars(&vars(&[("THREERT_PIPELINE_CACHE", "")]), true).pipeline_cache, None);
        // The light tiles: everywhere but Android unless asked.
        assert_eq!([light_tiles_of(None, false), light_tiles_of(None, true), light_tiles_of(Some("1"), true), light_tiles_of(Some("0"), false)], [true, false, true, false]);
        // (And the environment map's light sampling, which is the scene's preparation's: lib.rs `env_nee_on`.)
        assert_eq!([crate::env_nee_of(None, false), crate::env_nee_of(None, true), crate::env_nee_of(Some("1"), true), crate::env_nee_of(Some("0"), false)], [true, false, true, false]);
    }

    /// The other shader files are WGSL as the specification writes it: wesl's parser is as strict as a browser's
    /// (Chrome's Tint) where naga is not. naga took `a * b ^ c`, which WGSL's grammar forbids (a bitwise operator's
    /// operands are unary expressions): the scene library's surface materials and both particle passes would not
    /// compile in a browser until 2026-11-14. (The kernel's files: `every_kernel_variant_is_valid`.)
    #[test]
    fn the_other_shaders_are_wgsl_as_the_specification_writes_it() {
        for src in [include_str!("../shaders/live.wgsl"), include_str!("../shaders/particles.wgsl"), include_str!("../shaders/gpu_particles.wgsl")] {
            wesl_wgsl(&[RECORDS, src].concat(), &[], &[]);
        }
        wesl_wgsl(include_str!("../shaders/grid_sim.wgsl"), &[], &[]);
        wesl_wgsl(include_str!("../shaders/skin.wgsl"), &[], &[]);
        wesl_wgsl(include_str!("../shaders/leaf_gather.wgsl"), &[], &[]);
    }

    /// The kernel is compiled with its loops bounded and nothing else checked, unless asked.
    #[test]
    fn the_kernels_loops_are_bounded() {
        let of = |v| { let c = kernel_checks_of(v); (c.force_loop_bounding, c.bounds_checks, c.ray_query_initialization_tracking) };
        assert_eq!([of(None), of(Some(0)), of(Some(1)), of(Some(2)), of(Some(12))], [(true, false, false), (false, false, false), (true, true, true), (false, true, false), (true, false, true)]);
    }

    /// The skinning pass is valid WGSL, and its block size and dispatch row are the ones the CPU lays its work out by.
    #[test]
    fn the_skinning_pass_is_valid_and_its_sizes_are_the_cpus() {
        let src = include_str!("../shaders/skin.wgsl");
        let module = naga::front::wgsl::parse_str(src).expect("skin.wgsl parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("skin.wgsl validates");
        assert!(src.contains(&format!("@compute @workgroup_size({SKIN_BLOCK})")) && src.contains(&format!("const ROW: u32 = {SKIN_ROW}u;")));
        assert_eq!((std::mem::size_of::<SkinJob>(), std::mem::size_of::<crate::skin::SkinCorner>(), std::mem::size_of::<crate::skin::PaletteRows>()), (16, 64, 48));
        // (The pass that takes the leaves again after it: the same blocks and rows.)
        let src = include_str!("../shaders/leaf_gather.wgsl");
        let module = naga::front::wgsl::parse_str(src).expect("leaf_gather.wgsl parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("leaf_gather.wgsl validates");
        assert!(src.contains(&format!("@compute @workgroup_size({SKIN_BLOCK})")) && src.contains(&format!("const ROW: u32 = {SKIN_ROW}u;")));
        assert_eq!(std::mem::size_of::<LeafJob>(), 16);
    }

    /// Each switch changes what it says, under both traversals (that every combination is valid WGSL:
    /// `every_kernel_variant_is_valid`). Strict guards every ray query the hardware kernel makes; software traversal
    /// has none and takes only `env_pdf`'s index; the cutout switches are hardware traversal's.
    #[test]
    fn the_kernel_switches_change_what_they_say() {
        let t = kernel_template();
        let v = |hwrt: bool, k: KernelSwitches| kernel_wgsl(&t, &KernelVariant::new(hwrt, k, 0));
        for hwrt in [false, true] {
            let plain = v(hwrt, KernelSwitches::default());
            let strict = v(hwrt, KernelSwitches { strict: true, ..Default::default() });
            let queries = strict.matches("rayQueryInitialize(").count();
            assert_eq!(queries, if hwrt { 2 } else { 0 });
            assert_eq!(strict.matches("ray_bad(o, d, tmax").count(), queries, "every ray query is guarded");
            assert_eq!(strict.matches("fn ray_bad(").count(), hwrt as usize);
            assert!(!strict.contains("env_cdf_at(y - 1u)") && !strict.contains("env_cdf_at(base + x - 1u)") && plain.contains("env_cdf_at(y - 1u)"));
            let lit = v(hwrt, KernelSwitches { strict: true, cutout_lite: 2, last_direct: true, last_sun: false, light_pick: true });
            assert_eq!(lit.matches("radiance += dz;").count(), 1);
            assert!(!lit.contains("fn sun_light("), "last_direct takes the directional lights already");
            let sun = v(hwrt, KernelSwitches { last_sun: true, ..Default::default() });
            assert_eq!((sun.matches("radiance += dz;").count(), sun.matches("fn sun_light(").count(), sun.matches("sun_light(s, lobes(").count()), (1, 1, 1));
            assert!(!plain.contains("sun_light"));
            assert_eq!(lit.matches("env_on = take_env && has_env && !lights_only;").count(), 1);
            assert_eq!(lit.contains("let steps = 4u;"), hwrt);
            assert_eq!(v(hwrt, KernelSwitches { cutout_lite: 2, ..Default::default() }) == plain, !hwrt);
        }
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

    /// A scene's features other than units are `FEAT` bits, not source: the kernel's text is the same for each, and
    /// what they switch is fixed when a pipeline sets `FEAT` (naga's override processing; `OBJECT_STRIDE` from F_TINT).
    #[test]
    fn scene_features_are_feat_bits() {
        let t = kernel_template();
        let v = |f: u32| kernel_wgsl(&t, &KernelVariant::new(false, KernelSwitches::default(), f));
        let base = v(0);
        for f in [F_TINT, F_UNLIT, F_BACKGROUND, F_VCOLOR, F_GRID] {
            assert_eq!(v(f), base, "{f:#x}");
        }
        assert_ne!(v(F_UNITS), base);
        let m = naga::front::wgsl::parse_str(&base).unwrap();
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap();
        for (feat, stride) in [(0, 3.0), (F_TINT, 4.0)] {
            let c = naga::back::PipelineConstants::from([("LIVE".to_string(), 0.0), ("FEAT".to_string(), feat as f64)]);
            let (m, _) = naga::back::pipeline_constants::process_overrides(&m, &info, None, &c).unwrap();
            let (_, k) = m.constants.iter().find(|(_, k)| k.name.as_deref() == Some("OBJECT_STRIDE")).expect("OBJECT_STRIDE");
            assert_eq!(m.global_expressions[k.init], naga::Expression::Literal(naga::Literal::U32(stride as u32)));
        }
    }
}

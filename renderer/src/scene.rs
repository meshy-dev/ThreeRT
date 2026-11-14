//! The engine's world (world.rs) + asset files -> flat GPU-ready scene (world-space triangles, materials, textures,
//! lights).

use crate::desc::{self, Light, Sky};
use crate::skin;
use crate::world::{self, Kind, World};
use anyhow::{anyhow, Context, Result};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec3A};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub const MAT_DOUBLE_SIDED: i32 = 1;
pub const MAT_EMITTER_CARD: i32 = 2;
/// three.js MeshBasicMaterial (`desc::Material::unlit`; the kernel's F_UNLIT, `@if(unlit)`: gpu.rs `KernelVariant::flags`).
pub const MAT_UNLIT: i32 = 4;
pub const TRI_NO_SHADOW: u32 = 1;
/// Alpha-tested triangle (cutout material): traversal runs an any-hit test.
pub const TRI_MASKED: u32 = 2;
/// Triangle of a posed model (animated character): kept in its own acceleration structures, re-skinned in place.
pub const TRI_DYNAMIC: u32 = 4;
/// vec4s per object in the objects table (pathtrace.wgsl `OBJECT_STRIDE`): (pivot, power), (bounds size, time of the
/// last power edit), per-instance data (`custom`); in a scene with instance colours (`Scene::tinted`, the kernel's
/// F_TINT) a 4th, (colour, -): an instanced object's instance colour (`InstanceTable`), white otherwise. Other scenes
/// keep the 3 and the kernel as they were.
pub fn object_stride(tinted: bool) -> usize {
    if tinted {
        4
    } else {
        3
    }
}

/// An object-table entry, its colour last (a table holds its first `object_stride` vec4s).
pub type ObjectRow = [[f32; 4]; 4];

/// An object-table entry: pivot (world position), power, world bounds size, per-instance data, colour (the time of the
/// last power edit starts long ago).
pub fn object_row(pivot: Vec3, power: f32, size: impl Into<Vec3>, custom: [f32; 4], color: [f32; 3]) -> ObjectRow {
    let size: Vec3 = size.into();
    [[pivot.x, pivot.y, pivot.z, power.max(0.0)], [size.x, size.y, size.z, -1.0e9], custom, [color[0], color[1], color[2], 0.0]]
}

/// A glTF mesh placed by several static instances (hardware RT): its triangles once, in model space, in the `pool_*`
/// arrays; one BLAS, one TLAS instance per placement.
#[derive(Clone, Copy, Debug)]
pub struct InstMesh {
    pub first: u32,
    pub count: u32,
    /// The first `opaque` pool triangles are opaque (BLAS geometry 0), the rest cutout (geometry 1, any-hit test).
    pub opaque: u32,
    /// Triangles each placement flattens instead (glass, emissive, material functions), in the build's rest arrays.
    pub rest_first: u32,
    pub rest_count: u32,
    /// Model-space bounds (object bounds of its instances, picking).
    pub lo: Vec3A,
    pub hi: Vec3A,
    /// Its triangles change (a shared pose, `PoseGroup`): its BLAS is rebuilt with them.
    pub deforms: bool,
    /// An emitting `own_blas` mesh (`emitter_mesh_tris`): its triangles in world space, its one placement at identity,
    /// so they are light-sampled as flattened ones are (`prepare` gives them emitter entries).
    pub emits: bool,
}

/// Pool triangles of the deforming meshes, which the build puts first in the pool.
pub fn deforming_pool(meshes: &[InstMesh]) -> usize {
    meshes.iter().filter(|m| m.deforms).map(|m| m.count as usize).sum()
}

impl InstMesh {
    /// World bounds of a placement: the box of the transformed model-space box.
    pub fn placed_bounds(&self, m: Mat4) -> (Vec3A, Vec3A) {
        let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
        for c in 0..8 {
            let p = Vec3A::new(if c & 1 == 0 { self.lo.x } else { self.hi.x }, if c & 2 == 0 { self.lo.y } else { self.hi.y }, if c & 4 == 0 { self.lo.z } else { self.hi.z });
            let q = m.transform_point3a(p);
            lo = lo.min(q);
            hi = hi.max(q);
        }
        (lo, hi)
    }
}

/// One placement of an `InstMesh`.
#[derive(Clone, Debug)]
pub struct Instance {
    pub mesh: u32,
    pub world: Mat4,
    /// Object-table index (pivot, bounds, power).
    pub object: u32,
    pub cast_shadow: bool,
    /// The scene node's id (picking, selection outlines).
    pub name: String,
    /// An instanced object's instance (`Kind::Instanced`): its index there (picking reports it), and whether it is
    /// hidden (`instance_world`: TLAS mask 0, never flattened or picked).
    pub sub: Option<u32>,
    pub hidden: bool,
}

/// Kernel record of an instance (`instances` binding): world matrix rows, normal matrix rows (inverse transpose),
/// (first triangle in the GPU order, object index, texture-LOD offset bits, first cutout triangle - first), and the
/// world matrix rows of the previous frame (a movable instance's motion vectors; the same rows when it did not move).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuInstance {
    pub m: [[f32; 4]; 3],
    pub n: [[f32; 4]; 3],
    pub info: [u32; 4],
    pub p: [[f32; 4]; 3],
}

impl Instance {
    /// Kernel record with its mesh's first triangle at `first` in the GPU order; `opaque` = where its cutout
    /// geometry starts.
    pub fn gpu(&self, first: u32, opaque: u32) -> GpuInstance {
        let w = self.world;
        let nm = glam::Mat3::from_mat4(w).inverse().transpose();
        let row = |r: usize| [w.x_axis[r], w.y_axis[r], w.z_axis[r], w.w_axis[r]];
        let nrow = |r: usize| [nm.x_axis[r], nm.y_axis[r], nm.z_axis[r], 0.0];
        GpuInstance { m: [row(0), row(1), row(2)], n: [nrow(0), nrow(1), nrow(2)], info: [first, self.object, lod_offset(w).to_bits(), opaque], p: [row(0), row(1), row(2)] }
    }
    /// Model-space triangle `t` of its mesh in world space (the same transform the flattening path applies).
    pub fn tri(&self, t: &[Vec3A; 3]) -> [Vec3A; 3] {
        t.map(|v| self.world.transform_point3a(v))
    }
}

/// Procedural geometry placed in the scene (`Kind::Procedural`): an instance of its own AABB BLAS (hardware RT), its
/// kernel record after the model instances' in `instances` (`gpu`), its header and materials in the procedural region
/// (`ProcLayout`), its boxes and data in that region's blob of its geometry (shared by every object of the geometry).
#[derive(Clone, Debug)]
pub struct ProcObject {
    pub node: u32,
    pub name: String,
    pub world: Mat4,
    /// Object-table index (pivot, bounds, power, customData).
    pub object: u32,
    /// Index of its geometry function (`Scene::geometry_fns`).
    pub func: u32,
    /// Its geometry's boxes and data (inline or read from an asset file).
    pub blob: Arc<crate::procasset::ProcBlob>,
    /// GPU material indices, in the order the geometry function's `o.material` picks them.
    pub materials: Vec<u32>,
    pub cast_shadow: bool,
    /// Object-space bounds of its boxes.
    pub lo: Vec3,
    pub hi: Vec3,
    /// An instanced object's instance, as `Instance::sub` / `hidden` (a hidden one is a point: software traversal
    /// draws nothing either).
    pub sub: Option<u32>,
    pub hidden: bool,
}

/// Words of a procedural object's header (pathtrace.wgsl `proc_vec`, `proc_box`, `trace_procs`): box count, first box
/// word, first data word, material count, bounds min xyz (f32 bits), first material word, bounds max xyz, data length
/// (what the geometry function's `gdata` may read), first BVH node word and first leaf-order word (software traversal;
/// 0 otherwise), -, -.
pub const PROC_HEADER: usize = 16;

/// Procedural object `id` (a `Kind::Procedural`) as the build places it at world matrix `m` with object-table index
/// `object`, or None when its geometry has no boxes (nothing drawn). The build and the live view's in-place edits
/// (live/edits.rs `edit_geometry`) both make it here. Its geometry function's index is its place among the world's geometry
/// functions (the kernel's dispatch order).
pub fn proc_object(w: &World, id: u32, m: Mat4, object: u32, materials: &HashMap<u32, u32>, assets: &Path) -> Result<Option<ProcObject>> {
    let n = &w.objects[&id];
    let (geometry, mats) = match &n.kind {
        Kind::Procedural { geometry, materials } | Kind::Instanced { geometry, procedural: true, materials, .. } => (geometry, materials),
        _ => return Err(anyhow!("object {}: not procedural geometry", n.name)),
    };
    let g = w.proc_geometries.get(geometry).ok_or_else(|| anyhow!("object {}: unknown procedural geometry {geometry}", n.name))?;
    let blob = match &g.source {
        world::ProcSource::Inline(b) => b.clone(),
        world::ProcSource::Asset(uri) => crate::procasset::load(uri, assets).with_context(|| format!("object {}", n.name))?,
    };
    let func = w.geometry_fns.keys().position(|k| *k == g.function).ok_or_else(|| anyhow!("object {}: unknown geometry function {}", n.name, g.function))? as u32;
    if mats.is_empty() {
        return Err(anyhow!("object {}: a procedural mesh needs at least one material", n.name));
    }
    let materials = mats.iter().map(|k| materials.get(k).copied().ok_or_else(|| anyhow!("object {}: unknown material {k}", n.name))).collect::<Result<Vec<u32>>>()?;
    if blob.boxes.is_empty() {
        return Ok(None);
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for b in blob.boxes.chunks(6) {
        lo = lo.min(Vec3::new(b[0], b[1], b[2]));
        hi = hi.max(Vec3::new(b[3], b[4], b[5]));
    }
    Ok(Some(ProcObject { node: id, name: n.name.clone(), world: m, object, func, blob, materials, cast_shadow: n.cast_shadow, lo, hi, sub: None, hidden: false }))
}

/// A triangle as the hardware traversal's acceleration structures take it: as it is, or, with a vertex that is not
/// finite, three vertices at the origin. DXR and Vulkan leave a non-finite vertex undefined behaviour (only NaN in x
/// is defined, as an inactive primitive), and a degenerate triangle is one they define as never hit; the triangle
/// keeps its index. World positions are finite where they enter (world.rs: geometry checked, `world_matrix` and
/// `instance_world` place what is not at a point): this is for what overflows on the way (docs/RUNTIME.md "What the
/// kernel trusts"). Software traversal's tree leaves such a triangle out by itself (bvh.rs `build_tris`).
pub fn blas_tri(t: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    if t.iter().flatten().all(|v| v.is_finite()) {
        t
    } else {
        [[0.0; 3]; 3]
    }
}

/// Instance `i` of an instanced object at node matrix `m`: its world matrix and whether it is hidden: past the drawn
/// `count`, or a matrix that is not finite or not invertible (three.js code hides instances by scaling them to 0). A
/// hidden instance is placed at a point (scale 1e-6, at its translation when finite), so it costs the trace nothing,
/// keeps the TLAS free of non-finite numbers, and can come back in place.
pub fn instance_world(m: Mat4, t: &world::InstanceTable, i: usize) -> (Mat4, bool) {
    let w = m * t.matrix(i);
    let det = glam::Mat3::from_mat4(w).determinant();
    if i < t.count as usize && w.is_finite() && det.abs() > 1e-30 {
        return (w, false);
    }
    let at = if w.w_axis.is_finite() { w.w_axis.truncate() } else { m.w_axis.truncate() };
    (Mat4::from_scale_rotation_translation(Vec3::splat(1e-6), glam::Quat::IDENTITY, at), true)
}

impl ProcObject {
    /// Its object-table entry: pivot and power, world bounds size, per-instance data, colour (white).
    pub fn object_entry(&self, n: &world::Object) -> ObjectRow {
        self.entry(n.power, n.custom, [1.0; 3])
    }

    /// Its object-table entry with this power, per-instance data and colour.
    pub fn entry(&self, power: f32, custom: [f32; 4], color: [f32; 3]) -> ObjectRow {
        let (wlo, whi) = self.world_bounds();
        object_row(self.world.transform_point3(Vec3::ZERO), power, whi - wlo, custom, color)
    }

    /// Its header (`PROC_HEADER` words) and material indices, for a header at region word `header` (materials right
    /// after it) and its blob's boxes from region word `boxes` (data after them); `bvh`: the software traversal's node
    /// and leaf-order words (0 under hardware RT).
    pub fn header(&self, header: usize, boxes: usize, bvh: (usize, usize)) -> Vec<u32> {
        let b = |v: f32| v.to_bits();
        let (p, mats, data) = (self, header + PROC_HEADER, boxes + self.blob.boxes.len());
        let mut h = vec![(p.blob.boxes.len() / 6) as u32, boxes as u32, data as u32, p.materials.len() as u32, b(p.lo.x), b(p.lo.y), b(p.lo.z), mats as u32, b(p.hi.x), b(p.hi.y), b(p.hi.z), p.blob.data.len() as u32, bvh.0 as u32, bvh.1 as u32, 0, 0];
        h.extend_from_slice(&p.materials);
        h
    }

    /// Kernel record (pathtrace.wgsl `Instance`): world rows, normal rows (inverse transpose), (header word in the
    /// procedural region, object, geometry function, casts shadows), the previous frame's world rows (as m until it
    /// moves).
    pub fn gpu(&self, header: u32) -> GpuInstance {
        let w = self.world;
        let nm = glam::Mat3::from_mat4(w).inverse().transpose();
        let row = |r: usize| [w.x_axis[r], w.y_axis[r], w.z_axis[r], w.w_axis[r]];
        let nrow = |r: usize| [nm.x_axis[r], nm.y_axis[r], nm.z_axis[r], 0.0];
        GpuInstance { m: [row(0), row(1), row(2)], n: [nrow(0), nrow(1), nrow(2)], info: [header, self.object, self.func, self.cast_shadow as u32], p: [row(0), row(1), row(2)] }
    }

    /// World bounds: the box of the transformed object-space bounds.
    pub fn world_bounds(&self) -> (Vec3, Vec3) {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for c in 0..8 {
            let p = Vec3::new(if c & 1 == 0 { self.lo.x } else { self.hi.x }, if c & 2 == 0 { self.lo.y } else { self.hi.y }, if c & 4 == 0 { self.lo.z } else { self.hi.z });
            let q = self.world.transform_point3(p);
            lo = lo.min(q);
            hi = hi.max(q);
        }
        (lo, hi)
    }
}

/// The kernel's procedural region (pathtrace.wgsl `pword`, `gdata`; after the environment CDF in its binding,
/// `Params::procs`): every object's header (`PROC_HEADER`) and materials (`head`), then one blob per distinct geometry
/// (its boxes as f32 bits, then its data), each geometry once however many objects use it. Words are counted from the
/// region's start. The blobs' layout depends only on the geometries' hashes in order (`key`), so a rebuild with the same
/// geometries copies them on the GPU from the last upload instead of sending them again (gpu.rs `ProcCache`).
pub struct ProcLayout {
    pub head: Vec<u32>,
    /// Each object's header word.
    pub headers: Vec<u32>,
    /// Distinct geometries in order and their first word counted from the blobs' start.
    pub blobs: Vec<(Arc<crate::procasset::ProcBlob>, usize)>,
    /// Each object's geometry: its index in `blobs` (one BLAS per geometry, hardware RT).
    pub blob_of: Vec<usize>,
    pub blob_words: usize,
    /// The blobs carry the software traversal's BVHs.
    pub sw: bool,
}

impl ProcLayout {
    pub fn key(&self) -> Vec<[u8; 32]> {
        self.blobs.iter().map(|(b, _)| b.hash).collect()
    }
}

/// `sw`: software traversal (each blob also carries its BVH, `ProcBlob::sw_bvh`).
pub fn proc_layout(procs: &[ProcObject], sw: bool) -> ProcLayout {
    let mut blobs: Vec<(Arc<crate::procasset::ProcBlob>, usize)> = vec![];
    let mut at: HashMap<[u8; 32], usize> = HashMap::new();
    let mut blob_words = 0;
    let mut blob_of = Vec::with_capacity(procs.len());
    for p in procs {
        let k = *at.entry(p.blob.hash).or_insert_with(|| {
            blobs.push((p.blob.clone(), blob_words));
            blob_words += p.blob.words(sw);
            blobs.len() - 1
        });
        blob_of.push(k);
    }
    let head_len: usize = procs.iter().map(|p| PROC_HEADER + p.materials.len()).sum();
    let mut head = Vec::with_capacity(head_len);
    let mut headers = Vec::with_capacity(procs.len());
    for (p, &k) in procs.iter().zip(&blob_of) {
        let h = head.len();
        headers.push(h as u32);
        let boxes = head_len + blobs[k].1;
        let data = boxes + p.blob.boxes.len();
        let bvh = if sw { (data + p.blob.data.len(), data + p.blob.data.len() + p.blob.sw_bvh().0.len()) } else { (0, 0) };
        head.extend(p.header(h, boxes, bvh));
    }
    ProcLayout { head, headers, blobs, blob_of, blob_words, sw }
}

/// Flags of an instance's pool triangle as a world-space triangle (software traversal): the placement's shadow flag and
/// the material's cutout (the build's flag pass ran before the instances were flattened).
pub fn instance_tri_flags(inst: &Instance, a: &GpuTriAttr, materials: &[GpuMaterial]) -> u32 {
    let shadow = if inst.cast_shadow { 0 } else { TRI_NO_SHADOW };
    let cutout = materials.get(a.uv[3].to_bits() as usize).is_some_and(|m| m.uv2[3] > 0.0);
    shadow | if cutout { TRI_MASKED } else { 0 }
}

/// Triangle ranges of node `id` in build order: one, or two for an instanced node flattened by software traversal (its
/// glass / emissive part from the build, its mesh appended by `prepare`).
pub fn node_ranges<'a>(tri_nodes: &'a [(String, u32, u32)], id: &'a str) -> impl Iterator<Item = std::ops::Range<usize>> + 'a {
    tri_nodes.iter().filter(move |(n, _, _)| n == id).map(|(_, a, b)| *a as usize..*b as usize)
}

/// Texture-LOD constant change of a transform: texel density per metre scales with 1 / linear size, approximated by
/// the cube root of the volume scale (exact for uniform scale).
pub fn lod_offset(w: Mat4) -> f32 {
    -glam::Mat3::from_mat4(w).determinant().abs().max(1e-30).log2() / 3.0
}

/// Model-space shading attributes of a pool triangle placed by `w` (normals by the inverse transpose, tangent by the
/// matrix, texture-LOD constant by the scale), object `object`.
pub fn instance_attr(a: &GpuTriAttr, w: Mat4, object: u32) -> GpuTriAttr {
    let m3 = glam::Mat3::from_mat4(w);
    let nm = m3.inverse().transpose();
    let n = |v: [f32; 4]| {
        let r = (nm * Vec3::new(v[0], v[1], v[2])).normalize_or_zero();
        [r.x, r.y, r.z, v[3]]
    };
    let t = (m3 * Vec3::new(a.t[0], a.t[1], a.t[2])).normalize_or_zero();
    GpuTriAttr { n0: n(a.n0), n1: n(a.n1), n2: n(a.n2), uv: a.uv, t: [t.x, t.y, t.z, a.t[3]], x: [a.x[0] + lod_offset(w), f32::from_bits(object), a.x[2], a.x[3]] }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTriAttr {
    pub n0: [f32; 4], // normal0, w = uv0.x
    pub n1: [f32; 4], // normal1, w = uv0.y
    pub n2: [f32; 4], // normal2, w = uv1.x
    pub uv: [f32; 4], // uv1.y, uv2.x, uv2.y, material (bits)
    pub t: [f32; 4],  // tangent (dP/du), w = bitangent sign
    pub x: [f32; 4],  // texture LOD constant 0.5*log2(uv area / world area), object index (bits), -, -
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMaterial {
    pub base_color: [f32; 4], // rgb, opacity
    pub emissive: [f32; 4],   // rgb nits
    pub params: [f32; 4],     // metallic, roughness, specular, normal scale
    pub tex0: [i32; 4],       // base, normal, roughness(G), metalness(B)
    pub tex1: [i32; 4],       // emissive, occlusion(R), flags, material function + 1 (0 = none)
    pub uv: [f32; 4],         // uv repeat xy, transmission texture (R) + 1 (0 = none; glTF), -
    pub uv2: [f32; 4],        // uv offset xy, -, alpha cutoff (0 = opaque)
    pub layer: [f32; 4],      // transmission (glass), index of refraction, clearcoat, clearcoat roughness
    pub mp: [[f32; 4]; 8],    // material function parameters
    pub mt: [i32; 8],         // material function texture slots (-1 none)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuLight {
    pub pos: [f32; 4],   // xyz (rect: centre), w = type (0 directional, 1 point, 2 spot, 3 rect)
    pub dir: [f32; 4],   // emission direction, w = cos(half angle) directional / radius point & spot / half height rect
    pub color: [f32; 4], // directional: lux rgb; point/spot: candela rgb; rect: luminance (nits) rgb; w = range
    pub ext: [f32; 4],   // spot: cos outer, 1/(cos inner - cos outer); rect: width axis x half width; w = cast shadow
    pub x: [f32; 4],     // volumetric scattering scale, specular scale, rect: cos(barn door angle), -
}

pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: std::sync::Arc<image::RgbaImage>,
    pub srgb: bool,
    /// Identity of the source (file version or glTF image): GPU texture pages are reused while the set is unchanged.
    pub key: String,
    /// Repeat (true) or clamp outside 0..1.
    pub wrap: bool,
}

pub struct CameraData {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub right: Vec3,
    pub tan_half_fov: f32,
    pub exposure: desc::Exposure,
    pub dof: Option<desc::Dof>,
}

pub enum SkyData {
    None,
    Color(Vec3),
    /// `dynamic`: baked at `sky::DYNAMIC_SKY` (the live view re-bakes it as the sun moves); `glow`: night-sky radiance.
    Atmosphere { sun_dir: Vec3, sun_lux: Vec3, rayleigh: f32, mie: f32, g: f32, ground: Vec3, sun_node: Option<usize>, dynamic: bool, glow: crate::sky::Glow },
    Gradient(crate::sky::Gradient),
}

/// The atmosphere's sun as the scene sees it now: the direction toward it (Y up) and its illuminance above the
/// atmosphere (colour × intensity, lux), from the sun light's world transform. None when the sky has no sun.
pub fn atmosphere_sun(w: &World) -> Option<(u32, Vec3, Vec3)> {
    let Sky::Atmosphere { sun: Some(s), .. } = &w.environment.sky else { return None };
    let o = w.objects.get(s)?;
    let Kind::Light(Light::Directional { color, intensity, .. }) = &o.kind else { return None };
    let m = w.world_matrix(*s);
    Some((*s, -m.transform_vector3(Vec3::NEG_Z).normalize(), Vec3::from(*color) * *intensity))
}

pub struct Scene {
    pub name: String,
    /// The light tree keeps a slot for emitters added in place (`ltree::build_slotted`): set by the live view for a
    /// scene whose geometry it can edit (gpu.rs `EditRoom::editable`); offline renders build without it.
    pub emitter_slot: bool,
    /// Per object (with geometry): (pivot (world position), power), (world bounds size, time of the last power
    /// edit); indexed by GpuTriAttr.x[1].
    pub objects: Vec<[f32; 4]>,
    /// Material functions (id, WGSL code, helpers) in index order (GpuMaterial.tex1[3] - 1).
    pub shaders: Vec<(String, String, String)>,
    /// Scene clock for stills, s.
    pub time: f32,
    pub width: u32,
    pub height: u32,
    pub tris: Vec<[Vec3A; 3]>,
    pub tri_flags: Vec<u32>,
    /// Vertex colours (`tri_colors`) of the triangles in `tris` and `pool_tris` whose material has `vertexColors`, by
    /// index (shorter: white past the end); `prepare` lays them out as the GPU's triangles (F_VCOLOR).
    pub vcolors: Vec<[u32; 3]>,
    pub pool_vcolors: Vec<[u32; 3]>,
    /// The vertex colours as the GPU reads them (`prepare`): three words per triangle in GPU order, none in a scene
    /// without them (gpu.rs F_VCOLOR).
    pub gpu_vcolors: Vec<u32>,
    /// Where they start in `prev_verts` (vec4s: the previous vertices of the dynamic tail come first; `Params::procs.w`).
    pub vcolor_base: u32,
    pub attrs: Vec<GpuTriAttr>,
    pub materials: Vec<GpuMaterial>,
    pub textures: Vec<Texture>,
    pub lights: Vec<GpuLight>,
    pub camera: CameraData,
    pub sky: SkyData,
    pub fog: Option<desc::Fog>,
    /// Global wetness and rain intensity (0..1).
    pub wetness: f32,
    pub rain: f32,
    /// Cloud cover (0..1) of an atmosphere sky.
    pub cloud_cover: f32,
    pub preset: desc::Preset,
    pub stats: HashMap<&'static str, f64>,
    /// Triangle ranges per object (name, first, end), in `tris` order: picking and selection outlines.
    pub tri_nodes: Vec<(String, u32, u32)>,
    /// Triangle ranges of flattened instances of instanced objects (first, end, instance index): picking.
    pub tri_instances: Vec<(u32, u32, u32)>,
    /// Some object has instance colours (`InstanceTable::colors`): the kernel's F_TINT, 4 vec4s an object
    /// (`object_stride`).
    pub tinted: bool,
    /// three.js `scene.background` (`desc::Environment::background`): the kernel's F_BACKGROUND, `Params::background`.
    pub background: Option<Vec3>,
    /// The sky's aerial perspective (`aerial_params`): the kernel's F_AERIAL, `Params::aerial`.
    pub aerial: [f32; 4],
    /// Instanced static glTF meshes (model-space triangles, flags, attributes) and their placements. `prepare` appends
    /// the pool after the flattened triangles (hardware RT) or flattens the instances (software traversal).
    pub pool_tris: Vec<[Vec3A; 3]>,
    pub pool_attrs: Vec<GpuTriAttr>,
    pub meshes: Vec<InstMesh>,
    pub instances: Vec<Instance>,
    /// Kernel records of `instances` (`prepare`, hardware RT).
    pub gpu_instances: Vec<GpuInstance>,
    /// Where engine handles landed in the GPU tables, for edits without a rebuild (`LiveCore::commit`).
    pub retained: Retained,
    /// Particle systems (dynamic triangles a GPU pass places every frame) and the particle functions they run:
    /// (name, WGSL code, helpers).
    pub particles: Vec<ParticleSystem>,
    pub particle_fns: Vec<(String, String, String)>,
    /// Stateful GPU particles (`Kind::GpuParticles`; gpu.rs `GpuParticlesGpu`) and their programs (name, emit, update,
    /// helpers); the voxel occupancy grid they collide with.
    pub gpu_particles: Vec<GpuParticleSystem>,
    pub particle_programs: Vec<(String, String, String, String)>,
    /// The game's particle spawns when the scene was built (setup's), for an offline render's first step.
    pub particle_spawns: std::collections::BTreeMap<u32, Vec<f32>>,
    pub voxel_occupancy: Option<std::sync::Arc<std::sync::RwLock<world::Occupancy>>>,
    /// Participating media (`VolumeMesh` nodes; pathtrace.wgsl `Medium`) and the volume functions they run.
    pub media: Vec<MediumGpu>,
    /// Each medium's node (live pose / customData edits).
    pub media_nodes: Vec<u32>,
    pub volume_fns: Vec<(String, String, String)>,
    /// The density grids the media read (`Kind::Volume::grid`; gpu.rs `GridsGpu`), their programs (name, WGSL code,
    /// helpers), and the sources the game gave when the scene was built (setup's), for an offline render's first step.
    pub grids: Vec<GridBuild>,
    pub grid_programs: Vec<(String, String, String)>,
    pub grid_sources: std::collections::BTreeMap<u32, Vec<f32>>,
    /// Procedural geometry (`Kind::Procedural`) and the geometry functions it runs: (name, WGSL code, helpers).
    pub procs: Vec<ProcObject>,
    pub geometry_fns: Vec<(String, String, String)>,
    /// The kernel's procedural region and records (`prepare`: `proc_layout`, `ProcObject::gpu`); the region follows the
    /// environment's CDF in the `env_cdf` binding (`Params::procs`), the records the model instances' in `instances`.
    pub proc_layout: Option<ProcLayout>,
    pub proc_records: Vec<GpuInstance>,
    /// Software traversal's units (`prepare`: the instances it keeps as instances; units.rs), None under hardware
    /// traversal and for a scene without any.
    pub units: Option<crate::units::Units>,
    /// Translucent particles' triangles (`transparent` material): in the kernel's triangle tables after everything
    /// else (`trans_first`, set by `prepare`), in no traversal group: only the translucency pass sees them.
    pub trans_tris: Vec<[Vec3A; 3]>,
    pub trans_attrs: Vec<GpuTriAttr>,
    pub trans_first: u32,
}

impl Scene {
    /// vec4s an object in `objects` (`object_stride`).
    pub fn object_stride(&self) -> usize {
        object_stride(self.tinted)
    }
}

/// Engine handle -> GPU table index maps of a built scene. Parameter edits (power, light and material parameters,
/// environment) rewrite single table entries through them instead of rebuilding.
#[derive(Default, Clone)]
pub struct Retained {
    /// Material -> `materials` index.
    pub materials: HashMap<u32, u32>,
    /// (texture, sRGB requested) -> texture slot, as resolved at build.
    pub textures: HashMap<(u32, bool), i32>,
    /// Material function name -> index (GpuMaterial.tex1[3] - 1).
    pub shaders: HashMap<String, i32>,
    /// Object -> object-table index (objects that draw triangles: meshes, models, rect-light cards).
    pub objects: HashMap<u32, u32>,
    /// Object of each `lights` entry, same order (`prepare` sorts both).
    pub light_nodes: Vec<u32>,
    /// Rect-light object with a visible card -> its card material.
    pub cards: HashMap<u32, u32>,
    /// Atmosphere sun: its colour comes from the sky bake.
    pub sun: Option<u32>,
    /// Posed models and movable objects (dynamic triangles) -> what re-transforming them in place needs.
    pub dynamic: HashMap<u32, DynamicModel>,
    /// Movable models placed as instances (hardware RT) -> their instance: a move updates its transform, not triangles.
    pub instance_of: HashMap<u32, u32>,
    /// Posed rigid models split by node (`RigidModel`): a new pose moves their nodes' instances.
    pub rigid: HashMap<u32, RigidModel>,
    /// Posed models sharing a pose (`PoseGroup`), and each member's group (members are in `instance_of` too).
    pub pose_groups: Vec<PoseGroup>,
    pub pose_member: HashMap<u32, u32>,
    /// `own_blas` meshes placed as instances (hardware RT): object -> its instance; (geometry, GPU material) -> the
    /// shared mesh (objects with the same geometry and material share its BLAS). live/edits.rs `edit_geometry` keeps both.
    pub own_instance: HashMap<u32, u32>,
    pub own_mesh: HashMap<(u32, u32), u32>,
    /// Emitting `own_blas` meshes (`InstMesh::emits`): object -> its mesh (its own, in world space).
    pub emitter_mesh: HashMap<u32, u32>,
    /// Instanced objects (`Kind::Instanced`) placed as instances -> where (live/edits.rs `update_instanced`).
    pub instanced: HashMap<u32, InstancedPlace>,
    /// Static models' instanced meshes by (uri, nodes), those with nothing flattened per placement: a model added in
    /// place is another instance of one (live/edits.rs `edit_geometry`).
    pub model_mesh: HashMap<(String, Option<Vec<String>>), u32>,
}

/// Where an instanced object's instances are: `capacity` of them from `first` in `instances` (triangles) or `procs`
/// (procedural), with object-table entries from `object`, one each; the first `drawn` were last written with their
/// matrices (the rest are hidden, and an update leaves them be).
#[derive(Clone, Copy, Debug)]
pub struct InstancedPlace {
    pub first: u32,
    pub capacity: u32,
    pub object: u32,
    pub procedural: bool,
    pub drawn: u32,
}

/// A posed glTF model without skins whose clip moves some of its nodes (Zero Day's 551 moving objects): the nodes the
/// clip leaves alone are ordinary static triangles; each moving mesh node is an instance of its own mesh (its triangles
/// in the node's space, one BLAS built once), placed at object x node transform, so a new pose moves instances and
/// rebuilds the TLAS only. Their glass / emissive / function triangles are flattened (shadow rays, emitter sampling):
/// `rest`, re-transformed in place from `rest_local`.
#[derive(Clone, Default)]
pub struct RigidModel {
    /// (glTF node, instance index).
    pub instances: Vec<(usize, u32)>,
    pub rest: DynamicModel,
    /// Per `rest` triangle: its node and its node-space triangle (positions, normals, uvs, tangent).
    pub rest_local: Vec<(usize, [Vec3; 3], [Vec3; 3], [Vec2; 3], Option<(Vec3, f32)>)>,
}

/// Posed skinned models in the same pose (the same model, nodes, clip and time: a crowd whose game shares a few walk
/// phases). One model-space mesh holds the pose, skinned once per new pose and its BLAS rebuilt; each model is an
/// instance of it, moved by its transform (hardware RT). Only meshes without triangles flattened per placement.
#[derive(Clone, Debug)]
pub struct PoseGroup {
    pub mesh: u32,
    pub uri: String,
    pub nodes: Option<Vec<String>>,
    /// The pose the mesh holds.
    pub animation: desc::Animation,
    /// Per pool triangle of the mesh, in order: its index in the model's triangles and its material.
    pub order: Vec<(u32, u32)>,
    /// (object, instance) of each member.
    pub members: Vec<(u32, u32)>,
}

/// What groups posed models into a `PoseGroup`: model, nodes, clip, time (bits), loop.
type PoseKey = (String, Option<Vec<String>>, String, u32, bool);

/// pathtrace.wgsl `Medium`: world -> local affine rows (the unit box / sphere's space), (function, shape or
/// `MEDIUM_HIDDEN`, homogeneous, animated),
/// (majorant 1/m, HG anisotropy, -, -), the node's customData, 32 parameter floats.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MediumGpu {
    pub w: [[f32; 4]; 3],
    pub a: [u32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub p: [f32; 32],
}

/// A density grid as built: its cells, where it lies in the grid atlas (from layer `z0`: the grids are stacked along
/// z), its data (field A, four numbers a cell; None: zeros), its program and parameters, the volume node and medium
/// that own it (the program runs in that node's box).
#[derive(Clone, Debug)]
pub struct GridBuild {
    pub id: u32,
    pub dims: [u32; 3],
    pub z0: u32,
    pub data: Option<std::sync::Arc<Vec<f32>>>,
    pub program: Option<u32>,
    pub params: [f32; 32],
    pub node: u32,
    pub medium: u32,
    /// `world::DensityGrid::made`.
    pub made: u32,
}

/// The world -> local rows of a medium whose node has world matrix `m` (pathtrace.wgsl `Medium.w0..w2`).
pub fn medium_rows(m: glam::Mat4) -> [[f32; 4]; 3] {
    let inv = m.inverse();
    [inv.row(0).to_array(), inv.row(1).to_array(), inv.row(2).to_array()]
}

/// `count` particles, each a quad (two triangles from `first`, build order; `final_first` in the GPU order after
/// `prepare`, hardware RT) and its own object (from `object`: pivot, bounds, per-particle data), placed every frame by
/// particle function `func` with `params` (8 vec4).
#[derive(Clone, Debug)]
pub struct ParticleSystem {
    /// Drawn by the translucency pass: `first` / `final_first` index the translucent triangles (`Scene::trans_tris`,
    /// from `trans_first` on the GPU).
    pub translucent: bool,
    pub first: u32,
    pub final_first: u32,
    pub count: u32,
    pub object: u32,
    pub func: u32,
    pub params: [f32; 32],
    /// Triangle flags (kept in the kernel's vertex data).
    pub flags: u32,
    /// Seed of the particles' fixed random numbers (the node).
    pub seed: u32,
    /// The node's customData (the particle function's `i.custom`).
    pub custom: [f32; 4],
    /// Hidden by its own `visible` (`World::hidden_built`): in the build, its pass idle until it is shown.
    pub hidden: bool,
}

/// A `GpuParticles` node as built: its quads (`first` in build order, `final_first` in the GPU's), its objects (and
/// states) from `object`, its program and parameters, emission and collision.
#[derive(Clone, Debug)]
pub struct GpuParticleSystem {
    pub node: u32,
    pub first: u32,
    pub final_first: u32,
    pub capacity: u32,
    pub object: u32,
    pub program: u32,
    pub params: [f32; 32],
    pub seed: u32,
    pub pivot: [f32; 3],
    pub rate: f32,
    pub collide: world::ParticleCollide,
}

/// A dynamic object's triangles (posed model, or movable mesh / model / rect-light card): their range in build order,
/// and per triangle the material and flags.
#[derive(Clone, Default)]
pub struct DynamicModel {
    pub first: u32,
    pub mats: Vec<u32>,
    pub flags: Vec<u32>,
    pub object: u32,
    /// Index of each triangle in the GPU triangle order (`prepare`; empty without hardware RT).
    pub finals: Vec<u32>,
}

struct TexCache {
    textures: Vec<Texture>,
    by_key: HashMap<(String, bool), i32>,
}

impl TexCache {
    fn add(&mut self, key: (String, bool), wrap: bool, img: impl FnOnce() -> Result<std::sync::Arc<image::RgbaImage>>) -> Result<i32> {
        if let Some(i) = self.by_key.get(&key) {
            return Ok(*i);
        }
        let img = img()?;
        let id = self.textures.len() as i32;
        self.textures.push(Texture { width: img.width(), height: img.height(), rgba: img, srgb: key.1, key: format!("{}|{}|{}", key.0, key.1, wrap), wrap });
        self.by_key.insert(key, id);
        Ok(id)
    }
}

/// Identity of a file version: (path, length, mtime). Assets and textures rarely change, so they are parsed /
/// decoded once per process.
pub(crate) fn file_key(path: &Path) -> String {
    let (len, mtime) = crate::files::version(path);
    format!("{}|{len}|{mtime}", path.display())
}

/// Process-wide parse / decode caches (bounded: cleared when they grow past a working set).
fn cached_gltf(key: &str, load: impl FnOnce() -> Result<skin::Model>) -> Result<Arc<skin::Model>> {
    static CACHE: std::sync::Mutex<Option<HashMap<String, Arc<skin::Model>>>> = std::sync::Mutex::new(None);
    let key = key.to_string();
    if let Some(g) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
        return Ok(g.clone());
    }
    let g = Arc::new(load()?);
    let mut c = CACHE.lock().unwrap();
    let c = c.get_or_insert_with(HashMap::new);
    if c.len() > 2048 {
        c.clear();
    }
    c.insert(key, g.clone());
    Ok(g)
}

fn cached_image(key: &str, load: impl FnOnce() -> Result<image::RgbaImage>) -> Result<Arc<image::RgbaImage>> {
    static CACHE: std::sync::Mutex<Option<HashMap<String, Arc<image::RgbaImage>>>> = std::sync::Mutex::new(None);
    let key = key.to_string();
    if let Some(i) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
        return Ok(i.clone());
    }
    let img = Arc::new(load()?);
    let mut c = CACHE.lock().unwrap();
    let c = c.get_or_insert_with(HashMap::new);
    // Byte budget, not a count: a street scene references ~100 maps and structural edits rebuild the scene.
    let bytes: usize = c.values().map(|i| i.as_raw().len()).sum();
    if bytes + img.as_raw().len() > 1536 << 20 {
        c.clear();
    }
    c.insert(key, img.clone());
    Ok(img)
}

/// Cache identity of generated texture bytes: their content. An address is not an identity: once a scene is dropped
/// (a render node's next job, a rebuild), the next scene's bytes can land at the same address with the same length
/// (every 1 x 1 default texture is 4 bytes), and the cache then served the old image, e.g. a white puddle mask for a
/// black one (std/wet-ground-dry failed one full test run in five).
fn bytes_key(kind: &str, bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{kind}:{:016x}|{}", h.finish(), bytes.len())
}

/// Decoded texture image + its cache identity (files: path version; generated: the bytes' content).
fn texture_image(t: &world::Texture, assets: &Path) -> Result<(String, Arc<image::RgbaImage>)> {
    match &t.source {
        world::TextureSource::File(uri) => {
            let path = assets.join(uri);
            let key = file_key(&path);
            Ok((key.clone(), cached_image(&key, || crate::files::image(&path).with_context(|| format!("texture {uri}")))?))
        }
        world::TextureSource::Encoded(bytes) => {
            let key = bytes_key("encoded", bytes);
            Ok((key.clone(), cached_image(&key, || Ok(image::load_from_memory(bytes).context("generated texture")?.to_rgba8()))?))
        }
        world::TextureSource::Pixels { width, height, rgba } => {
            let key = bytes_key(&format!("pixels{width}x{height}"), rgba);
            Ok((key.clone(), cached_image(&key, || image::RgbaImage::from_raw(*width, *height, rgba.to_vec()).context("generated texture: size does not match the pixels"))?))
        }
    }
}

/// A model file (cached per file version).
pub fn model_gltf(uri: &str, assets: &Path) -> Result<Arc<skin::Model>> {
    let path = assets.join(uri);
    cached_gltf(&file_key(&path), || skin::Model::new(import_gltf(&path).with_context(|| format!("asset {uri}"))?))
}

/// `gltf::import` with the images decoded in parallel: the crate decodes them one after another (Bistro's 405 JPEG /
/// PNG textures took ~2.5 s of its first frame).
fn import_gltf(path: &Path) -> gltf::Result<skin::Gltf> {
    use rayon::prelude::*;
    let base = path.parent().unwrap_or_else(|| Path::new("./"));
    let gltf::Gltf { document, blob } = crate::files::gltf(path)?;
    let buffers = gltf::import_buffers(&document, Some(base), blob)?;
    let images = document.images().collect::<Vec<_>>().into_par_iter().map(|i| gltf::image::Data::from_source(i.source(), Some(base), &buffers)).collect::<gltf::Result<Vec<_>>>()?;
    Ok((document, buffers, images))
}

/// GPU material of a material descriptor. `tex(texture, sRGB requested)` resolves texture slots: decoding them at
/// build, looking up the slots recorded then for parameter edits.
pub fn build_material(id: u32, m: &desc::Material, textures: &HashMap<u32, world::Texture>, shaders: &HashMap<String, i32>, tex: &mut dyn FnMut(u32, bool) -> Result<i32>) -> Result<GpuMaterial> {
    let mut slot = |s: &Option<u32>, srgb: bool| -> Result<i32> {
        match s {
            Some(tid) => tex(*tid, srgb),
            None => Ok(-1),
        }
    };
    let first_tex = [m.map, m.normal_map, m.roughness_map, m.emissive_map].into_iter().flatten().find_map(|t| textures.get(&t));
    let repeat = first_tex.map(|t| t.repeat).unwrap_or([1.0, 1.0]);
    let offset = first_tex.map(|t| t.offset).unwrap_or([0.0, 0.0]);
    let mut gm = GpuMaterial {
        base_color: [m.color[0], m.color[1], m.color[2], m.opacity],
        emissive: [m.emissive[0] * m.emissive_intensity, m.emissive[1] * m.emissive_intensity, m.emissive[2] * m.emissive_intensity, 0.0],
        params: [m.metalness, m.roughness, 0.5 * m.specular_intensity.unwrap_or(1.0), m.normal_scale.map(|s| s[0]).unwrap_or(1.0)],
        tex0: [slot(&m.map, true)?, slot(&m.normal_map, false)?, slot(&m.roughness_map, false)?, slot(&m.metalness_map, false)?],
        tex1: [slot(&m.emissive_map, true)?, slot(&m.ao_map, false)?, if m.unlit { MAT_UNLIT } else if m.double_sided { MAT_DOUBLE_SIDED } else { 0 }, 0],
        uv: [repeat[0], repeat[1], slot(&m.transmission_map, false)? as f32 + 1.0, 0.0],
        uv2: [offset[0], offset[1], 0.0, m.alpha_test.unwrap_or(0.0)],
        layer: [m.transmission.unwrap_or(0.0).clamp(0.0, 1.0), m.ior.unwrap_or(1.5).max(1.0), m.clearcoat.unwrap_or(0.0).clamp(0.0, 1.0), m.clearcoat_roughness.unwrap_or(0.0).clamp(0.0, 1.0)],
        mp: [[0.0; 4]; 8],
        mt: [-1; 8],
    };
    if let Some(sh) = &m.shader {
        apply_shader(&mut gm, sh, shaders, tex).with_context(|| format!("material {id}"))?;
    }
    Ok(gm)
}

/// A material function's binding on a GPU material: the function, its 32 parameters and its textures (`tex(texture,
/// sRGB requested)`).
pub fn apply_shader(gm: &mut GpuMaterial, sh: &desc::ShaderRef, shaders: &HashMap<String, i32>, tex: &mut dyn FnMut(u32, bool) -> Result<i32>) -> Result<()> {
    let f = *shaders.get(sh.id.as_str()).ok_or_else(|| anyhow!("unknown material function {}", sh.id))?;
    gm.tex1[3] = f + 1;
    for (k, v) in sh.params.iter().take(32).enumerate() {
        gm.mp[k / 4][k % 4] = *v;
    }
    for (k, t) in sh.textures.iter().take(8).enumerate() {
        if let Some(tid) = t {
            gm.mt[k] = tex(*tid, false)?;
        }
    }
    Ok(())
}

/// Analytic light of a light object with world matrix `m` (rect lights with a visible card are emitter geometry
/// instead, see `card_material`). Emission scales with the node's power.
/// `gpu_light` of a light that may be hidden (`shown`: `World::shown`): hidden, its entry stays where it is in the
/// table, black (the kernel skips black lights, and they are in no group's sphere: lib.rs `lights_with_groups`).
pub fn gpu_light_shown(l: &Light, m: Mat4, power: f32, shown: bool) -> Option<GpuLight> {
    let mut gl = gpu_light(l, m, power)?;
    if !shown {
        gl.color = [0.0, 0.0, 0.0, gl.color[3]];
    }
    Some(gl)
}

pub fn gpu_light(l: &Light, m: Mat4, power: f32) -> Option<GpuLight> {
    let pos = m.transform_point3(Vec3::ZERO);
    let dir = m.transform_vector3(Vec3::NEG_Z).normalize();
    Some(match l {
        Light::Directional { color, intensity, angular_diameter, cast_shadow, volumetric_scattering, specular_scale } => {
            let half = (angular_diameter * 0.5).to_radians();
            let intensity = intensity * power;
            GpuLight {
                pos: [pos.x, pos.y, pos.z, 0.0],
                // The cone's cosine stays below 1 (the largest f32 under it): a sun under ~0.04 degrees across had
                // 1 - cos = 0 in f32, an infinite cone pdf and NaN light, so it lit nothing.
                dir: [dir.x, dir.y, dir.z, half.cos().min(1.0 - f32::EPSILON / 2.0)],
                color: [color[0] * intensity, color[1] * intensity, color[2] * intensity, 0.0],
                ext: [0.0, 0.0, 0.0, *cast_shadow as u32 as f32],
                x: [*volumetric_scattering, *specular_scale, 0.0, 0.0],
            }
        }
        Light::Point { color, intensity, radius, range, cast_shadow, volumetric_scattering, specular_scale } => {
            let intensity = intensity * power;
            GpuLight {
                pos: [pos.x, pos.y, pos.z, 1.0],
                dir: [dir.x, dir.y, dir.z, *radius],
                color: [color[0] * intensity, color[1] * intensity, color[2] * intensity, *range],
                ext: [0.0, 0.0, 0.0, *cast_shadow as u32 as f32],
                x: [*volumetric_scattering, *specular_scale, 0.0, 0.0],
            }
        }
        Light::Spot { color, intensity, radius, range, angle, penumbra, cast_shadow, volumetric_scattering, specular_scale } => {
            let intensity = intensity * power;
            // UE: inner = outer * (1 - penumbra); falloff = saturate((cos - cosOuter) / (cosInner - cosOuter))^2
            let cos_outer = angle.cos();
            let cos_inner = (angle * (1.0 - penumbra)).cos();
            GpuLight {
                pos: [pos.x, pos.y, pos.z, 2.0],
                dir: [dir.x, dir.y, dir.z, *radius],
                color: [color[0] * intensity, color[1] * intensity, color[2] * intensity, *range],
                ext: [cos_outer, 1.0 / (cos_inner - cos_outer).max(1e-4), 0.0, *cast_shadow as u32 as f32],
                x: [*volumetric_scattering, *specular_scale, 0.0, 0.0],
            }
        }
        Light::Rect { visible: true, .. } => return None,
        Light::Rect { color, intensity, width, height, range, barn_door_angle, cast_shadow, volumetric_scattering, specular_scale, visible: false } => {
            // Analytic one-sided Lambertian rectangle (UE RectLight): lighting only, not visible to camera rays.
            let ux = m.transform_vector3(Vec3::X).normalize() * (width * 0.5);
            let half_h = height * 0.5;
            let k = intensity * power;
            GpuLight {
                pos: [pos.x, pos.y, pos.z, 3.0],
                dir: [dir.x, dir.y, dir.z, half_h],
                color: [color[0] * k, color[1] * k, color[2] * k, *range],
                ext: [ux.x, ux.y, ux.z, *cast_shadow as u32 as f32],
                x: [*volumetric_scattering, *specular_scale, barn_door_angle.clamp(0.0, 90.0).to_radians().cos(), 0.0],
            }
        }
    })
}

/// Emitter-card material of a visible rect light (the node's power scales it through its object, like meshes).
pub fn card_material(color: [f32; 3], intensity: f32) -> GpuMaterial {
    GpuMaterial {
        base_color: [0.0, 0.0, 0.0, 1.0],
        emissive: [color[0] * intensity, color[1] * intensity, color[2] * intensity, 0.0],
        params: [0.0, 1.0, 0.0, 1.0],
        tex0: [-1; 4],
        tex1: [-1, -1, MAT_EMITTER_CARD, 0],
        uv: [1.0, 1.0, 0.0, 0.0],
        ..Default::default()
    }
}

/// Builds the GPU scene from the engine's world. Asset files (models, file textures) resolve against `assets`.
/// The most volumes a scene may show at once: the kernel loops over at most 32 media with a 32-bit mask
/// (pathtrace.wgsl `media_track`, `media_march`, `media_shadows`). A scene past it is refused, not drawn without some.
pub const MAX_MEDIA: usize = 32;
/// A medium's shape (`MediumGpu::a[1]`: 0 box, 1 sphere) while its node is hidden: none, no ray crosses it
/// (pathtrace.wgsl `medium_span`).
pub const MEDIUM_HIDDEN: u32 = 2;

pub fn build(w: &World, assets: &Path, presets_dir: &Path, overrides: Option<&serde_json::Value>) -> Result<Scene> {
    let t0 = web_time::Instant::now();
    let st = &w.settings;
    let mut preset_json: serde_json::Value = serde_json::from_slice(&crate::files::preset(presets_dir, &st.preset).with_context(|| format!("preset {}", st.preset))?)?;
    if let Some(o) = &st.overrides {
        desc::merge_json(&mut preset_json, o);
    }
    if let Some(o) = overrides {
        desc::merge_json(&mut preset_json, o);
    }
    let preset: desc::Preset = serde_json::from_value(preset_json)?;
    let order = w.ordered();
    let mut worlds: HashMap<u32, Mat4> = HashMap::new();
    for id in &order {
        let o = &w.objects[id];
        let m = match o.parent.and_then(|p| worlds.get(&p)) {
            Some(pm) => *pm * o.local(),
            None => o.local(),
        };
        worlds.insert(*id, m);
    }

    let mut retained = Retained::default();
    let mut texcache = TexCache { textures: vec![], by_key: HashMap::new() };
    let mut materials: Vec<GpuMaterial> = vec![];
    let shaders: Vec<(String, String, String)> = w.shaders.iter().map(|(k, (c, h))| (k.clone(), c.clone(), h.clone())).collect();
    retained.shaders = shaders.iter().enumerate().map(|(i, (k, _, _))| (k.clone(), i as i32)).collect();
    let mut ids: Vec<&u32> = w.materials.keys().collect();
    ids.sort();
    // Decoding dominates the build (a 3 x 3 district: ~260 texture files and 53 models with ~220 embedded images, 2 s
    // one after another), so it runs up front in parallel: the texture files the materials use (a dry run of the
    // material loop lists them) and the model files of the objects shown. The loops below find them decoded.
    let mut used: Vec<u32> = vec![];
    for id in &ids {
        build_material(**id, &w.materials[*id], &w.textures, &retained.shaders, &mut |tid, _| {
            used.push(tid);
            Ok(0)
        })?;
    }
    used.sort_unstable();
    used.dedup();
    let models: Vec<&str> = {
        let mut m: Vec<&str> = order.iter().filter_map(|id| match &w.objects[id].kind {
            Kind::Model { uri, .. } if w.shown(*id) => Some(uri.as_str()),
            _ => None,
        }).collect();
        m.sort_unstable();
        m.dedup();
        m
    };
    let (decoded, model_images) = rayon::join(
        || used.par_iter().filter_map(|tid| w.textures.get(tid).map(|t| Ok((*tid, texture_image(t, assets)?)))).collect::<Result<HashMap<u32, (String, Arc<image::RgbaImage>)>>>(),
        || -> Result<HashMap<String, Arc<image::RgbaImage>>> {
            // The images the models' materials use, as RGBA too (gltf_material), keyed as gltf_material keys them.
            let glbs = models.par_iter().map(|uri| Ok((*uri, model_gltf(uri, assets)?))).collect::<Result<Vec<_>>>()?;
            let mut images: Vec<(String, &gltf::image::Data)> = vec![];
            for (uri, m) in &glbs {
                let (doc, _, data) = &m.gltf;
                let mut used: Vec<usize> = doc
                    .materials()
                    .flat_map(|gm| {
                        let pbr = gm.pbr_metallic_roughness();
                        [pbr.base_color_texture().map(|t| t.texture()), pbr.metallic_roughness_texture().map(|t| t.texture()), gm.normal_texture().map(|t| t.texture()), gm.emissive_texture().map(|t| t.texture()), gm.occlusion_texture().map(|t| t.texture())]
                    })
                    .flatten()
                    .map(|t| t.source().index())
                    .collect();
                used.sort_unstable();
                used.dedup();
                images.extend(used.into_iter().filter_map(|i| data.get(i).map(|d| (format!("{uri}#img{i}"), d))));
            }
            images.into_par_iter().map(|(k, d)| Ok((k, Arc::new(to_rgba(d)?)))).collect()
        },
    );
    let (decoded, model_images) = (decoded?, model_images?);
    for id in ids {
        let mut tex = |tid: u32, srgb_default: bool| -> Result<i32> {
            let t = w.textures.get(&tid).ok_or_else(|| anyhow!("unknown texture {tid}"))?;
            let srgb = t.srgb && srgb_default;
            let (key, img) = match decoded.get(&tid) {
                Some(d) => d.clone(),
                None => texture_image(t, assets)?,
            };
            let slot = texcache.add((key, srgb), t.wrap, || Ok(img))?;
            retained.textures.insert((tid, srgb_default), slot);
            Ok(slot)
        };
        let gm = build_material(*id, &w.materials[id], &w.textures, &retained.shaders, &mut tex)?;
        retained.materials.insert(*id, materials.len() as u32);
        materials.push(gm);
    }

    let mut tris: Vec<[Vec3A; 3]> = vec![];
    let mut tri_flags: Vec<u32> = vec![];
    let mut attrs: Vec<GpuTriAttr> = vec![];
    let mut lights: Vec<GpuLight> = vec![];
    let mut camera: Option<CameraData> = None;
    let mut asset_materials: HashMap<(String, usize), u32> = HashMap::new();
    let mut tri_nodes: Vec<(String, u32, u32)> = vec![];
    let mut particles: Vec<ParticleSystem> = vec![];
    let mut gpu_particles: Vec<GpuParticleSystem> = vec![];
    let particle_programs: Vec<(String, String, String, String)> = w.particle_programs.iter().map(|(k, (e, u, h))| (k.clone(), e.clone(), u.clone(), h.clone())).collect();
    let (mut trans_tris, mut trans_attrs): (Vec<[Vec3A; 3]>, Vec<GpuTriAttr>) = (vec![], vec![]);
    let particle_fns: Vec<(String, String, String)> = w.particle_fns.iter().map(|(k, (c, h))| (k.clone(), c.clone(), h.clone())).collect();
    let volume_fns: Vec<(String, String, String)> = w.volume_fns.iter().map(|(k, (c, h))| (k.clone(), c.clone(), h.clone())).collect();
    let geometry_fns: Vec<(String, String, String)> = w.geometry_fns.iter().map(|(k, (c, h))| (k.clone(), c.clone(), h.clone())).collect();
    let mut procs: Vec<ProcObject> = vec![];
    let mut media: Vec<MediumGpu> = vec![];
    let mut media_nodes: Vec<u32> = vec![];
    // Volumes shown that the loop below has yet to place: hidden ones take what the kernel has left after them.
    let mut media_to_show = w.objects.iter().filter(|(id, o)| matches!(o.kind, Kind::Volume { .. }) && w.shown(**id)).count();
    let mut grids: Vec<GridBuild> = vec![];
    let grid_programs: Vec<(String, String, String)> = w.grid_programs.iter().map(|(k, (c, h))| (k.clone(), c.clone(), h.clone())).collect();
    let mut objects: Vec<[f32; 4]> = vec![];
    // Static glTF models drawn more than once share their triangles (model space, one BLAS): opaque, shadow-casting
    // per node, non-emissive materials without functions, not mirrored. THREERT_INSTANCING=0 flattens everything.
    let instancing = std::env::var("THREERT_INSTANCING").map_or(true, |v| v != "0");
    let mut pool_tris: Vec<[Vec3A; 3]> = vec![];
    let (mut vcolors, mut pool_vcolors): (Vec<[u32; 3]>, Vec<[u32; 3]>) = (vec![], vec![]);
    // Whether a mesh's triangles carry its geometry's vertex colours: any of its materials asks for them.
    let colored = |g: &world::Geometry, mats: &mut dyn Iterator<Item = &u32>| !g.colors.is_empty() && mats.filter(|k| w.materials.get(k).is_some_and(|m| m.vertex_colors)).count() > 0;
    let mut pool_attrs: Vec<GpuTriAttr> = vec![];
    // Model-space triangles of instanced meshes that each placement flattens (glass, emissive, functions).
    let mut rest_tris: Vec<[Vec3A; 3]> = vec![];
    let mut rest_attrs: Vec<GpuTriAttr> = vec![];
    let mut meshes: Vec<InstMesh> = vec![];
    let mut instances: Vec<Instance> = vec![];
    let mut mesh_of: HashMap<(String, Option<Vec<String>>), Option<u32>> = HashMap::new();
    // Posed skinned models that share a pose with another share its mesh (`PoseGroup`). THREERT_POSE_SHARE=0 skins
    // each model alone (A/B).
    let mut pose_key_of: HashMap<u32, PoseKey> = HashMap::new();
    if instancing && std::env::var("THREERT_POSE_SHARE").map_or(true, |v| v != "0") {
        let mut count: HashMap<PoseKey, usize> = HashMap::new();
        for id in &order {
            let Kind::Model { uri, nodes, animation: Some(a), functions } = &w.objects[id].kind else { continue };
            if !w.shown(*id) || worlds[id].determinant() <= 0.0 {
                continue;
            }
            // (Rigid models whose nodes move are `RigidModel`s.)
            let glb = model_gltf(uri, assets)?;
            if !glb.animated() || (nodes.is_none() && glb.rigid()) {
                continue;
            }
            // (Copies with material functions share a pose only with the same functions: their own materials.)
            let file = if functions.is_empty() { uri.clone() } else { format!("{uri}\u{0}{functions:?}") };
            let key: PoseKey = (file, nodes.clone(), a.clip.clone(), a.time.to_bits(), a.r#loop);
            *count.entry(key.clone()).or_default() += 1;
            pose_key_of.insert(*id, key);
        }
        pose_key_of.retain(|_, k| count[k] >= 2);
    }
    let mut pose_group_of: HashMap<PoseKey, Option<u32>> = HashMap::new();
    // Instanced objects' shared meshes by (geometry, GPU material); their flattened instances' triangle ranges.
    let mut inst_mesh_of: HashMap<(u32, u32), u32> = HashMap::new();
    let mut tri_instances: Vec<(u32, u32, u32)> = vec![];
    // Instance colours anywhere: object entries of 4 vec4s, and the kernel's F_TINT (gpu.rs `tinted_source`).
    let tinted = w.instanced.iter().any(|(id, t)| t.colors.is_some() && w.shown(*id) && matches!(w.objects.get(id).map(|o| &o.kind), Some(Kind::Instanced { .. })));
    let stride = object_stride(tinted);
    for id in &order {
        let n = &w.objects[id];
        let m = worlds[id];
        let first = tris.len() as u32;
        let shown = w.shown(*id);
        let flags = if n.cast_shadow { 0 } else { TRI_NO_SHADOW };
        match &n.kind {
            Kind::Mesh { geometry, material, own_blas, materials: mesh_mats } if shown && instancing && mesh_mats.len() <= 1 && m.determinant() > 0.0 && (n.driven || (*own_blas && !w.movable(*id))) => {
                // Its own BLAS (scene code's `dynamicGeometry`, or a mesh a rigid body drives: its moves are the
                // instance's transform, `retained.instance_of`): placed like a model instance, its geometry's triangles
                // in model space in the pool, shared by the objects of the same geometry and material. Glass, emissive
                // and function materials are flattened (as a model's would be), so they are never instanced.
                let g = w.geometries.get(geometry).ok_or_else(|| anyhow!("object {}: unknown geometry {geometry}", n.name))?;
                let mat = *retained.materials.get(material).ok_or_else(|| anyhow!("object {}: unknown material {material}", n.name))?;
                let class = material_class(&materials[mat as usize]);
                let vc = colored(g, &mut std::iter::once(material));
                if class < 2 {
                    let mesh = match retained.own_mesh.get(&(*geometry, mat)) {
                        Some(&k) => Some(k),
                        None => {
                            let (t, a) = own_mesh_tris(g, mat);
                            (!t.is_empty()).then(|| {
                                meshes.push(own_inst_mesh(&t, pool_tris.len() as u32, class));
                                put_colors(&mut pool_vcolors, pool_tris.len(), vc.then(|| tri_colors(g, false, None)));
                                pool_tris.extend(t);
                                pool_attrs.extend(a);
                                let k = meshes.len() as u32 - 1;
                                retained.own_mesh.insert((*geometry, mat), k);
                                k
                            })
                        }
                    };
                    if let Some(mesh) = mesh {
                        let (lo, hi) = meshes[mesh as usize].placed_bounds(m);
                        let pivot = m.transform_point3(Vec3::ZERO);
                        let size = hi - lo;
                        let oi = (objects.len() / stride) as u32;
                        objects.extend_from_slice(&object_row(pivot, n.power, size, n.custom, [1.0; 3])[..stride]);
                        retained.objects.insert(*id, oi);
                        retained.own_instance.insert(*id, instances.len() as u32);
                        if n.driven {
                            retained.instance_of.insert(*id, instances.len() as u32);
                        }
                        instances.push(Instance { mesh, world: m, object: oi, cast_shadow: n.cast_shadow, name: n.name.clone(), sub: None, hidden: false });
                    }
                    continue;
                }
                // (A driven emitter's triangles cannot stay in world space: it is flattened, dynamic, below.)
                if emits_only(&materials[mat as usize]) && !n.driven {
                    // An emitting material: its own BLAS too, but its triangles in world space (placed at identity,
                    // never shared), so they are light-sampled as flattened triangles are and can be edited in place
                    // (live/edits.rs `edit_geometry`; ROADMAP § 6 step 3).
                    let oi = (objects.len() / stride) as u32;
                    let (t, a) = emitter_mesh_tris(g, m, mat, oi);
                    if !t.is_empty() {
                        let cutout = materials[mat as usize].uv2[3] > 0.0;
                        let mut im = own_inst_mesh(&t, pool_tris.len() as u32, u32::from(cutout));
                        im.emits = true;
                        meshes.push(im);
                        put_colors(&mut pool_vcolors, pool_tris.len(), vc.then(|| tri_colors(g, false, None)));
                        pool_tris.extend(t);
                        pool_attrs.extend(a);
                        let mesh = meshes.len() as u32 - 1;
                        objects.extend_from_slice(&object_row(m.transform_point3(Vec3::ZERO), n.power, im.hi - im.lo, n.custom, [1.0; 3])[..stride]);
                        retained.objects.insert(*id, oi);
                        retained.own_instance.insert(*id, instances.len() as u32);
                        retained.emitter_mesh.insert(*id, mesh);
                        instances.push(Instance { mesh, world: Mat4::IDENTITY, object: oi, cast_shadow: n.cast_shadow, name: n.name.clone(), sub: None, hidden: false });
                    }
                    continue;
                }
                put_colors(&mut vcolors, tris.len(), vc.then(|| tri_colors(g, false, None)));
                // (A driven mesh that cannot be an instance (glass, emissive, a material function): dynamic triangles,
                // re-transformed in place as any movable mesh's.)
                append_arrays(g, m, mat, if n.driven { flags | TRI_DYNAMIC } else { flags }, None, &mut tris, &mut tri_flags, &mut attrs);
                if n.driven {
                    retained.dynamic.insert(*id, DynamicModel { first, mats: attrs[first as usize..].iter().map(|a| a.uv[3].to_bits()).collect(), ..Default::default() });
                }
            }
            Kind::Instanced { geometry, procedural, materials: mats, capacity } if shown => {
                // Copies of one geometry: one shared mesh (one BLAS), an instance and an object entry per copy, the
                // copies' records and entries contiguous (live/edits.rs `update_instanced` rewrites them in place).
                // Glass, emissive and function materials and mirrored copies are flattened instead, each copy its
                // own triangles, as `own_blas` meshes are; changing their instances rebuilds.
                let empty = world::InstanceTable::default();
                let t = w.instanced.get(id).unwrap_or(&empty);
                let placed: Vec<(Mat4, bool)> = (0..*capacity as usize).map(|i| instance_world(m, t, i)).collect();
                let entry = |i: usize, wm: Mat4, size: Vec3| object_row(wm.transform_point3(Vec3::ZERO), n.power, size, t.custom(i, n.custom), t.color(i));
                if *procedural {
                    let object = (objects.len() / stride) as u32;
                    if let Some(p) = proc_object(w, *id, m, object, &retained.materials, assets)? {
                        retained.instanced.insert(*id, InstancedPlace { first: procs.len() as u32, capacity: *capacity, object, procedural: true, drawn: t.count });
                        for (i, &(wm, hidden)) in placed.iter().enumerate() {
                            let q = ProcObject { world: wm, object: object + i as u32, sub: Some(i as u32), hidden, ..p.clone() };
                            objects.extend_from_slice(&q.entry(n.power, t.custom(i, n.custom), t.color(i))[..stride]);
                            procs.push(q);
                        }
                    }
                    continue;
                }
                let g = w.geometries.get(geometry).ok_or_else(|| anyhow!("object {}: unknown geometry {geometry}", n.name))?;
                let material = mats.first().ok_or_else(|| anyhow!("object {}: an instanced mesh needs a material", n.name))?;
                let mat = *retained.materials.get(material).ok_or_else(|| anyhow!("object {}: unknown material {material}", n.name))?;
                let class = material_class(&materials[mat as usize]);
                let vc = colored(g, &mut std::iter::once(material));
                let mirrored = placed.iter().any(|(wm, hidden)| !hidden && wm.determinant() < 0.0);
                if instancing && class < 2 && !mirrored {
                    let mesh = match inst_mesh_of.get(&(*geometry, mat)) {
                        Some(&k) => Some(k),
                        None => {
                            let (tr, a) = own_mesh_tris(g, mat);
                            (!tr.is_empty()).then(|| {
                                meshes.push(own_inst_mesh(&tr, pool_tris.len() as u32, class));
                                put_colors(&mut pool_vcolors, pool_tris.len(), vc.then(|| tri_colors(g, false, None)));
                                pool_tris.extend(tr);
                                pool_attrs.extend(a);
                                let k = meshes.len() as u32 - 1;
                                inst_mesh_of.insert((*geometry, mat), k);
                                k
                            })
                        }
                    };
                    if let Some(mesh) = mesh {
                        let object = (objects.len() / stride) as u32;
                        retained.instanced.insert(*id, InstancedPlace { first: instances.len() as u32, capacity: *capacity, object, procedural: false, drawn: t.count });
                        for (i, &(wm, hidden)) in placed.iter().enumerate() {
                            let (lo, hi) = meshes[mesh as usize].placed_bounds(wm);
                            objects.extend_from_slice(&entry(i, wm, Vec3::from(hi - lo))[..stride]);
                            instances.push(Instance { mesh, world: wm, object: object + i as u32, cast_shadow: n.cast_shadow, name: n.name.clone(), sub: Some(i as u32), hidden });
                        }
                    }
                    continue;
                }
                for (i, &(wm, hidden)) in placed.iter().enumerate() {
                    if hidden {
                        continue;
                    }
                    let start = tris.len();
                    put_colors(&mut vcolors, start, vc.then(|| tri_colors(g, wm.determinant() < 0.0, None)));
                    append_arrays(g, wm, mat, flags, None, &mut tris, &mut tri_flags, &mut attrs);
                    let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
                    for v in tris[start..].iter().flatten() {
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                    let oi = (objects.len() / stride) as u32;
                    objects.extend_from_slice(&entry(i, wm, Vec3::from((hi - lo).max(Vec3A::ZERO)))[..stride]);
                    for a in &mut attrs[start..] {
                        a.x[1] = f32::from_bits(oi);
                    }
                    tri_nodes.push((n.name.clone(), start as u32, tris.len() as u32));
                    tri_instances.push((start as u32, tris.len() as u32, i as u32));
                }
            }
            Kind::Mesh { geometry, material, materials: mesh_mats, .. } if shown => {
                let g = w.geometries.get(geometry).ok_or_else(|| anyhow!("object {}: unknown geometry {geometry}", n.name))?;
                let mat = *retained.materials.get(material).ok_or_else(|| anyhow!("object {}: unknown material {material}", n.name))?;
                // Several materials: each geometry group's triangles with its own (three.js's material array).
                let gpu_mats = mesh_mats.iter().map(|k| retained.materials.get(k).copied().ok_or_else(|| anyhow!("object {}: unknown material {k}", n.name))).collect::<Result<Vec<u32>>>()?;
                let per_tri = group_materials(g, &gpu_mats);
                let movable = w.movable(*id);
                // Coloured where the triangle's own material asks (each group's; without groups, the mesh's).
                let wants = |k: &u32| w.materials.get(k).is_some_and(|m| m.vertex_colors);
                let vc_tri = group_materials(g, &mesh_mats.iter().map(|k| u32::from(wants(k))).collect::<Vec<_>>());
                let vc = !g.colors.is_empty() && match &vc_tri { Some(v) => v.iter().any(|f| *f == Some(1)), None => wants(material) };
                put_colors(&mut vcolors, tris.len(), vc.then(|| tri_colors(g, m.determinant() < 0.0, vc_tri.as_deref())));
                append_arrays(g, m, mat, if movable { flags | TRI_DYNAMIC } else { flags }, per_tri.as_deref(), &mut tris, &mut tri_flags, &mut attrs);
                if movable {
                    retained.dynamic.insert(*id, DynamicModel { first, mats: attrs[first as usize..].iter().map(|a| a.uv[3].to_bits()).collect(), ..Default::default() });
                }
            }
            Kind::Model { uri, nodes, animation, functions } if shown => {
                let glb = model_gltf(uri, assets)?;
                let (doc, _, images) = &glb.gltf;
                // (A model with material functions has its own copies of the file's materials.)
                let key = if functions.is_empty() { uri.clone() } else { format!("{uri}\u{0}{functions:?}") };
                // (Its meshes too: placements share a mesh only with the same functions.)
                let mesh_key = (key.clone(), nodes.clone());
                // (material index, instancing class: 0 opaque, 1 cutout, 2 flattened per placement: glass (shadow rays
                // pass), emissive (emitter sampling), material function)
                let mut resolve = |gm: Option<gltf::Material>| -> Result<(u32, u32)> {
                    let idx = gm.as_ref().and_then(|g| g.index()).unwrap_or(usize::MAX);
                    let i = match asset_materials.get(&(key.clone(), idx)) {
                        Some(i) => *i,
                        None => {
                            let name = gm.as_ref().and_then(|g| g.name()).unwrap_or_default().to_string();
                            let mut gpu = gltf_material(gm, images, uri, &mut texcache, &model_images)?;
                            // The first function whose prefix the material's name starts with, on the glTF shading.
                            if let Some(f) = functions.iter().find(|f| name.starts_with(f.prefix.as_str())) {
                                let mut tex = |tid: u32, srgb_default: bool| -> Result<i32> {
                                    let t = w.textures.get(&tid).ok_or_else(|| anyhow!("unknown texture {tid}"))?;
                                    let (tkey, img) = match decoded.get(&tid) {
                                        Some(d) => d.clone(),
                                        None => texture_image(t, assets)?,
                                    };
                                    texcache.add((tkey, t.srgb && srgb_default), t.wrap, || Ok(img))
                                };
                                apply_shader(&mut gpu, &f.shader, &retained.shaders, &mut tex).with_context(|| format!("{uri}: material {name}"))?;
                            }
                            let i = materials.len() as u32;
                            materials.push(gpu);
                            asset_materials.insert((key.clone(), idx), i);
                            i
                        }
                    };
                    Ok((i, material_class(&materials[i as usize])))
                };
                // A posed model's triangles are dynamic (re-skinned in place). A movable one is an instance whose
                // transform moves (hardware RT), unless it has triangles flattened per placement: then they are dynamic
                // too (re-transformed in place).
                let posed = animation.is_some() && glb.animated();
                let movable = w.movable(*id);
                let dynamic = posed || movable;
                let flags = if dynamic { flags | TRI_DYNAMIC } else { flags };
                // Posed rigid model: static nodes as triangles, moving nodes as instances (`RigidModel`); a movable one
                // too (each update re-places its instances from the object's transform).
                if instancing && posed && nodes.is_none() && glb.rigid() {
                    let a = animation.as_ref().unwrap();
                    let moving = glb.moving_nodes(&a.clip)?;
                    let globals = glb.node_globals(Some(a))?;
                    let static_tris = glb.node_triangles(m, Some(a), &|ni| !moving[ni], false, &|_| false)?;
                    let base_flags = if n.cast_shadow { 0 } else { TRI_NO_SHADOW };
                    for (_, t) in &static_tris {
                        let mat = resolve(t.material.and_then(|i| doc.materials().nth(i)))?.0;
                        push_tri(&mut tris, &mut tri_flags, &mut attrs, t.p, t.n, t.uv, t.tangent, mat, base_flags);
                    }
                    let mirrored = |ni: usize| (m * globals[ni]).determinant() < 0.0;
                    let local = glb.node_triangles(Mat4::IDENTITY, None, &|ni| moving[ni], true, &mirrored)?;
                    // Group by node (the order within a node is the file's).
                    let mut by_node: Vec<(usize, Vec<usize>)> = vec![];
                    for (k, (ni, _)) in local.iter().enumerate() {
                        match by_node.last_mut() {
                            Some((last, list)) if *last == *ni => list.push(k),
                            _ => by_node.push((*ni, vec![k])),
                        }
                    }
                    let oi = (objects.len() / stride) as u32;
                    let (mut olo, mut ohi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
                    for t in &tris[first as usize..] {
                        for v in t {
                            olo = olo.min(*v);
                            ohi = ohi.max(*v);
                        }
                    }
                    let mut rigid = RigidModel::default();
                    let rest_first = tris.len() as u32;
                    for (ni, list) in &by_node {
                        let w_n = m * globals[*ni];
                        let mut classed: [Vec<(usize, u32)>; 3] = Default::default();
                        for &k in list {
                            let (mat, class) = resolve(local[k].1.material.and_then(|i| doc.materials().nth(i)))?;
                            classed[class as usize].push((k, mat));
                        }
                        if classed[0].len() + classed[1].len() > 0 {
                            let first_pool = pool_tris.len() as u32;
                            let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
                            for list in &classed[..2] {
                                for &(k, mat) in list {
                                    let t = &local[k].1;
                                    for v in t.p {
                                        lo = lo.min(v.into());
                                        hi = hi.max(v.into());
                                    }
                                    pool_tris.push([t.p[0].into(), t.p[1].into(), t.p[2].into()]);
                                    pool_attrs.push(tri_attr(t.p, t.n, t.uv, t.tangent, mat));
                                }
                            }
                            for c in 0..8 {
                                let p = Vec3A::new(if c & 1 == 0 { lo.x } else { hi.x }, if c & 2 == 0 { lo.y } else { hi.y }, if c & 4 == 0 { lo.z } else { hi.z });
                                let q = w_n.transform_point3a(p);
                                olo = olo.min(q);
                                ohi = ohi.max(q);
                            }
                            let count = (classed[0].len() + classed[1].len()) as u32;
                            meshes.push(InstMesh { first: first_pool, count, opaque: classed[0].len() as u32, rest_first: 0, rest_count: 0, lo, hi, deforms: false, emits: false });
                            rigid.instances.push((*ni, instances.len() as u32));
                            instances.push(Instance { mesh: meshes.len() as u32 - 1, world: w_n, object: oi, cast_shadow: n.cast_shadow, name: n.name.clone(), sub: None, hidden: false });
                        }
                        let nm = Mat3::from_mat4(w_n).inverse().transpose();
                        for &(k, mat) in &classed[2] {
                            let t = &local[k].1;
                            let p = t.p.map(|v| w_n.transform_point3(v));
                            let nn = t.n.map(|v| (nm * v).normalize_or_zero());
                            let tg = t.tangent.map(|(v, s)| (w_n.transform_vector3(v).normalize_or_zero(), s));
                            push_tri(&mut tris, &mut tri_flags, &mut attrs, p, nn, t.uv, tg, mat, base_flags | TRI_DYNAMIC);
                            rigid.rest.mats.push(mat);
                            rigid.rest_local.push((*ni, t.p, t.n, t.uv, t.tangent));
                        }
                    }
                    for t in &tris[rest_first as usize..] {
                        for v in t {
                            olo = olo.min(*v);
                            ohi = ohi.max(*v);
                        }
                    }
                    let pivot = m.transform_point3(Vec3::ZERO);
                    let size = ohi - olo;
                    objects.extend_from_slice(&object_row(pivot, n.power, size, n.custom, [1.0; 3])[..stride]);
                    for a in &mut attrs[first as usize..] {
                        a.x[1] = f32::from_bits(oi);
                    }
                    retained.objects.insert(*id, oi);
                    rigid.rest.first = rest_first;
                    rigid.rest.object = oi;
                    if tris.len() as u32 > first {
                        tri_nodes.push((n.name.clone(), first, tris.len() as u32));
                    }
                    if std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "log") {
                        eprintln!("rigid: {uri}: {} static triangles, {} moving nodes as instances, {} flattened moving triangles", static_tris.len(), rigid.instances.len(), rigid.rest.mats.len());
                    }
                    retained.rigid.insert(*id, rigid);
                    continue;
                }
                // A model in a shared pose: an instance of the pose's mesh (`PoseGroup`), made by the group's first.
                if let Some(key) = pose_key_of.get(id) {
                    let group = match pose_group_of.get(key) {
                        Some(g) => *g,
                        None => {
                            let a = animation.as_ref().unwrap();
                            let local = glb.triangles(Mat4::IDENTITY, nodes.as_deref(), Some(a))?;
                            let mut classed: [Vec<(usize, u32)>; 3] = Default::default();
                            for (k, t) in local.iter().enumerate() {
                                let (mat, class) = resolve(t.material.and_then(|i| doc.materials().nth(i)))?;
                                classed[class as usize].push((k, mat));
                            }
                            // Triangles flattened per placement (glass, emissive, functions): each model skinned alone.
                            let g = (classed[2].is_empty() && !local.is_empty()).then(|| {
                                let first = pool_tris.len() as u32;
                                let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
                                let mut tri_order = Vec::with_capacity(local.len());
                                for list in &classed[..2] {
                                    for &(k, mat) in list {
                                        let t = &local[k];
                                        for v in t.p {
                                            lo = lo.min(v.into());
                                            hi = hi.max(v.into());
                                        }
                                        pool_tris.push([t.p[0].into(), t.p[1].into(), t.p[2].into()]);
                                        pool_attrs.push(tri_attr(t.p, t.n, t.uv, t.tangent, mat));
                                        tri_order.push((k as u32, mat));
                                    }
                                }
                                // Other poses reach further (a stride, a swing): object bounds a quarter wider each way.
                                let pad = (hi - lo) * 0.25;
                                meshes.push(InstMesh { first, count: tri_order.len() as u32, opaque: classed[0].len() as u32, rest_first: 0, rest_count: 0, lo: lo - pad, hi: hi + pad, deforms: true, emits: false });
                                retained.pose_groups.push(PoseGroup { mesh: meshes.len() as u32 - 1, uri: uri.clone(), nodes: nodes.clone(), animation: a.clone(), order: tri_order, members: vec![] });
                                retained.pose_groups.len() as u32 - 1
                            });
                            pose_group_of.insert(key.clone(), g);
                            g
                        }
                    };
                    if let Some(g) = group {
                        let mesh = retained.pose_groups[g as usize].mesh;
                        let (lo, hi) = meshes[mesh as usize].placed_bounds(m);
                        let pivot = m.transform_point3(Vec3::ZERO);
                        let size = hi - lo;
                        let oi = (objects.len() / stride) as u32;
                        objects.extend_from_slice(&object_row(pivot, n.power, size, n.custom, [1.0; 3])[..stride]);
                        retained.objects.insert(*id, oi);
                        let i = instances.len() as u32;
                        instances.push(Instance { mesh, world: m, object: oi, cast_shadow: n.cast_shadow, name: n.name.clone(), sub: None, hidden: false });
                        retained.instance_of.insert(*id, i);
                        retained.pose_member.insert(*id, g);
                        retained.pose_groups[g as usize].members.push((*id, i));
                        continue;
                    }
                }
                if instancing && !posed && m.determinant() > 0.0 {
                    let key = mesh_key.clone();
                    let mesh = match mesh_of.get(&key) {
                        Some(k) => *k,
                        None => {
                            // Model-space triangles by class: opaque and cutout ones go to the pool (one BLAS, opaque
                            // geometry then cutout geometry); glass, emissive and function triangles are flattened per
                            // placement (shadow-ray pass-through, emitter sampling).
                            let local = glb.triangles(Mat4::IDENTITY, nodes.as_deref(), None)?;
                            let mut classed: [Vec<(usize, u32)>; 3] = Default::default();
                            for (k, t) in local.iter().enumerate() {
                                let (mat, class) = resolve(t.material.and_then(|i| doc.materials().nth(i)))?;
                                classed[class as usize].push((k, mat));
                            }
                            // A pool triangle on exactly the same vertices as a flattened one (Zero Day: masked decals
                            // over emissive quads of the same shape) is flattened with it. Split between the model's
                            // BLAS and the world-space triangles, the two met the ray at one t with different rounding,
                            // and a decal hit that its alpha let through stepped the ray past the emitter too (black
                            // through the decal; the software BVH, which flattens both, saw the emitter).
                            if !classed[2].is_empty() {
                                let key = |k: usize| {
                                    let mut v = local[k].p.map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]);
                                    v.sort_unstable();
                                    v
                                };
                                let flat: std::collections::HashSet<[[u32; 3]; 3]> = classed[2].iter().map(|&(k, _)| key(k)).collect();
                                for c in 0..2 {
                                    let (stay, go): (Vec<_>, Vec<_>) = classed[c].iter().partition(|&&(k, _)| !flat.contains(&key(k)));
                                    classed[c] = stay;
                                    classed[2].extend(go);
                                }
                            }
                            let k = (classed[0].len() + classed[1].len() > 0).then(|| {
                                let (first, rest_first) = (pool_tris.len() as u32, rest_tris.len() as u32);
                                let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
                                for t in &local {
                                    for v in t.p {
                                        lo = lo.min(v.into());
                                        hi = hi.max(v.into());
                                    }
                                }
                                for (class, list) in classed.iter().enumerate() {
                                    let (pt, pa) = if class < 2 { (&mut pool_tris, &mut pool_attrs) } else { (&mut rest_tris, &mut rest_attrs) };
                                    for &(k, mat) in list {
                                        let t = &local[k];
                                        pt.push([t.p[0].into(), t.p[1].into(), t.p[2].into()]);
                                        pa.push(tri_attr(t.p, t.n, t.uv, t.tangent, mat));
                                    }
                                }
                                let count = (classed[0].len() + classed[1].len()) as u32;
                                meshes.push(InstMesh { first, count, opaque: classed[0].len() as u32, rest_first, rest_count: classed[2].len() as u32, lo, hi, deforms: false, emits: false });
                                meshes.len() as u32 - 1
                            });
                            if std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "log") {
                                eprintln!("instancing: {uri} {nodes:?}: {} opaque, {} cutout instanced; {} flattened per placement", classed[0].len(), classed[1].len(), classed[2].len());
                            }
                            if let Some(k) = k.filter(|&k| meshes[k as usize].rest_count == 0) {
                                retained.model_mesh.insert(key.clone(), k);
                            }
                            mesh_of.insert(key, k);
                            k
                        }
                    };
                    if let Some(mesh) = mesh.filter(|&k| !movable || meshes[k as usize].rest_count == 0) {
                        // Object entry from the placed mesh bounds (the box of the transformed model-space box).
                        let im = meshes[mesh as usize];
                        let (lo, hi) = im.placed_bounds(m);
                        let pivot = m.transform_point3(Vec3::ZERO);
                        let size = hi - lo;
                        let oi = (objects.len() / stride) as u32;
                        objects.extend_from_slice(&object_row(pivot, n.power, size, n.custom, [1.0; 3])[..stride]);
                        retained.objects.insert(*id, oi);
                        let inst = Instance { mesh, world: m, object: oi, cast_shadow: n.cast_shadow, name: n.name.clone(), sub: None, hidden: false };
                        // Its glass / emissive triangles as ordinary world-space ones (same object).
                        for k in im.rest_first..im.rest_first + im.rest_count {
                            tris.push(inst.tri(&rest_tris[k as usize]));
                            tri_flags.push(flags);
                            attrs.push(instance_attr(&rest_attrs[k as usize], m, oi));
                        }
                        if tris.len() as u32 > first {
                            tri_nodes.push((n.name.clone(), first, tris.len() as u32));
                        }
                        if movable {
                            retained.instance_of.insert(*id, instances.len() as u32);
                        }
                        instances.push(inst);
                        continue;
                    }
                }
                let mut mats = vec![];
                for t in glb.triangles(m, nodes.as_deref(), animation.as_ref())? {
                    let mat = resolve(t.material.and_then(|i| doc.materials().nth(i)))?.0;
                    push_tri(&mut tris, &mut tri_flags, &mut attrs, t.p, t.n, t.uv, t.tangent, mat, flags);
                    mats.push(mat);
                }
                if dynamic {
                    retained.dynamic.insert(*id, DynamicModel { first, mats, ..Default::default() });
                }
            }
            Kind::Light(l) => match (l, gpu_light_shown(l, m, n.power, shown)) {
                (_, Some(gl)) => {
                    lights.push(gl);
                    retained.light_nodes.push(*id);
                }
                (Light::Rect { color, intensity, width, height, .. }, None) if shown => {
                    // Visible one-sided emissive quad (importance sampled as an emitter).
                    let mat = materials.len() as u32;
                    materials.push(card_material(*color, *intensity));
                    retained.cards.insert(*id, mat);
                    let movable = w.movable(*id);
                    for (v, dir) in card_tris(m, *width, *height) {
                        push_tri(&mut tris, &mut tri_flags, &mut attrs, v, [dir; 3], [Vec2::ZERO; 3], None, mat, if movable { TRI_NO_SHADOW | TRI_DYNAMIC } else { TRI_NO_SHADOW });
                    }
                    if movable {
                        retained.dynamic.insert(*id, DynamicModel { first, mats: vec![mat; 2], ..Default::default() });
                    }
                }
                _ => {}
            },
            // (Hidden by its own `visible`: built while the kernel has room after the volumes shown, with no shape,
            // so that showing it is in place: gpu.rs `set_medium_hidden`.)
            Kind::Volume { function, params, shape, majorant, g, homogeneous, animated, grid } if shown || (w.hidden_built(*id) && media.len() + media_to_show < MAX_MEDIA) => {
                if shown {
                    media_to_show -= 1;
                }
                let func = volume_fns.iter().position(|(k, _, _)| k == function).ok_or_else(|| anyhow!("object {}: unknown volume function {function}", n.name))? as u32;
                let mut p = [0.0f32; 32];
                for (k, v) in params.iter().take(32).enumerate() {
                    p[k] = *v;
                }
                // Its density grid: where it lies in the atlas and its cells, as bits in b.zw (pathtrace.wgsl `vgrid`).
                let mut at = [0.0f32; 2];
                if let Some(gid) = grid {
                    let dg = w.grids.get(gid).ok_or_else(|| anyhow!("object {}: unknown density grid {gid}", n.name))?;
                    if *shape != 0 {
                        return Err(anyhow!("object {}: a density grid needs a box volume", n.name));
                    }
                    let k = match grids.iter().position(|b| b.id == *gid) {
                        Some(k) => k,
                        None => {
                            let program = match &dg.program {
                                Some((name, _)) => Some(grid_programs.iter().position(|(k, _, _)| k == name).ok_or_else(|| anyhow!("object {}: unknown grid program {name}", n.name))? as u32),
                                None => None,
                            };
                            let mut gp = [0.0f32; 32];
                            for (k, v) in dg.program.iter().flat_map(|(_, p)| p.iter()).take(32).enumerate() {
                                gp[k] = *v;
                            }
                            let z0 = grids.iter().map(|b| b.dims[2]).sum::<u32>();
                            let cells = grids.iter().chain([&GridBuild { id: 0, dims: dg.dims, z0, data: None, program: None, params: [0.0; 32], node: 0, medium: 0, made: 0 }]).map(|b| b.dims.iter().map(|&d| d as u64).product::<u64>()).sum::<u64>();
                            // (The atlas is as wide and high as the widest and highest grid: its texels count too.)
                            let atlas = grids.iter().map(|b| b.dims[0]).chain([dg.dims[0]]).max().unwrap() as u64 * grids.iter().map(|b| b.dims[1]).chain([dg.dims[1]]).max().unwrap() as u64 * (z0 + dg.dims[2]) as u64;
                            if cells > world::GRID_MAX_SCENE_CELLS || atlas > 2 * world::GRID_MAX_SCENE_CELLS || z0 + dg.dims[2] > world::GRID_MAX_LAYERS {
                                return Err(anyhow!("object {}: the scene's density grids exceed {} cells or {} layers along z together (or differ too much in width and height)", n.name, world::GRID_MAX_SCENE_CELLS, world::GRID_MAX_LAYERS));
                            }
                            grids.push(GridBuild { id: *gid, dims: dg.dims, z0, data: dg.data.clone(), program, params: gp, node: *id, medium: media.len() as u32, made: dg.made });
                            grids.len() - 1
                        }
                    };
                    let b = &grids[k];
                    at = [f32::from_bits(b.z0), f32::from_bits(b.dims[0] | b.dims[1] << 10 | b.dims[2] << 20)];
                }
                if media.len() >= MAX_MEDIA {
                    return Err(anyhow!("object {}: more than {MAX_MEDIA} volumes shown at once (the kernel takes {MAX_MEDIA}; the rest would not be drawn)", n.name));
                }
                media.push(MediumGpu { w: medium_rows(m), a: [func, if shown { *shape } else { MEDIUM_HIDDEN }, *homogeneous as u32, *animated as u32], b: [majorant.max(0.0), g.clamp(-0.99, 0.99), at[0], at[1]], c: n.custom, p });
                media_nodes.push(*id);
            }
            Kind::Procedural { .. } if shown => {
                if let Some(p) = proc_object(w, *id, m, (objects.len() / stride) as u32, &retained.materials, assets)? {
                    objects.extend_from_slice(&p.object_entry(n)[..stride]);
                    retained.objects.insert(*id, p.object);
                    procs.push(p);
                }
            }
            Kind::Particles { count, emitter, params, material, emitter_area } if (shown || w.hidden_built(*id)) && *count > 0 => {
                // Quads collapsed at the node until the particle pass places them (each its own object: pivot,
                // bounds and per-particle data for material functions).
                let mat = *retained.materials.get(material).ok_or_else(|| anyhow!("object {}: unknown material {material}", n.name))?;
                let func = particle_fns.iter().position(|(k, _, _)| k == emitter).ok_or_else(|| anyhow!("object {}: unknown particle function {emitter}", n.name))? as u32;
                let pivot = m.transform_point3(Vec3::ZERO);
                let p = Vec3A::from(pivot);
                let object = (objects.len() / stride) as u32;
                let corners = [[Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0)], [Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)]];
                let translucent = w.materials.get(material).is_some_and(|m| m.transparent);
                let first = if translucent { trans_tris.len() as u32 } else { first };
                for k in 0..*count {
                    for uv in corners {
                        let mut a = tri_attr([pivot; 3], [Vec3::Z; 3], uv, Some((Vec3::X, 1.0)), mat);
                        a.x[1] = f32::from_bits(object + k);
                        // Light sampling by a nominal area (half a quad per triangle; lib.rs prepare, emit_mis); -1: a
                        // particle not light-sampled (its emission reaches surfaces by their BSDF rays alone).
                        a.x[2] = if translucent { 0.0 } else if *emitter_area > 0.0 { emitter_area * 0.5 } else { -1.0 };
                        if translucent {
                            trans_tris.push([p; 3]);
                            trans_attrs.push(a);
                        } else {
                            tris.push([p; 3]);
                            tri_flags.push(flags | TRI_DYNAMIC);
                            attrs.push(a);
                        }
                    }
                    objects.extend_from_slice(&object_row(pivot, n.power, Vec3::ZERO, n.custom, [1.0; 3])[..stride]);
                }
                let mut pf = [0.0f32; 32];
                for (k, v) in params.iter().take(32).enumerate() {
                    pf[k] = *v;
                }
                particles.push(ParticleSystem { translucent, first, final_first: first, count: *count, object, func, params: pf, flags: flags | TRI_DYNAMIC, seed: *id, custom: n.custom, hidden: !shown });
                if !translucent {
                    tri_nodes.push((n.name.clone(), first, tris.len() as u32));
                }
            }
            Kind::GpuParticles { capacity, program, params, material, emitter_area, rate, collide } if shown && *capacity > 0 => {
                // As `Particles`: quads collapsed at the node, each its own object, until the pass places them.
                let mat = *retained.materials.get(material).ok_or_else(|| anyhow!("object {}: unknown material {material}", n.name))?;
                let prog = particle_programs.iter().position(|(k, ..)| k == program).ok_or_else(|| anyhow!("object {}: unknown particle program {program}", n.name))? as u32;
                let pivot = m.transform_point3(Vec3::ZERO);
                let p = Vec3A::from(pivot);
                let object = (objects.len() / stride) as u32;
                let corners = [[Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0)], [Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)]];
                for k in 0..*capacity {
                    for uv in corners {
                        let mut a = tri_attr([pivot; 3], [Vec3::Z; 3], uv, Some((Vec3::X, 1.0)), mat);
                        a.x[1] = f32::from_bits(object + k);
                        a.x[2] = if *emitter_area > 0.0 { emitter_area * 0.5 } else { -1.0 };
                        tris.push([p; 3]);
                        tri_flags.push(flags | TRI_DYNAMIC);
                        attrs.push(a);
                    }
                    objects.extend_from_slice(&object_row(pivot, n.power, Vec3::ZERO, n.custom, [1.0; 3])[..stride]);
                }
                let mut pf = [0.0f32; 32];
                for (k, v) in params.iter().take(32).enumerate() {
                    pf[k] = *v;
                }
                gpu_particles.push(GpuParticleSystem { node: *id, first, final_first: first, capacity: *capacity, object, program: prog, params: pf, seed: *id, pivot: pivot.to_array(), rate: *rate, collide: collide.clone() });
                tri_nodes.push((n.name.clone(), first, tris.len() as u32));
            }
            Kind::Camera(c) if st.camera == Some(*id) => camera = Some(camera_data(m, c)),
            _ => {}
        }
        if tris.len() as u32 > first && !matches!(n.kind, Kind::Particles { .. } | Kind::GpuParticles { .. } | Kind::Instanced { .. }) {
            // Object pivot + world bounds (UE ObjectPositionWS / ObjectBounds) for material functions, and the object's
            // power (scales everything it emits).
            let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
            for t in &tris[first as usize..] {
                for v in t {
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                }
            }
            let pivot = m.transform_point3(Vec3::ZERO);
            let size = hi - lo;
            let oi = (objects.len() / stride) as u32;
            objects.extend_from_slice(&object_row(pivot, n.power, size, n.custom, [1.0; 3])[..stride]);
            for a in &mut attrs[first as usize..] {
                a.x[1] = f32::from_bits(oi);
            }
            retained.objects.insert(*id, oi);
            if let Some(d) = retained.dynamic.get_mut(id) {
                d.object = oi;
            }
            tri_nodes.push((n.name.clone(), first, tris.len() as u32));
        }
    }

    let env = &w.environment;
    let sky = match &env.sky {
        Sky::Atmosphere { sun, ground_albedo, rayleigh_scale, mie_scale, mie_anisotropy, dynamic, glow, horizon_glow, .. } => {
            let sun_light = sun.and_then(|s| w.objects.get(&s).map(|o| (s, o)));
            let (sun_dir, sun_lux) = match sun_light.map(|(s, o)| (worlds.get(&s).copied().unwrap_or(Mat4::IDENTITY), &o.kind)) {
                Some((m, Kind::Light(Light::Directional { color, intensity, .. }))) => (-m.transform_vector3(Vec3::NEG_Z).normalize(), Vec3::from(*color) * *intensity),
                _ => (Vec3::Y, Vec3::ZERO),
            };
            let idx = sun_light.and_then(|(s, _)| retained.light_nodes.iter().position(|id| *id == s));
            retained.sun = sun_light.map(|(s, _)| s);
            SkyData::Atmosphere {
                sun_dir,
                sun_lux,
                rayleigh: *rayleigh_scale,
                mie: *mie_scale,
                g: *mie_anisotropy,
                ground: Vec3::from(*ground_albedo),
                sun_node: idx,
                dynamic: *dynamic,
                glow: crate::sky::glow_of(glow, horizon_glow),
            }
        }
        Sky::Color { color, intensity, .. } if *intensity > 0.0 => SkyData::Color(Vec3::from(*color) * *intensity),
        Sky::Color { .. } => SkyData::None,
        Sky::Hdri { .. } => return Err(anyhow!("hdri environments are not supported by the native renderer yet")),
        Sky::Gradient { zenith, horizon, ground, intensity, horizon_exponent, glow, clouds, .. } => {
            let cloud_map = match clouds.as_ref().and_then(|c| c.map.map(|m| (c, m))) {
                Some((c, tid)) => {
                    let t = w.textures.get(&tid).ok_or_else(|| anyhow!("unknown texture {tid}"))?;
                    let (key, image) = texture_image(t, assets)?;
                    Some(crate::sky::CloudMap {
                        key,
                        image,
                        threshold: c.threshold.unwrap_or([0.35, 0.85]),
                        gain: c.gain.unwrap_or(1.8),
                        offset: Vec3::from(c.color) * *intensity,
                        projection: c.projection.unwrap_or(0.35),
                        bias: c.bias.unwrap_or(0.25),
                        fade: c.fade.unwrap_or(4.0),
                    })
                }
                None => None,
            };
            SkyData::Gradient(crate::sky::Gradient {
                zenith: Vec3::from(*zenith) * *intensity,
                horizon: Vec3::from(*horizon) * *intensity,
                ground: Vec3::from(*ground) * *intensity,
                horizon_exponent: *horizon_exponent,
                glow: glow.as_ref().map(|g| (Vec3::from(g.color) * g.intensity * *intensity, g.height.max(1e-3), g.width.map(|w| (w, g.power.unwrap_or(6.0))))),
                clouds: if cloud_map.is_some() { None } else { clouds.as_ref().map(|c| (c.coverage, Vec3::from(c.color) * *intensity, c.scale.max(0.05), c.seed)) },
                cloud_map,
            })
        }
    };

    let mut stats = HashMap::new();
    stats.insert("load_ms", t0.elapsed().as_secs_f64() * 1000.0);
    let instanced: u64 = instances.iter().map(|i| meshes[i.mesh as usize].count as u64).sum();
    stats.insert("triangles", (tris.len() as u64 + instanced) as f64);
    stats.insert("instances", instances.len() as f64);
    stats.insert("instanced_triangles", instanced as f64);
    // Deforming meshes (shared poses) first in the pool: the previous frame's vertices (`prev_verts`, motion vectors)
    // cover the dynamic tail and, right after it, them.
    if meshes.iter().any(|m| m.deforms) {
        let (mut pt, mut pa) = (Vec::with_capacity(pool_tris.len()), Vec::with_capacity(pool_attrs.len()));
        let mut pv = vec![];
        for deforms in [true, false] {
            for m in meshes.iter_mut().filter(|m| m.deforms == deforms) {
                let r = m.first as usize..(m.first + m.count) as usize;
                if !pool_vcolors.is_empty() {
                    put_colors(&mut pv, pt.len(), Some(r.clone().map(|k| pool_vcolors.get(k).copied().unwrap_or(WHITE)).collect()));
                }
                m.first = pt.len() as u32;
                pt.extend_from_slice(&pool_tris[r.clone()]);
                pa.extend_from_slice(&pool_attrs[r]);
            }
        }
        (pool_tris, pool_attrs) = (pt, pa);
        if !pool_vcolors.is_empty() {
            pool_vcolors = pv;
        }
    }
    stats.insert("unique_triangles", (tris.len() + pool_tris.len()) as f64);
    // Cutout materials: their triangles take the any-hit path in traversal. Glass lets shadow rays through
    // (unattenuated: no caustics, like UE's raster shadows of translucency).
    for (f, a) in tri_flags.iter_mut().zip(&attrs) {
        let m = &materials[a.uv[3].to_bits() as usize];
        if m.uv2[3] > 0.0 {
            *f |= TRI_MASKED;
        }
        if m.layer[0] > 0.0 {
            *f |= TRI_NO_SHADOW;
        }
    }
    for d in retained.dynamic.values_mut().chain(retained.rigid.values_mut().map(|r| &mut r.rest)) {
        d.flags = tri_flags[d.first as usize..d.first as usize + d.mats.len()].to_vec();
    }
    Ok(Scene {
        name: st.name.clone(),
        emitter_slot: false,
        objects,
        shaders,
        time: st.time,
        width: st.width,
        height: st.height,
        tris,
        vcolors,
        pool_vcolors,
        gpu_vcolors: vec![],
        vcolor_base: 0,
        tri_flags,
        attrs,
        materials,
        textures: texcache.textures,
        lights,
        camera: camera.ok_or_else(|| anyhow!("no camera (settings.camera must name a camera object)"))?,
        sky,
        fog: env.fog.clone(),
        wetness: env.wetness.clamp(0.0, 1.0),
        rain: env.rain.clamp(0.0, 1.0),
        cloud_cover: env.cloud_cover.clamp(0.0, 1.0),
        preset,
        stats,
        tri_nodes,
        tri_instances,
        tinted,
        background: w.environment.background.map(Vec3::from),
        aerial: aerial_params(&w.environment.sky),
        retained,
        particles,
        particle_fns,
        gpu_particles,
        particle_programs,
        voxel_occupancy: w.voxel_occupancy.clone(),
        particle_spawns: w.particle_spawns.clone(),
        media,
        media_nodes,
        volume_fns,
        grids,
        grid_programs,
        grid_sources: w.grid_sources.clone(),
        procs,
        geometry_fns,
        proc_layout: None,
        proc_records: vec![],
        units: None,
        trans_tris,
        trans_attrs,
        trans_first: 0,
        pool_tris,
        pool_attrs,
        meshes,
        instances,
        gpu_instances: vec![],
    })
}

/// Camera of a camera descriptor with world matrix `m`.
pub fn camera_data(m: Mat4, c: &desc::Camera) -> CameraData {

    let fwd = m.transform_vector3(Vec3::NEG_Z).normalize();
    let up = m.transform_vector3(Vec3::Y).normalize();
    let right = fwd.cross(up).normalize();
    CameraData {
        position: m.transform_point3(Vec3::ZERO),
        forward: fwd,
        up: right.cross(fwd),
        right,
        tan_half_fov: (c.fov.to_radians() * 0.5).tan(),
        exposure: c.exposure.clone(),
        dof: c.dof.clone(),
    }
}

/// `Params::aerial` for a sky: [density multiple, Rayleigh scale, Mie scale, 0] for an atmosphere with aerial perspective,
/// zeros otherwise (no F_AERIAL).
pub fn aerial_params(sky: &Sky) -> [f32; 4] {
    match sky {
        Sky::Atmosphere { aerial, rayleigh_scale, mie_scale, .. } if *aerial > 0.0 => [*aerial, *rayleigh_scale, *mie_scale, 0.0],
        _ => [0.0; 4],
    }
}

/// `converted`: the model images already in RGBA, by texture key (scene::build converts them up front, in parallel).
fn gltf_material(gm: Option<gltf::Material>, images: &[gltf::image::Data], key: &str, tc: &mut TexCache, converted: &HashMap<String, Arc<image::RgbaImage>>) -> Result<GpuMaterial> {
    let Some(gm) = gm else {
        return Ok(GpuMaterial { base_color: [1.0, 1.0, 1.0, 1.0], params: [0.0, 1.0, 0.5, 1.0], tex0: [-1; 4], tex1: [-1, -1, 0, 0], uv: [1.0, 1.0, 0.0, 0.0], ..Default::default() });
    };
    let mut tex = |t: Option<gltf::texture::Texture>, srgb: bool| -> Result<i32> {
        let Some(t) = t else { return Ok(-1) };
        let src = t.source().index();
        let k = format!("{key}#img{src}");
        let pre = converted.get(&k).cloned();
        tc.add((k, srgb), true, || pre.map_or_else(|| Ok(Arc::new(to_rgba(&images[src])?)), Ok))
    };
    let pbr = gm.pbr_metallic_roughness();
    let mr = tex(pbr.metallic_roughness_texture().map(|t| t.texture()), false)?;
    let strength = gm.emissive_strength().unwrap_or(1.0);
    let e = gm.emissive_factor();
    let clearcoat = |k: &str| gm.extension_value("KHR_materials_clearcoat").and_then(|c| c[k].as_f64()).unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    Ok(GpuMaterial {
        base_color: pbr.base_color_factor(),
        emissive: [e[0] * strength, e[1] * strength, e[2] * strength, 0.0],
        params: [pbr.metallic_factor(), pbr.roughness_factor(), 0.5, gm.normal_texture().map(|n| n.scale()).unwrap_or(1.0)],
        tex0: [tex(pbr.base_color_texture().map(|t| t.texture()), true)?, tex(gm.normal_texture().map(|t| t.texture()), false)?, mr, mr],
        tex1: [tex(gm.emissive_texture().map(|t| t.texture()), true)?, tex(gm.occlusion_texture().map(|t| t.texture()), false)?, if gm.double_sided() { MAT_DOUBLE_SIDED } else { 0 }, 0],
        uv: [1.0, 1.0, gm.transmission().and_then(|t| t.transmission_texture()).map_or(Ok(-1), |t| tex(Some(t.texture()), false))? as f32 + 1.0, 0.0],
        // glTF MASK (and BLEND, approximated) -> alpha cutout
        uv2: [0.0, 0.0, 0.0, match gm.alpha_mode() { gltf::material::AlphaMode::Opaque => 0.0, gltf::material::AlphaMode::Mask => gm.alpha_cutoff().unwrap_or(0.5), gltf::material::AlphaMode::Blend => 0.5 }],
        // KHR_materials_transmission / _ior / _clearcoat factors (the transmission texture in uv.z; the clearcoat
        // textures are not read yet).
        layer: [
            gm.transmission().map_or(0.0, |t| t.transmission_factor()).clamp(0.0, 1.0),
            gm.ior().unwrap_or(1.5).max(1.0),
            clearcoat("clearcoatFactor"),
            clearcoat("clearcoatRoughnessFactor"),
        ],
        ..Default::default()
    })
}

fn to_rgba(img: &gltf::image::Data) -> Result<image::RgbaImage> {
    use gltf::image::Format;
    let (w, h) = (img.width, img.height);
    let px = &img.pixels;
    let rgba: Vec<u8> = match img.format {
        Format::R8G8B8A8 => px.clone(),
        Format::R8G8B8 => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        // Two channels decode from grey + alpha PNGs (glTF labels, cutouts).
        Format::R8G8 => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        Format::R8 => px.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        f => return Err(anyhow!("unsupported image format {f:?}")),
    };
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow!("bad image"))
}

/// Triangles of scene-code geometry (three.js conventions: UV v up, flipped to the renderer's v down) under `world`.
/// A visible rect light's one-sided emissive quad (two triangles, winding so the geometric normal faces the light's
/// direction), slightly in front of the light.
fn card_tris(m: Mat4, width: f32, height: f32) -> [([Vec3; 3], Vec3); 2] {
    let pos = m.transform_point3(Vec3::ZERO);
    let dir = m.transform_vector3(Vec3::NEG_Z).normalize();
    let ux = m.transform_vector3(Vec3::X).normalize() * (width * 0.5);
    let uy = m.transform_vector3(Vec3::Y).normalize() * (height * 0.5);
    let c = pos + dir * 0.001;
    let p = [c - ux - uy, c + ux - uy, c + ux + uy, c - ux + uy];
    [([p[0], p[2], p[1]], dir), ([p[0], p[3], p[2]], dir)]
}

/// A dynamic object's triangles at its current transform / pose, with their shading attributes (materials as built):
/// what an in-place update writes. None: it no longer matches its build (a different kind).
pub fn dynamic_tris(w: &World, id: u32, d: &DynamicModel, assets: &Path) -> Result<Option<Vec<([Vec3; 3], GpuTriAttr)>>> {
    let Some(o) = w.objects.get(&id) else { return Ok(None) };
    let m = w.world_matrix(id);
    Ok(match &o.kind {
        Kind::Mesh { geometry, materials, .. } => {
            let Some(g) = w.geometries.get(geometry) else { return Ok(None) };
            let (mut tris, mut flags, mut attrs) = (vec![], vec![], vec![]);
            // The triangles the build drew (groups' only, with several materials), each with the material it had.
            let drawn = group_materials(g, &vec![0; materials.len()]);
            append_arrays(g, m, 0, 0, drawn.as_deref(), &mut tris, &mut flags, &mut attrs);
            for (k, a) in attrs.iter_mut().enumerate() {
                a.uv[3] = f32::from_bits(d.mats.get(k).copied().unwrap_or(0));
            }
            Some(tris.iter().zip(attrs).map(|(t, a)| (t.map(Vec3::from), a)).collect())
        }
        Kind::Model { uri, nodes, animation, .. } => {
            let tris = model_gltf(uri, assets)?.triangles(m, nodes.as_deref(), animation.as_ref())?;
            if tris.len() != d.mats.len() {
                return Ok(None);
            }
            Some(tris.iter().zip(&d.mats).map(|(t, &mat)| (t.p, tri_attr(t.p, t.n, t.uv, t.tangent, mat))).collect())
        }
        Kind::Light(Light::Rect { width, height, visible: true, .. }) => {
            Some(card_tris(m, *width, *height).iter().zip(&d.mats).map(|((v, dir), &mat)| (*v, tri_attr(*v, [*dir; 3], [Vec2::ZERO; 3], None, mat))).collect())
        }
        _ => None,
    })
}

/// Instancing class of a GPU material: 0 opaque, 1 cutout (any-hit), 2 flattened per placement (glass: shadow rays pass;
/// emissive: emitter sampling; material functions).
pub fn material_class(g: &GpuMaterial) -> u32 {
    let flat = g.layer[0] > 0.0 || g.tex1[3] != 0 || g.emissive[..3].iter().any(|e| *e > 0.0);
    if flat {
        2
    } else if g.uv2[3] > 0.0 {
        1
    } else {
        0
    }
}

/// White, three times: a triangle without vertex colours.
pub const WHITE: [u32; 3] = [u32::MAX; 3];

/// The vertex colours of `g`'s triangles as `append_arrays` makes them (the same triangles, in the same order; with a
/// mirroring placement the same vertices swapped), each vertex as sRGB 8-bit RGBA (a palette's colours exactly).
/// `per_tri` (a mesh with several materials, as `group_materials` gives it): None skips a triangle (as `append_arrays`
/// does), Some(0) leaves it white (its material has no `vertexColors`, three.js's per-material flag), else coloured.
pub fn tri_colors(g: &world::Geometry, mirrored: bool, per_tri: Option<&[Option<u32>]>) -> Vec<[u32; 3]> {
    let enc = |c: f32| (crate::post::linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0).round() as u32;
    let col = |i: u32| {
        let k = i as usize * 3;
        match g.colors.get(k..k + 3) {
            Some(c) => enc(c[0]) | enc(c[1]) << 8 | enc(c[2]) << 16 | 0xff << 24,
            None => u32::MAX,
        }
    };
    g.indices
        .chunks_exact(3)
        .enumerate()
        .filter_map(|(k, t)| match per_tri.map(|v| v.get(k).copied().flatten()) {
            Some(None) => None,
            Some(Some(0)) => Some(WHITE),
            _ => Some(if mirrored { [col(t[0]), col(t[2]), col(t[1])] } else { [col(t[0]), col(t[1]), col(t[2])] }),
        })
        .collect()
}

/// `c` (vertex colours of triangles from index `at` on) into `dst`, white before them where it had none.
pub fn put_colors(dst: &mut Vec<[u32; 3]>, at: usize, c: Option<Vec<[u32; 3]>>) {
    let Some(c) = c.filter(|c| !c.is_empty()) else { return };
    dst.resize(at, WHITE);
    dst.extend(c);
}

/// An `own_blas` mesh's triangles in model space and their attributes, as the pool holds them. The build and the live
/// view's in-place edits (live/edits.rs `edit_geometry`) both make them here, so either path gives the same data.
pub fn own_mesh_tris(g: &world::Geometry, mat: u32) -> (Vec<[Vec3A; 3]>, Vec<GpuTriAttr>) {
    let (mut tris, mut flags, mut attrs) = (vec![], vec![], vec![]);
    append_arrays(g, Mat4::IDENTITY, mat, 0, None, &mut tris, &mut flags, &mut attrs);
    (tris, attrs)
}

/// A material that only emits (no glass, no material function): an `own_blas` mesh of it is an emitter mesh
/// (`InstMesh::emits`) rather than flattened.
pub fn emits_only(g: &GpuMaterial) -> bool {
    g.layer[0] <= 0.0 && g.tex1[3] == 0 && g.emissive[..3].iter().any(|e| *e > 0.0)
}

/// An emitting `own_blas` mesh's triangles in world space (`m`) and their attributes carrying its object-table index,
/// as the pool holds them (light sampling reads them without the placement). The build and the live view's in-place
/// edits both make them here.
pub fn emitter_mesh_tris(g: &world::Geometry, m: Mat4, mat: u32, object: u32) -> (Vec<[Vec3A; 3]>, Vec<GpuTriAttr>) {
    let (mut tris, mut flags, mut attrs) = (vec![], vec![], vec![]);
    append_arrays(g, m, mat, 0, None, &mut tris, &mut flags, &mut attrs);
    for a in &mut attrs {
        a.x[1] = f32::from_bits(object);
    }
    (tris, attrs)
}

/// The `InstMesh` of an `own_blas` mesh's triangles at pool index `first` (class: `material_class`, 0 or 1).
pub fn own_inst_mesh(tris: &[[Vec3A; 3]], first: u32, class: u32) -> InstMesh {
    let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
    for t in tris {
        for v in t {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    let count = tris.len() as u32;
    InstMesh { first, count, opaque: if class == 0 { count } else { 0 }, rest_first: 0, rest_count: 0, lo, hi, deforms: false, emits: false }
}

/// The GPU material of each triangle of a mesh with several (`Kind::Mesh::materials`, here GPU indices) by its
/// geometry's groups; None for a triangle in no group (not drawn, as three.js). None for a mesh with one material.
fn group_materials(g: &world::Geometry, mats: &[u32]) -> Option<Vec<Option<u32>>> {
    if mats.len() <= 1 || g.groups.is_empty() {
        return None;
    }
    let mut out = vec![None; g.indices.len() / 3];
    for &[start, count, mi] in &g.groups {
        let m = mats[(mi as usize).min(mats.len() - 1)];
        for t in (start / 3) as usize..((start + count) / 3) as usize {
            if let Some(slot) = out.get_mut(t) {
                *slot = Some(m);
            }
        }
    }
    Some(out)
}

/// A geometry's triangles at `world`, with `mat`, or per triangle `per_tri`'s (`group_materials`: None not drawn).
#[allow(clippy::too_many_arguments)]
fn append_arrays(g: &world::Geometry, world: Mat4, mat: u32, flags: u32, per_tri: Option<&[Option<u32>]>, tris: &mut Vec<[Vec3A; 3]>, tri_flags: &mut Vec<u32>, attrs: &mut Vec<GpuTriAttr>) {
    let nm = glam::Mat3::from_mat4(world).inverse().transpose();
    let mirrored = world.determinant() < 0.0;
    let p = |i: usize| world.transform_point3(Vec3::new(g.positions[i * 3], g.positions[i * 3 + 1], g.positions[i * 3 + 2]));
    let n = |i: usize| (nm * Vec3::new(g.normals[i * 3], g.normals[i * 3 + 1], g.normals[i * 3 + 2])).normalize_or_zero();
    let uv = |i: usize| Vec2::new(g.uvs[i * 2], 1.0 - g.uvs[i * 2 + 1]);
    tris.reserve(g.indices.len() / 3);
    for (k, t) in g.indices.chunks_exact(3).enumerate() {
        let mat = match per_tri {
            Some(v) => match v.get(k).copied().flatten() {
                Some(m) => m,
                None => continue,
            },
            None => mat,
        };
        let (a, b, c) = if mirrored { (t[0], t[2], t[1]) } else { (t[0], t[1], t[2]) };
        let (a, b, c) = (a as usize, b as usize, c as usize);
        let pos = [p(a), p(b), p(c)];
        let gn = (pos[1] - pos[0]).cross(pos[2] - pos[0]).normalize_or_zero();
        let mut nrm = [n(a), n(b), n(c)];
        for v in &mut nrm {
            if *v == Vec3::ZERO {
                *v = gn;
            }
        }
        push_tri(tris, tri_flags, attrs, pos, nrm, [uv(a), uv(b), uv(c)], None, mat, flags);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_tri(
    tris: &mut Vec<[Vec3A; 3]>,
    tri_flags: &mut Vec<u32>,
    attrs: &mut Vec<GpuTriAttr>,
    p: [Vec3; 3],
    n: [Vec3; 3],
    uv: [Vec2; 3],
    tangent: Option<(Vec3, f32)>,
    mat: u32,
    flags: u32,
) {
    tris.push([p[0].into(), p[1].into(), p[2].into()]);
    tri_flags.push(flags);
    attrs.push(tri_attr(p, n, uv, tangent, mat));
}

/// Shading attributes of a triangle (normals, UVs, tangent frame, texel density, material).
pub fn tri_attr(p: [Vec3; 3], n: [Vec3; 3], uv: [Vec2; 3], tangent: Option<(Vec3, f32)>, mat: u32) -> GpuTriAttr {
    // Per-triangle tangent frame from UVs (glTF convention: v grows downward in the image).
    let (t, sign) = tangent.unwrap_or_else(|| {
        let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
        let (d1, d2) = (uv[1] - uv[0], uv[2] - uv[0]);
        let det = d1.x * d2.y - d2.x * d1.y;
        if det.abs() < 1e-12 {
            let any = if n[0].x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
            return (n[0].cross(any).normalize(), 1.0);
        }
        let r = 1.0 / det;
        let t = ((e1 * d2.y - e2 * d1.y) * r).normalize_or_zero();
        let b = (e2 * d1.x - e1 * d2.x) * r;
        // glTF/MikkTSpace: bitangent = cross(N, T) * w, pointing toward decreasing v (image "up").
        let w = if n[0].cross(t).dot(b) < 0.0 { 1.0 } else { -1.0 };
        (t, w)
    });
    // Texel density for ray-cone mip selection: lod = log2(cone width) + 0.5*log2(uv area / world area) + log2(size).
    let world_area = 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length();
    let (d1, d2) = (uv[1] - uv[0], uv[2] - uv[0]);
    let uv_area = 0.5 * (d1.x * d2.y - d2.x * d1.y).abs();
    let lod_k = if world_area > 1e-12 && uv_area > 1e-14 { 0.5 * (uv_area / world_area).log2() } else { 0.0 };
    GpuTriAttr {
        n0: [n[0].x, n[0].y, n[0].z, uv[0].x],
        n1: [n[1].x, n[1].y, n[1].z, uv[0].y],
        n2: [n[2].x, n[2].y, n[2].z, uv[1].x],
        uv: [uv[1].y, uv[2].x, uv[2].y, f32::from_bits(mat)],
        t: [t.x, t.y, t.z, sign],
        x: [lod_k, 0.0, 0.0, 0.0],
    }
}

#[cfg(test)]
mod instance_tests {
    use super::*;

    /// The kernel's procedural data (pathtrace.wgsl `proc_box`, `surface_proc`, `trace_procs`): per object a header,
    /// then materials, boxes and data, the header pointing at each.
    /// Vertex colours per triangle as append_arrays makes the triangles: sRGB 8-bit (0.5 linear -> 188), a mirroring
    /// placement's swapped vertices, groups' skipped triangles skipped; white where the geometry has none.
    #[test]
    fn tri_colors_follow_the_triangles() {
        let g = world::Geometry { positions: vec![0.0; 12], normals: vec![0.0; 12], uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0] };
        let (r, gr, half, black) = (0xff0000ffu32, 0xff00ff00u32, 0xffbcbcbcu32, 0xff000000u32);
        assert_eq!(tri_colors(&g, false, None), vec![[r, gr, half], [r, half, black]]);
        assert_eq!(tri_colors(&g, true, None), vec![[r, half, gr], [r, black, half]]);
        assert_eq!(tri_colors(&g, false, Some(&[None, Some(1)])), vec![[r, half, black]]);
        assert_eq!(tri_colors(&g, false, Some(&[Some(0), Some(1)])), vec![WHITE, [r, half, black]], "a group whose material has no vertexColors");
        assert_eq!(tri_colors(&world::Geometry { colors: vec![], ..g }, false, None), vec![WHITE; 2]);
    }

    #[test]
    fn proc_data_layout() {
        let g = |boxes: Vec<f32>, data: Vec<u32>| Arc::new(crate::procasset::ProcBlob::new(boxes, data).unwrap());
        let p = |blob, materials: Vec<u32>| ProcObject { node: 0, name: "p".into(), world: Mat4::IDENTITY, object: 0, func: 0, blob, materials, cast_shadow: true, lo: Vec3::splat(-1.0), hi: Vec3::splat(2.0), sub: None, hidden: false };
        let ga = g(vec![-1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0], vec![5, 6, 7]);
        let a = p(ga.clone(), vec![9, 4]);
        let b = p(g(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0], vec![]), vec![1]);
        // A third object of a's geometry: its blob is a's (one copy).
        let c = p(ga, vec![2]);
        let l = proc_layout(&[a, b, c], false);
        assert_eq!(l.blobs.len(), 2, "one blob per distinct geometry");
        assert_eq!(l.blob_of, vec![0, 1, 0]);
        assert_eq!(l.blob_words, 12 + 3 + 6);
        // The region as the kernel sees it: head, then the blobs.
        let mut w = l.head.clone();
        for (blob, _) in &l.blobs {
            w.extend(blob.boxes.iter().map(|v| v.to_bits()));
            w.extend_from_slice(&blob.data);
        }
        let h = &l.headers;
        assert_eq!(h[..2], [0, (PROC_HEADER + 2) as u32]);
        assert_eq!((w[h[2] as usize + 1], w[h[2] as usize + 2]), (w[1], w[2]), "the third object points at the first's blob");
        let (boxes, data, mats) = (w[1] as usize, w[2] as usize, w[7] as usize);
        assert_eq!((w[0], w[3]), (2, 2), "box and material counts");
        assert_eq!(&w[mats..mats + 2], &[9, 4]);
        assert_eq!(f32::from_bits(w[boxes + 6]), 1.0, "second box min x");
        assert_eq!(&w[data..data + 3], &[5, 6, 7]);
        assert_eq!((f32::from_bits(w[4]), f32::from_bits(w[10])), (-1.0, 2.0), "bounds");
        let h1 = h[1] as usize;
        assert_eq!((w[h1], w[h1 + 3], w[w[h1 + 7] as usize]), (1, 1, 1));
        assert_eq!(w.len(), l.head.len() + l.blob_words);
        assert_eq!((w[11], w[h1 + 11]), (3, 0), "data lengths");
    }

    #[test]
    fn generated_textures_are_cached_by_content_not_address() {
        // One scene's 1 x 1 white texture is dropped; the next scene's black one of the same length may take its
        // address. It must decode as black.
        let tex = |v: u8| world::Texture { source: world::TextureSource::Pixels { width: 1, height: 1, rgba: Arc::new(vec![v, v, v, 255]) }, srgb: false, wrap: true, repeat: [1.0, 1.0], offset: [0.0, 0.0] };
        let assets = Path::new(".");
        let white = tex(255);
        let (kw, iw) = texture_image(&white, assets).unwrap();
        assert_eq!(iw.as_raw()[0], 255);
        drop(white);
        let black = tex(0);
        let (kb, ib) = texture_image(&black, assets).unwrap();
        assert_ne!(kw, kb);
        assert_eq!(ib.as_raw()[0], 0, "the black texture must not be served the white one's cached image");
        // Same content, same key (shared decode).
        assert_eq!(texture_image(&tex(0), assets).unwrap().0, kb);
    }

    /// An instance's placed triangle and attributes equal flattening the same triangle at the instance transform.
    #[test]
    fn instance_matches_flattening() {
        let p = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.5, 0.2)];
        let n = [Vec3::new(0.0, -0.37, 0.93).normalize(); 3];
        let uv = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        let w = Mat4::from_scale_rotation_translation(Vec3::splat(2.0), glam::Quat::from_rotation_y(0.7) * glam::Quat::from_rotation_x(-0.3), Vec3::new(5.0, 1.0, -3.0));
        let local = tri_attr(p, n, uv, None, 7);
        let inst = Instance { mesh: 0, world: w, object: 3, cast_shadow: true, name: "x".into(), sub: None, hidden: false };
        let placed = inst.tri(&[p[0].into(), p[1].into(), p[2].into()]);
        let wp = p.map(|v| w.transform_point3(v));
        let flat = tri_attr(wp, n.map(|v| (glam::Mat3::from_mat4(w).inverse().transpose() * v).normalize()), uv, None, 7);
        let got = instance_attr(&local, w, 3);
        for k in 0..3 {
            assert!((Vec3::from(placed[k]) - wp[k]).length() < 1e-5);
        }
        for (a, b) in [(got.n0, flat.n0), (got.n1, flat.n1), (got.n2, flat.n2), (got.t, flat.t)] {
            assert!((Vec3::new(a[0], a[1], a[2]) - Vec3::new(b[0], b[1], b[2])).length() < 1e-5, "{a:?} vs {b:?}");
            assert_eq!(a[3], b[3]);
        }
        // Uniform scale 2: texel density per metre halves (LOD constant - 1), exactly as the world-space triangle.
        assert!((got.x[0] - flat.x[0]).abs() < 1e-5);
        assert!((lod_offset(w) + 1.0).abs() < 1e-5);
        assert_eq!(got.x[1].to_bits(), 3);
        assert_eq!(got.uv, flat.uv);
    }

    /// Software traversal flattens instances: a cutout material's triangles keep TRI_MASKED (any-hit alpha test),
    /// the placement's shadow flag applies to all.
    #[test]
    fn flattened_instance_keeps_cutout_flag() {
        let opaque = GpuMaterial::default();
        let mut cutout = GpuMaterial::default();
        cutout.uv2[3] = 0.5;
        let mats = [opaque, cutout];
        let p = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let a = |m: u32| tri_attr(p, [Vec3::Z; 3], [Vec2::ZERO, Vec2::X, Vec2::Y], None, m);
        let inst = |shadow: bool| Instance { mesh: 0, world: Mat4::IDENTITY, object: 0, cast_shadow: shadow, name: "x".into(), sub: None, hidden: false };
        assert_eq!(instance_tri_flags(&inst(true), &a(0), &mats), 0);
        assert_eq!(instance_tri_flags(&inst(true), &a(1), &mats), TRI_MASKED);
        assert_eq!(instance_tri_flags(&inst(false), &a(1), &mats), TRI_MASKED | TRI_NO_SHADOW);
    }

    /// A node's triangles may come in two ranges (an instance's glass part flattened at build, its mesh appended by
    /// software traversal): selection outlines need both.
    /// An instanced object: one shared mesh, an instance and an object entry per copy (capacity, not count), hidden
    /// past the count and where a matrix scales to 0; mirrored copies flatten every drawn copy.
    #[test]
    fn instanced_objects_share_one_mesh() {
        let mut w = World::default();
        w.set_material(1, serde_json::from_value(serde_json::json!({ "name": "m", "color": [1, 1, 1], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": false })).unwrap());
        let g = world::Geometry { positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], normals: vec![0.0, 0.0, 1.0].repeat(3), uvs: vec![0.0; 6], indices: vec![0, 1, 2], groups: vec![], colors: vec![] };
        w.set_geometry(1, g).unwrap();
        w.set_object(2, "copies".into(), None, Kind::Instanced { geometry: 1, procedural: false, materials: vec![1], capacity: 4 }, true, false).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(9, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_settings(world::Settings { camera: Some(9), ..Default::default() });
        let m = [Mat4::IDENTITY, Mat4::from_translation(Vec3::X), Mat4::from_scale(Vec3::ZERO), Mat4::from_translation(Vec3::Y)];
        w.set_instances(2, 3, Some(m.iter().flat_map(|m| m.to_cols_array()).collect()), Some(vec![0.5; 12]), None).unwrap();
        let sc = build(&w, Path::new("."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!((sc.meshes.len(), sc.instances.len(), sc.tris.len(), sc.objects.len() / 4), (1, 4, 0, 4), "tinted: 4 vec4s an object");
        assert_eq!(sc.instances.iter().map(|i| (i.sub, i.hidden)).collect::<Vec<_>>(), vec![(Some(0), false), (Some(1), false), (Some(2), true), (Some(3), true)]);
        assert_eq!((sc.objects[4][0], sc.objects[4 + 3]), (1.0, [0.5, 0.5, 0.5, 0.0]));
        assert!(sc.tinted && sc.retained.instanced[&2].capacity == 4);
        let mut m = m.map(|m| m.to_cols_array()).concat();
        m[0] = -1.0;
        w.set_instances(2, 3, Some(m), None, None).unwrap();
        let sc = build(&w, Path::new("."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!((sc.instances.len(), sc.tris.len(), sc.tri_instances.iter().map(|r| r.2).collect::<Vec<_>>()), (0, 2, vec![0, 1]), "mirrored: the drawn copies flattened");
    }

    #[test]
    fn node_ranges_returns_every_range() {
        let nodes = vec![("lamp".to_string(), 10, 12), ("box".to_string(), 12, 20), ("lamp".to_string(), 40, 90)];
        assert_eq!(node_ranges(&nodes, "lamp").collect::<Vec<_>>(), vec![10..12, 40..90]);
        assert_eq!(node_ranges(&nodes, "box").collect::<Vec<_>>(), vec![12..20]);
        assert_eq!(node_ranges(&nodes, "none").count(), 0);
    }

    /// More volumes shown than the kernel takes (32): the scene is refused, not drawn without the last ones; hidden
    /// volumes do not count.
    #[test]
    fn a_scene_past_the_kernels_volumes_is_refused() {
        let build_with = |n: u32, hidden: u32| {
            let mut w = World::default();
            w.set_volume_fn("f", "o.extinction = 1.0;".into(), String::new());
            let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
            w.set_object(1000, "camera".into(), None, world::Kind::Camera(cam), false, false).unwrap();
            w.set_settings(world::Settings { camera: Some(1000), ..Default::default() });
            for id in 1..=n + hidden {
                let kind = world::Kind::Volume { function: "f".into(), params: vec![], shape: 0, majorant: 1.0, g: 0.0, homogeneous: false, animated: false, grid: None };
                w.set_object(id, format!("v{id}"), None, kind, false, false).unwrap();
                if id > n {
                    w.set_visible(id, false).unwrap();
                }
            }
            build(&w, Path::new("."), &crate::default_presets_dir(), None)
        };
        assert_eq!(build_with(MAX_MEDIA as u32, 0).unwrap().media.len(), MAX_MEDIA);
        let e = build_with(MAX_MEDIA as u32 + 1, 0).err().expect("33 volumes refused");
        assert!(format!("{e:#}").contains("more than 32 volumes"), "{e:#}");
        assert_eq!(build_with(MAX_MEDIA as u32, 4).unwrap().media.len(), MAX_MEDIA, "hidden ones do not count");
        // Hidden by their own `visible`, they keep a place while there is room after those shown (shown in place).
        let b = build_with(MAX_MEDIA as u32 - 2, 4).unwrap();
        assert_eq!((b.media.len(), b.media.iter().filter(|m| m.a[1] == MEDIUM_HIDDEN).count()), (MAX_MEDIA, 2));
    }
}

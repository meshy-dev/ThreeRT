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
/// three.js MeshBasicMaterial (`desc::Material::unlit`; the kernel's F_UNLIT).
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

/// An instanced object's copy's entry: without power when the copy emits and is hidden (its emitters stay in the light
/// list, at no weight until it is shown).
pub fn copy_row(mut row: ObjectRow, dark: bool) -> ObjectRow {
    if dark {
        row[0][3] = 0.0;
    }
    row
}

/// A mesh its instances place: its triangles once, in model space, in the `pool_*` arrays; one BLAS, one TLAS
/// instance per placement. A geometry's (`section_mesh`) or a model's (`model_sections`); its materials are its
/// instances' (`Instance::rows`).
#[derive(Clone, Copy, Debug)]
pub struct InstMesh {
    pub first: u32,
    pub count: u32,
    /// A model's second mesh, of the triangles that lie on another's vertices (`split_coincident`), or `NO_OVER`:
    /// every placement of the model has an instance of each.
    pub over: u32,
    /// Model-space bounds (object bounds of its instances, picking).
    pub lo: Vec3A,
    pub hi: Vec3A,
    /// Its triangles change (a posed skinned model's, `Skinned`): its BLAS is rebuilt with them.
    pub deforms: bool,
    /// Its sections (`Section`), in `Scene::sections`: a geometry of its BLAS each, a row of its instances each.
    pub sect_first: u32,
    pub sect_count: u32,
    /// Its triangles carry its geometry's vertex colours (a material says whether it reads them).
    pub colored: bool,
    /// Its vertices (indexed.rs): the first and how many the run holds. Its triangles' indices count from the first;
    /// `prepare` sets them, or the edit that places the mesh.
    pub vfirst: u32,
    pub vcount: u32,
}

/// No second mesh (`InstMesh::over`).
pub const NO_OVER: u32 = u32::MAX;

/// A run of a mesh's triangles that an object draws with one of its materials: a geometry group, or the whole of a
/// geometry drawn with one material. A geometry of the mesh's BLAS; an instance has a row for each (`Instance::rows`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    /// Its first triangle in the mesh, and how many.
    pub first: u32,
    pub count: u32,
    /// Which of the object's materials (a group's material index; past the last, the last).
    pub slot: u32,
}

/// The most sections a mesh may have: a hit's section is 16 bits of the record a pixel is left to the kernel's second
/// pass by (pathtrace.wgsl `finish`).
pub const MESH_SECTIONS: usize = 1 << 16;

/// An instance's row for a section of its mesh, as the kernel reads it (the words' heap, heap.rs `Stream::Rows`):
/// the material with the class bits `ROW_*`, the section's first triangle in the triangles' table, its first emitter in
/// the light list (`NO_EMITTER`: it does not emit), and a word the kernel does not read (`ROW_UNSAMPLED`).
pub type Row = [u32; 4];
pub const ROW_MATERIAL: u32 = 0xff_ffff;
/// The material is alpha-tested; it is glass (shadow rays pass).
pub const ROW_CUTOUT: u32 = 1 << 24;
pub const ROW_GLASS: u32 = 1 << 25;
pub const NO_EMITTER: u32 = u32::MAX;
/// In a row's last word: the section's triangles are not in the light list although its material may emit (a
/// particle system the lights are not sampled by: its emission reaches surfaces by their BSDF rays alone).
pub const ROW_UNSAMPLED: u32 = 1;
/// Whether a row's section is in the light list.
fn row_emits(r: &Row, materials: &[GpuMaterial]) -> bool {
    r[3] & ROW_UNSAMPLED == 0 && materials.get(row_material(r) as usize).is_some_and(emissive)
}

/// The row of a section drawn with material `mat`; where its triangles and its emitters are is set where the tables
/// are laid out (lib.rs `prepare`, the live edits).
pub fn row_of(mat: u32, materials: &[GpuMaterial]) -> Row {
    let class = materials.get(mat as usize).map_or(0, |m| [0, ROW_CUTOUT, ROW_GLASS][mesh_class(m) as usize]);
    [mat | class, 0, NO_EMITTER, 0]
}

/// The material a row names.
pub fn row_material(r: &Row) -> u32 {
    r[0] & ROW_MATERIAL
}

/// Whether an instance with these rows casts shadows: as its object says, and never when every section is glass
/// (such an instance is out of a shadow ray's way by its mask; glass among other sections is passed at the hit).
pub fn rows_cast(rows: &[Row], cast_shadow: bool) -> bool {
    cast_shadow && !rows.iter().all(|r| r[0] & ROW_GLASS != 0)
}

/// The bits of an instance's record for its rows: which classes are among them.
pub fn rows_bits(rows: &[Row]) -> u32 {
    let any = |bit: u32| rows.iter().any(|r| r[0] & bit != 0);
    (if any(ROW_CUTOUT) { INST_CUTOUT } else { 0 }) | if any(ROW_GLASS) { INST_GLASS } else { 0 }
}

/// How many emitters an instance with these rows has in the light list: every triangle of its sections whose
/// material emits, a section after another, a section's in its triangles' order (`section_emitters`).
pub fn rows_emitters(sections: &[Section], rows: &[Row], materials: &[GpuMaterial]) -> u32 {
    sections.iter().zip(rows).filter(|(_, r)| row_emits(r, materials)).map(|(s, _)| s.count).sum()
}

/// A triangle's place among its instance's emitters, its material and its section, from its attributes (its
/// section and its index in it), the instance's rows and where each section's emitters start (`section_emitters`).
/// None: its section's material does not emit.
pub fn section_slot(a: &GpuTriAttr, rows: &[Row], starts: &[Option<u32>]) -> Option<(usize, u32, u32)> {
    let s = a.uv[3].to_bits() as usize;
    Some(((starts.get(s).copied()?? + a.x[3].to_bits()) as usize, row_material(rows.get(s)?), s as u32))
}

/// Where each section's emitters start among its instance's (None: its material does not emit).
pub fn section_emitters(sections: &[Section], rows: &[Row], materials: &[GpuMaterial]) -> Vec<Option<u32>> {
    let mut at = 0;
    sections
        .iter()
        .zip(rows)
        .map(|(s, r)| {
            row_emits(r, materials).then(|| {
                at += s.count;
                at - s.count
            })
        })
        .collect()
}

/// Pool triangles of the deforming meshes, which the build puts first in the pool.
pub fn deforming_pool(meshes: &[InstMesh]) -> usize {
    meshes.iter().filter(|m| m.deforms).map(|m| m.count as usize).sum()
}

impl InstMesh {
    /// Its BLAS geometries (`sections`: the scene's): one a section, its first triangle in the mesh and how many.
    /// Every one is flagged opaque, whatever an instance's materials are (a query forces opacity; a cutout's and
    /// glass's hits are told by the instance's rows).
    pub fn parts(&self, sections: &[Section]) -> Vec<(u32, u32)> {
        self.sections(sections).iter().map(|s| (s.first, s.count)).collect()
    }

    /// Its sections among the scene's.
    pub fn sections<'a>(&self, sections: &'a [Section]) -> &'a [Section] {
        &sections[self.sect_first as usize..(self.sect_first + self.sect_count) as usize]
    }

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
    /// Hidden by its object's visibility (`World::shown`) or for a placement without size (`placed_world`), which
    /// keep its mesh and its place above the triangles: it comes back in place. (One that an edit took out is hidden
    /// and not this: a rebuild draws it again.)
    pub masked: bool,
    /// Its first emitter in the light list, if it has emitters (`rows_emitters`): set where the list is made (lib.rs
    /// `prepare`, the live edits). A section's row has where the section's start, for the rays that hit one.
    pub first_emitter: u32,
    /// Its rows in `Scene::rows`, one a section of its mesh (`InstMesh::sect_count`), and the record's bits for them
    /// (`rows_bits`).
    pub rows: u32,
    pub bits: u32,
    /// An instance of a mesh that deforms: where its vertices' positions of the previous frame begin (the kernel's
    /// `prev_verts`: its motion is its pose's and its transform's). 0 otherwise.
    pub prev: u32,
    /// Its mesh's first vertex (`indexed::Indexed::meshes`): its triangles' indices count from it.
    pub verts: u32,
}

/// Kernel record of an instance (`instances` binding): world matrix rows, normal matrix rows (inverse transpose),
/// (first triangle in the GPU order, object index, its mesh's first vertex, its rows' first), and the
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
    /// Kernel record with its mesh's first triangle at `first` in the GPU order. (n0.w is left free: a unit's record
    /// takes it for its tree's root, units.rs `unit_record`. n1.w: its bits. n2.w: `prev`, as bits.)
    pub fn gpu(&self, first: u32) -> GpuInstance {
        let w = self.world;
        let nm = glam::Mat3::from_mat4(w).inverse().transpose();
        let row = |r: usize| [w.x_axis[r], w.y_axis[r], w.z_axis[r], w.w_axis[r]];
        let nrow = |r: usize, w: f32| [nm.x_axis[r], nm.y_axis[r], nm.z_axis[r], w];
        let bits = f32::from_bits(self.bits | if mirrors(w) { INST_MIRRORED } else { 0 });
        GpuInstance { m: [row(0), row(1), row(2)], n: [nrow(0, 0.0), nrow(1, bits), nrow(2, f32::from_bits(self.prev))], info: [first, self.object, self.verts, self.rows], p: [row(0), row(1), row(2)] }
    }
    /// A triangle's attributes as a flattened triangle has them: the material its row names in place of the
    /// section, and no index in it (`rows`: the instance's).
    pub fn resolved(&self, a: &GpuTriAttr, rows: &[Row]) -> GpuTriAttr {
        let mat = rows.get(a.uv[3].to_bits() as usize).map_or(0, row_material);
        GpuTriAttr { uv: [a.uv[0], a.uv[1], a.uv[2], f32::from_bits(mat)], x: [a.x[0], a.x[1], a.x[2], 0.0], ..*a }
    }
    /// Model-space triangle `t` of its mesh in world space (the same transform the flattening path applies).
    pub fn tri(&self, t: &[Vec3A; 3]) -> [Vec3A; 3] {
        t.map(|v| self.world.transform_point3a(v))
    }
    /// A triangle of its mesh (vertices `t`, attributes `a`, vertex colours `c`) as flattening has it: in world space
    /// (`tri`, `instance_attr`), and under a mirroring instance with its second and third vertex changed places, so
    /// that it faces as the mesh's does (as `append_arrays` flattens a mirrored placement).
    pub fn flat(&self, t: &[Vec3A; 3], a: &GpuTriAttr, c: Option<[u32; 3]>) -> ([Vec3A; 3], GpuTriAttr, Option<[u32; 3]>) {
        let (w, b) = (self.tri(t), instance_attr(a, self.world, self.object));
        if !mirrors(self.world) {
            return (w, b, c);
        }
        // (The UVs are packed along the normals: uv0 in n0.w n1.w, uv1 in n2.w uv.x, uv2 in uv.y uv.z.)
        let swapped = GpuTriAttr { n1: [b.n2[0], b.n2[1], b.n2[2], b.n1[3]], n2: [b.n1[0], b.n1[1], b.n1[2], b.uv[1]], uv: [b.uv[2], b.n2[3], b.uv[0], b.uv[3]], t1: b.t2, t2: b.t1, ..b };
        ([w[0], w[2], w[1]], swapped, c.map(|c| [c[0], c[2], c[1]]))
    }
}

/// A triangle's tangents, one a corner: the direction of u on the surface and the frame's hand (1 or -1). A file's,
/// or generated (tangents.rs); a triangle without has none (`GpuTriAttr::t`'s hand is 0).
pub type CornerTangents = [(Vec3, f32); 3];

/// A triangle in a node's own space (vertices, normals, UVs, tangents) placed by `w` as the build flattens it: under
/// a mirroring `w` the second and third vertex change places, so that it faces as its mesh does, and its tangents'
/// frame changes hand (`Instance::flat` does the same with a mesh's triangle).
#[allow(clippy::type_complexity)]
pub fn placed_tri(w: Mat4, p: &[Vec3; 3], n: &[Vec3; 3], uv: &[Vec2; 3], tangent: Option<CornerTangents>) -> ([Vec3; 3], [Vec3; 3], [Vec2; 3], Option<CornerTangents>) {
    let nm = Mat3::from_mat4(w).inverse().transpose();
    let (k, hand) = if mirrors(w) { ([0, 2, 1], -1.0) } else { ([0, 1, 2], 1.0) };
    (k.map(|i| w.transform_point3(p[i])), k.map(|i| (nm * n[i]).normalize_or_zero()), k.map(|i| uv[i]), tangent.map(|t| k.map(|i| (w.transform_vector3(t[i].0).normalize_or_zero(), t[i].1 * hand))))
}

/// Bit of an instance record's n1.w, beside the units' flags there (units.rs `UNIT_*`): the instance mirrors its mesh
/// (the kernel's `inst_sign`).
pub const INST_MIRRORED: u32 = 4;
/// More bits there: a row of the instance is a cutout's, a glass's (what a walk of a unit asks before it reads a
/// triangle's row).
pub const INST_CUTOUT: u32 = 16;
pub const INST_GLASS: u32 = 32;
/// Its mesh deforms (a posed skinned model's): its motion is its vertices' too (`Instance::prev`).
pub const INST_DEFORMS: u32 = 64;
/// Its mesh is a particle system's (`ParticleSystem::mesh`): a quad a particle, and an object a particle from the
/// record's on (a hit's object is its triangle's index in the mesh / 2 past it).
pub const INST_PARTICLES: u32 = 128;
/// Every bit of n1.w that is the instance's own, not a unit's.
pub const INST_BITS: u32 = INST_MIRRORED | INST_CUTOUT | INST_GLASS | INST_DEFORMS | INST_PARTICLES;

/// Whether a placement mirrors what it places (a negative determinant).
pub fn mirrors(w: Mat4) -> bool {
    glam::Mat3::from_mat4(w).determinant() < 0.0
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
    if i < t.count as usize {
        return placed_world(w, m.w_axis.truncate());
    }
    (at_point(if w.w_axis.is_finite() { w.w_axis.truncate() } else { m.w_axis.truncate() }), true)
}

/// An instance's world matrix as the structure above the triangles takes it, and whether the placement has no size:
/// not finite, or not invertible (scene code hides by scaling to 0). Such an instance stands at a point (its
/// translation, or `fallback`), hidden.
pub fn placed_world(w: Mat4, fallback: Vec3) -> (Mat4, bool) {
    if w.is_finite() && glam::Mat3::from_mat4(w).determinant().abs() > 1e-30 {
        return (w, false);
    }
    (at_point(if w.w_axis.is_finite() { w.w_axis.truncate() } else { fallback }), true)
}

fn at_point(at: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(Vec3::splat(1e-6), glam::Quat::IDENTITY, at)
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

/// Flags of an instance's pool triangle as a world-space triangle (software traversal): the placement's shadow flag,
/// the material's cutout, and no shadow for glass (the build's flag pass ran before the instances were flattened).
pub fn instance_tri_flags(inst: &Instance, a: &GpuTriAttr, materials: &[GpuMaterial]) -> u32 {
    let m = materials.get(a.uv[3].to_bits() as usize);
    let shadow = if inst.cast_shadow && !m.is_some_and(|m| m.layer[0] > 0.0) { 0 } else { TRI_NO_SHADOW };
    shadow | if m.is_some_and(|m| m.uv2[3] > 0.0) { TRI_MASKED } else { 0 }
}

/// Triangle ranges of node `id` in build order: one, or two for an instanced node flattened by software traversal (its
/// glass / function part from the build, its mesh appended by `prepare`).
pub fn node_ranges<'a>(tri_nodes: &'a [(String, u32, u32)], id: &'a str) -> impl Iterator<Item = std::ops::Range<usize>> + 'a {
    tri_nodes.iter().filter(move |(n, _, _)| n == id).map(|(_, a, b)| *a as usize..*b as usize)
}

/// Model-space shading attributes of a pool triangle placed by `w` (normals by the inverse transpose, tangents by
/// the matrix, their frame's hand changed by a mirroring one), object `object`.
pub fn instance_attr(a: &GpuTriAttr, w: Mat4, object: u32) -> GpuTriAttr {
    let m3 = glam::Mat3::from_mat4(w);
    let nm = m3.inverse().transpose();
    let n = |v: [f32; 4]| {
        let r = (nm * Vec3::new(v[0], v[1], v[2])).normalize_or_zero();
        [r.x, r.y, r.z, v[3]]
    };
    let flip = if mirrors(w) { -1.0 } else { 1.0 };
    let t = |v: [f32; 4]| {
        let r = (m3 * Vec3::new(v[0], v[1], v[2])).normalize_or_zero();
        [r.x, r.y, r.z, v[3] * flip]
    };
    GpuTriAttr { n0: n(a.n0), n1: n(a.n1), n2: n(a.n2), uv: a.uv, t: t(a.t), x: [a.x[0], f32::from_bits(object), a.x[2], a.x[3]], t1: t(a.t1), t2: t(a.t2) }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTriAttr {
    pub n0: [f32; 4], // normal0, w = uv0.x
    pub n1: [f32; 4], // normal1, w = uv0.y
    pub n2: [f32; 4], // normal2, w = uv1.x
    pub uv: [f32; 4], // uv1.y, uv2.x, uv2.y, material (bits)
    pub t: [f32; 4],  // its first corner's tangent (dP/du), w = the frame's hand (0: the triangle has no tangents)
    pub x: [f32; 4],  // -, object index (bits), -, its emitter entry + 1 (bits; 0: not light-sampled)
    pub t1: [f32; 4], // its second corner's tangent
    pub t2: [f32; 4], // its third corner's
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMaterial {
    pub base_color: [f32; 4], // rgb, opacity
    pub emissive: [f32; 4],   // rgb nits
    pub params: [f32; 4],     // metallic, roughness, specular, normal scale
    pub tex0: [i32; 4],       // base, normal, roughness(G), metalness(B)
    pub tex1: [i32; 4],       // emissive, occlusion(R), flags, material function + 1 (0 = none)
    pub uv: [f32; 4],         // uv repeat xy, transmission texture (R) + 1 (0 = none; glTF), 1 = it reads vertex colours
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

/// The environment beside its sky, as the world describes it: one derivation (`of`) for a build and for an edit, one
/// place that says what the kernel takes from it (`write`, `features`).
#[derive(Clone, Debug)]
pub struct EnvSettings {
    pub fog: Option<desc::Fog>,
    /// three.js `scene.background` (`desc::Environment::background`): the kernel's F_BACKGROUND, `Params::background`.
    pub background: Option<[f32; 3]>,
    /// The sky's aerial perspective (`aerial_params`): the kernel's F_AERIAL, `Params::aerial`.
    pub aerial: [f32; 4],
    /// Global wetness and rain intensity (0..1).
    pub wetness: f32,
    pub rain: f32,
    /// Cloud cover (0..1) of an atmosphere sky: the sky is baked with it (`bake_sky`).
    pub cloud_cover: f32,
}

impl EnvSettings {
    pub fn of(w: &World) -> EnvSettings {
        let e = &w.environment;
        EnvSettings { fog: e.fog.clone(), background: e.background, aerial: aerial_params(&e.sky), wetness: e.wetness.clamp(0.0, 1.0), rain: e.rain.clamp(0.0, 1.0), cloud_cover: e.cloud_cover.clamp(0.0, 1.0) }
    }

    /// The kernel's parameters for them.
    pub fn write(&self, p: &mut crate::gpu::Params) {
        p.time[1] = self.wetness;
        p.time[2] = self.rain;
        p.background = self.background.map_or([0.0; 4], |c| [c[0], c[1], c[2], 1.0]);
        p.aerial = self.aerial;
        [p.fog, p.fog_color, p.vol, p.vol2] = crate::fog_params(self.fog.as_ref());
    }

    /// The kernel features they ask for (`gpu::environment_features`).
    pub fn features(&self) -> u32 {
        crate::gpu::environment_features(self.fog.as_ref(), self.background.is_some(), self.aerial)
    }
}

pub enum SkyData {
    None,
    Color(Vec3),
    /// `dynamic`: baked at `sky::DYNAMIC_SKY` (the live view re-bakes it as the sun moves); `glow`: night-sky radiance.
    Atmosphere { sun_dir: Vec3, sun_lux: Vec3, rayleigh: f32, mie: f32, g: f32, ground: Vec3, dynamic: bool, glow: crate::sky::Glow },
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
    pub attrs: Vec<GpuTriAttr>,
    pub materials: Vec<GpuMaterial>,
    pub textures: Vec<Texture>,
    pub lights: Vec<GpuLight>,
    pub camera: CameraData,
    pub sky: SkyData,
    /// The environment beside its sky (`EnvSettings::of`).
    pub environment: EnvSettings,
    pub preset: desc::Preset,
    pub stats: HashMap<&'static str, f64>,
    /// Triangle ranges per object (name, first, end), in `tris` order: picking and selection outlines.
    pub tri_nodes: Vec<(String, u32, u32)>,
    /// Triangle ranges of flattened instances of instanced objects (first, end, instance index): picking.
    pub tri_instances: Vec<(u32, u32, u32)>,
    /// Some object has instance colours (`InstanceTable::colors`): the kernel's F_TINT, 4 vec4s an object
    /// (`object_stride`).
    pub tinted: bool,
    /// Instanced static glTF meshes (model-space triangles, flags, attributes) and their placements. `prepare` appends
    /// the pool after the flattened triangles (hardware RT) or flattens the instances (software traversal).
    pub pool_tris: Vec<[Vec3A; 3]>,
    pub pool_attrs: Vec<GpuTriAttr>,
    pub meshes: Vec<InstMesh>,
    pub instances: Vec<Instance>,
    /// What the skinning pass reads of the models whose meshes deform (`Skinned::source`): a model's triangles as
    /// its file has them, in its mesh's order. One a model, whatever its placements and poses.
    pub skin_sources: Vec<Vec<[skin::SkinCorner; 3]>>,
    /// Beside each: the model's palette as its file has it (its bind pose), and, under software traversal with units
    /// (lib.rs `unit_trees`), the tree over that pose's triangles: the shape of the tree of each mesh skinned from the
    /// source, whose boxes are taken again at each pose (`units::refit`). The source's triangles are in its order.
    pub skin_rest: Vec<Vec<skin::PaletteRows>>,
    pub skin_trees: Vec<Vec<crate::bvh::GpuNode>>,
    /// The triangles as shared vertices and indices (indexed.rs), in the GPU's order: `prepare` makes them.
    pub indexed: crate::indexed::Indexed,
    /// The meshes' sections (`InstMesh::sect_first`) and the instances' rows (`Instance::rows`): as the kernel reads
    /// the rows once `prepare` has set where each section's triangles and emitters are.
    pub sections: Vec<Section>,
    pub rows: Vec<Row>,
    /// Kernel records of `instances` (`prepare`, hardware RT).
    pub gpu_instances: Vec<GpuInstance>,
    /// Where engine handles landed in the GPU tables, for edits without a rebuild (`LiveCore::commit`).
    pub retained: Retained,
    /// Particle systems (a mesh each, whose vertices a GPU pass places every frame) and the particle functions they run:
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
    /// The kernel's procedural region and records (`prepare`: `proc_layout`, `ProcObject::gpu`); the region is a
    /// section of the words' heap (heap.rs `Stream::Procs`), the records follow the model instances' in `instances`.
    pub proc_layout: Option<ProcLayout>,
    pub proc_records: Vec<GpuInstance>,
    /// Software traversal's units (`prepare`: the instances it keeps as instances; units.rs), None under hardware
    /// traversal and for a scene without any.
    pub units: Option<crate::units::Units>,
}

impl Scene {
    /// The materials of the translucent particle systems (the translucency pass's: its kernel has their functions
    /// alone), each once, in order.
    pub fn translucent_mats(&self) -> Vec<u32> {
        let mut m: Vec<u32> = vec![];
        for p in self.particles.iter().filter(|p| p.translucent) {
            m.extend(self.instances.iter().filter(|i| i.mesh == p.mesh).map(|i| row_material(&self.rows[i.rows as usize])));
        }
        m.sort_unstable();
        m.dedup();
        m
    }

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
    /// A model file's material -> `materials` index: by (the file, with its functions if it has any; the material's
    /// index in the file, usize::MAX for the file's default one).
    pub asset_materials: HashMap<(String, usize), u32>,
    /// Object -> where what it draws landed (`Proxy`): meshes, models, rect-light cards, instanced objects.
    pub proxies: HashMap<u32, Proxy>,
    /// Object of each `lights` entry, same order (`prepare` sorts both).
    pub light_nodes: Vec<u32>,
    /// Atmosphere sun: its colour comes from the sky bake.
    pub sun: Option<u32>,
    /// Posed skinned models' meshes (`Skinned`: a model's proxy names its own, `MeshKind::Skinned`). One an edit took
    /// out again has no mesh (`u32::MAX`) until the next one placed takes its slot.
    pub skinned: Vec<Skinned>,
    /// Posed skinned models an edit can place another of: (uri, nodes) -> one of `skinned` that the build placed, with
    /// the file's own materials (what the pass skins it from, its mesh's sections and rows are the model's).
    pub skinned_model: HashMap<(String, Option<Vec<String>>), u32>,
    /// Meshes and instanced objects: (geometry, whether by its groups: `by_groups`) -> the shared mesh (objects of the
    /// same geometry share its BLAS, whatever their materials: those are their instances' rows). live/edits.rs
    /// `edit_geometry` keeps it.
    pub own_mesh: HashMap<(u32, bool), u32>,
    /// Whether the scene said a mesh's geometry will be edited or driven (`own_blas`, `driven`): it gets room for
    /// edits in place (gpu.rs `EditRoom`).
    pub edits_meshes: bool,
    /// Static models' instanced meshes by (uri, nodes), those with nothing flattened per placement: a model added in
    /// place is another instance of one (live/edits.rs `edit_geometry`).
    pub model_mesh: HashMap<(String, Option<Vec<String>>), u32>,
    /// Posed rigid models' parts by (uri, `rigid_pose_key`), each its node (`STILL_NODES`) and its mesh, where every
    /// part is a mesh with nothing flattened beside: one added in place is an instance of each.
    pub rigid_meshes: HashMap<(String, String), Vec<(usize, u32)>>,
    /// A model's mesh's rows as every instance of it has them (the file's materials, `model_sections`; where a
    /// section's triangles and emitters are is each instance's to fill).
    pub mesh_rows: HashMap<u32, Vec<Row>>,
}

/// Where what one object of the world draws landed in the GPU's tables: its entry in the objects table and how its
/// geometry is held. Edits in place go through it (live/edits.rs `LiveCore::commit`).
#[derive(Clone, Default)]
pub struct Proxy {
    /// Object-table index (pivot, bounds, power, customData). None: an instanced object (an entry a copy: from
    /// `InstancedPlace::object` for instances; a flattened one's are the build's alone).
    pub object: Option<u32>,
    /// A rect light's visible card: its material.
    pub card: Option<u32>,
    pub draw: Draw,
}

/// How a drawn object's geometry is held.
#[derive(Clone, Default)]
pub enum Draw {
    /// As the build placed it, with nothing kept to change it by: triangles flattened into the static groups, a
    /// static model's instance. An edit that would change it rebuilds.
    #[default]
    Fixed,
    /// World-space triangles in the dynamic groups, re-transformed (a movable object) or re-skinned (a posed model)
    /// on the CPU.
    Dynamic(DynamicModel),
    /// An instance of a model-space mesh (hardware RT, or software traversal's units).
    Instance(InstanceRef),
    /// Procedural geometry: its index in `procs` (its record less the first procedural record).
    Procedural(u32),
    /// A posed rigid model split by node: a new pose moves its nodes' instances.
    Rigid(RigidModel),
    /// An instanced object's copies (live/edits.rs `update_instanced`).
    Instanced(InstancedPlace),
}

/// One instance of a mesh: its index in `instances`, which is its record (the build's, or one of the edit room's).
#[derive(Clone, Copy, Debug)]
pub struct InstanceRef {
    pub instance: u32,
    pub mesh: MeshKind,
    /// A move is its transform's, not its triangles' (a mesh a body drives, a movable model the build placed).
    pub moves: bool,
}

/// What mesh an instance is of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MeshKind {
    /// A model's.
    Model,
    /// A posed skinned model's own (`Retained::skinned`): skinned again at a new pose; the model moves as its instance.
    Skinned(u32),
    /// A geometry's mesh, shared by the objects of the geometry (`Retained::own_mesh`).
    Own,
    /// The rect lights' card (`card_mesh`).
    Card,
}

impl MeshKind {
    /// An `own_blas` mesh: what a geometry edit makes and places again (a model's is the build's).
    pub fn own(self) -> bool {
        matches!(self, MeshKind::Own)
    }
}

impl Retained {
    /// The object's proxy, made if it has none yet (the build, an edit that places it).
    pub fn proxy(&mut self, id: u32) -> &mut Proxy {
        self.proxies.entry(id).or_default()
    }

    /// Its object-table index.
    pub fn object(&self, id: u32) -> Option<u32> {
        self.proxies.get(&id)?.object
    }

    /// Its instance, if it is one of a mesh.
    pub fn instance(&self, id: u32) -> Option<&InstanceRef> {
        match &self.proxies.get(&id)?.draw {
            Draw::Instance(p) => Some(p),
            _ => None,
        }
    }

    pub fn instanced(&self, id: u32) -> Option<&InstancedPlace> {
        match &self.proxies.get(&id)?.draw {
            Draw::Instanced(p) => Some(p),
            _ => None,
        }
    }

    pub fn rigid(&self, id: u32) -> Option<&RigidModel> {
        match &self.proxies.get(&id)?.draw {
            Draw::Rigid(r) => Some(r),
            _ => None,
        }
    }

    /// Whether any object's move is an instance's transform (a movable or driven instance, a rigid model's nodes).
    pub fn moves_instances(&self) -> bool {
        self.proxies.values().any(|p| matches!(&p.draw, Draw::Rigid(_)) || matches!(&p.draw, Draw::Instance(i) if i.moves))
    }

    /// The records of the instances an edit moves in place: a mesh's or a model's that moves (a body's, a movable
    /// one's), a posed rigid model's parts that move (`RigidModel::moving`), an instanced object's copies.
    pub fn moving_records(&self) -> std::collections::HashSet<u32> {
        self.proxies
            .values()
            .flat_map(|p| match &p.draw {
                Draw::Instance(i) if i.moves => vec![i.instance],
                Draw::Rigid(r) => r.moving(),
                Draw::Instanced(c) if c.mesh.is_some() => (c.first..c.first + c.capacity).collect(),
                _ => vec![],
            })
            .collect()
    }

    /// Whether any object is a mesh the scene edits or drives (what geometry edits place again).
    pub fn has_own_instances(&self) -> bool {
        self.edits_meshes
    }

    /// The triangles re-transformed on the CPU: the dynamic objects' and the rigid models' flattened ones.
    pub fn dynamics_mut(&mut self) -> impl Iterator<Item = &mut DynamicModel> {
        self.proxies.values_mut().filter_map(|p| match &mut p.draw {
            Draw::Dynamic(d) => Some(d),
            _ => None,
        })
    }
}

/// Where an instanced object's instances are: `capacity` of them from `first` in `instances` (instances of `mesh`) or
/// in `procs` (procedural copies: no mesh), with object-table entries from `object`, one each; the first `drawn`
/// were last written with their matrices (the rest are hidden, and an update leaves them be).
#[derive(Clone, Copy, Debug)]
pub struct InstancedPlace {
    pub first: u32,
    pub capacity: u32,
    pub object: u32,
    pub mesh: Option<u32>,
    pub drawn: u32,
}

/// A posed glTF model without skins whose clips move some of its nodes (Zero Day's 551 moving objects), in parts
/// (`RigidPart`), each an instance of a mesh every placement of the model shares: the nodes no clip of the file
/// moves and the pose does not name merged into one mesh in the model's space, placed by the object's transform;
/// each other node a mesh in the node's space, placed at object x node transform, so a new pose, of whichever clip,
/// moves instances only (a part's emitters are its
/// instance's in the light list, filled again when it moves). What `split_coincident` takes out of a part's mesh
/// is the part's second mesh, an instance beside the part's (with its node, in `instances`).
#[derive(Clone, Default)]
pub struct RigidModel {
    /// (glTF node, instance index): the nodes a pose may move; and a part's second mesh's (its node `STILL_NODES`
    /// where the part is the nodes left alone: `part_world`).
    pub instances: Vec<(usize, u32)>,
    /// The instance of the nodes it leaves alone, if any is drawn. It moves with the object alone; its emitters are
    /// still ones until the object first moves (`still_moves`: live/edits.rs `repose_rigid`).
    pub still: Option<u32>,
    pub still_moves: bool,
    /// The object's transform its parts were last placed by, as `World::world_matrix` gives it (an update compares
    /// with it to tell a move from a new pose).
    pub world: Mat4,
    /// `rigid_pose_key` of the pose its parts were made for: a pose with another is other parts (a build's to make).
    pub pose: String,
}

/// In place of a node: the nodes no pose of a rigid model moves, merged (their space is the model's).
pub const STILL_NODES: usize = usize::MAX;

/// What a rigid model's parts go by, beside the model: the nodes its pose names (`skin::Model::moving_nodes`: they
/// are parts with the nodes the file's clips move, whichever clip plays).
pub fn rigid_pose_key(a: &desc::Animation) -> String {
    a.nodes.keys().cloned().collect::<Vec<_>>().join("\u{0}")
}

impl RigidModel {
    /// The records of all its instances.
    pub fn records(&self) -> Vec<u32> {
        self.instances.iter().map(|&(_, i)| i).chain(self.still).collect()
    }

    /// Those whose emitters are moving ones: its nodes', and the still part's once the object has moved.
    pub fn moving(&self) -> Vec<u32> {
        self.instances.iter().map(|&(_, i)| i).chain(self.still.filter(|_| self.still_moves)).collect()
    }

    /// A part's transform for the object's `m` and the pose's node transforms.
    pub fn part_world(m: Mat4, node: usize, globals: &[Mat4]) -> Mat4 {
        if node == STILL_NODES { m } else { m * globals[node] }
    }
}

/// A part of a rigid model as the build makes it once for the placements of the model with a clip: its node
/// (`STILL_NODES`) and its mesh (None: it has no triangle); the mesh has what `split_coincident` took out
/// (`InstMesh::over`).
struct RigidPart {
    node: usize,
    mesh: Option<u32>,
}

/// A posed skinned model's mesh: the model's triangles in its own space, a mesh a model. The skinning pass skins it at
/// each new pose (gpu.rs `skin`: the CPU sends the pose's matrices) and its BLAS is built again; the model moves as
/// its instance (hardware RT). Never shared: two models in one pose are two meshes (a mesh shared came apart when
/// one of them took another pose, and that was a rebuild). Only meshes without triangles flattened per placement.
#[derive(Clone, Debug)]
pub struct Skinned {
    pub mesh: u32,
    pub uri: String,
    pub nodes: Option<Vec<String>>,
    /// The pose the mesh holds.
    pub animation: desc::Animation,
    /// Per pool triangle of the mesh, in order: its index in the model's triangles, its section and its index in it
    /// (the model's: one list for all its placements).
    pub order: Arc<Vec<(u32, u32, u32)>>,
    /// Its object, and its instance.
    pub object: u32,
    pub instance: u32,
    /// What the skinning pass skins it from (`Scene::skin_sources`), how many matrices its pose is
    /// (`skin::Model::palette`), and the primitives the pose its mesh was made for mirrors (`skin::Model::mirrored`:
    /// a pose that mirrors others has its triangles in another order, a build's to make).
    pub source: u32,
    pub palette_len: u32,
    pub mirrors: Vec<bool>,
}

/// pathtrace.wgsl `Medium`: world -> local affine rows (the unit box / sphere's space), (function, shape or
/// `MEDIUM_HIDDEN`, homogeneous, animated), (majorant 1/m, HG anisotropy, its density grid's first layer in the atlas
/// and its cells, as bits: 0 without one), the node's customData, 32 parameter floats.
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

/// Where density grid `gid` lies in the atlas, as a medium's record has it (b.zw: its first layer and its cells, as
/// bits). The first volume that names a grid has it built (`grids`: stacked along z in the order they come; `node`
/// and `medium`: that volume's), its program by its rank among `programs`. The build and the live view's media
/// table both come here.
pub fn grid_of(w: &World, grids: &mut Vec<GridBuild>, programs: &[(String, String, String)], gid: u32, node: u32, medium: u32) -> Result<[f32; 2]> {
    let k = match grids.iter().position(|b| b.id == gid) {
        Some(k) => k,
        None => {
            let dg = w.grids.get(&gid).ok_or_else(|| anyhow!("unknown density grid {gid}"))?;
            let program = match &dg.program {
                Some((name, _)) => Some(programs.iter().position(|(k, _, _)| k == name).ok_or_else(|| anyhow!("unknown grid program {name}"))? as u32),
                None => None,
            };
            let mut gp = [0.0f32; 32];
            for (k, v) in dg.program.iter().flat_map(|(_, p)| p.iter()).take(32).enumerate() {
                gp[k] = *v;
            }
            let z0 = grids.iter().map(|b| b.dims[2]).sum::<u32>();
            let cells = grids.iter().map(|b| b.dims).chain([dg.dims]).map(|d| d.iter().map(|&d| d as u64).product::<u64>()).sum::<u64>();
            // (The atlas is as wide and high as the widest and highest grid: its texels count too.)
            let atlas = grids.iter().map(|b| b.dims[0]).chain([dg.dims[0]]).max().unwrap() as u64 * grids.iter().map(|b| b.dims[1]).chain([dg.dims[1]]).max().unwrap() as u64 * (z0 + dg.dims[2]) as u64;
            if cells > world::GRID_MAX_SCENE_CELLS || atlas > 2 * world::GRID_MAX_SCENE_CELLS || z0 + dg.dims[2] > world::GRID_MAX_LAYERS {
                return Err(anyhow!("the scene's density grids exceed {} cells or {} layers along z together (or differ too much in width and height)", world::GRID_MAX_SCENE_CELLS, world::GRID_MAX_LAYERS));
            }
            grids.push(GridBuild { id: gid, dims: dg.dims, z0, data: dg.data.clone(), program, params: gp, node, medium, made: dg.made });
            grids.len() - 1
        }
    };
    let b = &grids[k];
    Ok([f32::from_bits(b.z0), f32::from_bits(b.dims[0] | b.dims[1] << 10 | b.dims[2] << 20)])
}

impl MediumGpu {
    /// A volume's record: the object `o` (a `Kind::Volume`) placed by `m`, its function's number, whether it is shown
    /// (a hidden one is built with no shape, to be shown in place), and its density grid's place (`grid`: b.zw).
    pub fn of(o: &world::Object, m: Mat4, function: u32, shown: bool, grid: [f32; 2]) -> Option<MediumGpu> {
        let Kind::Volume { params, shape, majorant, g, homogeneous, animated, .. } = &o.kind else { return None };
        let mut p = [0.0f32; 32];
        for (k, v) in params.iter().take(32).enumerate() {
            p[k] = *v;
        }
        Some(MediumGpu { w: medium_rows(m), a: [function, if shown { *shape } else { MEDIUM_HIDDEN }, *homogeneous as u32, *animated as u32], b: [majorant.max(0.0), g.clamp(-0.99, 0.99), grid[0], grid[1]], c: o.custom, p })
    }
}

/// The world -> local rows of a medium whose node has world matrix `m` (pathtrace.wgsl `Medium.w0..w2`).
pub fn medium_rows(m: glam::Mat4) -> [[f32; 4]; 3] {
    let inv = m.inverse();
    [inv.row(0).to_array(), inv.row(1).to_array(), inv.row(2).to_array()]
}

/// `count` particles, each a quad and its own object (from `object`: pivot, bounds, per-particle data), placed every
/// frame by particle function `func` with `params` (8 vec4). The quads are a mesh of the system's own, which one
/// instance places as the pass writes it.
#[derive(Clone, Debug)]
pub struct ParticleSystem {
    /// Its mesh among the instanced ones: four vertices and two triangles a particle, written by the pass in the
    /// world (its instance does not place it).
    pub mesh: u32,
    /// A particle's nominal area where the lights are sampled by the system (m²; 0: they are not).
    pub emitter_area: f32,
    /// Drawn by the translucency pass: its instance is hidden from the scene's acceleration structure and is in
    /// that pass's.
    pub translucent: bool,
    pub count: u32,
    pub object: u32,
    pub func: u32,
    pub params: [f32; 32],
    /// Seed of the particles' fixed random numbers (the node).
    pub seed: u32,
    /// The node's customData (the particle function's `i.custom`).
    pub custom: [f32; 4],
    /// Hidden by its own `visible` (`World::hidden_built`): in the build, its pass idle until it is shown.
    pub hidden: bool,
}

/// A `GpuParticles` node as built: its quads (a mesh of its own, `mesh`, as `ParticleSystem::mesh`), its objects (and
/// states) from `object`, its program and parameters, emission and collision.
#[derive(Clone, Debug)]
pub struct GpuParticleSystem {
    pub mesh: u32,
    pub emitter_area: f32,
    pub node: u32,
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
    fn add(&mut self, key: (String, bool), wrap: bool, mut img: impl FnMut() -> Result<std::sync::Arc<image::RgbaImage>>) -> Result<i32> {
        if let Some(i) = self.by_key.get(&key) {
            return Ok(*i);
        }
        let img = img()?;
        let id = self.textures.len() as i32;
        self.textures.push(Texture::of(&key.0, key.1, wrap, img));
        self.by_key.insert(key, id);
        Ok(id)
    }
}

impl Texture {
    /// A texture of the pages: an image (its source's identity `key`: `texture_image`), read as sRGB or not, wrapped
    /// or clamped.
    pub fn of(key: &str, srgb: bool, wrap: bool, img: Arc<image::RgbaImage>) -> Texture {
        Texture { width: img.width(), height: img.height(), rgba: img, srgb, key: Texture::key_of(key, srgb, wrap), wrap }
    }

    /// The pages' key of the texture `of` makes.
    pub fn key_of(key: &str, srgb: bool, wrap: bool) -> String {
        format!("{key}|{srgb}|{wrap}")
    }

    /// A world's texture as a material's slot has it (`srgb_default`: whether the slot is a colour's), decoded. As
    /// the build makes it, so the pages know one they have by its key.
    pub fn of_world(t: &world::Texture, srgb_default: bool, assets: &Path) -> Result<Texture> {
        let (key, img) = texture_image(t, assets)?;
        Ok(Texture::of(&key, t.srgb && srgb_default, t.wrap, img))
    }
}

/// The textures a material's GPU form names, as `build_material` asks for them: (texture, sRGB requested).
pub fn material_textures(id: u32, m: &desc::Material, textures: &HashMap<u32, world::Texture>, shaders: &HashMap<String, i32>) -> Result<Vec<(u32, bool)>> {
    let mut out = vec![];
    build_material(id, m, textures, shaders, &mut |tid, srgb| {
        out.push((tid, srgb));
        Ok(0)
    })?;
    Ok(out)
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
            let path = crate::files::asset_path(assets, uri);
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
        // A stream's identity is its name and size, whatever its pixels: the pages keep its place (texpack.rs
        // `stream_name`), and the app's pictures go straight into it (gpu.rs `write_stream`).
        world::TextureSource::Stream { name, width, height, rgba } => {
            let image = image::RgbaImage::from_raw(*width, *height, rgba.to_vec()).context("stream texture: size does not match the pixels")?;
            Ok((crate::texpack::stream_key(name, *width, *height), Arc::new(image)))
        }
    }
}

/// A model file (cached per file version).
pub fn model_gltf(uri: &str, assets: &Path) -> Result<Arc<skin::Model>> {
    let path = crate::files::asset_path(assets, uri);
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
        uv: [repeat[0], repeat[1], slot(&m.transmission_map, false)? as f32 + 1.0, if m.vertex_colors { 1.0 } else { 0.0 }],
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
/// table, black (the kernel skips black lights, and they are in no group's sphere: lib.rs `light_groups`).
pub fn gpu_light_shown(l: &Light, m: Mat4, power: f32, shown: bool) -> Option<GpuLight> {
    let mut gl = gpu_light(l, m, power)?;
    if !shown {
        gl.color = [0.0, 0.0, 0.0, gl.color[3]];
    }
    Some(gl)
}

/// A spot light's cone for the kernel (`local_light`): the cosine of the outer cone's half angle `angle` (radians),
/// outside which there is no light, and 1 / (the inner cone's cosine - it): `lightAngleScale` of glTF 2.0
/// KHR_lights_punctual's reference attenuation (the difference no less than 0.001, as there: a cone without penumbra
/// has an edge that wide), the offset left to the kernel's subtraction.
/// `penumbra` (0..1) is the share of the angle over which the light fades: three.js's `SpotLight.penumbra` ("Percent
/// of the spotlight cone that is attenuated due to penumbra"), whose inner cone is `angle` x (1 - `penumbra`) (three.js
/// WebGLLights: `penumbraCos`), glTF's `innerConeAngle`.
pub fn spot_cone(angle: f32, penumbra: f32) -> (f32, f32) {
    let angle = angle.clamp(0.0, std::f32::consts::PI);
    let (cos_outer, cos_inner) = (angle.cos(), (angle * (1.0 - penumbra.clamp(0.0, 1.0))).cos());
    (cos_outer, 1.0 / (cos_inner - cos_outer).max(1e-3))
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
            let (cos_outer, inv_width) = spot_cone(*angle, *penumbra);
            GpuLight {
                pos: [pos.x, pos.y, pos.z, 2.0],
                dir: [dir.x, dir.y, dir.z, *radius],
                color: [color[0] * intensity, color[1] * intensity, color[2] * intensity, *range],
                ext: [cos_outer, inv_width, 0.0, *cast_shadow as u32 as f32],
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

/// The scene's sky as the renderer bakes it, and the atmosphere's sun (its light object: `Retained::sun`). `matrix`:
/// an object's world matrix (the build has its own, a product in another order than `World::world_matrix`'s: the
/// same to the bit only for an object without a parent).
pub fn sky_data(w: &World, assets: &Path, matrix: &dyn Fn(u32) -> Mat4) -> Result<(SkyData, Option<u32>)> {
    let mut sun_id = None;
    let sky = match &w.environment.sky {
        Sky::Atmosphere { sun, ground_albedo, rayleigh_scale, mie_scale, mie_anisotropy, dynamic, glow, horizon_glow, .. } => {
            let sun_light = sun.and_then(|s| w.objects.get(&s).map(|o| (s, o)));
            let (sun_dir, sun_lux) = match sun_light.map(|(s, o)| (matrix(s), &o.kind)) {
                Some((m, Kind::Light(Light::Directional { color, intensity, .. }))) => (-m.transform_vector3(Vec3::NEG_Z).normalize(), Vec3::from(*color) * *intensity),
                _ => (Vec3::Y, Vec3::ZERO),
            };
            sun_id = sun_light.map(|(s, _)| s);
            SkyData::Atmosphere {
                sun_dir,
                sun_lux,
                rayleigh: *rayleigh_scale,
                mie: *mie_scale,
                g: *mie_anisotropy,
                ground: Vec3::from(*ground_albedo),
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
    Ok((sky, sun_id))
}

/// A world's render preset: the preset its settings name, the settings' overrides over it, then `overrides`.
pub fn preset_of(w: &World, presets_dir: &Path, overrides: Option<&serde_json::Value>) -> Result<desc::Preset> {
    let st = &w.settings;
    let mut preset_json: serde_json::Value = serde_json::from_slice(&crate::files::preset(presets_dir, &st.preset).with_context(|| format!("preset {}", st.preset))?)?;
    if let Some(o) = &st.overrides {
        desc::merge_json(&mut preset_json, o);
    }
    if let Some(o) = overrides {
        desc::merge_json(&mut preset_json, o);
    }
    Ok(serde_json::from_value(preset_json)?)
}

pub fn build(w: &World, assets: &Path, presets_dir: &Path, overrides: Option<&serde_json::Value>) -> Result<Scene> {
    let t0 = web_time::Instant::now();
    let st = &w.settings;
    let preset = preset_of(w, presets_dir, overrides)?;
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
            let slot = texcache.add((key, srgb), t.wrap, || Ok(img.clone()))?;
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
    // per node. THREERT_INSTANCING=0 flattens everything.
    let instancing = std::env::var("THREERT_INSTANCING").map_or(true, |v| v != "0");
    let mut pool_tris: Vec<[Vec3A; 3]> = vec![];
    let (mut vcolors, mut pool_vcolors): (Vec<[u32; 3]>, Vec<[u32; 3]>) = (vec![], vec![]);
    // Whether a mesh's triangles carry its geometry's vertex colours: any of its materials asks for them.
    let colored = |g: &world::Geometry, mats: &mut dyn Iterator<Item = &u32>| !g.colors.is_empty() && mats.filter(|k| w.materials.get(k).is_some_and(|m| m.vertex_colors)).count() > 0;
    let mut pool_attrs: Vec<GpuTriAttr> = vec![];
    // Model-space triangles of instanced meshes that each placement flattens (`flatten_coincident`'s pairs).
    let mut meshes: Vec<InstMesh> = vec![];
    let mut instances: Vec<Instance> = vec![];
    let (mut sections, mut rows): (Vec<Section>, Vec<Row>) = (vec![], vec![]);
    // The geometries whose meshes carry vertex colours: one with colours that some object draws with a material that
    // reads them (the mesh is every object's of that geometry, so it is settled before the first is placed).
    let colored_meshes: std::collections::HashSet<(u32, bool)> = w
        .objects
        .values()
        .filter_map(|o| match &o.kind {
            Kind::Mesh { geometry, material, materials, .. } => Some((geometry, materials.iter().chain(std::iter::once(material)).collect::<Vec<_>>(), by_groups(w.geometries.get(geometry)?, materials.len()))),
            Kind::Instanced { geometry, materials, procedural: false, .. } => Some((geometry, materials.iter().take(1).collect(), false)),
            _ => None,
        })
        .filter(|(g, mats, _)| w.geometries.get(g).is_some_and(|g| !g.colors.is_empty()) && mats.iter().any(|k| w.materials.get(k).is_some_and(|m| m.vertex_colors)))
        .map(|(g, _, grouped)| (*g, grouped))
        .collect();
    let mut mesh_of: HashMap<(String, Option<Vec<String>>), Option<u32>> = HashMap::new();
    // A posed rigid model's parts, by (model, clip): its placements share their meshes.
    let mut rigid_parts: HashMap<(String, String), std::rc::Rc<Vec<RigidPart>>> = HashMap::new();
    // What the skinning pass reads, by (model, nodes, the primitives mirrored): shared by the model's placements.
    let mut skin_sources: Vec<Vec<[skin::SkinCorner; 3]>> = vec![];
    let mut skin_rest: Vec<Vec<skin::PaletteRows>> = vec![];
    let mut skin_source_of: HashMap<(String, Option<Vec<String>>, Vec<bool>), (u32, Arc<Vec<(u32, u32, u32)>>)> = HashMap::new();
    // Flattened instances' triangle ranges.
    let mut tri_instances: Vec<(u32, u32, u32)> = vec![];
    // The rect lights' card mesh, once a scene has a card (`card_mesh`).
    let mut card: Option<u32> = None;
    // Instance colours anywhere: object entries of 4 vec4s, and the kernel's F_TINT.
    let tinted = w.instanced.iter().any(|(id, t)| t.colors.is_some() && w.shown(*id) && matches!(w.objects.get(id).map(|o| &o.kind), Some(Kind::Instanced { .. })));
    let stride = object_stride(tinted);
    for id in &order {
        let n = &w.objects[id];
        let m = worlds[id];
        let first = tris.len() as u32;
        let shown = w.shown(*id);
        let flags = if n.cast_shadow { 0 } else { TRI_NO_SHADOW };
        match &n.kind {
            Kind::Mesh { geometry, material, own_blas, materials: mesh_mats } if instancing => {
                // A mesh is an instance of its geometry's mesh: the triangles in model space in the pool, one BLAS,
                // shared by the objects of the geometry whatever their materials, which are the instance's rows
                // (`section_mesh`, `Row`: a cutout, glass, one that emits). A move is the instance's transform
                // (`InstanceRef::moves`). One that is hidden (`World::shown`), or placed without size
                // (`placed_world`), is an instance too, which no ray sees and whose object entry has no power: it
                // comes back in place (`Instance::masked`).
                let g = w.geometries.get(geometry).ok_or_else(|| anyhow!("object {}: unknown geometry {geometry}", n.name))?;
                let mat = *retained.materials.get(material).ok_or_else(|| anyhow!("object {}: unknown material {material}", n.name))?;
                // Several materials: each geometry group's triangles with its own (three.js's material array).
                let gpu_mats = mesh_mats.iter().map(|k| retained.materials.get(k).copied().ok_or_else(|| anyhow!("object {}: unknown material {k}", n.name))).collect::<Result<Vec<u32>>>()?;
                let grouped = by_groups(g, gpu_mats.len());
                let key = (*geometry, grouped);
                let mesh = match retained.own_mesh.get(&key) {
                    Some(&k) => Some(k),
                    None => {
                        let d = section_mesh(g, grouped, pool_tris.len() as u32, sections.len() as u32, colored_meshes.contains(&key));
                        (!d.tris.is_empty()).then(|| {
                            meshes.push(d.mesh);
                            sections.extend(d.sections);
                            put_colors(&mut pool_vcolors, pool_tris.len(), d.colors);
                            pool_tris.extend(d.tris);
                            pool_attrs.extend(d.attrs);
                            let k = meshes.len() as u32 - 1;
                            retained.own_mesh.insert(key, k);
                            k
                        })
                    }
                };
                if let Some(mesh) = mesh {
                    let (m, gone) = placed_world(m, Vec3::ZERO);
                    let unseen = !shown || gone;
                    let (lo, hi) = meshes[mesh as usize].placed_bounds(m);
                    let pivot = m.transform_point3(Vec3::ZERO);
                    let size = hi - lo;
                    let oi = (objects.len() / stride) as u32;
                    objects.extend_from_slice(&object_row(pivot, if unseen { 0.0 } else { n.power }, size, n.custom, [1.0; 3])[..stride]);
                    retained.edits_meshes |= *own_blas || n.driven;
                    *retained.proxy(*id) = Proxy { object: Some(oi), card: None, draw: Draw::Instance(InstanceRef { instance: instances.len() as u32, mesh: MeshKind::Own, moves: n.driven || w.movable(*id) }) };
                    let mine = section_rows(meshes[mesh as usize].sections(&sections), if grouped { &gpu_mats } else { std::slice::from_ref(&mat) }, &materials);
                    instances.push(Instance { mesh, world: m, object: oi, cast_shadow: rows_cast(&mine, n.cast_shadow), name: n.name.clone(), sub: None, hidden: unseen, masked: unseen, first_emitter: 0, rows: rows.len() as u32, bits: rows_bits(&mine), prev: 0, verts: 0 });
                    rows.extend(mine);
                }
            }
            Kind::Instanced { geometry, procedural, materials: mats, capacity } if shown => {
                // Copies of one geometry: one shared mesh (one BLAS), an instance and an object entry per copy, the
                // copies' records and entries contiguous (live/edits.rs `update_instanced` rewrites them in place).
                // With a material that emits each copy has its emitters in the light list (`rows_emitters`), a
                // hidden copy's at no weight (its object has no power). A copy may mirror
                // the mesh (`INST_MIRRORED`). Without instancing each copy is its own triangles, and changing the
                // instances rebuilds.
                let empty = world::InstanceTable::default();
                let t = w.instanced.get(id).unwrap_or(&empty);
                let placed: Vec<(Mat4, bool)> = (0..*capacity as usize).map(|i| instance_world(m, t, i)).collect();
                let entry = |i: usize, wm: Mat4, size: Vec3| object_row(wm.transform_point3(Vec3::ZERO), n.power, size, t.custom(i, n.custom), t.color(i));
                if *procedural {
                    let object = (objects.len() / stride) as u32;
                    if let Some(p) = proc_object(w, *id, m, object, &retained.materials, assets)? {
                        retained.proxy(*id).draw = Draw::Instanced(InstancedPlace { first: procs.len() as u32, capacity: *capacity, object, mesh: None, drawn: t.count });
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
                let emits = emissive(&materials[mat as usize]);
                let vc = colored(g, &mut std::iter::once(material));
                if instancing {
                    // (The geometry whole, with the object's first material: the mesh an object of one material has.)
                    let key = (*geometry, false);
                    let mesh = match retained.own_mesh.get(&key) {
                        Some(&k) => Some(k),
                        None => {
                            let d = section_mesh(g, false, pool_tris.len() as u32, sections.len() as u32, colored_meshes.contains(&key));
                            (!d.tris.is_empty()).then(|| {
                                meshes.push(d.mesh);
                                sections.extend(d.sections);
                                put_colors(&mut pool_vcolors, pool_tris.len(), d.colors);
                                pool_tris.extend(d.tris);
                                pool_attrs.extend(d.attrs);
                                let k = meshes.len() as u32 - 1;
                                retained.own_mesh.insert(key, k);
                                k
                            })
                        }
                    };
                    if let Some(mesh) = mesh {
                        let object = (objects.len() / stride) as u32;
                        retained.proxy(*id).draw = Draw::Instanced(InstancedPlace { first: instances.len() as u32, capacity: *capacity, object, mesh: Some(mesh), drawn: t.count });
                        let mine = section_rows(meshes[mesh as usize].sections(&sections), std::slice::from_ref(&mat), &materials);
                        for (i, &(wm, hidden)) in placed.iter().enumerate() {
                            let (lo, hi) = meshes[mesh as usize].placed_bounds(wm);
                            objects.extend_from_slice(&copy_row(entry(i, wm, Vec3::from(hi - lo)), emits && hidden)[..stride]);
                            instances.push(Instance { mesh, world: wm, object: object + i as u32, cast_shadow: rows_cast(&mine, n.cast_shadow), name: n.name.clone(), sub: Some(i as u32), hidden, masked: false, first_emitter: 0, rows: rows.len() as u32, bits: rows_bits(&mine), prev: 0, verts: 0 });
                            rows.extend(mine.iter().copied());
                        }
                    }
                    continue;
                }
                // Flattened: nothing kept to change it by (`Draw::Fixed`; an entry a copy, as the instances have).
                retained.proxy(*id);
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
                    retained.proxy(*id).draw = Draw::Dynamic(DynamicModel { first, mats: attrs[first as usize..].iter().map(|a| a.uv[3].to_bits()).collect(), ..Default::default() });
                }
            }
            Kind::Model { uri, nodes, animation, functions } if shown => {
                let glb = model_gltf(uri, assets)?;
                let (doc, _, images) = &glb.gltf;
                // (A model with material functions has its own copies of the file's materials.)
                let key = if functions.is_empty() { uri.clone() } else { format!("{uri}\u{0}{functions:?}") };
                // (Its meshes too: placements share a mesh only with the same functions.)
                let mesh_key = (key.clone(), nodes.clone());
                // A material of the file: its index, its class in a mesh (`mesh_class`), whether it only emits.
                let mut resolve = |gm: Option<gltf::Material>| -> Result<(u32, u32, bool)> {
                    let idx = gm.as_ref().and_then(|g| g.index()).unwrap_or(usize::MAX);
                    let i = match asset_materials.get(&(key.clone(), idx)) {
                        Some(i) => *i,
                        None => {
                            let name = gm.as_ref().and_then(|g| g.name()).unwrap_or_default().to_string();
                            let mut gpu = gltf_material(gm, images, uri, &mut |key, wrap, img| texcache.add(key, wrap, img), &model_images)?;
                            // The first function whose prefix the material's name starts with, on the glTF shading.
                            if let Some(f) = functions.iter().find(|f| name.starts_with(f.prefix.as_str())) {
                                let mut tex = |tid: u32, srgb_default: bool| -> Result<i32> {
                                    let t = w.textures.get(&tid).ok_or_else(|| anyhow!("unknown texture {tid}"))?;
                                    let (tkey, img) = match decoded.get(&tid) {
                                        Some(d) => d.clone(),
                                        None => texture_image(t, assets)?,
                                    };
                                    texcache.add((tkey, t.srgb && srgb_default), t.wrap, || Ok(img.clone()))
                                };
                                apply_shader(&mut gpu, &f.shader, &retained.shaders, &mut tex).with_context(|| format!("{uri}: material {name}"))?;
                            }
                            let i = materials.len() as u32;
                            materials.push(gpu);
                            asset_materials.insert((key.clone(), idx), i);
                            i
                        }
                    };
                    Ok((i, mesh_class(&materials[i as usize]), emissive(&materials[i as usize])))
                };
                // A posed model's triangles are dynamic (re-skinned in place). A movable one is an instance whose
                // transform moves (hardware RT), unless it has triangles flattened per placement: then they are dynamic
                // too (re-transformed in place).
                let posed = animation.is_some() && glb.animated();
                let movable = w.movable(*id);
                let dynamic = posed || movable;
                let flags = if dynamic { flags | TRI_DYNAMIC } else { flags };
                // Posed rigid model: its parts as instances (`RigidModel`: the nodes its clip leaves alone merged into
                // one mesh, each node it moves a mesh); a movable one too (each update re-places its instances from
                // the object's transform).
                if instancing && posed && nodes.is_none() && glb.rigid() {
                    let a = animation.as_ref().unwrap();
                    let globals = glb.node_globals(Some(a))?;
                    let pkey = (key.clone(), rigid_pose_key(a));
                    if !rigid_parts.contains_key(&pkey) {
                        let moving = glb.moving_nodes(a)?;
                        let apart = |ni: usize| moving[ni];
                        // The nodes left alone, in the model's space as the pose has them (the pose does not move
                        // them); then each node apart, in its own (the order within a node is the file's).
                        let mut lists: Vec<(usize, Vec<crate::skin::ModelTri>)> = vec![];
                        let still: Vec<_> = glb.node_triangles(Mat4::IDENTITY, Some(a), &|ni| !apart(ni), false)?.into_iter().map(|(_, t)| t).collect();
                        if !still.is_empty() {
                            lists.push((STILL_NODES, still));
                        }
                        for (ni, t) in glb.node_triangles(Mat4::IDENTITY, None, &apart, true)? {
                            match lists.last_mut() {
                                Some((last, list)) if *last == ni => list.push(t),
                                _ => lists.push((ni, vec![t])),
                            }
                        }
                        let mut parts = vec![];
                        for (node, local) in &lists {
                            let mut classed: [Vec<(usize, u32)>; 4] = Default::default();
                            let mut emitting: std::collections::HashSet<u32> = Default::default();
                            for (k, t) in local.iter().enumerate() {
                                let (mat, class, emits) = resolve(t.material.and_then(|i| doc.materials().nth(i)))?;
                                classed[class as usize].push((k, mat));
                                if emits {
                                    emitting.insert(mat);
                                }
                            }
                            let over = split_coincident(&mut classed, &emitting, &|k| local[k].p);
                            let mesh = model_meshes(local, &classed, &over, &mut Pool { tris: &mut pool_tris, attrs: &mut pool_attrs, meshes: &mut meshes, sections: &mut sections, instances: &mut instances, rows: &mut rows }, &mut retained.mesh_rows);
                            parts.push(RigidPart { node: *node, mesh });
                        }
                        // (What an edit can place again: every part a mesh whole in the pool, the file's own materials.)
                        if functions.is_empty() && parts.iter().all(|p| p.mesh.is_some_and(|k| meshes[k as usize].over == NO_OVER)) {
                            retained.rigid_meshes.insert((uri.clone(), rigid_pose_key(a)), parts.iter().filter_map(|p| Some((p.node, p.mesh?))).collect());
                        }
                        rigid_parts.insert(pkey.clone(), std::rc::Rc::new(parts));
                    }
                    let parts = rigid_parts[&pkey].clone();
                    let oi = (objects.len() / stride) as u32;
                    let (mut olo, mut ohi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
                    // (Where the object stands as an update finds it, `World::world_matrix`: the same product as `m`,
                    // multiplied in another order, so not always the same bits.)
                    let mut rigid = RigidModel { world: w.world_matrix(*id), pose: rigid_pose_key(a), ..Default::default() };
                    for part in parts.iter() {
                        // (A part placed without size, a node its clip scales to nothing: at a point; `repose_rigid`
                        // keeps it from the rays.)
                        let (w_n, _) = placed_world(RigidModel::part_world(m, part.node, &globals), m.w_axis.truncate());
                        // (Its mesh, and the mesh of what lies over it: an instance each, the second with the part's
                        // node among the instances a pose places.)
                        let over = part.mesh.map(|k| meshes[k as usize].over).filter(|&k| k != NO_OVER);
                        for (mesh, second) in part.mesh.map(|k| (k, false)).into_iter().chain(over.map(|k| (k, true))) {
                            let (lo, hi) = meshes[mesh as usize].placed_bounds(w_n);
                            (olo, ohi) = (olo.min(lo.into()), ohi.max(hi.into()));
                            let mine = retained.mesh_rows[&mesh].clone();
                            let i = instances.len() as u32;
                            if part.node == STILL_NODES && !second {
                                rigid.still = Some(i);
                            } else {
                                rigid.instances.push((part.node, i));
                            }
                            instances.push(Instance { mesh, world: w_n, object: oi, cast_shadow: rows_cast(&mine, n.cast_shadow), name: n.name.clone(), sub: None, hidden: false, masked: false, first_emitter: 0, rows: rows.len() as u32, bits: rows_bits(&mine), prev: 0, verts: 0 });
                            rows.extend(mine);
                        }
                    }
                    let pivot = m.transform_point3(Vec3::ZERO);
                    let size = (ohi - olo).max(Vec3A::ZERO);
                    objects.extend_from_slice(&object_row(pivot, n.power, size, n.custom, [1.0; 3])[..stride]);
                    *retained.proxy(*id) = Proxy { object: Some(oi), card: None, draw: Draw::Rigid(rigid) };
                    continue;
                }
                // A posed skinned model is an instance of a mesh of its own, in its own space, which the skinning pass
                // skins at each new pose (`Skinned`). One with emitting triangles is not yet: skinned alone, below.
                if instancing && posed && m.determinant() != 0.0 && !(nodes.is_none() && glb.rigid()) {
                    let a = animation.as_ref().unwrap();
                    let local = glb.triangles(Mat4::IDENTITY, nodes.as_deref(), Some(a))?;
                    let mut classed: [Vec<(usize, u32)>; 4] = Default::default();
                    for (k, t) in local.iter().enumerate() {
                        // (A mesh that deforms does not keep its emitters: the light list does not follow a pose yet.)
                        let (mat, class, emits) = resolve(t.material.and_then(|i| doc.materials().nth(i)))?;
                        classed[if emits { FLAT } else { class as usize }].push((k, mat));
                    }
                    if classed[FLAT].is_empty() && !local.is_empty() {
                        let (order, sects, mine) = model_sections(&kept(&classed));
                        // (The model's corners in its mesh's order: one copy for every placement of the model.)
                        let mirrors = glb.mirrored(Some(a))?;
                        let file = if functions.is_empty() { uri.clone() } else { format!("{uri}\u{0}{functions:?}") };
                        let skey = (file, nodes.clone(), mirrors.clone());
                        let (source, tri_order) = match skin_source_of.get(&skey) {
                            Some(s) => s.clone(),
                            None => {
                                let corners = glb.skin_source(nodes.as_deref(), Some(a))?;
                                skin_sources.push(order.iter().map(|&(k, _, _)| corners[k]).collect());
                                skin_rest.push(glb.palette(None)?);
                                let s = (skin_sources.len() as u32 - 1, Arc::new(order.iter().map(|&(k, section, rank)| (k as u32, section, rank)).collect::<Vec<_>>()));
                                skin_source_of.insert(skey, s.clone());
                                s
                            }
                        };
                        let first = pool_tris.len() as u32;
                        let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
                        for &(k, section, rank) in &order {
                            let t = &local[k];
                            for v in t.p {
                                lo = lo.min(v.into());
                                hi = hi.max(v.into());
                            }
                            pool_tris.push([t.p[0].into(), t.p[1].into(), t.p[2].into()]);
                            pool_attrs.push(section_attr(t.n, t.uv, t.tangent, section, rank));
                        }
                        // Other poses reach further (a stride, a swing): object bounds a quarter wider each way; one
                        // the game poses may take any pose (lie down): as far as its size each way.
                        let pad = if a.nodes.is_empty() { (hi - lo) * 0.25 } else { Vec3A::splat((hi - lo).length()) };
                        meshes.push(InstMesh { lo: lo - pad, hi: hi + pad, deforms: true, sect_first: sections.len() as u32, sect_count: sects.len() as u32, ..own_inst_mesh(&pool_tris[first as usize..], first) });
                        let mesh = meshes.len() as u32 - 1;
                        sections.extend(sects);
                        retained.mesh_rows.insert(mesh, mine.clone());
                        let (lo, hi) = meshes[mesh as usize].placed_bounds(m);
                        let pivot = m.transform_point3(Vec3::ZERO);
                        let size = hi - lo;
                        let oi = (objects.len() / stride) as u32;
                        objects.extend_from_slice(&object_row(pivot, n.power, size, n.custom, [1.0; 3])[..stride]);
                        let i = instances.len() as u32;
                        instances.push(Instance { mesh, world: m, object: oi, cast_shadow: rows_cast(&mine, n.cast_shadow), name: n.name.clone(), sub: None, hidden: false, masked: false, first_emitter: 0, rows: rows.len() as u32, bits: rows_bits(&mine), prev: 0, verts: 0 });
                        rows.extend(mine);
                        retained.skinned.push(Skinned { mesh, uri: uri.clone(), nodes: nodes.clone(), animation: a.clone(), order: tri_order, object: *id, instance: i, source, palette_len: glb.palette_len() as u32, mirrors });
                        *retained.proxy(*id) = Proxy { object: Some(oi), card: None, draw: Draw::Instance(InstanceRef { instance: i, mesh: MeshKind::Skinned(retained.skinned.len() as u32 - 1), moves: true }) };
                        if functions.is_empty() {
                            retained.skinned_model.entry((uri.clone(), nodes.clone())).or_insert(retained.skinned.len() as u32 - 1);
                        }
                        continue;
                    }
                }
                if instancing && !posed {
                    let key = mesh_key.clone();
                    let mesh = match mesh_of.get(&key) {
                        Some(k) => *k,
                        None => {
                            // Model-space triangles go to the pool by section, a material of the file (one BLAS),
                            // those that only emit among them (each instance has its emitters in the light list).
                            let local = glb.triangles(Mat4::IDENTITY, nodes.as_deref(), None)?;
                            let mut classed: [Vec<(usize, u32)>; 4] = Default::default();
                            let mut emitting: std::collections::HashSet<u32> = Default::default();
                            for (k, t) in local.iter().enumerate() {
                                let (mat, class, emits) = resolve(t.material.and_then(|i| doc.materials().nth(i)))?;
                                classed[class as usize].push((k, mat));
                                if emits {
                                    emitting.insert(mat);
                                }
                            }
                            let over = split_coincident(&mut classed, &emitting, &|k| local[k].p);
                            let k = model_meshes(&local, &classed, &over, &mut Pool { tris: &mut pool_tris, attrs: &mut pool_attrs, meshes: &mut meshes, sections: &mut sections, instances: &mut instances, rows: &mut rows }, &mut retained.mesh_rows);
                            // (The mesh's bounds are the model's: its object's, and what picking goes by.)
                            if let Some(k) = k {
                                let im = &mut meshes[k as usize];
                                for v in local.iter().flat_map(|t| t.p) {
                                    (im.lo, im.hi) = (im.lo.min(v.into()), im.hi.max(v.into()));
                                }
                            }
                            if std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "log") {
                                eprintln!("instancing: {uri} {nodes:?}: {} opaque, {} cutout, {} glass in the mesh; {} in a second mesh", classed[0].len(), classed[1].len(), classed[2].len(), over.len());
                            }
                            // (What an edit can place another instance of: a mesh whole in the pool.)
                            if let Some(k) = k.filter(|&k| meshes[k as usize].over == NO_OVER) {
                                retained.model_mesh.insert(key.clone(), k);
                            }
                            mesh_of.insert(key, k);
                            k
                        }
                    };
                    if let Some(mesh) = mesh {
                        // Object entry from the placed mesh bounds (the box of the transformed model-space box). A
                        // model placed without size is an instance no ray sees (`placed_world`), as a mesh is.
                        let im = meshes[mesh as usize];
                        let pivot = m.transform_point3(Vec3::ZERO);
                        let (world, gone) = placed_world(m, pivot);
                        let (lo, hi) = im.placed_bounds(world);
                        let oi = (objects.len() / stride) as u32;
                        objects.extend_from_slice(&object_row(pivot, if gone { 0.0 } else { n.power }, hi - lo, n.custom, [1.0; 3])[..stride]);
                        retained.proxy(*id).object = Some(oi);
                        // (Its proxy has the instance where the instance is all of it: a model with a second mesh is
                        // two instances, the build's to change, `Draw::Fixed`.)
                        if im.over == NO_OVER {
                            retained.proxy(*id).draw = Draw::Instance(InstanceRef { instance: instances.len() as u32, mesh: MeshKind::Model, moves: movable });
                        }
                        for mesh in [mesh, im.over].into_iter().filter(|&k| k != NO_OVER) {
                            let mine = retained.mesh_rows[&mesh].clone();
                            instances.push(Instance { mesh, world, object: oi, cast_shadow: rows_cast(&mine, n.cast_shadow), name: n.name.clone(), sub: None, hidden: gone, masked: gone, first_emitter: 0, rows: rows.len() as u32, bits: rows_bits(&mine), prev: 0, verts: 0 });
                            rows.extend(mine);
                        }
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
                    retained.proxy(*id).draw = Draw::Dynamic(DynamicModel { first, mats, ..Default::default() });
                }
            }
            Kind::Light(l) => match (l, gpu_light_shown(l, m, n.power, shown)) {
                (_, Some(gl)) => {
                    lights.push(gl);
                    retained.light_nodes.push(*id);
                }
                (Light::Rect { color, intensity, width, height, .. }, None) if shown => {
                    // Its card, a one-sided emissive quad the lights are sampled by as any emitter: an instance of
                    // the scene's card mesh, sized by its transform (`card_world`), its material its row's. It
                    // casts no shadow. A move is the instance's.
                    let mat = materials.len() as u32;
                    materials.push(card_material(*color, *intensity));
                    let mesh = *card.get_or_insert_with(|| card_mesh(&mut Pool { tris: &mut pool_tris, attrs: &mut pool_attrs, meshes: &mut meshes, sections: &mut sections, instances: &mut instances, rows: &mut rows }));
                    let pivot = m.transform_point3(Vec3::ZERO);
                    let (world, gone) = placed_world(card_world(m, *width, *height), pivot);
                    let (lo, hi) = meshes[mesh as usize].placed_bounds(world);
                    let oi = (objects.len() / stride) as u32;
                    objects.extend_from_slice(&object_row(pivot, if gone { 0.0 } else { n.power }, hi - lo, n.custom, [1.0; 3])[..stride]);
                    *retained.proxy(*id) = Proxy { object: Some(oi), card: Some(mat), draw: Draw::Instance(InstanceRef { instance: instances.len() as u32, mesh: MeshKind::Card, moves: w.movable(*id) }) };
                    let mine = [row_of(mat, &materials)];
                    instances.push(Instance { mesh, world, object: oi, cast_shadow: false, name: n.name.clone(), sub: None, hidden: gone, masked: gone, first_emitter: 0, rows: rows.len() as u32, bits: rows_bits(&mine), prev: 0, verts: 0 });
                    rows.extend(mine);
                }
                _ => {}
            },
            // (Hidden by its own `visible`: built while the kernel has room after the volumes shown, with no shape,
            // so that showing it is in place: gpu.rs `set_medium_hidden`.)
            Kind::Volume { function, shape, grid, .. } if shown || (w.hidden_built(*id) && media.len() + media_to_show < MAX_MEDIA) => {
                if shown {
                    media_to_show -= 1;
                }
                let func = volume_fns.iter().position(|(k, _, _)| k == function).ok_or_else(|| anyhow!("object {}: unknown volume function {function}", n.name))? as u32;
                // Its density grid: where it lies in the atlas and its cells, as bits in b.zw (pathtrace.wgsl `vgrid`).
                let mut at = [0.0f32; 2];
                if let Some(gid) = grid {
                    if *shape != 0 {
                        return Err(anyhow!("object {}: a density grid needs a box volume", n.name));
                    }
                    at = grid_of(w, &mut grids, &grid_programs, *gid, *id, media.len() as u32).with_context(|| format!("object {}", n.name))?;
                }
                if media.len() >= MAX_MEDIA {
                    return Err(anyhow!("object {}: more than {MAX_MEDIA} volumes shown at once (the kernel takes {MAX_MEDIA}; the rest would not be drawn)", n.name));
                }
                media.push(MediumGpu::of(n, m, func, shown, at).expect("a volume"));
                media_nodes.push(*id);
            }
            Kind::Procedural { .. } if shown => {
                if let Some(p) = proc_object(w, *id, m, (objects.len() / stride) as u32, &retained.materials, assets)? {
                    objects.extend_from_slice(&p.object_entry(n)[..stride]);
                    *retained.proxy(*id) = Proxy { object: Some(p.object), card: None, draw: Draw::Procedural(procs.len() as u32) };
                    procs.push(p);
                }
            }
            Kind::Particles { count, emitter, params, material, emitter_area } if (shown || w.hidden_built(*id)) && *count > 0 => {
                // A particle system is geometry as any other: a mesh of its own in the pool, a quad a particle, and one
                // instance of it; each particle its own object (pivot, bounds and per-particle data for material
                // functions). The quads are collapsed at the node until the particle pass writes their vertices where
                // the particles are (the instance places nothing), and the mesh's BLAS is built again after the pass.
                // (Whatever THREERT_INSTANCING says: a system has no flattened form.)
                let mat = *retained.materials.get(material).ok_or_else(|| anyhow!("object {}: unknown material {material}", n.name))?;
                let func = particle_fns.iter().position(|(k, _, _)| k == emitter).ok_or_else(|| anyhow!("object {}: unknown particle function {emitter}", n.name))? as u32;
                let pivot = m.transform_point3(Vec3::ZERO);
                let object = (objects.len() / stride) as u32;
                let translucent = w.materials.get(material).is_some_and(|m| m.transparent);
                let mut pf = [0.0f32; 32];
                for (k, v) in params.iter().take(32).enumerate() {
                    pf[k] = *v;
                }
                for _ in 0..*count {
                    objects.extend_from_slice(&object_row(pivot, n.power, Vec3::ZERO, n.custom, [1.0; 3])[..stride]);
                }
                let mesh = particle_mesh(*count, pivot.into(), mat, object, !translucent && *emitter_area > 0.0, n, &materials, &mut Pool { tris: &mut pool_tris, attrs: &mut pool_attrs, meshes: &mut meshes, sections: &mut sections, instances: &mut instances, rows: &mut rows });
                // (A translucent system is the translucency pass's: its instance is in that pass's acceleration
                // structure, not in the scene's.)
                instances.last_mut().expect("the system's instance").hidden = translucent;
                particles.push(ParticleSystem { mesh, emitter_area: *emitter_area, translucent, count: *count, object, func, params: pf, seed: *id, custom: n.custom, hidden: !shown });
            }
            Kind::GpuParticles { capacity, program, params, material, emitter_area, rate, collide } if shown && *capacity > 0 => {
                // As `Particles`: a mesh of quads collapsed at the node and one instance, each particle its own object.
                let mat = *retained.materials.get(material).ok_or_else(|| anyhow!("object {}: unknown material {material}", n.name))?;
                let prog = particle_programs.iter().position(|(k, ..)| k == program).ok_or_else(|| anyhow!("object {}: unknown particle program {program}", n.name))? as u32;
                let pivot = m.transform_point3(Vec3::ZERO);
                let object = (objects.len() / stride) as u32;
                let mut pf = [0.0f32; 32];
                for (k, v) in params.iter().take(32).enumerate() {
                    pf[k] = *v;
                }
                for _ in 0..*capacity {
                    objects.extend_from_slice(&object_row(pivot, n.power, Vec3::ZERO, n.custom, [1.0; 3])[..stride]);
                }
                let mesh = particle_mesh(*capacity, pivot.into(), mat, object, *emitter_area > 0.0, n, &materials, &mut Pool { tris: &mut pool_tris, attrs: &mut pool_attrs, meshes: &mut meshes, sections: &mut sections, instances: &mut instances, rows: &mut rows });
                gpu_particles.push(GpuParticleSystem { mesh, emitter_area: *emitter_area, node: *id, capacity: *capacity, object, program: prog, params: pf, seed: *id, pivot: pivot.to_array(), rate: *rate, collide: collide.clone() });
            }
            Kind::Camera(c) if st.camera == Some(*id) => camera = Some(camera_data(m, c)),
            _ => {}
        }
        if tris.len() as u32 > first && !matches!(n.kind, Kind::Instanced { .. }) {
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
            let proxy = retained.proxy(*id);
            proxy.object = Some(oi);
            if let Draw::Dynamic(d) = &mut proxy.draw {
                d.object = oi;
            }
            tri_nodes.push((n.name.clone(), first, tris.len() as u32));
        }
    }

    let (sky, sun) = sky_data(w, assets, &|id| worlds.get(&id).copied().unwrap_or(Mat4::IDENTITY))?;
    retained.sun = sun;

    let mut stats = HashMap::new();
    stats.insert("load_ms", t0.elapsed().as_secs_f64() * 1000.0);
    let instanced: u64 = instances.iter().map(|i| meshes[i.mesh as usize].count as u64).sum();
    stats.insert("triangles", (tris.len() as u64 + instanced) as f64);
    stats.insert("instances", instances.len() as f64);
    stats.insert("instanced_triangles", instanced as f64);
    // Deforming meshes (posed skinned models') first in the pool: the previous frame's vertices (`prev_verts`, motion vectors)
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
    // Cutout materials: their triangles take the any-hit path in traversal. Glass casts no shadow of its own: what a
    // shadow ray keeps past it is the kernel's (`glass_tr`; in full with `A_GLASS_SHADOWS`).
    for (f, a) in tri_flags.iter_mut().zip(&attrs) {
        let m = &materials[a.uv[3].to_bits() as usize];
        if m.uv2[3] > 0.0 {
            *f |= TRI_MASKED;
        }
        if m.layer[0] > 0.0 {
            *f |= TRI_NO_SHADOW;
        }
    }
    for d in retained.dynamics_mut() {
        d.flags = tri_flags[d.first as usize..d.first as usize + d.mats.len()].to_vec();
    }
    retained.asset_materials = asset_materials;
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
        tri_flags,
        attrs,
        materials,
        textures: texcache.textures,
        lights,
        camera: camera.ok_or_else(|| anyhow!("no camera (settings.camera must name a camera object)"))?,
        sky,
        environment: EnvSettings::of(w),
        preset,
        stats,
        tri_nodes,
        tri_instances,
        tinted,
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
        pool_tris,
        pool_attrs,
        meshes,
        instances,
        skin_sources,
        skin_rest,
        skin_trees: vec![],
        indexed: Default::default(),
        sections,
        rows,
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
/// A model's material as the GPU has it. `texture((key, sRGB), wrap, image)` gives a texture's slot, and decodes the
/// image only where the texture is not there yet: the build's set (`TexCache`), or the pages of a scene that runs
/// (live/edits.rs `place_models`). `converted`: images decoded before.
#[allow(clippy::type_complexity)]
pub fn gltf_material(gm: Option<gltf::Material>, images: &[gltf::image::Data], key: &str, texture: &mut dyn FnMut((String, bool), bool, &mut dyn FnMut() -> Result<Arc<image::RgbaImage>>) -> Result<i32>, converted: &HashMap<String, Arc<image::RgbaImage>>) -> Result<GpuMaterial> {
    let Some(gm) = gm else {
        return Ok(GpuMaterial { base_color: [1.0, 1.0, 1.0, 1.0], params: [0.0, 1.0, 0.5, 1.0], tex0: [-1; 4], tex1: [-1, -1, 0, 0], uv: [1.0, 1.0, 0.0, 0.0], ..Default::default() });
    };
    let mut tex = |t: Option<gltf::texture::Texture>, srgb: bool| -> Result<i32> {
        let Some(t) = t else { return Ok(-1) };
        let src = t.source().index();
        let k = format!("{key}#img{src}");
        let pre = converted.get(&k).cloned();
        texture((k, srgb), true, &mut || pre.clone().map_or_else(|| Ok(Arc::new(to_rgba(&images[src])?)), Ok))
    };
    let pbr = gm.pbr_metallic_roughness();
    let mr = tex(pbr.metallic_roughness_texture().map(|t| t.texture()), false)?;
    let strength = gm.emissive_strength().unwrap_or(1.0);
    let e = gm.emissive_factor();
    let clearcoat = |k: &str| gm.extension_value("KHR_materials_clearcoat").and_then(|c| c[k].as_f64()).unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    // KHR_materials_unlit: shown, never lit nor lighting (three.js MeshBasicMaterial, the kernel's unlit switch). An
    // unlit material that says an emission shows it (its factor x strength, nits) as its colour: a far city's lit
    // windows, seen and not light-sampled (tens of thousands of emitters fewer).
    let unlit = gm.extension_value("KHR_materials_unlit").is_some();
    let base_color = if unlit && e.iter().any(|&c| c > 0.0) { [e[0] * strength, e[1] * strength, e[2] * strength, 1.0] } else { pbr.base_color_factor() };
    let (e, strength) = if unlit { ([0.0; 3], 0.0) } else { (e, strength) };
    Ok(GpuMaterial {
        base_color,
        emissive: [e[0] * strength, e[1] * strength, e[2] * strength, 0.0],
        params: [pbr.metallic_factor(), pbr.roughness_factor(), 0.5, gm.normal_texture().map(|n| n.scale()).unwrap_or(1.0)],
        tex0: [tex(pbr.base_color_texture().map(|t| t.texture()), true)?, tex(gm.normal_texture().map(|t| t.texture()), false)?, mr, mr],
        tex1: [tex(gm.emissive_texture().map(|t| t.texture()), true)?, tex(gm.occlusion_texture().map(|t| t.texture()), false)?, if unlit { MAT_UNLIT } else if gm.double_sided() { MAT_DOUBLE_SIDED } else { 0 }, 0],
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

/// The visible rect lights' card, a mesh of the pool: a unit quad in a light's own space, 1 mm in front of the light
/// (which shines down -Z), its two triangles wound so that their geometric normal is the light's direction; one
/// section. An instance a light sizes it (`card_world`), and the instance's row has the light's material.
fn card_mesh(pool: &mut Pool) -> u32 {
    let p = [Vec3A::new(-0.5, -0.5, -0.001), Vec3A::new(0.5, -0.5, -0.001), Vec3A::new(0.5, 0.5, -0.001), Vec3A::new(-0.5, 0.5, -0.001)];
    let first = pool.tris.len() as u32;
    for (k, t) in [[p[0], p[2], p[1]], [p[0], p[3], p[2]]].into_iter().enumerate() {
        pool.tris.push(t);
        pool.attrs.push(section_attr([Vec3::NEG_Z; 3], [Vec2::ZERO; 3], None, 0, k as u32));
    }
    pool.meshes.push(InstMesh { sect_first: pool.sections.len() as u32, sect_count: 1, ..own_inst_mesh(&pool.tris[first as usize..], first) });
    pool.sections.push(Section { first: 0, count: 2, slot: 0 });
    pool.meshes.len() as u32 - 1
}

/// Where a rect light at `m`, `width` by `height`, has its card: the light's axes, each of its own length (a light's
/// scale does not size it), the card's width and height along the first two.
pub fn card_world(m: Mat4, width: f32, height: f32) -> Mat4 {
    let axis = |v: Vec3| m.transform_vector3(v).normalize();
    Mat4::from_cols((axis(Vec3::X) * width).extend(0.0), (axis(Vec3::Y) * height).extend(0.0), axis(Vec3::Z).extend(0.0), m.transform_point3(Vec3::ZERO).extend(1.0))
}

/// The transform of the instance that draws object `node`: the object's, or for a rect light its card's.
pub fn drawn_world(w: &World, node: u32) -> Mat4 {
    let m = w.world_matrix(node);
    match w.objects.get(&node).map(|o| &o.kind) {
        Some(Kind::Light(Light::Rect { width, height, visible: true, .. })) => card_world(m, *width, *height),
        _ => m,
    }
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
            Some(tris.iter().zip(&d.mats).map(|(t, &mat)| (t.p, tri_attr(t.n, t.uv, t.tangent, mat))).collect())
        }
        _ => None,
    })
}

/// A model's triangles by class (`classed`: opaque, cutout, glass; each a triangle's index and material): those that
/// lie on exactly another's vertices leave the mesh, in two cases: a cutout on a triangle that emits (a material of
/// `emitting`; Zero Day: masked decals over emissive quads of the same shape), and glass on an opaque triangle or
/// a cutout. A cutout hit that its alpha lets through steps the ray on past it (trace_hw.wgsl), and past what lies at
/// the same t: in one BLAS the emitter under the decal went with it (black through the decal); and which of two
/// triangles at one distance a closest hit reports is not said. Returns what leaves, each with its class: the
/// model's second mesh (`InstMesh::over`), a BLAS apart.
pub fn split_coincident(classed: &mut [Vec<(usize, u32)>; 4], emitting: &std::collections::HashSet<u32>, p: &dyn Fn(usize) -> [Vec3; 3]) -> Vec<(usize, u32, u32)> {
    let key = |k: usize| {
        let mut v = p(k).map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]);
        v.sort_unstable();
        v
    };
    type Keys = std::collections::HashSet<[[u32; 3]; 3]>;
    let emitters: Keys = classed[0].iter().filter(|(_, mat)| emitting.contains(mat)).map(|&(k, _)| key(k)).collect();
    let under: Keys = classed[..2].iter().flatten().map(|&(k, _)| key(k)).collect();
    let mut over = vec![];
    for (class, on) in [(1, &emitters), (2, &under)] {
        let (stay, go): (Vec<_>, Vec<_>) = classed[class].iter().partition(|&&(k, _)| !on.contains(&key(k)));
        classed[class] = stay;
        over.extend(go.into_iter().map(|(k, mat)| (k, mat, class as u32)));
    }
    over
}

/// A model's triangles `list` (each one's index in `local`, its material and its class) as a mesh of the pool, laid
/// out by section (`model_sections`); its rows, the file's materials, go to `mesh_rows`.
fn model_mesh(local: &[crate::skin::ModelTri], list: &[(usize, u32, u32)], pool: &mut Pool, mesh_rows: &mut HashMap<u32, Vec<Row>>) -> u32 {
    let first = pool.tris.len() as u32;
    let (order, sects, mine) = model_sections(list);
    for &(k, section, rank) in &order {
        let t = &local[k];
        pool.tris.push([t.p[0].into(), t.p[1].into(), t.p[2].into()]);
        pool.attrs.push(section_attr(t.n, t.uv, t.tangent, section, rank));
    }
    pool.meshes.push(InstMesh { sect_first: pool.sections.len() as u32, sect_count: sects.len() as u32, ..own_inst_mesh(&pool.tris[first as usize..], first) });
    pool.sections.extend(sects);
    let k = pool.meshes.len() as u32 - 1;
    mesh_rows.insert(k, mine);
    k
}

/// A model's meshes (`model_mesh`) of its triangles by class after `split_coincident` took `over` out: the mesh, with
/// the second one if anything left (`InstMesh::over`). None: the model has no triangle.
fn model_meshes(local: &[crate::skin::ModelTri], classed: &[Vec<(usize, u32)>; 4], over: &[(usize, u32, u32)], pool: &mut Pool, mesh_rows: &mut HashMap<u32, Vec<Row>>) -> Option<u32> {
    // (Nothing under what lies over: that is the mesh.)
    let (main, over) = match kept(classed) {
        main if main.is_empty() => (over.to_vec(), vec![]),
        main => (main, over.to_vec()),
    };
    (!main.is_empty()).then(|| {
        let k = model_mesh(local, &main, pool, mesh_rows);
        if !over.is_empty() {
            pool.meshes[k as usize].over = model_mesh(local, &over, pool, mesh_rows);
        }
        k
    })
}

/// Whether a material created since the build can be drawn in place (tables.rs `add_materials`): 0 only when it is
/// opaque, does not emit and is no glass (the build sorts triangles and lights by those).
pub fn material_class(g: &GpuMaterial) -> u32 {
    let other = g.layer[0] > 0.0 || g.emissive[..3].iter().any(|e| *e > 0.0);
    if other {
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

/// Whether an object with `materials` materials draws `g` by its groups (three.js's material array: each group with
/// its own, a triangle in no group not drawn) and not whole, with one material.
pub fn by_groups(g: &world::Geometry, materials: usize) -> bool {
    materials > 1 && !g.groups.is_empty()
}

/// What `section_mesh` makes: a mesh's triangles in model space and their attributes as the pool holds them, their
/// vertex colours if it carries them, the mesh and its sections.
pub struct MeshData {
    pub tris: Vec<[Vec3A; 3]>,
    pub attrs: Vec<GpuTriAttr>,
    pub colors: Option<Vec<[u32; 3]>>,
    pub mesh: InstMesh,
    pub sections: Vec<Section>,
}

/// The mesh of a geometry at pool index `first`, its sections from `sect_first` on: a section after another, each
/// the triangles of a group in the geometry's order (`grouped`: `by_groups`; a triangle is its last group's, and
/// one in no group is not drawn), or the whole geometry as one. A triangle's attributes hold its section (uv.w) and
/// its index in it (x.w, `rank_in_sections`): nothing of a material, so every object of the geometry shares the mesh.
/// The build and the live view's in-place edits both make it here.
pub fn section_mesh(g: &world::Geometry, grouped: bool, first: u32, sect_first: u32, colored: bool) -> MeshData {
    let mut group: Vec<Option<u32>> = vec![if grouped { None } else { Some(0) }; g.indices.len() / 3];
    if grouped {
        for (k, &[start, count, _]) in g.groups.iter().enumerate() {
            for t in (start / 3) as usize..((start + count) / 3) as usize {
                if let Some(slot) = group.get_mut(t) {
                    *slot = Some(k as u32);
                }
            }
        }
    }
    let (mut t, mut flags, mut a) = (vec![], vec![], vec![]);
    append_arrays(g, Mat4::IDENTITY, 0, 0, Some(&group), &mut t, &mut flags, &mut a);
    // (`append_arrays` left each triangle's group where its material goes.)
    let group_of = |k: usize| a[k].uv[3].to_bits();
    let mut order: Vec<usize> = (0..t.len()).collect();
    order.sort_by_key(|&k| group_of(k));
    let (mut sections, mut attrs): (Vec<Section>, Vec<GpuTriAttr>) = (vec![], Vec::with_capacity(order.len()));
    let mut last = None;
    for &k in &order {
        if last != Some(group_of(k)) {
            last = Some(group_of(k));
            sections.push(Section { first: attrs.len() as u32, count: 0, slot: if grouped { g.groups[group_of(k) as usize][2] } else { 0 } });
        }
        let (at, s) = (sections.len() as u32 - 1, sections.last_mut().unwrap());
        let mut b = a[k];
        (b.uv[3], b.x[3]) = (f32::from_bits(at), f32::from_bits(s.count));
        s.count += 1;
        attrs.push(b);
    }
    let tris: Vec<[Vec3A; 3]> = order.iter().map(|&k| t[k]).collect();
    let colored = colored && !g.colors.is_empty();
    let colors = colored.then(|| {
        let c = tri_colors(g, false, Some(&group.iter().map(|x| x.map(|_| 1)).collect::<Vec<_>>()));
        order.iter().map(|&k| c[k]).collect()
    });
    let mesh = InstMesh { sect_first, sect_count: sections.len() as u32, colored, ..own_inst_mesh(&tris, first) };
    MeshData { tris, attrs, colors, mesh, sections }
}

/// Each triangle's index in its section (x.w) as the triangles lie in `attrs`: its rank among those of its section
/// (uv.w). What a mesh's triangles get again when they are put in another order (a unit's tree's): an emitter's
/// place in its section's run is this.
pub fn rank_in_sections(attrs: &mut [GpuTriAttr]) {
    let mut seen: Vec<u32> = vec![];
    for a in attrs {
        let s = a.uv[3].to_bits() as usize;
        if seen.len() <= s {
            seen.resize(s + 1, 0);
        }
        a.x[3] = f32::from_bits(seen[s]);
        seen[s] += 1;
    }
}

/// A model's mesh laid out by section, a section a material in the order the materials first appear among the
/// triangles that stay in the mesh (`kept`: each one's index in the model's list, its GPU material and that
/// material's class, `mesh_class`), a section's triangles in the list's order. Returns the pool's order (the index
/// in the model's list, the triangle's section and its index in it), the sections, and an instance's rows for them
/// (`Retained::mesh_rows`: every instance of the mesh has the file's materials).
#[allow(clippy::type_complexity)]
pub fn model_sections(kept: &[(usize, u32, u32)]) -> (Vec<(usize, u32, u32)>, Vec<Section>, Vec<Row>) {
    let mut sorted = kept.to_vec();
    sorted.sort_unstable_by_key(|k| k.0);
    let mut section_of: HashMap<u32, u32> = HashMap::new();
    let (mut lists, mut rows): (Vec<Vec<usize>>, Vec<Row>) = (vec![], vec![]);
    for &(k, mat, class) in &sorted {
        let at = *section_of.entry(mat).or_insert_with(|| {
            lists.push(vec![]);
            rows.push([mat | [0, ROW_CUTOUT, ROW_GLASS][class as usize], 0, NO_EMITTER, 0]);
            lists.len() as u32 - 1
        });
        lists[at as usize].push(k);
    }
    let (mut order, mut sections) = (Vec::with_capacity(sorted.len()), Vec::with_capacity(lists.len()));
    for (at, list) in lists.iter().enumerate() {
        sections.push(Section { first: order.len() as u32, count: list.len() as u32, slot: at as u32 });
        order.extend(list.iter().enumerate().map(|(rank, &k)| (k, at as u32, rank as u32)));
    }
    (order, sections, rows)
}

/// A model's triangle as its mesh holds it: its attributes with its section and its index in it.
pub fn section_attr(n: [Vec3; 3], uv: [Vec2; 3], tangent: Option<CornerTangents>, section: u32, rank: u32) -> GpuTriAttr {
    let mut a = tri_attr(n, uv, tangent, section);
    a.x[3] = f32::from_bits(rank);
    a
}

/// The triangles of `classed` that stay in a mesh (`FLAT`'s leave it), for `model_sections`.
pub fn kept(classed: &[Vec<(usize, u32)>; 4]) -> Vec<(usize, u32, u32)> {
    classed[..FLAT].iter().enumerate().flat_map(|(class, list)| list.iter().map(move |&(k, mat)| (k, mat, class as u32))).collect()
}

/// An instance's rows for `sections`, drawn with the object's materials `mats` (GPU indices; a slot past the last is
/// the last, as three.js clamps).
pub fn section_rows(sections: &[Section], mats: &[u32], materials: &[GpuMaterial]) -> Vec<Row> {
    sections.iter().map(|s| row_of(mats[(s.slot as usize).min(mats.len() - 1)], materials)).collect()
}

/// A material that emits: the triangles drawn with it are their instance's emitters in the light list
/// (`rows_emitters`; a model's mesh: `InstMesh::emitters`).
pub fn emissive(g: &GpuMaterial) -> bool {
    g.emissive[..3].iter().any(|e| *e > 0.0)
}

/// The class of a material, which an instance's row of it says (`row_of`): 0 opaque, 1 cutout (alpha-tested),
/// 2 glass (shadow rays pass). A material function or emission is no class: the kernel places a
/// function's inputs by the instance (`mat_in_at`), and the light list has an instance's emitters.
pub fn mesh_class(g: &GpuMaterial) -> u32 {
    if g.layer[0] > 0.0 {
        2
    } else {
        u32::from(g.uv2[3] > 0.0)
    }
}

/// `classed`'s last list: what leaves a mesh and is flattened at each placement (`flatten_coincident`'s pairs; the
/// emitters of a mesh that deforms).
pub const FLAT: usize = 3;

/// The build's arrays a mesh of the pool and its instance are added to.
pub struct Pool<'a> {
    pub tris: &'a mut Vec<[Vec3A; 3]>,
    pub attrs: &'a mut Vec<GpuTriAttr>,
    pub meshes: &'a mut Vec<InstMesh>,
    pub sections: &'a mut Vec<Section>,
    pub instances: &'a mut Vec<Instance>,
    pub rows: &'a mut Vec<Row>,
}

/// A particle system's mesh and its one instance (`ParticleSystem::mesh`): `count` quads collapsed at `pivot` until
/// the pass places them, one section of material `mat`; the instance places nothing (the pass writes the vertices
/// in the world), has the system's first object (`object`: one a particle) and, unless the lights are sampled by
/// the system (`sampled`), a row that keeps the material's emission out of the light list. The mesh.
fn particle_mesh(count: u32, pivot: Vec3A, mat: u32, object: u32, sampled: bool, n: &world::Object, materials: &[GpuMaterial], pool: &mut Pool) -> u32 {
    let corners = [[Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0)], [Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)]];
    let first = pool.tris.len() as u32;
    for k in 0..count {
        for (j, uv) in corners.into_iter().enumerate() {
            pool.tris.push([pivot; 3]);
            pool.attrs.push(section_attr([Vec3::Z; 3], uv, Some([(Vec3::X, 1.0); 3]), 0, k * 2 + j as u32));
        }
    }
    pool.meshes.push(InstMesh { deforms: true, sect_first: pool.sections.len() as u32, sect_count: 1, ..own_inst_mesh(&pool.tris[first as usize..], first) });
    let mesh = pool.meshes.len() as u32 - 1;
    pool.sections.push(Section { first: 0, count: count * 2, slot: 0 });
    let mut mine = [row_of(mat, materials)];
    if !sampled {
        mine[0][3] = ROW_UNSAMPLED;
    }
    pool.instances.push(Instance { mesh, world: Mat4::IDENTITY, object, cast_shadow: rows_cast(&mine, n.cast_shadow), name: n.name.clone(), sub: None, hidden: false, masked: false, first_emitter: 0, rows: pool.rows.len() as u32, bits: rows_bits(&mine) | INST_PARTICLES, prev: 0, verts: 0 });
    pool.rows.extend(mine);
    mesh
}

/// The `InstMesh` of triangles at pool index `first`, with their bounds; its sections are the caller's to set.
pub fn own_inst_mesh(tris: &[[Vec3A; 3]], first: u32) -> InstMesh {
    let (mut lo, mut hi) = (Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN));
    for t in tris {
        for v in t {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    let count = tris.len() as u32;
    InstMesh { first, count, over: NO_OVER, lo, hi, deforms: false, sect_first: 0, sect_count: 0, colored: false, vfirst: 0, vcount: 0 }
}

/// The GPU material of each triangle of a mesh with several (`Kind::Mesh::materials`, here GPU indices) by its
/// geometry's groups; None for a triangle in no group (not drawn, as three.js). None for a mesh with one material.
pub fn group_materials(g: &world::Geometry, mats: &[u32]) -> Option<Vec<Option<u32>>> {
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
    tangent: Option<CornerTangents>,
    mat: u32,
    flags: u32,
) {
    tris.push([p[0].into(), p[1].into(), p[2].into()]);
    tri_flags.push(flags);
    attrs.push(tri_attr(n, uv, tangent, mat));
}

/// Shading attributes of a triangle: normals, UVs, its tangents if it has them, its material.
pub fn tri_attr(n: [Vec3; 3], uv: [Vec2; 3], tangent: Option<CornerTangents>, mat: u32) -> GpuTriAttr {
    let t = |c: usize| tangent.map_or([0.0; 4], |t| [t[c].0.x, t[c].0.y, t[c].0.z, t[c].1]);
    // (Its texel density, for the mip choice, is the hit's to compute: pathtrace.wgsl `mesh_density`.)
    GpuTriAttr {
        n0: [n[0].x, n[0].y, n[0].z, uv[0].x],
        n1: [n[1].x, n[1].y, n[1].z, uv[0].y],
        n2: [n[2].x, n[2].y, n[2].z, uv[1].x],
        uv: [uv[1].y, uv[2].x, uv[2].y, f32::from_bits(mat)],
        t: t(0),
        x: [0.0; 4],
        t1: t(1),
        t2: t(2),
    }
}

#[cfg(test)]
mod spot_cone_tests {
    use super::*;

    /// The kernel's falloff (pathtrace.wgsl `local_light`) at an angle from the spot's axis.
    fn falloff(angle: f32, penumbra: f32, at: f32) -> f32 {
        let (cos_outer, inv_width) = spot_cone(angle, penumbra);
        let t = ((at.cos() - cos_outer) * inv_width).clamp(0.0, 1.0);
        t * t
    }

    /// Nothing outside the cone, everything inside the inner cone (angle x (1 - penumbra)), and between them glTF
    /// KHR_lights_punctual's reference attenuation; a cone without penumbra is hard.
    #[test]
    fn the_cone_fades_over_the_penumbra() {
        let (angle, penumbra) = (52f32.to_radians(), 0.4);
        assert_eq!(falloff(angle, penumbra, angle + 1e-3), 0.0);
        assert_eq!(falloff(angle, penumbra, 1.2), 0.0);
        assert_eq!(falloff(angle, penumbra, angle * 0.6 - 1e-3), 1.0);
        assert_eq!(falloff(angle, penumbra, 0.0), 1.0);
        for at in [0.6f32, 0.7, 0.8, 0.9] {
            let (ci, co) = ((angle * 0.6).cos(), angle.cos());
            let scale = 1.0 / (ci - co).max(0.001);
            let want = (at.cos() * scale - co * scale).clamp(0.0, 1.0).powi(2);
            assert!((falloff(angle, penumbra, at) - want).abs() < 1e-5 && want > 0.0 && want < 1.0, "{at}: {want}");
        }
        assert_eq!(falloff(0.35, 0.0, 0.34), 1.0);
        assert_eq!(falloff(0.35, 0.0, 0.3501), 0.0);
        // The kernel reads the pair from the light's `ext.xy`.
        let l = gpu_light(&Light::Spot { color: [1.0; 3], intensity: 1.0, radius: 0.0, range: 0.0, angle, penumbra, cast_shadow: true, volumetric_scattering: 1.0, specular_scale: 1.0 }, Mat4::IDENTITY, 1.0).unwrap();
        assert_eq!((l.ext[0], l.ext[1]), spot_cone(angle, penumbra));
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

    /// An instance's placed triangle and attributes equal flattening the same triangle at the instance transform: as
    /// the flattening path has them (`Instance::flat`) against the triangle built in world space as `append_arrays`
    /// builds it, under a transform that keeps the hand and under one that mirrors (the second and third vertex
    /// change places, the tangent frame changes hand), with distinct normals, UVs, tangents and vertex colours a
    /// vertex.
    #[test]
    fn instance_matches_flattening() {
        let p = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.5, 0.2)];
        let n = [Vec3::new(0.0, -0.37, 0.93).normalize(), Vec3::new(0.1, -0.3, 0.9).normalize(), Vec3::new(-0.2, -0.4, 0.8).normalize()];
        let uv = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.25), Vec2::new(0.5, 1.0)];
        let colors = [1, 2, 3];
        let keeps = Mat4::from_scale_rotation_translation(Vec3::splat(2.0), glam::Quat::from_rotation_y(0.7) * glam::Quat::from_rotation_x(-0.3), Vec3::new(5.0, 1.0, -3.0));
        let tg = [(Vec3::new(1.0, 0.1, 0.0).normalize(), 1.0), (Vec3::new(0.9, 0.0, 0.3).normalize(), 1.0), (Vec3::new(0.8, -0.2, 0.1).normalize(), 1.0)];
        let local = tri_attr(n, uv, Some(tg), 7);
        for (w, mirrored) in [(keeps, false), (keeps * Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)), true)] {
            assert_eq!(mirrors(w), mirrored);
            let inst = Instance { mesh: 0, world: w, object: 3, cast_shadow: true, name: "x".into(), sub: None, hidden: false, masked: false, first_emitter: 0, rows: 0, bits: 0, prev: 0, verts: 0 };
            assert_eq!(inst.gpu(0).n[1][3].to_bits(), if mirrored { INST_MIRRORED } else { 0 }, "the record says whether it mirrors");
            let (placed, got, got_colors) = inst.flat(&[p[0].into(), p[1].into(), p[2].into()], &local, Some(colors));
            // The world-space triangle: its vertices in the order the build flattens a placement in.
            let order = if mirrored { [0, 2, 1] } else { [0, 1, 2] };
            let nm = glam::Mat3::from_mat4(w).inverse().transpose();
            let wp = order.map(|k| w.transform_point3(p[k]));
            let hand = if mirrored { -1.0 } else { 1.0 };
            let flat = tri_attr(order.map(|k| (nm * n[k]).normalize()), order.map(|k| uv[k]), Some(order.map(|k| (w.transform_vector3(tg[k].0).normalize(), tg[k].1 * hand))), 7);
            for k in 0..3 {
                assert!((Vec3::from(placed[k]) - wp[k]).length() < 1e-5, "mirrored {mirrored}: vertex {k}");
            }
            assert!((placed[1] - placed[0]).cross(placed[2] - placed[0]).dot(Vec3A::from(nm * n[0])) > 0.0, "mirrored {mirrored}: it faces as its normals do");
            for (a, b) in [(got.n0, flat.n0), (got.n1, flat.n1), (got.n2, flat.n2), (got.t, flat.t), (got.t1, flat.t1), (got.t2, flat.t2)] {
                assert!((Vec3::new(a[0], a[1], a[2]) - Vec3::new(b[0], b[1], b[2])).length() < 1e-5, "mirrored {mirrored}: {a:?} vs {b:?}");
                assert_eq!(a[3], b[3], "mirrored {mirrored}: the UV packed there, the tangent frame's hand");
            }
            assert_eq!(got.uv, flat.uv, "mirrored {mirrored}");
            // (`placed_tri` places a triangle the same way.)
            let (_, pn, puv, ptg) = placed_tri(w, &p, &n, &uv, Some(tg));
            let by = tri_attr(pn, puv, ptg, 7);
            for (a, b) in [(by.n0, flat.n0), (by.n1, flat.n1), (by.n2, flat.n2), (by.t, flat.t), (by.t1, flat.t1), (by.t2, flat.t2)] {
                assert!((Vec3::new(a[0], a[1], a[2]) - Vec3::new(b[0], b[1], b[2])).length() < 1e-5 && a[3] == b[3], "mirrored {mirrored}, placed_tri: {a:?} vs {b:?}");
            }
            assert_eq!(got_colors, Some(order.map(|k| colors[k])), "mirrored {mirrored}: the vertex colours follow their vertices");
            assert_eq!(got.x[1].to_bits(), 3);
        }
    }

    /// Software traversal flattens instances: a cutout material's triangles keep TRI_MASKED (any-hit alpha test),
    /// the placement's shadow flag applies to all.
    #[test]
    fn flattened_instance_keeps_cutout_flag() {
        let opaque = GpuMaterial::default();
        let mut cutout = GpuMaterial::default();
        cutout.uv2[3] = 0.5;
        let mats = [opaque, cutout];
        let a = |m: u32| tri_attr([Vec3::Z; 3], [Vec2::ZERO, Vec2::X, Vec2::Y], None, m);
        let inst = |shadow: bool| Instance { mesh: 0, world: Mat4::IDENTITY, object: 0, cast_shadow: shadow, name: "x".into(), sub: None, hidden: false, masked: false, first_emitter: 0, rows: 0, bits: 0, prev: 0, verts: 0 };
        assert_eq!(instance_tri_flags(&inst(true), &a(0), &mats), 0);
        assert_eq!(instance_tri_flags(&inst(true), &a(1), &mats), TRI_MASKED);
        assert_eq!(instance_tri_flags(&inst(false), &a(1), &mats), TRI_MASKED | TRI_NO_SHADOW);
    }

    /// A node's triangles may come in two ranges (what the build flattened of an instance's mesh, the mesh appended
    /// by software traversal): selection outlines need both.
    /// An instanced object: one shared mesh, an instance and an object entry per copy (capacity, not count), hidden
    /// past the count and where a matrix scales to 0; a copy may mirror the mesh.
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
        assert!(sc.tinted && sc.retained.instanced(2).unwrap().capacity == 4);
        let mut m = m.map(|m| m.to_cols_array()).concat();
        m[0] = -1.0;
        w.set_instances(2, 3, Some(m), None, None).unwrap();
        let sc = build(&w, Path::new("."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!((sc.instances.len(), sc.tris.len()), (4, 0), "a copy that mirrors the mesh is an instance as the others");
        assert_eq!(sc.instances.iter().map(|i| i.gpu(0).n[1][3].to_bits()).collect::<Vec<_>>(), [INST_MIRRORED, 0, 0, 0], "and its record says so");
    }

    /// A visible rect light's card is an instance of one card mesh: the lights share the mesh, each instance has its
    /// light's size and place, its material in its row, and casts no shadow; its two triangles stand 1 mm in front of
    /// the light, facing where it shines, and are its emitters, marked as a card's. A light without size has an
    /// instance no ray sees. No world-space triangle. No GPU.
    #[test]
    fn a_rect_lights_card_is_an_instance_of_the_card_mesh() {
        let mut w = World::default();
        let rect = |width: f32, height: f32| Kind::Light(serde_json::from_value(serde_json::json!({ "type": "rect", "color": [1, 0.5, 0.25], "castShadow": true, "intensity": 3, "width": width, "height": height, "range": 0 })).unwrap());
        w.set_object(1, "a".into(), None, rect(2.0, 1.0), true, false).unwrap();
        // (Turned to shine along +X, and scaled: a light's scale does not size its card.)
        w.set_transform(1, Vec3::new(1.0, 2.0, 3.0), glam::Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2), Vec3::new(5.0, 7.0, 9.0)).unwrap();
        w.set_object(2, "b".into(), None, rect(0.5, 0.5), true, true).unwrap();
        w.set_object(3, "none".into(), None, rect(0.0, 1.0), true, false).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(9, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_settings(world::Settings { camera: Some(9), ..Default::default() });
        let mut sc = build(&w, Path::new("."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!((sc.tris.len(), sc.meshes.len(), sc.instances.len(), sc.lights.len()), (0, 1, 3, 0), "one mesh, an instance a card, no table light");
        let m = sc.meshes[0];
        assert_eq!((m.count, m.sect_count, m.deforms), (2, 1, false));
        let inst = |id: u32| sc.instances[sc.retained.instance(id).expect("a card is an instance").instance as usize].clone();
        let (a, b, none) = (inst(1), inst(2), inst(3));
        assert_eq!([a.mesh, b.mesh, none.mesh], [0; 3], "the cards share the mesh");
        assert_eq!([(a.cast_shadow, a.hidden), (b.cast_shadow, b.hidden), (none.cast_shadow, none.hidden && none.masked)], [(false, false), (false, false), (false, true)], "no card casts a shadow; one without size is unseen");
        assert_eq!([sc.retained.instance(1).unwrap().moves, sc.retained.instance(2).unwrap().moves], [false, true], "a movable light's card moves as its instance");
        // The first card in the world: 2 m along the light's X (the world's Z here), 1 m up, 1 mm in front (+X).
        let corners: Vec<Vec3> = sc.pool_tris[m.first as usize..(m.first + m.count) as usize].iter().flatten().map(|p| a.world.transform_point3((*p).into())).collect();
        let (lo, hi) = corners.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        assert!(lo.abs_diff_eq(Vec3::new(1.001, 1.5, 2.0), 1e-5) && hi.abs_diff_eq(Vec3::new(1.001, 2.5, 4.0), 1e-5), "{lo} to {hi}");
        for t in corners.chunks(3) {
            assert!((t[1] - t[0]).cross(t[2] - t[0]).normalize().abs_diff_eq(Vec3::X, 1e-5), "a triangle faces where the light shines");
        }
        // Each row has its card's own material, which emits; the emitters are the instances'.
        let mats: Vec<u32> = [&a, &b].iter().map(|i| row_material(&sc.rows[i.rows as usize])).collect();
        assert!(mats[0] != mats[1] && mats.iter().all(|&k| sc.materials[k as usize].tex1[2] == MAT_EMITTER_CARD) && sc.retained.proxies[&1].card == Some(mats[0]));
        let p = crate::prepare_as(true, false, &mut sc, &mut vec![]);
        // (A card no ray sees keeps its place in the list, as every hidden instance: its object has no power.)
        assert_eq!(p.emitter_tris.len(), 6, "two triangles a card");
        assert!(p.emitter_tris.iter().all(|e| e.card && e.inst != crate::NO_INST) && p.gpu_emitter_tris.iter().all(|e| e.flags == crate::EMITTER_CARD));
    }

    /// An instanced object whose material only emits is placed as any other: one shared mesh, which has the emitters,
    /// an instance a copy; a hidden copy's entry has no power. `prepare` gives every copy, the hidden too, its run of
    /// the light list, of emitters that move: a hidden copy's at its mesh's own area and no weight.
    #[test]
    fn an_emitting_instanced_object_is_instances() {
        let mut w = World::default();
        w.set_material(1, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0, 0, 0], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 1, 1], "emissiveIntensity": 5, "doubleSided": false })).unwrap());
        let g = world::Geometry { positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], normals: vec![0.0, 0.0, 1.0].repeat(3), uvs: vec![0.0; 6], indices: vec![0, 1, 2], groups: vec![], colors: vec![] };
        w.set_geometry(1, g).unwrap();
        w.set_object(2, "lamps".into(), None, Kind::Instanced { geometry: 1, procedural: false, materials: vec![1], capacity: 4 }, true, false).unwrap();
        w.set_power(2, 2.0).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(9, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_settings(world::Settings { camera: Some(9), ..Default::default() });
        let m = [Mat4::from_scale(Vec3::splat(2.0)), Mat4::from_translation(Vec3::X), Mat4::from_scale(Vec3::ZERO), Mat4::from_translation(Vec3::Y)];
        w.set_instances(2, 3, Some(m.iter().flat_map(|m| m.to_cols_array()).collect()), None, None).unwrap();
        let mut sc = build(&w, Path::new("."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!((sc.meshes.len(), sc.meshes[0].sect_count, sc.instances.len(), sc.tris.len()), (1, 1, 4, 0));
        assert_eq!(sc.instances.iter().map(|i| (i.rows, i.bits)).collect::<Vec<_>>(), [(0, 0), (1, 0), (2, 0), (3, 0)], "a row a copy");
        let (st, first) = (sc.object_stride(), sc.retained.instanced(2).unwrap().object as usize);
        assert_eq!((0..4).map(|i| sc.objects[(first + i) * st][3]).collect::<Vec<_>>(), [2.0, 2.0, 0.0, 0.0], "a hidden copy's entry has no power");
        let prep = crate::prepare_as(true, false, &mut sc, &mut vec![]);
        let e = &prep.emitter_tris;
        assert_eq!(e.iter().map(|e| (e.inst, e.obj as usize - first, e.moves, e.area)).collect::<Vec<_>>(), [(0, 0, true, 2.0), (1, 1, true, 0.5), (2, 2, true, 0.5), (3, 3, true, 0.5)]);
        assert_eq!(sc.instances.iter().map(|i| i.first_emitter).collect::<Vec<_>>(), [0, 1, 2, 3]);
        let power = |i: usize| crate::emitter_power(&e[i], &sc.materials, &sc.objects, st);
        assert_eq!([power(0), power(1), power(2), power(3)], [4.0 * power(1), power(1), 0.0, 0.0], "weights: by size, none when hidden");
        assert!(power(1) > 0.0 && crate::switchable_power(&e[2], &sc.materials, &sc.objects, st) > 0.0, "a hidden copy's emitter is one the light tree would have");
    }

    /// A model placed mirrored (a negative scale) is an instance as one placed straight is, of the same mesh: a model
    /// that stays and a rigid model's moving node (tests/render/data/panel-decal.glb). The meshes do not depend on
    /// the placement's hand; the instances' records say that they mirror, a second mesh's instance too.
    #[test]
    fn a_mirrored_model_is_an_instance_of_the_same_mesh() {
        let scene = |x: f32| {
            let mut w = World::default();
            let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
            w.set_object(9, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
            w.set_settings(world::Settings { camera: Some(9), ..Default::default() });
            let model = |animation| Kind::Model { uri: "tests/render/data/panel-decal.glb".into(), nodes: None, animation, functions: vec![] };
            w.set_object(1, "still".into(), None, model(None), true, false).unwrap();
            w.set_object(2, "posed".into(), None, model(Some(desc::Animation { clip: "slide".into(), time: 0.25, r#loop: true, nodes: Default::default() })), true, false).unwrap();
            for id in [1, 2] {
                w.set_transform(id, Vec3::new(id as f32, 0.0, 0.0), glam::Quat::from_rotation_y(0.4), Vec3::new(x, 1.0, 1.0)).unwrap();
            }
            build(&w, Path::new(".."), &crate::default_presets_dir(), None).unwrap()
        };
        let (straight, mirrored) = (scene(1.0), scene(-1.0));
        let meshes = |sc: &Scene| sc.meshes.iter().map(|m| (m.first, m.count, m.sect_count, m.over)).collect::<Vec<_>>();
        assert_eq!(meshes(&mirrored), meshes(&straight));
        assert_eq!(mirrored.pool_tris, straight.pool_tris, "the meshes' triangles are the model's own, whatever places them");
        assert_eq!((mirrored.instances.len(), mirrored.tris.len()), (straight.instances.len(), straight.tris.len()));
        assert_eq!((&mirrored.sections, &mirrored.rows), (&straight.sections, &straight.rows), "and their sections and rows");
        assert!(straight.instances.len() >= 2 && straight.instances.iter().all(|i| i.gpu(0).n[1][3].to_bits() & INST_MIRRORED == 0));
        assert!(mirrored.instances.iter().all(|i| i.gpu(0).n[1][3].to_bits() & INST_MIRRORED != 0), "every instance of a mirrored model mirrors");
        // (The cutout on an emitter: a mesh apart, a still model's and a moving node's, an instance each.)
        assert!(straight.tris.is_empty() && straight.meshes.iter().filter(|m| m.over != NO_OVER).count() == 2 && straight.instances.len() == 4);
    }

    /// A mesh of a geometry drawn by its groups lies by section, a group after another: a triangle in no group is not
    /// drawn, one in two groups is its last group's. Its triangles hold their section and their index in it and
    /// nothing of a material; an instance's rows say the materials, by the groups' slots: their classes, whether it
    /// casts, how many emitters it has and where each section's start. Drawn with one material, the geometry whole is
    /// one section.
    #[test]
    fn a_mesh_lies_by_section_and_an_instances_rows_say_its_materials() {
        // Five quads along x: a group each but the third, which has none; a fifth group takes the first quad's second
        // triangle.
        let (mut positions, mut indices) = (vec![], vec![]);
        for q in 0..5u32 {
            let x = q as f32;
            positions.extend([x, 0.0, 0.0, x + 1.0, 0.0, 0.0, x + 1.0, 1.0, 0.0, x, 1.0, 0.0]);
            indices.extend([0, 1, 2, 0, 2, 3].map(|i| 4 * q + i));
        }
        let colors: Vec<f32> = (0..20).flat_map(|v| [(v / 4) as f32 / 4.0, 0.0, 0.0]).collect();
        let g = world::Geometry { normals: [0.0, 0.0, 1.0].repeat(20), uvs: vec![0.0; 40], positions, indices, groups: vec![[0, 6, 0], [6, 6, 1], [18, 6, 2], [24, 6, 3], [3, 3, 1]], colors: vec![] };
        assert!(by_groups(&g, 2) && !by_groups(&g, 1) && !by_groups(&world::Geometry { groups: vec![], ..g.clone() }, 4));
        let d = section_mesh(&g, true, 10, 7, true);
        let sect = |first, count, slot| Section { first, count, slot };
        assert_eq!(d.sections, [sect(0, 1, 0), sect(1, 2, 1), sect(3, 2, 2), sect(5, 2, 3), sect(7, 1, 1)]);
        assert_eq!(d.tris.iter().map(|t| t[0].x.min(t[1].x).min(t[2].x)).collect::<Vec<_>>(), [0.0, 1.0, 1.0, 3.0, 3.0, 4.0, 4.0, 0.0], "each triangle in its last group's section");
        assert_eq!(d.attrs.iter().map(|a| (a.uv[3].to_bits(), a.x[3].to_bits())).collect::<Vec<_>>(), [(0, 0), (1, 0), (1, 1), (2, 0), (2, 1), (3, 0), (3, 1), (4, 0)]);
        let im = d.mesh;
        assert_eq!((im.first, im.count, im.sect_first, im.sect_count, im.colored), (10, 8, 7, 5, false), "no colours: the geometry has none");
        assert_eq!((im.lo, im.hi), (Vec3A::new(0.0, 0.0, 0.0), Vec3A::new(5.0, 1.0, 0.0)), "bounds of what is drawn");
        let mut all = vec![sect(9, 9, 9); 7];
        all.extend(&d.sections);
        assert_eq!(im.parts(&all), [(0, 1), (1, 2), (3, 2), (5, 2), (7, 1)], "a geometry a section");
        assert!(d.colors.is_none());
        // Its geometry's colours, where it has them and they are asked for: the drawn triangles', as they lie.
        let tinted = section_mesh(&world::Geometry { colors, ..g.clone() }, true, 0, 0, true);
        let red = |c: &[u32; 3]| c[0] & 0xff;
        assert!(tinted.mesh.colored && !section_mesh(&world::Geometry { colors: vec![1.0; 60], ..g.clone() }, true, 0, 0, false).mesh.colored);
        assert_eq!(tinted.colors.as_ref().map(|c| c.iter().map(red).collect::<Vec<_>>()), Some(vec![0, 137, 137, 225, 225, 255, 255, 0]));
        // An instance's rows: glass, a cutout, an emitter, a plain material by the slots 0 to 3.
        let materials = [GpuMaterial { layer: [1.0, 1.5, 0.0, 0.0], ..Default::default() }, GpuMaterial { uv2: [0.0, 0.0, 0.0, 0.5], ..Default::default() }, GpuMaterial { emissive: [1.0, 1.0, 1.0, 0.0], ..Default::default() }, GpuMaterial::default()];
        let rows = section_rows(&d.sections, &[0, 1, 2, 3], &materials);
        assert_eq!(rows, [[ROW_GLASS, 0, NO_EMITTER, 0], [1 | ROW_CUTOUT, 0, NO_EMITTER, 0], [2, 0, NO_EMITTER, 0], [3, 0, NO_EMITTER, 0], [1 | ROW_CUTOUT, 0, NO_EMITTER, 0]]);
        assert_eq!(rows.iter().map(row_material).collect::<Vec<_>>(), [0, 1, 2, 3, 1]);
        assert_eq!((rows_cast(&rows, true), rows_cast(&rows, false), rows_bits(&rows)), (true, false, INST_CUTOUT | INST_GLASS));
        let starts = section_emitters(&d.sections, &rows, &materials);
        assert_eq!((rows_emitters(&d.sections, &rows, &materials), &starts[..]), (2, &[None, None, Some(0), None, None][..]));
        assert_eq!((section_slot(&d.attrs[4], &rows, &starts), section_slot(&d.attrs[5], &rows, &starts)), (Some((1, 2, 2)), None), "an emitter's place (its section's start and its index), its material and its section");
        // Two materials for four slots: a slot past the last is the last. Emitters in two sections, one after another.
        let two = section_rows(&d.sections, &[2, 0], &materials);
        assert_eq!(two.iter().map(row_material).collect::<Vec<_>>(), [2, 0, 0, 0, 0]);
        assert_eq!((rows_emitters(&d.sections, &two, &materials), rows_bits(&two), rows_cast(&two, true)), (1, INST_GLASS, true));
        let lit = section_rows(&d.sections, &[2, 3, 3, 2], &materials);
        assert_eq!((rows_emitters(&d.sections, &lit, &materials), section_emitters(&d.sections, &lit, &materials)), (3, vec![Some(0), None, None, Some(1), None]));
        // All glass: it casts no shadow, whatever its object says.
        let glass = section_rows(&d.sections, &[0, 0], &materials);
        assert_eq!((rows_cast(&glass, true), rows_bits(&glass)), (false, INST_GLASS));
        // Put in another order (a unit's tree's), the triangles are numbered again in their sections.
        let mut back: Vec<GpuTriAttr> = d.attrs.iter().rev().copied().collect();
        rank_in_sections(&mut back);
        assert_eq!(back.iter().map(|a| (a.uv[3].to_bits(), a.x[3].to_bits())).collect::<Vec<_>>(), [(4, 0), (3, 0), (3, 1), (2, 0), (2, 1), (1, 0), (1, 1), (0, 0)]);
        // One material: the geometry whole, as it has its triangles, one section.
        let one = section_mesh(&g, false, 0, 0, false);
        assert_eq!((one.sections.clone(), one.mesh.sect_count, one.attrs.len()), (vec![sect(0, 10, 0)], 1, 10));
        assert_eq!(one.attrs.iter().map(|a| (a.uv[3].to_bits(), a.x[3].to_bits())).collect::<Vec<_>>(), (0..10).map(|k| (0, k)).collect::<Vec<_>>());
        assert_eq!(section_rows(&one.sections, &[1], &materials), [[1 | ROW_CUTOUT, 0, NO_EMITTER, 0]]);
    }

    /// A model's mesh lies by section, a section a material of the file (tests/render/data/frame-pane.glb: an opaque,
    /// a cutout and a glass quad; placed twice): one mesh of three sections, nothing flattened. Each instance has its
    /// three rows, with the classes, and casts (not all of it is glass); its record has the rows' first, a row where
    /// its section's triangles start; the kernel's features for a cutout and for glass among other sections are set.
    #[test]
    fn a_models_mesh_lies_by_section_a_material_of_the_file() {
        let mut w = World::default();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(9, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_settings(world::Settings { camera: Some(9), ..Default::default() });
        for id in [1, 2] {
            w.set_object(id, format!("model{id}"), None, Kind::Model { uri: "tests/render/data/frame-pane.glb".into(), nodes: None, animation: None, functions: vec![] }, true, false).unwrap();
            w.set_transform(id, Vec3::new(id as f32 * 5.0, 0.0, 0.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        }
        let mut sc = build(&w, Path::new(".."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!(sc.meshes.iter().map(|m| (m.count, m.sect_count, m.over)).collect::<Vec<_>>(), [(6, 3, NO_OVER)]);
        assert_eq!((sc.instances.len(), sc.tris.len()), (2, 0));
        assert_eq!(sc.meshes[0].parts(&sc.sections), [(0, 2), (2, 2), (4, 2)], "a geometry a section");
        assert_eq!(sc.pool_attrs.iter().map(|a| (a.uv[3].to_bits(), a.x[3].to_bits())).collect::<Vec<_>>(), [(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)], "a triangle's section and its index in it");
        assert_eq!(sc.instances.iter().map(|i| (i.rows, i.bits, i.cast_shadow)).collect::<Vec<_>>(), [(0, INST_CUTOUT | INST_GLASS, true), (3, INST_CUTOUT | INST_GLASS, true)], "rows of its own an instance");
        for i in &sc.instances {
            let mine = &sc.rows[i.rows as usize..][..3];
            let mut classes: Vec<u32> = mine.iter().map(|r| r[0] & (ROW_CUTOUT | ROW_GLASS)).collect();
            classes.sort_unstable();
            assert_eq!(classes, [0, ROW_CUTOUT, ROW_GLASS], "an opaque, a cutout and a glass section");
            assert_eq!(mine, &sc.retained.mesh_rows[&i.mesh][..], "the mesh's rows, the file's materials");
        }
        crate::prepare_as(true, false, &mut sc, &mut vec![]);
        for (r, i) in sc.gpu_instances.iter().zip(&sc.instances) {
            assert_eq!(r.info[3], i.rows, "the record has its rows' first");
            assert_eq!(sc.rows[i.rows as usize..][..3].iter().map(|row| row[1] - r.info[0]).collect::<Vec<_>>(), [0, 2, 4], "a row has where its section starts");
        }
        let f = crate::gpu::instance_features(&sc);
        assert_eq!(f & (crate::gpu::F_IGLASS | crate::gpu::F_MASKED), crate::gpu::F_IGLASS | crate::gpu::F_MASKED);
        // The sections of a list of triangles: a material after another as they first appear, the list's order in each.
        let (order, sections, rows) = model_sections(&[(5, 9, 2), (0, 7, 0), (3, 9, 2), (1, 8, 1), (4, 7, 0)]);
        assert_eq!(order, [(0, 0, 0), (4, 0, 1), (1, 1, 0), (3, 2, 0), (5, 2, 1)]);
        assert_eq!(sections, [Section { first: 0, count: 2, slot: 0 }, Section { first: 2, count: 1, slot: 1 }, Section { first: 3, count: 2, slot: 2 }]);
        assert_eq!(rows, [[7, 0, NO_EMITTER, 0], [8 | ROW_CUTOUT, 0, NO_EMITTER, 0], [9 | ROW_GLASS, 0, NO_EMITTER, 0]]);
    }

    /// A model placed without size (scene code hides by scaling to 0) is an instance no ray sees, of the model's
    /// mesh, as one with a size is: nothing of it is a world-space triangle, and its object has no power.
    #[test]
    fn a_model_without_size_is_an_instance_no_ray_sees() {
        let mut w = World::default();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(9, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_settings(world::Settings { camera: Some(9), ..Default::default() });
        for (id, scale) in [(1, Vec3::ONE), (2, Vec3::ZERO), (3, Vec3::new(1.0, 0.0, 1.0))] {
            w.set_object(id, format!("model{id}"), None, Kind::Model { uri: "tests/render/data/frame-pane.glb".into(), nodes: None, animation: None, functions: vec![] }, true, false).unwrap();
            w.set_transform(id, Vec3::new(id as f32 * 5.0, 0.0, 0.0), glam::Quat::IDENTITY, scale).unwrap();
        }
        let sc = build(&w, Path::new(".."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!((sc.tris.len(), sc.meshes.len(), sc.instances.len()), (0, 1, 3), "one mesh, an instance a placement");
        assert_eq!(sc.instances.iter().map(|i| (i.hidden, i.masked)).collect::<Vec<_>>(), [(false, false), (true, true), (true, true)]);
        assert!(sc.instances.iter().all(|i| i.world.is_finite() && glam::Mat3::from_mat4(i.world).determinant() != 0.0), "each stands somewhere");
        let power = |id: u32| sc.objects[sc.retained.proxies[&id].object.unwrap() as usize * object_stride(sc.tinted)][3];
        assert_eq!([power(1), power(2), power(3)], [1.0, 0.0, 0.0], "an unseen model's object has no power");
        assert!(matches!(sc.retained.proxies[&2].draw, Draw::Instance(_)), "it comes back as its instance does");
    }

    /// A cutout on exactly an emitter's vertices leaves the mesh for the model's second one (`split_coincident`), in a
    /// model that stays and in a rigid model's moving node (tests/render/data/panel-decal.glb: panel.glb, a grey quad
    /// and an emitting one, with a masked quad on the emitting one's vertices): the emitter stays in its mesh, the
    /// cutout is a mesh apart with an instance beside the first's, and nothing is a world-space triangle.
    #[test]
    fn a_cutout_on_an_emitter_is_the_models_second_mesh() {
        let mut w = World::default();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(9, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_settings(world::Settings { camera: Some(9), ..Default::default() });
        let model = |animation| Kind::Model { uri: "tests/render/data/panel-decal.glb".into(), nodes: None, animation, functions: vec![] };
        w.set_object(1, "still".into(), None, model(None), true, false).unwrap();
        w.set_object(2, "posed".into(), None, model(Some(desc::Animation { clip: "slide".into(), time: 0.25, r#loop: true, nodes: Default::default() })), true, false).unwrap();
        let sc = build(&w, Path::new(".."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!(sc.tris.len(), 0, "no world-space triangle");
        // An object's instances: (triangles of the mesh, whether it has a second mesh, the instance's emitters, its rows' classes).
        let of = |name: &str| {
            sc.instances
                .iter()
                .filter(|i| i.name == name)
                .map(|i| {
                    let m = &sc.meshes[i.mesh as usize];
                    let rows = &sc.rows[i.rows as usize..][..m.sect_count as usize];
                    (m.count, m.over != NO_OVER, rows_emitters(m.sections(&sc.sections), rows, &sc.materials), rows.iter().map(|r| r[0] & (ROW_CUTOUT | ROW_GLASS)).collect::<Vec<_>>())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(of("still"), [(4, true, 2, vec![0, 0]), (2, false, 0, vec![ROW_CUTOUT])], "the model's mesh (the grey quad and the emitter), then the cutout's");
        let still: Vec<&Instance> = sc.instances.iter().filter(|i| i.name == "still").collect();
        assert!(still[0].world == still[1].world && still[0].object == still[1].object && sc.meshes[still[0].mesh as usize].over == still[1].mesh, "the second instance is of the mesh's second, at the same place, of the same object");
        assert!(matches!(sc.retained.proxies[&1].draw, Draw::Fixed), "a model of two instances is the build's to change");
        // The posed model's moving node: the same two meshes of its own, both instances the pose's to place.
        assert_eq!(of("posed"), of("still"));
        let Draw::Rigid(r) = &sc.retained.proxies[&2].draw else { panic!("the posed model is rigid") };
        assert!(r.still.is_none() && r.instances.len() == 2 && r.instances[0].0 == r.instances[1].0, "{:?}", r.instances);
    }

    /// A posed rigid model is its parts (tests/render/data/panel-slider.glb: a node its clip leaves alone, a grey quad
    /// and an emitting one, and a node it moves, a grey quad): the nodes left alone one mesh in the model's space,
    /// the moved node a mesh in its own, both shared by the placements of the model with the clip, whatever its time.
    /// Nothing is flattened. The part left alone's emitters are still ones, movable object or not.
    #[test]
    fn a_posed_rigid_model_is_instances_of_its_parts_meshes() {
        let mut w = World::default();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(9, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_settings(world::Settings { camera: Some(9), ..Default::default() });
        for (id, time, movable) in [(1, 0.25, false), (2, 0.75, true)] {
            let animation = Some(desc::Animation { clip: "slide".into(), time, r#loop: true, nodes: Default::default() });
            w.set_object(id, format!("model{id}"), None, Kind::Model { uri: "tests/render/data/panel-slider.glb".into(), nodes: None, animation, functions: vec![] }, true, movable).unwrap();
            w.set_transform(id, Vec3::new(id as f32 * 20.0, 0.0, 0.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        }
        let mut sc = build(&w, Path::new(".."), &crate::default_presets_dir(), None).unwrap();
        assert_eq!(sc.meshes.iter().map(|m| (m.count, m.sect_count, m.over)).collect::<Vec<_>>(), [(4, 2, NO_OVER), (2, 1, NO_OVER)], "the part left alone (two materials), the moved node");
        assert_eq!((sc.meshes[0].lo.x, sc.meshes[0].hi.x, sc.meshes[1].lo.x, sc.meshes[1].hi.x), (1.0, 9.0, -1.0, 1.0), "the part left alone in the model's space (its node 5 m along x), the moved node in its own");
        assert_eq!((sc.instances.len(), sc.tris.len()), (4, 0), "two instances a placement, nothing flattened");
        let rigid = |sc: &Scene, id: u32| match &sc.retained.proxies[&id].draw {
            Draw::Rigid(r) => r.clone(),
            _ => panic!("a posed rigid model"),
        };
        for (id, y, movable) in [(1, 0.75, false), (2, 2.25, true)] {
            let r = rigid(&sc, id);
            let (still, node) = (&sc.instances[r.still.expect("it has nodes left alone") as usize], &sc.instances[r.instances[0].1 as usize]);
            assert_eq!((r.instances.len(), r.instances[0].0, still.mesh, node.mesh), (1, 1, 0, 1), "model {id}: the meshes every placement shares");
            let at = id as f32 * 20.0;
            assert_eq!((still.world.w_axis.truncate(), node.world.w_axis.truncate()), (Vec3::new(at, 0.0, 0.0), Vec3::new(at + 11.0, y, 0.0)), "model {id}: the object's place, and the node's by the clip");
            assert_eq!((r.records().len(), r.moving(), movable), (2, vec![r.instances[0].1], id == 2), "model {id}: the node alone has moving emitters at the build");
            assert_eq!(&sc.rows[still.rows as usize..][..2], &sc.retained.mesh_rows[&0][..], "model {id}: its rows are the mesh's");
        }
        // Its emitters are its instances': the part left alone's two triangles a placement, still ones (in the light
        // tree) until the object first moves.
        let prep = crate::prepare_as(true, false, &mut sc, &mut vec![]);
        let of = |id: u32| { let r = rigid(&sc, id); prep.emitter_tris.iter().filter(|e| Some(e.inst) == r.still).map(|e| e.moves).collect::<Vec<bool>>() };
        assert_eq!((of(1), of(2)), (vec![false; 2], vec![false; 2]));
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

//! The voxel engine's objects in the renderer (ROADMAP § 6 steps 2c-4 and 6; docs/VOXEL.md): the level's voxels are a
//! `voxel_core::Level` (objects in one world lattice, edits in fixed point, connectivity to the anchors, loose parts
//! falling), loaded from a level's procedural geometry files and drawn as world objects. Scene code only sends commands
//! (`__engine.voxelLevel`, `voxelExplode`, `voxelEdit`, `voxelFrame`); the voxels never pass through QuickJS.
//!
//! Two routes, one switch for the whole level:
//! - meshes: each object's exposed faces greedy-meshed (`voxel_core::greedy_mesh`), one `own_blas` mesh per palette
//!   material; an edit meshes the touched objects again (and the terrain tiles beside them) and replaces their
//!   geometry in place (live.rs `edit_geometry`);
//! - boxes: each object one procedural geometry with one box (`VOXEL_BRICKS`); an edit lays the touched objects out
//!   again and replaces their data in place.
//!
//! An edit's removed voxels are debris (kept for scene code: `voxelDebris`). What it leaves hanging on no anchor is a
//! new object that falls (`voxelFrame` ticks the level 60 times a second: its mesh is written again where it is, in
//! place) until it rests on something; there it is static again.

use crate::world::{Geometry, Kind, World};
use anyhow::{anyhow, Result};
use glam::{Mat4, Quat, Vec3};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::rc::Rc;
use web_time::Instant;
use voxel_core::{decode, greedy_mesh, gravity_for, layout, to_fp, Edit, Granularity, Level, MeshPart, Placement, Shape, VoxelGrid, FALL_FP, RADII};

/// A loose part of fewer voxels is debris, not an object (`voxel_core::MIN_PIECE`).
pub const MIN_PIECE: usize = voxel_core::MIN_PIECE;
/// Ids the native objects, geometries and materials take: above any the scene code's binding hands out.
const FIRST_ID: u32 = 1 << 30;
/// Voxels per terrain tile side (games/voxel-level/plan.ts `TILE`).
const TILE: i32 = 64;
/// The level's ticks a second (falling objects), and the most it makes up in one frame.
const TICK_RATE: f64 = 60.0;
const MAX_TICKS: u32 = 8;

/// What scene code sends (`voxelLevel`): the route, the palette (index = voxel material), the procedural geometry
/// function's name (boxes; the scene registers it), and the objects: their file, placement and, for terrain tiles, the
/// tile (neighbouring tiles hide each other's side faces).
#[derive(Deserialize, Debug, Clone)]
pub struct LevelDesc {
    pub route: String,
    pub palette: Vec<PaletteEntry>,
    #[serde(default)]
    pub function: String,
    pub objects: Vec<ObjectDesc>,
    /// Keep the removed voxels that are in no new object (debris) for scene code to take (`voxelDebris`).
    #[serde(default)]
    pub debris: bool,
    /// "vertex": the mesh route makes one mesh per class of palette entries (alike in all but their colour), each vertex
    /// its voxel's colour (F_VCOLOR; `voxel_core::merge_by_class`), instead of one mesh per palette entry (the default).
    #[serde(default)]
    pub colors: String,
    /// The cell size (m) of the occupancy grid stateful GPU particles collide with (`collide.voxels`; 0: none, the
    /// default). Kept up to date as edits carve and parts land.
    #[serde(default)]
    pub occupancy: f64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PaletteEntry {
    pub name: String,
    /// Linear RGB.
    pub color: [f32; 3],
    pub roughness: f32,
    pub metalness: f32,
    /// Emission, nits (its colour the entry's): voxels that glow, light-sampled as emitter meshes (ROADMAP § 6 step 4).
    #[serde(default)]
    pub emissive: f32,
    /// How far an explosion reaches it: "soft" (the default: the whole radius), "medium" (0.7 of it), "hard" (0.4),
    /// "indestructible" (never: cuts leave it too).
    #[serde(default)]
    pub hardness: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ObjectDesc {
    pub name: String,
    pub uri: String,
    /// Metres; taken to the nearest voxel (every object stands in one lattice of the voxel size).
    pub position: [f32; 3],
    /// Quarter turns about y. (Numbers come from scene code as floats.)
    #[serde(default)]
    pub turn: f64,
    #[serde(default)]
    pub tile: Option<[f64; 2]>,
    /// It holds what touches it and never falls (terrain tiles are anchors without it).
    #[serde(default)]
    pub anchor: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Mesh,
    Boxes,
}

/// How a level object (`voxel_core::Level::objects`, same index) is drawn: its placement as a transform (world =
/// `world` x (`corner` + voxel x size)), world bounds, and the world objects that draw it.
struct VoxelObject {
    name: String,
    world: Mat4,
    /// Object-space position of the grid's voxel (0, 0, 0) corner, and that corner in voxels.
    corner: Vec3,
    off: [i32; 3],
    tile: Option<(i32, i32)>,
    lo: Vec3,
    hi: Vec3,
    /// The file whose geometry it still shares (None once edited: its own).
    shared: Option<String>,
    /// Mesh route: (material, world object, geometry) per material. Boxes: one entry, material 0.
    parts: Vec<(u8, u32, u32)>,
    /// A falling object's mesh with its voxel (0, 0, 0) at the origin (mesh route): written again where the object is,
    /// every tick it moves.
    flying: Option<Vec<MeshPart>>,
}

/// What scene code asked of the level before it was loaded (in setup: edits and the ticks after them, so the live
/// view and the offline render start from the same world), done once it is (`realize`).
enum Queued {
    Edit(EditOp),
    Frame(f64),
}

/// An edit as scene code sends it (`voxelEdit`): the shape in world metres, and what to do with the voxels inside.
#[derive(Clone, Debug)]
pub struct EditOp {
    pub shape: ShapeM,
    pub kind: EditKind,
}

#[derive(Clone, Copy, Debug)]
pub enum ShapeM {
    Sphere([f32; 3], f32),
    Box([f32; 3], [f32; 3]),
    Capsule([f32; 3], [f32; 3], f32),
}

#[derive(Clone, Copy, Debug)]
pub enum EditKind {
    /// Remove them, each hardness class within its share of the radius (1 / 256ths; `RADII`: an explosion).
    Carve([i32; 4]),
    Paint(u8),
    Add(u8),
}

impl EditOp {
    /// `{"op": "carve" | "explode" | "paint" | "add", "sphere": [x, y, z, r] | "box": [x0, y0, z0, x1, y1, z1] |
    /// "capsule": [x0, y0, z0, x1, y1, z1, r], "material": n}`: "explode" reaches each material by its hardness,
    /// "carve" cuts everything but the indestructible.
    pub fn parse(j: &serde_json::Value) -> Result<EditOp> {
        let nums = |k: &str, n: usize| -> Option<Vec<f32>> {
            let a = j[k].as_array()?;
            let v: Vec<f32> = a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
            (v.len() == n && a.len() == n && v.iter().all(|x| x.is_finite())).then_some(v)
        };
        let shape = if let Some(v) = nums("sphere", 4) {
            ShapeM::Sphere([v[0], v[1], v[2]], v[3])
        } else if let Some(v) = nums("box", 6) {
            ShapeM::Box([v[0], v[1], v[2]], [v[3], v[4], v[5]])
        } else if let Some(v) = nums("capsule", 7) {
            ShapeM::Capsule([v[0], v[1], v[2]], [v[3], v[4], v[5]], v[6])
        } else {
            return Err(anyhow!("voxelEdit: a shape of finite numbers is needed: sphere [x, y, z, r], box [x0, y0, z0, x1, y1, z1] or capsule [x0, y0, z0, x1, y1, z1, r]"));
        };
        let material = || j["material"].as_u64().filter(|m| (1..256).contains(m)).map(|m| m as u8).ok_or_else(|| anyhow!("voxelEdit: material must be a palette index from 1 to 255"));
        let kind = match j["op"].as_str() {
            Some("explode") => EditKind::Carve(RADII),
            Some("carve") => EditKind::Carve([256, 256, 256, 0]),
            Some("paint") => EditKind::Paint(material()?),
            Some("add") => EditKind::Add(material()?),
            other => return Err(anyhow!("voxelEdit: op {other:?} is none of explode, carve, paint, add")),
        };
        Ok(EditOp { shape, kind })
    }
}

/// The level's voxel objects (one per game session; scene code fills `pending`, the session realizes it).
#[derive(Default)]
pub struct VoxelWorld {
    /// Edits and landings since the frame log last took them (frame log `voxel`).
    pub ticks: (u32, u32),
    pending: Option<LevelDesc>,
    route: Option<Route>,
    size: f32,
    palette: Vec<u32>,
    function: String,
    /// The voxels (None until a level is loaded) and how each of its objects is drawn (same index).
    level: Option<Level>,
    objects: Vec<Option<VoxelObject>>,
    tiles: HashMap<(i32, i32), usize>,
    next_id: u32,
    /// Debris kept for scene code (`LevelDesc::debris`): world-space centre and palette index of each removed voxel
    /// that is in no new object, since `take_debris` last took them (at most `MAX_DEBRIS`; of an edit that removes
    /// more than `EDIT_DEBRIS`, every n-th).
    keep_debris: bool,
    debris: Vec<f32>,
    queued: Vec<Queued>,
    /// Time not yet ticked (s).
    behind: f64,
    /// Vertex colours (`LevelDesc::colors`): each palette entry's class and linear colour; `palette` then holds the
    /// classes' materials.
    vertex_colors: bool,
    class_of: Vec<u8>,
    entry_color: Vec<[f32; 3]>,
    /// The occupancy grid (`LevelDesc::occupancy`), shared with the world (`World::voxel_occupancy`).
    occupancy: Option<std::sync::Arc<std::sync::RwLock<crate::world::Occupancy>>>,
}

/// Debris voxels kept between two `take_debris` calls, and of one edit.
const MAX_DEBRIS: usize = 1 << 16;
const EDIT_DEBRIS: usize = 4096;
/// The occupancy grid's most words (64 MB): a bigger level's grid takes coarser cells.
const MAX_OCCUPANCY_WORDS: usize = 1 << 24;

impl VoxelWorld {
    /// Scene code's `voxelLevel`: kept until the session knows the asset root (`realize`).
    pub fn request(&mut self, d: LevelDesc) -> Result<()> {
        if d.route != "mesh" && d.route != "boxes" {
            return Err(anyhow!("voxelLevel: route \"{}\" is neither \"mesh\" nor \"boxes\"", d.route));
        }
        if d.palette.len() > 256 {
            return Err(anyhow!("voxelLevel: more than 256 palette entries"));
        }
        self.pending = Some(d);
        Ok(())
    }

    fn id(&mut self) -> u32 {
        if self.next_id == 0 {
            self.next_id = FIRST_ID;
        }
        self.next_id += 1;
        self.next_id
    }

    fn grid(&self, k: usize) -> &Rc<VoxelGrid> {
        &self.level.as_ref().unwrap().objects[k].as_ref().unwrap().grid
    }

    /// Loads the requested level into the world: decodes the files, places the objects (meshed or as boxes). Returns
    /// what it did (ms, objects, triangles) for the log.
    pub fn realize(&mut self, w: &mut World, assets: &Path) -> Result<Option<serde_json::Value>> {
        let Some(d) = self.pending.take() else { return Ok(None) };
        let t0 = Instant::now();
        let route = if d.route == "mesh" { Route::Mesh } else { Route::Boxes };
        self.route = Some(route);
        self.keep_debris = d.debris;
        self.function = d.function.clone();
        self.palette = vec![0; d.palette.len()];
        self.vertex_colors = d.colors == "vertex" && route == Route::Mesh;
        if self.vertex_colors {
            // One white material per class (roughness, metalness and emission alike: an emitting entry's colour is its
            // emission's, so it keeps a class of its own), the colours in the meshes' vertices.
            let mut classes: Vec<(u32, u32, u32, [u32; 3])> = vec![];
            self.palette.clear();
            for p in &d.palette {
                let glow = if p.emissive > 0.0 && p.emissive.is_finite() { p.emissive } else { 0.0 };
                let key = (p.roughness.to_bits(), p.metalness.to_bits(), glow.to_bits(), if glow > 0.0 { p.color.map(f32::to_bits) } else { [0; 3] });
                let c = match classes.iter().position(|k| *k == key) {
                    Some(c) => c,
                    None => {
                        let id = self.id();
                        let m: crate::desc::Material = serde_json::from_value(serde_json::json!({
                            "name": format!("class{} {}", classes.len(), p.name), "color": [1.0, 1.0, 1.0], "opacity": 1.0, "metalness": p.metalness, "roughness": p.roughness,
                            "emissive": if glow > 0.0 { p.color } else { [0.0; 3] }, "emissiveIntensity": glow, "doubleSided": false, "transparent": false, "vertexColors": true,
                        }))?;
                        w.set_material(id, m);
                        self.palette.push(id);
                        classes.push(key);
                        classes.len() - 1
                    }
                };
                self.class_of.push(c as u8);
                self.entry_color.push(p.color);
            }
        }
        // Every entry, 0 too (no voxel uses it, but a procedural mesh's material list is indexed by entry).
        let per_entry = !self.vertex_colors;
        for (k, p) in d.palette.iter().enumerate().filter(|_| per_entry) {
            let id = self.id();
            let glow = if p.emissive > 0.0 && p.emissive.is_finite() { p.emissive } else { 0.0 };
            let m: crate::desc::Material = serde_json::from_value(serde_json::json!({
                "name": p.name, "color": p.color, "opacity": 1.0, "metalness": p.metalness, "roughness": p.roughness,
                "emissive": if glow > 0.0 { p.color } else { [0.0; 3] }, "emissiveIntensity": glow, "doubleSided": false, "transparent": false,
            }))?;
            w.set_material(id, m);
            self.palette[k] = id;
        }
        // Decode each file once.
        let mut files: HashMap<String, (Rc<VoxelGrid>, Vec3, f32)> = HashMap::new();
        for o in &d.objects {
            if files.contains_key(&o.uri) {
                continue;
            }
            crate::world::check_uri(&o.uri)?;
            let blob = crate::procasset::load(&o.uri, assets)?;
            let (g, corner, size) = decode(&blob.data).map_err(|e| anyhow!("{}: {e}", o.uri))?;
            files.insert(o.uri.clone(), (Rc::new(g), Vec3::from(corner), size));
        }
        let t_decode = t0.elapsed().as_secs_f64() * 1e3;
        self.size = d.objects.first().map_or(0.1, |o| files[&o.uri].2);
        let size = self.size;
        let mut level = Level::new(gravity_for(size as f64, TICK_RATE));
        for (k, p) in d.palette.iter().enumerate() {
            level.hardness[k] = match p.hardness.as_str() {
                "" | "soft" => voxel_core::SOFT,
                "medium" => voxel_core::MEDIUM,
                "hard" => voxel_core::HARD,
                "indestructible" => voxel_core::INDESTRUCTIBLE,
                other => return Err(anyhow!("voxelLevel: palette entry {}: hardness \"{other}\" is none of soft, medium, hard, indestructible", p.name)),
            };
        }
        // Every object in the level's lattice: its position to the nearest voxel, its turn in quarters.
        let mut snapped = 0usize;
        for o in &d.objects {
            let (g, corner, osize) = files[&o.uri].clone();
            if osize != size {
                return Err(anyhow!("voxelLevel: {} has voxels of {osize} m, the level's are {size} m", o.uri));
            }
            if o.position.iter().any(|v| !v.is_finite() || (v / size).abs() > 1e6) || !o.turn.is_finite() {
                return Err(anyhow!("voxelLevel: {}: its position and turn must be finite (within a million voxels)", o.name));
            }
            let pos = o.position.map(|v| (v as f64 / size as f64).round() as i32);
            snapped += (0..3).any(|a| (o.position[a] as f64 / size as f64 - pos[a] as f64).abs() > 1e-3) as usize;
            let turn = o.turn.round() as i32;
            let position = Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32) * size;
            let world = Mat4::from_rotation_translation(Quat::from_rotation_y(turn.rem_euclid(4) as f32 * std::f32::consts::FRAC_PI_2), position);
            let off = corner.to_array().map(|c| (c / size).round() as i32);
            let tile = o.tile.map(|t| (t[0].round() as i32, t[1].round() as i32));
            let k = level.add_object(g, Placement::new(pos, turn, off), tile.is_some() || o.anchor) as usize;
            // (Each tile meshes against its own neighbours: no shared geometry.)
            let mut v = VoxelObject { name: o.name.clone(), world, corner, off, tile, lo: Vec3::ZERO, hi: Vec3::ZERO, shared: tile.is_none().then(|| o.uri.clone()), parts: vec![], flying: None };
            v.bounds(&level.objects[k].as_ref().unwrap().grid, size);
            if let Some(t) = tile {
                self.tiles.insert(t, k);
            }
            self.objects.push(Some(v));
        }
        let t_lattice = Instant::now();
        // What hangs on no anchor as the level was built (objects placed in the air) stays until an edit touches it.
        let floating = level.floating();
        // THREERT_VOXEL_FLOATING=1: which objects those parts belong to (by name up to its number), for the level's author.
        if floating.0 > 0 && std::env::var("THREERT_VOXEL_FLOATING").is_ok() {
            let mut by: std::collections::BTreeMap<String, (usize, usize, usize)> = Default::default();
            for g in level.floating_groups() {
                let Some(k) = level.object_at(g[0]) else { continue };
                let name = &self.objects[k as usize].as_ref().unwrap().name;
                let kind = name.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches('-').to_string();
                let e = by.entry(kind).or_default();
                *e = (e.0 + 1, e.1 + g.len(), e.2.max(g.len()));
            }
            for (kind, (groups, voxels, largest)) in by {
                eprintln!("voxels: floating {kind}: {groups} groups, {voxels} voxels, largest {largest}");
            }
        }
        let lattice_ms = t_lattice.elapsed().as_secs_f64() * 1e3;
        let (lattice_bricks, lattice_mb) = (level.lattice_bricks(), level.connectivity_bytes() as f64 / (1 << 20) as f64);
        self.level = Some(level);
        // Place them: geometry per file (shared) or per object (tiles), then the objects.
        let t_place = Instant::now();
        let mut by_file: HashMap<String, Vec<(u8, u32)>> = HashMap::new();
        let mut triangles = 0usize;
        for k in 0..self.objects.len() {
            let shared = self.objects[k].as_ref().unwrap().shared.clone();
            let geometry: Vec<(u8, u32)> = match shared.as_ref().and_then(|f| by_file.get(f).cloned()) {
                Some(g) => g,
                None => {
                    let g = self.make_geometry(w, k, None)?;
                    if let Some(f) = shared {
                        by_file.insert(f, g.clone());
                    }
                    g
                }
            };
            triangles += self.place(w, k, &geometry)?;
        }
        let place_ms = t_place.elapsed().as_secs_f64() * 1e3;
        let t_occ = Instant::now();
        if d.occupancy > 0.0 && d.occupancy.is_finite() {
            let occ = self.build_occupancy(d.occupancy as f32);
            self.occupancy = Some(std::sync::Arc::new(std::sync::RwLock::new(occ)));
            w.voxel_occupancy = self.occupancy.clone();
        }
        let occupancy_ms = t_occ.elapsed().as_secs_f64() * 1e3;
        let queued = std::mem::take(&mut self.queued);
        let (edits, frames) = (queued.iter().filter(|q| matches!(q, Queued::Edit(..))).count(), queued.iter().filter(|q| matches!(q, Queued::Frame(_))).count());
        for q in queued {
            match q {
                Queued::Edit(op) => {
                    self.edit(w, &op)?;
                }
                Queued::Frame(dt) => {
                    self.frame(w, dt)?;
                }
            }
        }
        Ok(Some(serde_json::json!({
            "route": d.route, "objects": self.objects.len(), "files": files.len(), "decode_ms": t_decode,
            "place_ms": place_ms, "triangles": triangles, "explosions": edits, "frames": frames,
            "lattice_ms": lattice_ms, "lattice_bricks": lattice_bricks, "lattice_mb": lattice_mb, "floating_groups": floating.0, "floating_voxels": floating.1, "snapped": snapped,
            "occupancy_ms": occupancy_ms, "occupancy_bytes": self.occupancy.as_ref().map_or(0, |o| o.read().unwrap().words.len() * 4),
        })))
    }

    /// Object `k`'s geometry where it is: per material (mesh) or one procedural geometry (boxes). Returns (material,
    /// geometry id) per part.
    fn make_geometry(&mut self, w: &mut World, k: usize, mut counts: Option<&mut (usize, usize)>) -> Result<Vec<(u8, u32)>> {
        let size = self.size;
        let grid = self.grid(k).clone();
        let corner = self.objects[k].as_ref().unwrap().corner;
        match self.route {
            Some(Route::Boxes) => {
                let l = layout(&grid, Granularity::Object, corner.to_array(), size);
                if l.boxes.is_empty() {
                    return Ok(vec![]);
                }
                let id = self.id();
                w.set_proc_geometry(id, self.function.clone(), l.boxes, l.data)?;
                Ok(vec![(0, id)])
            }
            _ => {
                let mesh = {
                    let (objects, level, tiles) = (&self.objects, self.level.as_ref().unwrap(), &self.tiles);
                    let o = objects[k].as_ref().unwrap();
                    let mut outside = |x: i32, y: i32, z: i32| -> bool { tile_solid(objects, level, tiles, o, x, y, z) };
                    greedy_mesh(&grid, corner.to_array(), size, &mut outside)
                };
                if let Some(c) = counts.as_deref_mut() {
                    c.0 += mesh.faces;
                    c.1 += mesh.quads * 2;
                }
                let parts = if self.vertex_colors { voxel_core::merge_by_class(mesh.parts, &self.class_of, &self.entry_color) } else { mesh.parts };
                let mut out = vec![];
                for p in parts {
                    let (id, m) = (self.id(), p.material);
                    w.set_geometry(id, part_geometry(p, Vec3::ZERO))?;
                    out.push((m, id));
                }
                Ok(out)
            }
        }
    }

    /// Creates object `k`'s world objects for `geometry` (one per part), returns its triangles.
    fn place(&mut self, w: &mut World, k: usize, geometry: &[(u8, u32)]) -> Result<usize> {
        let (s, r, t) = self.objects[k].as_ref().unwrap().world.to_scale_rotation_translation();
        let name = self.objects[k].as_ref().unwrap().name.clone();
        let mut parts = vec![];
        let mut tris = 0;
        for &(m, g) in geometry {
            let id = self.id();
            let kind = match self.route {
                Some(Route::Boxes) => Kind::Procedural { geometry: g, materials: self.palette.clone() },
                _ => {
                    tris += w.geometries.get(&g).map_or(0, |g| g.indices.len() / 3);
                    Kind::Mesh { geometry: g, material: self.palette.get(m as usize).copied().unwrap_or(0), own_blas: true, materials: vec![] }
                }
            };
            w.set_object(id, format!("{name}#{m}"), None, kind, true, false)?;
            w.set_transform(id, t, r, s)?;
            parts.push((m, id, g));
        }
        self.objects[k].as_mut().unwrap().parts = parts;
        Ok(tris)
    }

    /// Scene code's `voxelExplode`: an explosion at a world position (m) of a radius, each material within its
    /// hardness's share of it.
    pub fn explode(&mut self, w: &mut World, center: Vec3, radius: f32) -> Result<serde_json::Value> {
        self.edit(w, &EditOp { shape: ShapeM::Sphere(center.to_array(), radius), kind: EditKind::Carve(RADII) })
    }

    /// An edit (`voxelEdit`): the level's voxels changed (`voxel_core::Level`), the touched objects drawn again in place,
    /// the loose parts made falling objects, the removed objects gone. Returns what it did and how long each part took.
    pub fn edit(&mut self, w: &mut World, op: &EditOp) -> Result<serde_json::Value> {
        if self.route.is_none() && self.pending.is_some() {
            self.queued.push(Queued::Edit(op.clone()));
            return Ok(serde_json::json!({ "queued": true }));
        }
        if self.route.is_none() {
            return Err(anyhow!("voxelEdit: no voxel level loaded"));
        }
        self.ticks.0 += 1;
        let size = self.size as f64;
        let fp = |p: [f32; 3]| p.map(|v| to_fp(v as f64 / size));
        let shape = match op.shape {
            ShapeM::Sphere(c, r) => Shape::Sphere { c: fp(c), r: to_fp(r as f64 / size) },
            ShapeM::Box(a, b) => Shape::Box { lo: fp(a), hi: fp(b) },
            ShapeM::Capsule(a, b, r) => Shape::Capsule { a: fp(a), b: fp(b), r: to_fp(r as f64 / size) },
        };
        let Some(shape) = shape.checked() else { return Err(anyhow!("voxelEdit: the shape is too large (at most {} voxels across a box or line's bounds in all)", voxel_core::MAX_CELLS)) };
        let t0 = Instant::now();
        let level = self.level.as_mut().unwrap();
        let e = match op.kind {
            EditKind::Carve(radii) => level.carve(shape, radii),
            EditKind::Paint(m) => level.paint(shape, m),
            EditKind::Add(m) => level.add(shape, m),
        };
        let carve_ms = t0.elapsed().as_secs_f64() * 1e3;
        let (lo, hi) = shape.cells();
        let s = self.size;
        let bounds = (Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) * s, Vec3::new(hi[0] as f32 + 1.0, hi[1] as f32 + 1.0, hi[2] as f32 + 1.0) * s);
        let mut out = self.apply(w, &e, vec![bounds])?;
        out["carve_ms"] = carve_ms.into();
        Ok(out)
    }

    /// What an edit of the level did, drawn: `boxes` are the world boxes whose occupancy (for particles) to set again.
    fn apply(&mut self, w: &mut World, e: &Edit, mut boxes: Vec<(Vec3, Vec3)>) -> Result<serde_json::Value> {
        let size = self.size;
        let route = self.route.unwrap();
        // Rebuild: new geometry for each touched object, swapped in place. A terrain tile beside a touched one shows
        // faces it hid where voxels went at their shared edge (mesh route).
        let t1 = Instant::now();
        let mut dirty: BTreeSet<usize> = e.changed.keys().map(|&k| k as usize).collect();
        if route == Route::Mesh {
            dirty.extend(self.edge_neighbours(e));
        }
        // The objects left with no voxel are gone. A terrain tile among them leaves the tiles' table, and the four
        // tiles beside it show the faces they hid against it.
        for &k in &e.removed {
            if let Some(o) = self.objects.get_mut(k as usize).and_then(|o| o.take()) {
                for p in o.parts {
                    w.remove(p.1);
                }
                if let Some((tx, tz)) = o.tile {
                    self.tiles.remove(&(tx, tz));
                    if route == Route::Mesh {
                        dirty.extend([(-1, 0), (1, 0), (0, -1), (0, 1)].iter().filter_map(|d| self.tiles.get(&(tx + d.0, tz + d.1)).copied()));
                    }
                }
            }
        }
        // (Only objects that still exist: a neighbour may be one of the removed.)
        let level = self.level.as_ref().unwrap();
        dirty.retain(|&k| self.objects.get(k).is_some_and(|o| o.is_some()) && level.objects.get(k).is_some_and(|o| o.is_some()));
        let mut counts = (0usize, 0usize);
        for &k in &dirty {
            let geometry = self.make_geometry(w, k, Some(&mut counts))?;
            self.replace(w, k, &geometry)?;
        }
        let rebuild_ms = t1.elapsed().as_secs_f64() * 1e3;
        // The new objects: loose parts (falling: their mesh kept to be written again as they move) and added voxels.
        let t2 = Instant::now();
        let (mut piece_mesh_ms, mut piece_world_ms) = (0.0f64, 0.0f64);
        for &id in &e.created {
            let k = id as usize;
            let (t, falling, lo, hi) = {
                let o = self.level.as_ref().unwrap().objects[k].as_ref().unwrap();
                (o.place.t, o.fall.is_some(), o.lo, o.hi)
            };
            let corner = Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32) * size;
            if self.objects.len() <= k {
                self.objects.resize_with(k + 1, || None);
            }
            let mut v = VoxelObject { name: format!("part{k}"), world: Mat4::IDENTITY, corner, off: t, tile: None, lo: Vec3::ZERO, hi: Vec3::ZERO, shared: None, parts: vec![], flying: None };
            (v.lo, v.hi) = (Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) * size, Vec3::new(hi[0] as f32 + 1.0, hi[1] as f32 + 1.0, hi[2] as f32 + 1.0) * size);
            // (Where it was is no longer occupied; where it lands will be.)
            boxes.push((v.lo, v.hi));
            self.objects[k] = Some(v);
            let tm = Instant::now();
            let geometry = if falling && route == Route::Mesh {
                let mut mesh = greedy_mesh(self.grid(k), [0.0; 3], size, &mut |_, _, _| false);
                counts.0 += mesh.faces;
                counts.1 += mesh.quads * 2;
                if self.vertex_colors {
                    mesh.parts = voxel_core::merge_by_class(mesh.parts, &self.class_of, &self.entry_color);
                }
                let mut out = vec![];
                for p in &mesh.parts {
                    let gid = self.id();
                    w.set_geometry(gid, part_geometry(p.clone(), corner))?;
                    out.push((p.material, gid));
                }
                self.objects[k].as_mut().unwrap().flying = Some(mesh.parts);
                out
            } else {
                self.make_geometry(w, k, Some(&mut counts))?
            };
            piece_mesh_ms += tm.elapsed().as_secs_f64() * 1e3;
            let tw = Instant::now();
            self.place(w, k, &geometry)?;
            piece_world_ms += tw.elapsed().as_secs_f64() * 1e3;
        }
        let pieces_ms = t2.elapsed().as_secs_f64() * 1e3;
        if self.keep_debris && !e.debris.is_empty() {
            // Every n-th of a large edit's, so that one edit's debris stays a few thousand voxels.
            let step = e.debris.len().div_ceil(EDIT_DEBRIS);
            for (c, m) in e.debris.iter().step_by(step).take(MAX_DEBRIS.saturating_sub(self.debris.len() / 4)) {
                self.debris.extend([(c[0] as f32 + 0.5) * size, (c[1] as f32 + 0.5) * size, (c[2] as f32 + 0.5) * size, *m as f32]);
            }
        }
        let t3 = Instant::now();
        self.refresh_occupancy(&boxes);
        let occupancy_ms = t3.elapsed().as_secs_f64() * 1e3;
        Ok(serde_json::json!({
            "rebuild_ms": rebuild_ms, "pieces_ms": pieces_ms, "voxels": e.voxels, "touched": dirty.len(), "pieces": e.pieces, "piece_voxels": e.piece_voxels,
            "dropped": e.debris.len(), "removed": e.removed.len(), "piece_mesh_ms": piece_mesh_ms, "piece_world_ms": piece_world_ms,
            "batches": self.level.as_ref().unwrap().falling(), "faces": counts.0, "triangles": counts.1, "objects": self.objects.iter().flatten().count(),
            "occupancy_ms": occupancy_ms, "world_objects": self.world_objects(),
        }))
    }

    /// Scene code's `voxelDebris`: the debris kept since the last call (x, y, z, palette index per voxel).
    pub fn take_debris(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.debris)
    }

    /// Scene code's `voxelFrame`, once a frame: the level ticked for the time that passed (60 ticks a second, at most
    /// `MAX_TICKS` a call), the falling objects written again where they are now, the landed ones where they rest.
    pub fn frame(&mut self, w: &mut World, dt: f64) -> Result<serde_json::Value> {
        if self.route.is_none() && self.pending.is_some() {
            self.queued.push(Queued::Frame(dt));
            return Ok(serde_json::json!({ "queued": true }));
        }
        let Some(level) = self.level.as_mut() else { return Ok(serde_json::json!({ "batches": 0 })) };
        if level.falling() == 0 {
            self.behind = 0.0;
            return Ok(serde_json::json!({ "batches": 0 }));
        }
        let t0 = Instant::now();
        self.behind += dt.clamp(0.0, 0.25);
        let ticks = ((self.behind * TICK_RATE).floor() as u32).min(MAX_TICKS);
        self.behind = (self.behind - ticks as f64 / TICK_RATE).min(1.0 / TICK_RATE);
        let (mut landed, mut removed): (Vec<u32>, Vec<u32>) = (vec![], vec![]);
        for _ in 0..ticks {
            let t = level.tick();
            landed.extend(t.landed);
            removed.extend(t.removed);
        }
        let size = self.size;
        let falling: Vec<(usize, Vec3)> = level
            .objects
            .iter()
            .enumerate()
            .filter_map(|(k, o)| {
                let o = o.as_ref()?;
                let f = o.fall.as_ref()?;
                Some((k, Vec3::new(o.place.t[0] as f32, o.place.t[1] as f32 - (f.drop as f64 / FALL_FP as f64) as f32, o.place.t[2] as f32) * size))
            })
            .collect();
        let mut tris = 0usize;
        if ticks > 0 {
            for &(k, corner) in &falling {
                tris += self.rewrite(w, k, corner)?;
            }
        }
        let rebuild_ms = t0.elapsed().as_secs_f64() * 1e3;
        let t1 = Instant::now();
        let mut boxes = vec![];
        for &id in &landed {
            let k = id as usize;
            let (t, lo, hi) = {
                let o = self.level.as_ref().unwrap().objects[k].as_ref().unwrap();
                (o.place.t, o.lo, o.hi)
            };
            let corner = Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32) * size;
            self.rewrite(w, k, corner)?;
            let o = self.objects[k].as_mut().unwrap();
            o.flying = None;
            o.off = t;
            (o.lo, o.hi) = (Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) * size, Vec3::new(hi[0] as f32 + 1.0, hi[1] as f32 + 1.0, hi[2] as f32 + 1.0) * size);
            boxes.push((o.lo, o.hi));
        }
        for &id in &removed {
            if let Some(o) = self.objects.get_mut(id as usize).and_then(|o| o.take()) {
                for p in o.parts {
                    w.remove(p.1);
                }
            }
        }
        self.refresh_occupancy(&boxes);
        let settle_ms = t1.elapsed().as_secs_f64() * 1e3;
        self.ticks.1 += landed.len() as u32;
        Ok(serde_json::json!({ "batches": falling.len(), "ticks": ticks, "rebuilt_tris": tris, "rebuild_ms": rebuild_ms, "settled": landed.len(), "settle_ms": settle_ms, "world_objects": self.world_objects() }))
    }

    /// A falling (or just landed) object drawn with its voxel (0, 0, 0)'s corner at `corner`: its geometries written
    /// again there (the same triangles: in place). Returns its triangles.
    fn rewrite(&mut self, w: &mut World, k: usize, corner: Vec3) -> Result<usize> {
        let size = self.size;
        let grid = self.grid(k).clone();
        let Some(o) = self.objects[k].as_mut() else { return Ok(0) };
        o.corner = corner;
        let mut tris = 0;
        match &o.flying {
            Some(parts) => {
                for (p, &(_, _, gid)) in parts.iter().zip(&o.parts) {
                    tris += p.indices.len() / 3;
                    w.set_geometry(gid, part_geometry(p.clone(), corner))?;
                }
            }
            None => {
                // Boxes: the object's layout again at its corner (the same data: in place).
                if let Some(&(_, _, gid)) = o.parts.first() {
                    let l = layout(&grid, Granularity::Object, corner.to_array(), size);
                    w.set_proc_geometry(gid, self.function.clone(), l.boxes, l.data)?;
                }
            }
        }
        Ok(tris)
    }

    /// Object `k` drawn by `geometry` now: parts kept get their new geometry (replaced in place), new materials new
    /// objects, vanished ones removed.
    fn replace(&mut self, w: &mut World, k: usize, geometry: &[(u8, u32)]) -> Result<()> {
        let Some(o) = self.objects[k].as_mut() else { return Ok(()) };
        let old = std::mem::take(&mut o.parts);
        // (A static object's mesh is in its own space from now on.)
        o.flying = None;
        let (s, r, t) = o.world.to_scale_rotation_translation();
        let name = o.name.clone();
        let mut parts = vec![];
        for &(m, g) in geometry {
            let kind = match self.route {
                Some(Route::Boxes) => Kind::Procedural { geometry: g, materials: self.palette.clone() },
                _ => Kind::Mesh { geometry: g, material: self.palette.get(m as usize).copied().unwrap_or(0), own_blas: true, materials: vec![] },
            };
            let id = match old.iter().find(|p| p.0 == m) {
                Some(p) => p.1,
                None => {
                    let id = self.id();
                    w.set_object(id, format!("{name}#{m}"), None, kind.clone(), true, false)?;
                    w.set_transform(id, t, r, s)?;
                    parts.push((m, id, g));
                    continue;
                }
            };
            // Same object, its own new geometry: re-created with it (in place).
            w.set_object(id, format!("{name}#{m}"), None, kind, true, false)?;
            parts.push((m, id, g));
        }
        for p in old {
            if !geometry.iter().any(|g| g.0 == p.0) {
                w.remove(p.1);
            }
        }
        // Its old geometries (unless another object still shares them) are no longer drawn.
        let o = self.objects[k].as_mut().unwrap();
        o.shared = None;
        o.parts = parts;
        Ok(())
    }

    /// The terrain tiles beside the tiles an edit removed voxels from at their shared edge: their faces against those
    /// voxels show now (meshes only).
    fn edge_neighbours(&self, e: &Edit) -> Vec<usize> {
        let Some(level) = self.level.as_ref() else { return vec![] };
        let mut out = BTreeSet::new();
        for &id in e.changed.keys() {
            let Some(o) = self.objects[id as usize].as_ref() else { continue };
            let (Some((tx, tz)), Some(lo)) = (o.tile, level.objects[id as usize].as_ref()) else { continue };
            // The tile's corner in world cells (tiles are not turned).
            let t = lo.place.t;
            let (x0, z0) = (t[0] - o.off[0], t[2] - o.off[2]);
            // (By the bounds of what it lost: touching the tile's edge.)
            let Some(&(blo, bhi)) = e.bounds.get(&id) else { continue };
            let sides = [blo[0] - x0 <= 0, bhi[0] - x0 >= TILE - 1, blo[2] - z0 <= 0, bhi[2] - z0 >= TILE - 1];
            out.extend([(-1, 0), (1, 0), (0, -1), (0, 1)].iter().zip(sides).filter(|(_, s)| *s).filter_map(|(d, _)| self.tiles.get(&(tx + d.0, tz + d.1)).copied()));
        }
        out.into_iter().collect()
    }

    /// The occupancy grid at `cell` m (coarser if the level's would pass `MAX_OCCUPANCY_WORDS`): over the objects'
    /// bounds and a cell around them, every cell holding a voxel set.
    fn build_occupancy(&self, cell: f32) -> crate::world::Occupancy {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for (o, _) in self.statics().filter(|(_, g)| g.bounds().is_some()) {
            lo = lo.min(o.lo);
            hi = hi.max(o.hi);
        }
        if lo.x > hi.x {
            return crate::world::Occupancy::default();
        }
        let mut cell = cell;
        let (origin, dims) = loop {
            let origin = lo - Vec3::splat(cell);
            let n = ((hi + Vec3::splat(cell) - origin) / cell).ceil();
            let dims = [n.x as u32, n.y as u32, n.z as u32];
            let words = (dims[0] as usize).div_ceil(32) * dims[1] as usize * dims[2] as usize;
            if words <= MAX_OCCUPANCY_WORDS {
                break (origin, dims);
            }
            cell *= 2.0;
        };
        let mut occ = crate::world::Occupancy { origin: origin.to_array(), cell, dims, words: vec![], dirty: vec![] };
        occ.words = vec![0; occ.row_words() * dims[1] as usize * dims[2] as usize];
        for (o, grid) in self.statics() {
            Self::mark(&mut occ, o, grid, self.size, None);
        }
        occ
    }

    /// Sets the cells of object `o`'s voxels (`grid`; those in the cell box `within`, inclusive, if given).
    fn mark(occ: &mut crate::world::Occupancy, o: &VoxelObject, grid: &VoxelGrid, size: f32, within: Option<([i32; 3], [i32; 3])>) {
        let org = Vec3::from(occ.origin);
        let (rw, n) = (occ.row_words(), occ.dims.map(|v| v as i32));
        // A voxel's centre in cells: base + its grid coordinates along the (quarter-turned) axes.
        let to_cells = |p: Vec3| (o.world.transform_point3(o.corner + p * size) - org) / occ.cell;
        let base = to_cells(Vec3::splat(0.5));
        let (ax, ay, az) = (to_cells(Vec3::new(1.5, 0.5, 0.5)) - base, to_cells(Vec3::new(0.5, 1.5, 0.5)) - base, to_cells(Vec3::new(0.5, 0.5, 1.5)) - base);
        for (bk, br) in &grid.bricks {
            let b0 = Vec3::new(bk.0 as f32, bk.1 as f32, bk.2 as f32) * 8.0;
            if let Some((wl, wh)) = within {
                // The brick's cell box (its corners' voxel centres), skipped if outside.
                let (mut l, mut h) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for c in 0..8 {
                    let v = b0 + Vec3::new((c & 1) as f32, ((c >> 1) & 1) as f32, (c >> 2) as f32) * 7.0;
                    let q = base + ax * v.x + ay * v.y + az * v.z;
                    l = l.min(q);
                    h = h.max(q);
                }
                if h.x < wl[0] as f32 || h.y < wl[1] as f32 || h.z < wl[2] as f32 || l.x >= (wh[0] + 1) as f32 || l.y >= (wh[1] + 1) as f32 || l.z >= (wh[2] + 1) as f32 {
                    continue;
                }
            }
            for (v, &m) in br.iter().enumerate() {
                if m == 0 {
                    continue;
                }
                let p = b0 + Vec3::new((v & 7) as f32, ((v >> 3) & 7) as f32, (v >> 6) as f32);
                let q = (base + ax * p.x + ay * p.y + az * p.z).floor();
                let c = [q.x as i32, q.y as i32, q.z as i32];
                if (0..3).any(|a| c[a] < 0 || c[a] >= n[a]) || within.is_some_and(|(wl, wh)| (0..3).any(|a| c[a] < wl[a] || c[a] > wh[a])) {
                    continue;
                }
                let w = (c[1] as usize * n[2] as usize + c[2] as usize) * rw + c[0] as usize / 32;
                occ.words[w] |= 1 << (c[0] as u32 % 32);
            }
        }
    }

    /// The static objects (a falling one is in no occupancy until it lands) with their voxels.
    fn statics(&self) -> impl Iterator<Item = (&VoxelObject, &VoxelGrid)> {
        let level = self.level.as_ref();
        self.objects.iter().enumerate().filter_map(move |(k, o)| {
            let lo = level?.objects.get(k)?.as_ref()?;
            lo.fall.is_none().then_some((o.as_ref()?, &*lo.grid))
        })
    }

    /// The occupancy grid's cells in world boxes `boxes` set again from the objects (after an edit or a landing), the
    /// words changed noted for upload.
    fn refresh_occupancy(&self, boxes: &[(Vec3, Vec3)]) {
        let Some(occ) = &self.occupancy else { return };
        let mut occ = occ.write().unwrap();
        if occ.words.is_empty() {
            return;
        }
        let (org, rw, n) = (Vec3::from(occ.origin), occ.row_words(), occ.dims.map(|v| v as i32));
        for &(lo, hi) in boxes {
            let l = ((lo - org) / occ.cell).floor();
            let h = ((hi - org) / occ.cell).floor();
            let wl = [l.x as i32, l.y as i32, l.z as i32].map(|v| v.max(0));
            let wh = [(h.x as i32).min(n[0] - 1), (h.y as i32).min(n[1] - 1), (h.z as i32).min(n[2] - 1)];
            if (0..3).any(|a| wl[a] > wh[a]) {
                continue;
            }
            for y in wl[1]..=wh[1] {
                for z in wl[2]..=wh[2] {
                    let row = (y as usize * n[2] as usize + z as usize) * rw;
                    for x in wl[0]..=wh[0] {
                        occ.words[row + x as usize / 32] &= !(1 << (x as u32 % 32));
                    }
                }
            }
            // Every object with voxels in those cells: the cells reach past the box (up to a cell), so a neighbour just
            // outside it may share them.
            let (cl, ch) = (org + Vec3::new(wl[0] as f32, wl[1] as f32, wl[2] as f32) * occ.cell, org + Vec3::new((wh[0] + 1) as f32, (wh[1] + 1) as f32, (wh[2] + 1) as f32) * occ.cell);
            for (o, grid) in self.statics().filter(|(o, _)| o.hi.cmpge(cl).all() && o.lo.cmple(ch).all()) {
                Self::mark(&mut occ, o, grid, self.size, Some((wl, wh)));
            }
            // Per layer, its rows from the box's first to its last (whole rows: few, contiguous ranges).
            for y in wl[1]..=wh[1] {
                let a = (y as usize * n[2] as usize + wl[2] as usize) * rw;
                let b = (y as usize * n[2] as usize + wh[2] as usize + 1) * rw;
                occ.dirty.push((a, b));
            }
        }
    }

    /// Native world objects (all parts of all voxel objects).
    pub fn world_objects(&self) -> usize {
        self.objects.iter().flatten().map(|o| o.parts.len()).sum()
    }

    /// The level's hash (`voxel_core::Level::hash`: every object's placement, state and voxels), None before a level.
    pub fn hash(&self) -> Option<u64> {
        self.level.as_ref().map(|l| l.hash())
    }
}

/// A mesh part as a geometry, its positions moved by `at`.
fn part_geometry(p: MeshPart, at: Vec3) -> Geometry {
    let n = p.positions.len() / 3;
    let positions = if at == Vec3::ZERO { p.positions } else { p.positions.chunks(3).flat_map(|v| [v[0] + at.x, v[1] + at.y, v[2] + at.z]).collect() };
    Geometry { positions, normals: p.normals, uvs: vec![0.0; n * 2], indices: p.indices, groups: vec![], colors: p.colors }
}

impl VoxelObject {
    /// World bounds: the box of its grid's bounds under its placement.
    fn bounds(&mut self, grid: &VoxelGrid, size: f32) {
        let Some((lo, hi)) = grid.bounds() else { return };
        let a = self.corner + Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) * size;
        let b = self.corner + Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32) * size;
        let (mut wlo, mut whi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for c in 0..8 {
            let p = Vec3::new(if c & 1 == 0 { a.x } else { b.x }, if c & 2 == 0 { a.y } else { b.y }, if c & 4 == 0 { a.z } else { b.z });
            let q = self.world.transform_point3(p);
            wlo = wlo.min(q);
            whi = whi.max(q);
        }
        (self.lo, self.hi) = (wlo, whi);
    }
}

/// Whether voxel (x, y, z) of object `o`'s grid, empty there, is filled by a neighbouring terrain tile or lies under the
/// bedrock: a terrain tile's faces against it never show (as the generator's meshes, tools/voxel-level/generate.ts
/// `tileSurroundings`, but against the tiles' voxels now, carved or not). Not a tile: never.
fn tile_solid(objects: &[Option<VoxelObject>], level: &Level, tiles: &HashMap<(i32, i32), usize>, o: &VoxelObject, x: i32, y: i32, z: i32) -> bool {
    let Some((tx, tz)) = o.tile else { return false };
    let q = [x + o.off[0], y + o.off[1], z + o.off[2]];
    if q[1] < 0 {
        return true;
    }
    if (0..TILE).contains(&q[0]) && (0..TILE).contains(&q[2]) {
        return false;
    }
    let (nx, nz) = (tx + q[0].div_euclid(TILE), tz + q[2].div_euclid(TILE));
    let Some(&n) = tiles.get(&(nx, nz)) else { return false };
    let (Some(t), Some(g)) = (objects[n].as_ref(), level.objects[n].as_ref()) else { return false };
    g.grid.get(q[0].rem_euclid(TILE) - t.off[0], q[1] - t.off[1], q[2].rem_euclid(TILE) - t.off[2]) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> std::path::PathBuf {
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
    }

    /// The level of tests/render/voxel-cut-falls.ts: a slab (anchor) and two posts, one on the other, with the particles'
    /// occupancy grid.
    fn level(w: &mut World) -> VoxelWorld {
        let mut v = VoxelWorld::default();
        let d: LevelDesc = serde_json::from_value(serde_json::json!({
            "route": "mesh", "debris": true, "occupancy": 0.2,
            "palette": [{ "name": "none", "color": [0, 0, 0], "roughness": 1, "metalness": 0 }, { "name": "black", "color": [0, 0, 0], "roughness": 1, "metalness": 0 },
                        { "name": "post", "color": [1, 1, 1], "roughness": 1, "metalness": 0, "hardness": "hard" }],
            "objects": [{ "name": "slab", "uri": "tests/render/data/voxel-slab.tpg", "position": [0, 0, 0], "anchor": true },
                        { "name": "post", "uri": "tests/render/data/voxel-post.tpg", "position": [0.0, 0.4, 0.0] },
                        { "name": "post-above", "uri": "tests/render/data/voxel-post.tpg", "position": [0.0, 2.0, 0.0], "turn": 1 }],
        }))
        .unwrap();
        v.request(d).unwrap();
        let r = v.realize(w, &assets()).unwrap().unwrap();
        assert_eq!((r["floating_groups"].as_u64(), r["snapped"].as_u64()), (Some(0), Some(0)), "{r}");
        v
    }

    fn op(j: serde_json::Value) -> EditOp {
        EditOp::parse(&j).unwrap()
    }

    /// A cut through the lower post: its upper part and the post on it become one falling object, drawn by its own world
    /// objects; ticked, it lands 0.2 m lower and is static. The debris is the cut's voxels. The same commands on a
    /// second level give the same hash; the particles' occupancy grid is what a fresh build gives.
    #[test]
    fn a_cut_drops_what_it_frees_and_edits_repeat() {
        let run = || {
            let mut w = World::default();
            let mut v = level(&mut w);
            let (h0, objects) = (v.hash().unwrap(), v.world_objects());
            assert_eq!(objects, 3);
            let e = v.edit(&mut w, &op(serde_json::json!({ "op": "carve", "box": [-0.5, 0.8, -0.5, 0.5, 1.0, 0.5] }))).unwrap();
            assert_eq!((e["voxels"].as_u64(), e["pieces"].as_u64(), e["piece_voxels"].as_u64(), e["removed"].as_u64()), (Some(32), Some(1), Some(16 * (10 + 16)), Some(1)), "{e}");
            assert_eq!(v.take_debris().len(), 32 * 4);
            assert_eq!(v.world_objects(), 3, "the stump, the slab, the loose part (the upper post is gone)");
            let part = v.objects.iter().position(|o| o.as_ref().is_some_and(|o| o.flying.is_some())).unwrap();
            let top = |v: &VoxelWorld| v.objects[part].as_ref().unwrap().corner.y;
            let y0 = top(&v);
            let mut landed = 0;
            for _ in 0..60 {
                let f = v.frame(&mut w, 1.0 / 60.0).unwrap();
                landed += f["settled"].as_u64().unwrap_or(0);
            }
            assert_eq!(landed, 1);
            assert!((y0 - top(&v) - 0.2).abs() < 1e-5, "fell {} m", y0 - top(&v));
            assert!(v.objects[part].as_ref().unwrap().flying.is_none());
            assert_eq!(v.frame(&mut w, 1.0 / 60.0).unwrap()["batches"], 0);
            // An explosion at the stump: the post is hard (0.4 of the radius), the slab soft (a crater 0.3 m deep under it).
            let e = v.explode(&mut w, Vec3::new(0.0, 0.6, 0.0), 0.5).unwrap();
            assert!(e["voxels"].as_u64().unwrap() > 100, "{e}");
            // (It took the stump's foot: what stood on it falls into the crater. Edits reach static voxels only: let it
            // land.)
            for _ in 0..60 {
                v.frame(&mut w, 1.0 / 60.0).unwrap();
            }
            assert_eq!(v.frame(&mut w, 1.0 / 60.0).unwrap()["batches"], 0);
            // Paint and added voxels.
            let e = v.edit(&mut w, &op(serde_json::json!({ "op": "paint", "sphere": [0.0, 2.0, 0.0, 0.4], "material": 1 }))).unwrap();
            assert!(e["voxels"].as_u64().unwrap() > 0 && e["pieces"] == 0, "{e}");
            let e = v.edit(&mut w, &op(serde_json::json!({ "op": "add", "box": [0.5, 0.4, 0.5, 0.7, 0.6, 0.7], "material": 2 }))).unwrap();
            assert_eq!(e["voxels"], 8, "{e}");
            for _ in 0..120 {
                v.frame(&mut w, 1.0 / 60.0).unwrap();
            }
            // The occupancy grid after the edits and landings is what marking every static object's voxels afresh gives
            // (in the same grid: a fresh build would lay it over the level's new bounds).
            let mut fresh = v.occupancy.as_ref().unwrap().read().unwrap().clone();
            fresh.words.iter_mut().for_each(|x| *x = 0);
            for (o, grid) in v.statics() {
                VoxelWorld::mark(&mut fresh, o, grid, v.size, None);
            }
            assert_eq!(v.occupancy.as_ref().unwrap().read().unwrap().words, fresh.words);
            assert!(fresh.words.iter().any(|&x| x != 0));
            (h0, v.hash().unwrap())
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert_ne!(a.0, a.1);
    }

    /// A terrain tile carved away whole, with its neighbour carved at their shared edge: the edit goes through (the
    /// neighbour is meshed again, not the tile that is gone), the tile leaves the tiles' table, and a later edit of the
    /// neighbour's edge does too.
    #[test]
    fn a_tile_carved_away_whole_is_gone() {
        let mut w = World::default();
        let mut v = VoxelWorld::default();
        let tile = |tx: i32| serde_json::json!({ "name": format!("tile-{tx}"), "uri": "tests/render/data/voxel-slab.tpg", "position": [6.4 * tx as f64, 0.0, 0.0], "tile": [tx, 0] });
        let d: LevelDesc = serde_json::from_value(serde_json::json!({
            "route": "mesh",
            "palette": [{ "name": "none", "color": [0, 0, 0], "roughness": 1, "metalness": 0 }, { "name": "ground", "color": [0.5, 0.5, 0.5], "roughness": 1, "metalness": 0 }],
            "objects": [tile(0), tile(1), tile(2)],
        }))
        .unwrap();
        v.request(d).unwrap();
        v.realize(&mut w, &assets()).unwrap();
        assert_eq!((v.tiles.len(), v.world_objects()), (3, 3));
        // Everything of tile 0 (x -0.8 .. 0.8 m) and tile 1's voxels up to its own x = 0 (world 6.4 m).
        let e = v.edit(&mut w, &op(serde_json::json!({ "op": "carve", "box": [-1.0, -1.0, -1.0, 6.4, 1.0, 1.0] }))).unwrap();
        assert_eq!((e["removed"].as_u64(), e["touched"].as_u64()), (Some(1), Some(1)), "{e}");
        assert_eq!(e["voxels"].as_u64(), Some(1024 + 512));
        assert_eq!((v.tiles.len(), v.world_objects()), (2, 2));
        assert!(!v.tiles.contains_key(&(0, 0)) && v.objects[0].is_none());
        // Tile 1's left edge again (its neighbour there is gone), and tile 2 whole: tile 1 is meshed again for both.
        let e = v.edit(&mut w, &op(serde_json::json!({ "op": "carve", "box": [6.4, -1.0, -1.0, 6.5, 1.0, 1.0] }))).unwrap();
        assert_eq!((e["removed"].as_u64(), e["touched"].as_u64(), e["voxels"].as_u64()), (Some(0), Some(1), Some(64)), "{e}");
        let e = v.edit(&mut w, &op(serde_json::json!({ "op": "carve", "box": [11.0, -1.0, -1.0, 14.0, 1.0, 1.0] }))).unwrap();
        assert_eq!((e["removed"].as_u64(), e["touched"].as_u64()), (Some(1), Some(1)), "tile 2 gone, tile 1 beside it drawn again: {e}");
        assert_eq!((v.tiles.len(), v.world_objects()), (1, 1));
    }

    /// Edits are checked: shapes and materials must be well-formed, and an edit before any level is an error.
    #[test]
    fn edits_are_checked() {
        for bad in [
            serde_json::json!({ "op": "carve" }),
            serde_json::json!({ "op": "carve", "sphere": [0, 0, 0] }),
            serde_json::json!({ "op": "carve", "sphere": [0, 0, 0, "x"] }),
            serde_json::json!({ "op": "melt", "sphere": [0, 0, 0, 1] }),
            serde_json::json!({ "op": "paint", "sphere": [0, 0, 0, 1] }),
            serde_json::json!({ "op": "add", "sphere": [0, 0, 0, 1], "material": 0 }),
            serde_json::json!({ "op": "add", "sphere": [0, 0, 0, 1], "material": 256 }),
        ] {
            assert!(EditOp::parse(&bad).is_err(), "{bad}");
        }
        let mut w = World::default();
        let mut v = VoxelWorld::default();
        assert!(v.edit(&mut w, &op(serde_json::json!({ "op": "carve", "sphere": [0, 0, 0, 1] }))).is_err());
        let mut v = level(&mut w);
        // A box of a kilometre is refused; a huge radius is cut down to the largest (12.8 m).
        assert!(v.edit(&mut w, &op(serde_json::json!({ "op": "carve", "box": [-500, -500, -500, 500, 500, 500] }))).is_err());
        let e = v.edit(&mut w, &op(serde_json::json!({ "op": "explode", "sphere": [0, 0, 0, 1e9] }))).unwrap();
        assert!(e["voxels"].as_u64().unwrap() > 0);
    }

    /// Meshing time on the benchmark level's files (`cargo test --release voxel_mesh_timing -- --ignored --nocapture`;
    /// THREERT_ASSETS: the asset root, default ../assets): every terrain tile (64 x 64 columns) and building part,
    /// median and maximum.
    #[test]
    #[ignore]
    fn voxel_mesh_timing() {
        let assets = std::path::PathBuf::from(std::env::var("THREERT_ASSETS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../assets").into()));
        let dir = assets.join("voxel/level/s1v2-object");
        let mut tiles = vec![];
        let mut parts = vec![];
        for e in std::fs::read_dir(&dir).expect("generate the level: node tools/voxel-level/generate.ts --granularity object") {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".tpg") {
                continue;
            }
            let blob = crate::procasset::ProcBlob::parse(&std::fs::read(&p).unwrap()).unwrap();
            let (g, corner, size) = voxel_core::decode(&blob.data).unwrap();
            let t = web_time::Instant::now();
            let reps = 5;
            let mut faces = 0;
            for _ in 0..reps {
                faces = voxel_core::greedy_mesh(&g, corner, size, &mut |_, y, _| y < 0).faces;
            }
            let us = t.elapsed().as_secs_f64() * 1e6 / reps as f64;
            let row = (us, g.count(), faces);
            if name.starts_with("tile-") && !name.contains('x') {
                tiles.push(row);
            } else if name.starts_with("house-") {
                parts.push(row);
            }
        }
        for (what, mut v) in [("64 x 64 terrain tile", tiles), ("building part", parts)] {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            let med = v[v.len() / 2];
            let max = v[v.len() - 1];
            println!("{what}: {} of them, meshing median {:.0} us ({} voxels, {} faces), max {:.0} us ({} voxels, {} faces)", v.len(), med.0, med.1, med.2, max.0, max.1, max.2);
        }
    }
}

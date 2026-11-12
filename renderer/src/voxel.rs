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

use crate::physics::PhysicsWorld;
use crate::world::{Geometry, Kind, World};
use physics_core::{BodyDesc, ColliderDesc};
use anyhow::{anyhow, Result};
use glam::{Mat4, Quat, Vec3};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
/// The most voxel bodies a level holds unless it says otherwise (`LevelDesc::max_bodies`): past it one goes, its
/// voxels debris (`add_body` says which, and when the new part is debris instead). 300 is what was measured to hold
/// 60 fps with every one of them in the air at once, natively on 4 threads (docs/VOXEL.md "Voxel bodies": at 400 a
/// few frames of a thousand went over; with the physics on one thread, `THREERT_PHYSICS_THREADS=1`, 128 is about at
/// the frame).
pub const MAX_BODIES: usize = 300;
/// The most voxels one `add` puts on a body (`edit_body`): 65,536, a block of 4 m at 0.1 m. More is refused, nothing
/// added: a body's collider, mesh and mass are made again from all its cells at every edit and its contacts cost by its
/// size (the review of #24, item 6: a block of 200³ cut loose is 2.8 s of edit and 619 MB), so a body is not grown
/// into a building by one call.
pub const MAX_BODY_ADD: usize = 1 << 16;
/// The most cells a voxel body grows to by adds (`edit_body`): 2^21, a block of 128³. Past it an add is refused: an edit
/// of a body that size takes half a second (its collider, mesh and mass are made from all its cells) and 171 MB (the
/// review of #24, item 6); the physics refuses a voxel shape past 2^24 cells, which no body reaches this way.
pub const MAX_BODY_CELLS: usize = 1 << 21;
/// The largest box a voxel body's cells may span: 1,024 cells a side (102.4 m at 0.1 m, more than any building of the
/// benchmark level) and 2^25 cells in all. Its mesh is made over that box (voxel_core `greedy_mesh`: a byte a cell and
/// 3/8 of one in bit columns, so at most 47 MB a redraw), and its cells stay within the physics' coordinate range. An
/// add past it is refused; a part past it is debris, not a body.
pub const MAX_BODY_SIDE: i32 = 1024;
pub const MAX_BODY_BOX: i64 = 1 << 25;
/// The most "gone" records kept for scene code (`take_gone`) when nothing takes them: older ones are let go.
const MAX_GONE: usize = 1 << 16;

/// The most edits and frames scene code can ask for before the level is loaded (in setup), kept until it is: they are
/// outside the script engine's memory, so they are counted. 4,096: a setup that cuts a level into its starting state
/// takes a few dozen edits and a second of frames (the tests: one cut and 60 frames); one that asks for thousands is
/// doing work that belongs after the load, in frames.
pub const MAX_QUEUED: usize = 4096;
/// A level object's collider holds its cells within this many cells of one that is not solid (six-connected steps):
/// a shell. The benchmark level has 515 million solid cells, 480 million of them terrain down to the bedrock; a
/// collider of every one took 26 s to build and 1.3 GiB. Four cells are 0.4 m of voxels of 0.1 m: a body would have
/// to cross them in one tick (24 m/s, a fall of 29 m) to get behind the shell, and one that does falls out under
/// the level and is taken out (`sync_bodies`). An edit keeps the shell whole around what it carved.
const SHELL: i32 = 4;
/// Friction of voxel colliders (the level's and the bodies').
const FRICTION: f32 = 0.6;
/// kg/m³ of a palette entry that names no density, by its hardness: soft (earth, wood), medium (brick), hard
/// (concrete), indestructible (steel).
const DENSITY: [f32; 4] = [900.0, 1800.0, 2400.0, 7800.0];

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
    /// Physics of the level (ROADMAP § 6 step 8), on unless `false`: every object is a fixed collider of its voxels,
    /// kept up to date as edits carve it (rays and sweeps of the physics world meet the level, bodies rest on it), and
    /// what an edit leaves hanging on no anchor is a rigid body. `false`: no collider, and loose parts fall straight
    /// down through the lattice as before step 8 (`voxel_core::Level::tick`).
    #[serde(default)]
    pub physics: Option<bool>,
    /// The most voxel bodies at once (`MAX_BODIES` by default; at most `physics_core::bridge::MAX_ENGINE_BODIES`).
    #[serde(default, rename = "maxBodies")]
    pub max_bodies: Option<f64>,
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
    /// kg/m³ of its voxels in a rigid body (0, the default: by its hardness, `DENSITY`).
    #[serde(default)]
    pub density: f32,
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
    /// `false`: no collider for this object (bodies and rays pass through it); a part that comes loose from it is a
    /// body all the same.
    #[serde(default)]
    pub collide: Option<bool>,
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

/// What `VoxelWorld::check_script` ends in, natively and in wasm32 alike (recorded: a change of the voxel bodies'
/// behaviour, of the physics or of a platform's arithmetic shows here).
pub const EXPECTED_SCRIPT: u64 = 0x2b38_c79e_27e8_9063;

/// A brick as a collider's shell reads it (`VoxelWorld::brick_at`).
#[derive(Clone, Copy)]
enum BrickRef<'a> {
    Solid,
    Empty,
    Cells(&'a [u8; 512]),
}

/// A loose part as a rigid body (ROADMAP § 6 step 8): the level no longer holds it. Its voxels are its own grid, in
/// the body's space (cell (x, y, z) from (x, y, z) x size); the physics world moves it and the world objects that
/// draw it follow (`VoxelWorld::sync_bodies`).
struct VoxelBody {
    /// The engine's body and its voxel collider (ids of the bridge's engine range).
    body: u32,
    collider: u32,
    grid: VoxelGrid,
    /// Where its objects were last placed.
    pose: ([f32; 3], [f32; 4]),
}

/// What scene code asked of the level before it was loaded (in setup: edits and the ticks after them, so the live
/// view and the offline render start from the same world), done once it is (`realize`). The ticks are the lattice's
/// (`frame`: parts falling cell by cell on a level without physics). On a level that collides a queued edit's loose
/// parts are bodies as soon as it is replayed and the queued ticks have nothing to move: the physics world is
/// stepped by the engine's clock only (`api::physics_advance`, `physics_settle` for an offline render's scene time),
/// never from scene code, so that a live run and an offline one tick it alike.
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
    /// After the ids wrapped: those still in use then, passed over (`id`).
    in_use: Option<std::collections::HashSet<u32>>,
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
    /// Physics (`LevelDesc::physics`): each level object's fixed collider (same index; None: none, or carved away),
    /// the loose parts that are bodies (by their index in `objects`), each palette entry's density, the cap on
    /// bodies, and how many it retired.
    physics: bool,
    /// The level's fixed body (ROADMAP § 6 step 10 c1): its colliders hang on it, so joints can hold to the level.
    level_body: Option<u32>,
    colliders: Vec<Option<u32>>,
    bodies: BTreeMap<usize, VoxelBody>,
    density: Vec<f32>,
    max_bodies: usize,
    pub retired: u64,
    /// Parts that came loose at the cap and were made debris, not bodies (`add_body`).
    pub capped: u64,
    /// The bodies the edit being made has carved or made: none of them is retired for the cap while it runs.
    this_edit: std::collections::BTreeSet<usize>,
    /// The bodies gone since scene code last asked (`take_gone`): engine body id and why ("cap", "carved", "fell").
    gone: Vec<(u32, &'static str)>,
    /// Under this height (m) a body has left the level: 100 m below its lowest cell.
    floor: f32,
    /// Time spent finding the colliders' cells (`shell`), ms: of the load, and of every edit since.
    pub shell_ms: f64,
}

/// Debris voxels kept between two `take_debris` calls, and of one edit.
const MAX_DEBRIS: usize = 1 << 16;
const EDIT_DEBRIS: usize = 4096;
/// The occupancy grid's most words (64 MB): a bigger level's grid takes coarser cells.
const MAX_OCCUPANCY_WORDS: usize = 1 << 24;

impl VoxelWorld {
    /// Files the requested level reads from the asset root (its objects' uris), before `realize` loads them: a client
    /// that fetches the session's files first (`GameSession::files`) must fetch these too.
    pub fn files(&self) -> Vec<String> {
        self.pending.iter().flat_map(|d| d.objects.iter().map(|o| o.uri.clone())).collect()
    }

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

    /// The level's fixed body (its colliders hang on it; joints to the level hold to it), once a level that collides is
    /// loaded.
    pub fn level_body(&self) -> Option<u32> {
        self.level_body
    }

    fn id(&mut self) -> u32 {
        if self.next_id == 0 {
            self.next_id = FIRST_ID;
        }
        loop {
            if self.next_id == u32::MAX {
                // (The ids ran out: from the first again, past the ones still in use, which are all the voxel world's
                // own: its objects' and their geometries', its colliders', its bodies'.)
                self.next_id = FIRST_ID;
                self.in_use = Some(self.live_ids());
            }
            self.next_id += 1;
            if !self.in_use.as_ref().is_some_and(|u| u.contains(&self.next_id)) {
                return self.next_id;
            }
        }
    }

    /// Every id the voxel world holds now (`id`'s after a wrap).
    fn live_ids(&self) -> std::collections::HashSet<u32> {
        let mut u: std::collections::HashSet<u32> = self.objects.iter().flatten().flat_map(|o| o.parts.iter().flat_map(|p| [p.1, p.2])).collect();
        u.extend(self.colliders.iter().flatten());
        u.extend(self.level_body);
        u.extend(self.bodies.values().flat_map(|b| [b.body, b.collider]));
        u
    }

    fn grid(&self, k: usize) -> &Rc<VoxelGrid> {
        &self.level.as_ref().unwrap().objects[k].as_ref().unwrap().grid
    }

    /// Loads the requested level into the world: decodes the files, places the objects (meshed or as boxes). Returns
    /// what it did (ms, objects, triangles) for the log.
    pub fn realize(&mut self, w: &mut World, ph: &mut Option<PhysicsWorld>, assets: &Path) -> Result<Option<serde_json::Value>> {
        let Some(d) = self.pending.take() else { return Ok(None) };
        self.physics = d.physics.unwrap_or(true);
        self.max_bodies = d.max_bodies.map_or(MAX_BODIES, |n| (n.max(0.0) as usize).min(physics_core::bridge::MAX_ENGINE_BODIES));
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
            if !(p.density == 0.0 || (p.density.is_finite() && (10.0..=30_000.0).contains(&p.density))) {
                return Err(anyhow!("voxelLevel: palette entry {}: density must be from 10 to 30000 kg/m³ (or 0: by its hardness)", p.name));
            }
            self.density.push(if p.density > 0.0 { p.density } else { DENSITY[(level.hardness[k] & 3) as usize] });
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
        // The level's colliders: every object a fixed voxel shape of its cells in the world's lattice.
        let t_col = Instant::now();
        let (mut colliders, mut collider_cells) = (0usize, 0usize);
        self.colliders = vec![None; self.objects.len()];
        self.floor = self.level.as_ref().map_or(f32::MIN, |l| l.objects.iter().flatten().map(|o| o.lo[1]).min().map_or(f32::MIN, |y| y as f32 * size - 100.0));
        if self.physics {
            let p = ph.get_or_insert_with(Default::default);
            // The level's fixed body first (at the origin, unturned: its colliders keep their world poses).
            let lb = self.id();
            p.bridge().engine_level_body(lb).map_err(|e| anyhow!(e))?;
            self.level_body = Some(lb);
            let (mut full, mut shapes) = (HashMap::new(), HashMap::new());
            for k in 0..self.objects.len() {
                if d.objects.get(k).is_some_and(|o| o.collide == Some(false)) {
                    continue;
                }
                if let Some(n) = self.collide(p, k, &mut full, &mut shapes) {
                    colliders += 1;
                    collider_cells += n;
                }
            }
        }
        let colliders_ms = t_col.elapsed().as_secs_f64() * 1e3;
        let queued = std::mem::take(&mut self.queued);
        let (edits, frames) = (queued.iter().filter(|q| matches!(q, Queued::Edit(..))).count(), queued.iter().filter(|q| matches!(q, Queued::Frame(_))).count());
        for q in queued {
            match q {
                Queued::Edit(op) => {
                    self.edit(w, ph, &op)?;
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
            "colliders": colliders, "collider_cells": collider_cells, "colliders_ms": colliders_ms, "shell_ms": self.shell_ms,
        })))
    }

    /// Level object `k`'s fixed collider: the shell of its cells (`shell`), in its grid's own cells, placed as the
    /// object is (`collider_pose`). Objects of one file have one grid between them and so one shape (`shapes`: by
    /// grid, the shape and its cells): they share it until an edit gives one of them cells of its own
    /// (`Physics::edit_voxels`). A terrain tile's shell depends on its neighbours: its own shape. Returns the
    /// collider's cells, or None if it got none (no voxel; a cell past what a shape may reach: said once).
    fn collide(&mut self, p: &mut PhysicsWorld, k: usize, full: &mut HashMap<(usize, (i32, i32, i32)), bool>, shapes: &mut HashMap<usize, (physics_core::SharedVoxels, usize)>) -> Option<usize> {
        let o = self.level.as_ref()?.objects.get(k)?.as_ref()?;
        let (grid, place) = (Rc::as_ptr(&o.grid) as usize, o.place);
        let alone = self.objects.get(k)?.as_ref()?.tile.is_some();
        let made = match shapes.get(&grid).filter(|_| !alone) {
            Some(s) => Ok(s.clone()),
            None => {
                let t = Instant::now();
                let cells = self.shell(k, None, full);
                self.shell_ms += t.elapsed().as_secs_f64() * 1e3;
                if cells.is_empty() {
                    return None;
                }
                let n = cells.len();
                physics_core::SharedVoxels::new(self.size, cells).map(|s| {
                    if !alone {
                        shapes.insert(grid, (s.clone(), n));
                    }
                    (s, n)
                })
            }
        };
        let id = self.id();
        let (position, rotation) = collider_pose(&place, self.size);
        match made.and_then(|(shape, n)| p.bridge().engine_collider(id, self.level_body, &ColliderDesc { position, rotation, friction: FRICTION, ..ColliderDesc::new(physics_core::Shape::SharedVoxels(shape)) }).map(|()| n)) {
            Ok(n) => {
                if self.colliders.len() <= k {
                    self.colliders.resize(k + 1, None);
                }
                self.colliders[k] = Some(id);
                Some(n)
            }
            Err(e) => {
                eprintln!("voxels: {} has no collider ({e})", self.objects[k].as_ref().map_or("?", |o| o.name.as_str()));
                None
            }
        }
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

    /// An edit or a frame asked for before the level is loaded, kept for `realize`: at most `MAX_QUEUED`.
    fn queue(&mut self, q: Queued) -> Result<()> {
        if self.queued.len() >= MAX_QUEUED {
            return Err(anyhow!("voxels: at most {MAX_QUEUED} edits and frames can be asked for before the level is loaded"));
        }
        self.queued.push(q);
        Ok(())
    }

    /// Scene code's `voxelExplode`: an explosion at a world position (m) of a radius, each material within its
    /// hardness's share of it.
    pub fn explode(&mut self, w: &mut World, ph: &mut Option<PhysicsWorld>, center: Vec3, radius: f32) -> Result<serde_json::Value> {
        self.edit(w, ph, &EditOp { shape: ShapeM::Sphere(center.to_array(), radius), kind: EditKind::Carve(RADII) })
    }

    /// An edit (`voxelEdit`): the level's voxels changed (`voxel_core::Level`), the touched objects drawn again in place,
    /// the loose parts made falling objects, the removed objects gone. Returns what it did and how long each part took.
    pub fn edit(&mut self, w: &mut World, ph: &mut Option<PhysicsWorld>, op: &EditOp) -> Result<serde_json::Value> {
        if self.route.is_none() && self.pending.is_some() {
            self.queue(Queued::Edit(op.clone()))?;
            return Ok(serde_json::json!({ "queued": true }));
        }
        if self.route.is_none() {
            return Err(anyhow!("voxelEdit: no voxel level loaded"));
        }
        self.ticks.0 += 1;
        self.this_edit.clear();
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
        let mut out = self.apply(w, ph, &e, vec![bounds])?;
        out["carve_ms"] = carve_ms.into();
        // The bodies the shape reaches are carved too (a carve only: paint and added voxels are the level's).
        if let (EditKind::Carve(radii), Some(p)) = (op.kind, ph.as_mut().filter(|_| !self.bodies.is_empty())) {
            let t = Instant::now();
            let b = self.carve_bodies(w, p, op.shape, radii, None)?;
            out["body_voxels"] = b.0.into();
            out["bodies_carved"] = b.1.into();
            out["bodies_ms"] = (t.elapsed().as_secs_f64() * 1e3).into();
        }
        // Paint reaches the bodies too; added voxels are the level's (`edit_body` adds to a body).
        if let (EditKind::Paint(m), Some(p)) = (op.kind, ph.as_mut().filter(|_| !self.bodies.is_empty())) {
            let b = self.paint_bodies(w, p, op.shape, m, None)?;
            out["body_voxels"] = b.0.into();
            out["bodies_painted"] = b.1.into();
        }
        out["bodies"] = self.bodies.len().into();
        Ok(out)
    }

    /// What an edit of the level did, drawn: `boxes` are the world boxes whose occupancy (for particles) to set again.
    fn apply<'e>(&mut self, w: &mut World, ph: &mut Option<PhysicsWorld>, e: &'e Edit, mut boxes: Vec<(Vec3, Vec3)>) -> Result<serde_json::Value> {
        let size = self.size;
        let route = self.route.unwrap();
        // The colliders of the objects that lost cells lose them (by the cells, not by the object's size); those of
        // the objects that are gone go.
        let t_col = Instant::now();
        if let Some(p) = ph.as_mut().filter(|_| self.physics) {
            // (Each object's lost cells out, and its shell made whole around them: the cells an edit brought within
            // `SHELL` of the open. A terrain tile carved at its edge bares its neighbours' sides: theirs too, over
            // the carved tile's bounds, whether or not the neighbour lost cells of its own: each object is made whole
            // over the box around its own bounds and every carved neighbour's.)
            let mut work: BTreeMap<usize, (&'e [[i32; 3]], ([i32; 3], [i32; 3]))> = BTreeMap::new();
            let mut over = |k: usize, cells: Option<&'e [[i32; 3]]>, (lo, hi): ([i32; 3], [i32; 3])| {
                let w = work.entry(k).or_insert((&[][..], (lo, hi)));
                if let Some(c) = cells {
                    w.0 = c;
                }
                w.1 = ([0, 1, 2].map(|a| w.1 .0[a].min(lo[a])), [0, 1, 2].map(|a| w.1 .1[a].max(hi[a])));
            };
            for (&k, cells) in &e.cleared {
                let Some(&(lo, hi)) = e.bounds.get(&k) else { continue };
                let around = (lo.map(|x| x - SHELL - 1), hi.map(|x| x + SHELL + 1));
                over(k as usize, Some(cells.as_slice()), around);
                if let Some((tx, tz)) = self.objects.get(k as usize).and_then(|o| o.as_ref()).and_then(|o| o.tile) {
                    for n in [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().filter_map(|d| self.tiles.get(&(tx + d.0, tz + d.1))) {
                        over(*n, None, around);
                    }
                }
            }
            let mut full = HashMap::new();
            for (k, (cells, (lo, hi))) in work {
                let Some(c) = self.colliders.get(k).copied().flatten() else { continue };
                // (In the object's own cells, as its collider has them. An object that is gone: its collider goes below.)
                let Some(o) = self.level.as_ref().and_then(|l| l.objects.get(k)?.as_ref()) else { continue };
                let t = Instant::now();
                let fill = self.shell(k, Some(o.place.box_to_grid(lo, hi)), &mut full);
                self.shell_ms += t.elapsed().as_secs_f64() * 1e3;
                let cells: Vec<[i32; 3]> = cells.iter().map(|&w| o.place.to_grid(w)).collect();
                match p.bridge().engine_edit_voxels(c, &cells, &fill) {
                    Ok(0) => self.colliders[k] = None,
                    Ok(_) => {}
                    Err(err) => {
                        eprintln!("voxels: a collider was not edited ({err})");
                        // (One the edit took out, past the cells a shape holds, is gone: no id is kept for it.)
                        if p.bridge_ref().engine_voxels(c).is_none() {
                            self.colliders[k] = None;
                        }
                    }
                }
            }
            for &k in &e.removed {
                if let Some(c) = self.colliders.get_mut(k as usize).and_then(|c| c.take()) {
                    p.bridge().engine_remove(None, Some(c));
                }
            }
        }
        let colliders_ms = t_col.elapsed().as_secs_f64() * 1e3;
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
                    w.drop_geometry(p.2);
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
            // A loose part is a rigid body: the level hands it out, the physics world has it from here.
            if let (true, Some(p)) = (falling && self.physics, ph.as_mut()) {
                let o = self.level.as_mut().unwrap().take(id).expect("a falling object");
                let grid = Rc::try_unwrap(o.grid).unwrap_or_else(|g| (*g).clone());
                boxes.push((Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) * size, Vec3::new(hi[0] as f32 + 1.0, hi[1] as f32 + 1.0, hi[2] as f32 + 1.0) * size));
                let tm = Instant::now();
                self.add_body(w, p, k, grid, corner, Quat::IDENTITY, ([0.0; 3], [0.0; 3]))?;
                piece_mesh_ms += tm.elapsed().as_secs_f64() * 1e3;
                continue;
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
            // (Added voxels that hold: a collider of their own.)
            if let (false, true, Some(p)) = (falling, self.physics, ph.as_mut()) {
                self.collide(p, k, &mut HashMap::new(), &mut HashMap::new());
            }
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
            "occupancy_ms": occupancy_ms, "world_objects": self.world_objects(), "colliders_ms": colliders_ms,
        }))
    }

    /// Brick `key` of level object `k`'s grid as its collider's shell sees it: the object's own; and for a terrain
    /// tile, what lies beside and under it: solid under the bedrock, the neighbouring tile's brick beside it (tiles
    /// stand side by side on one lattice of bricks; one that does not is taken as standing alone, its sides bare).
    /// With the brick, the object and key it is at (the fullness cache's key).
    fn brick_at(&self, k: usize, key: (i32, i32, i32)) -> (BrickRef<'_>, (usize, (i32, i32, i32))) {
        let own = |k: usize, key: (i32, i32, i32)| match self.level.as_ref().and_then(|l| l.objects.get(k)?.as_ref()).and_then(|o| o.grid.bricks.get(&key)) {
            Some(b) => (BrickRef::Cells(b), (k, key)),
            None => (BrickRef::Empty, (k, key)),
        };
        let Some(o) = self.objects.get(k).and_then(|o| o.as_ref()) else { return (BrickRef::Empty, (k, key)) };
        let (Some((tx, tz)), true) = (o.tile, o.off.iter().all(|x| x % 8 == 0)) else { return own(k, key) };
        // In the tile's own bricks: 8 a side from its corner.
        let q = (key.0 + o.off[0] / 8, key.1 + o.off[1] / 8, key.2 + o.off[2] / 8);
        if q.1 < 0 {
            return (BrickRef::Solid, (k, key));
        }
        let side = TILE / 8;
        if (0..side).contains(&q.0) && (0..side).contains(&q.2) {
            return own(k, key);
        }
        let Some(&n) = self.tiles.get(&(tx + q.0.div_euclid(side), tz + q.2.div_euclid(side))) else { return (BrickRef::Empty, (k, key)) };
        match self.objects[n].as_ref().filter(|t| t.off.iter().all(|x| x % 8 == 0)) {
            Some(t) => own(n, (q.0.rem_euclid(side) - t.off[0] / 8, q.1 - t.off[1] / 8, q.2.rem_euclid(side) - t.off[2] / 8)),
            None => (BrickRef::Empty, (k, key)),
        }
    }

    /// The cells of brick `key` of object `k` that are in its collider's shell (`SHELL`), added to `out` (grid cells).
    /// `full`: which bricks are solid through, remembered over a pass of many bricks (a brick is asked about by each
    /// of its 26 neighbours; the level has a million of them, most deep in the ground).
    fn shell_of(&self, k: usize, key: (i32, i32, i32), out: &mut Vec<[i32; 3]>, full: &mut HashMap<(usize, (i32, i32, i32)), bool>) {
        let mut around = [BrickRef::Empty; 27];
        let mut all = true;
        for (i, b) in around.iter_mut().enumerate() {
            let d = ((i % 3) as i32 - 1, ((i / 3) % 3) as i32 - 1, (i / 9) as i32 - 1);
            let (brick, at) = self.brick_at(k, (key.0 + d.0, key.1 + d.1, key.2 + d.2));
            *b = brick;
            all &= match brick {
                BrickRef::Solid => true,
                BrickRef::Empty => false,
                BrickRef::Cells(c) => *full.entry(at).or_insert_with(|| c.iter().all(|&m| m != 0)),
            };
            // (Not solid all around: the rest are read below as they are needed.)
            if !all && i > 13 {
                for (j, b) in around.iter_mut().enumerate().skip(i + 1) {
                    let d = ((j % 3) as i32 - 1, ((j / 3) % 3) as i32 - 1, (j / 9) as i32 - 1);
                    *b = self.brick_at(k, (key.0 + d.0, key.1 + d.1, key.2 + d.2)).0;
                }
                break;
            }
        }
        let BrickRef::Cells(centre) = around[13] else { return };
        // (Solid all around: every cell is more than `SHELL` from an empty one.)
        if all {
            return;
        }
        // The brick and `SHELL` cells around it, a row of bits along x for each (y, z): the cells that are not solid,
        // then those within `SHELL` steps of one.
        const W: usize = 8 + 2 * SHELL as usize;
        let mut reach = [[0u32; W]; W];
        for (wy, plane) in reach.iter_mut().enumerate() {
            for (wz, row) in plane.iter_mut().enumerate() {
                let (sy, sz) = (wy + 8 - SHELL as usize, wz + 8 - SHELL as usize);
                for wx in 0..W {
                    let sx = wx + 8 - SHELL as usize;
                    let solid = match around[(sx >> 3) + 3 * (sy >> 3) + 9 * (sz >> 3)] {
                        BrickRef::Solid => true,
                        BrickRef::Empty => false,
                        BrickRef::Cells(c) => c[(sx & 7) + 8 * (sy & 7) + 64 * (sz & 7)] != 0,
                    };
                    *row |= (!solid as u32) << wx;
                }
            }
        }
        for _ in 0..SHELL {
            let was = reach;
            for y in 0..W {
                for z in 0..W {
                    let mut r = was[y][z] | (was[y][z] << 1) | (was[y][z] >> 1);
                    if y > 0 {
                        r |= was[y - 1][z];
                    }
                    if y + 1 < W {
                        r |= was[y + 1][z];
                    }
                    if z > 0 {
                        r |= was[y][z - 1];
                    }
                    if z + 1 < W {
                        r |= was[y][z + 1];
                    }
                    reach[y][z] = r;
                }
            }
        }
        for (v, &m) in centre.iter().enumerate() {
            let (x, y, z) = (v & 7, (v >> 3) & 7, v >> 6);
            if m != 0 && reach[y + SHELL as usize][z + SHELL as usize] >> (x + SHELL as usize) & 1 != 0 {
                out.push([key.0 * 8 + x as i32, key.1 * 8 + y as i32, key.2 * 8 + z as i32]);
            }
        }
    }

    /// The cells of level object `k`'s collider (grid cells): its shell, whole, or of the bricks that reach into the
    /// box `within` of its grid.
    fn shell(&self, k: usize, within: Option<([i32; 3], [i32; 3])>, full: &mut HashMap<(usize, (i32, i32, i32)), bool>) -> Vec<[i32; 3]> {
        let mut out = vec![];
        let Some(o) = self.level.as_ref().and_then(|l| l.objects.get(k)?.as_ref()) else { return out };
        match within {
            None => {
                for &key in o.grid.bricks.keys() {
                    self.shell_of(k, key, &mut out, full);
                }
            }
            Some((lo, hi)) => {
                for bx in lo[0] >> 3..=hi[0] >> 3 {
                    for by in lo[1] >> 3..=hi[1] >> 3 {
                        for bz in lo[2] >> 3..=hi[2] >> 3 {
                            if o.grid.bricks.contains_key(&(bx, by, bz)) {
                                self.shell_of(k, (bx, by, bz), &mut out, full);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Every level object's collider against its shell found by brute force (the checks': `examples/physics_fuzz.rs`):
    /// its cells within `SHELL` six-connected steps of a cell that is not solid, solid being the object's own cells and,
    /// for a terrain tile on the bricks' lattice, its neighbours' and the bedrock under it (as `brick_at` reads them).
    /// Err names the first object whose collider differs, and how. Ok: the objects checked.
    pub fn check_shells(&self, p: &PhysicsWorld) -> std::result::Result<usize, String> {
        let Some(level) = self.level.as_ref() else { return Ok(0) };
        let cell = |k: usize, g: [i32; 3]| level.objects.get(k).and_then(|o| o.as_ref()).is_some_and(|o| o.grid.get(g[0], g[1], g[2]) != 0);
        let mut checked = 0;
        for (k, c) in self.colliders.iter().enumerate() {
            let (Some(c), Some(o)) = (*c, level.objects.get(k).and_then(|o| o.as_ref())) else { continue };
            let tile = self.objects.get(k).and_then(|o| o.as_ref()).filter(|v| v.off.iter().all(|x| x % 8 == 0)).and_then(|v| v.tile.map(|t| (t, v.off)));
            let solid = |g: [i32; 3]| -> bool {
                let Some(((tx, tz), off)) = tile else { return cell(k, g) };
                let t = [g[0] + off[0], g[1] + off[1], g[2] + off[2]];
                if t[1] < 0 {
                    return true;
                }
                if (0..TILE).contains(&t[0]) && (0..TILE).contains(&t[2]) {
                    return cell(k, g);
                }
                let Some(&n) = self.tiles.get(&(tx + t[0].div_euclid(TILE), tz + t[2].div_euclid(TILE))) else { return false };
                match self.objects.get(n).and_then(|v| v.as_ref()).filter(|v| v.off.iter().all(|x| x % 8 == 0)) {
                    Some(v) => cell(n, [t[0].rem_euclid(TILE) - v.off[0], t[1] - v.off[1], t[2].rem_euclid(TILE) - v.off[2]]),
                    None => false,
                }
            };
            let mut want: Vec<[i32; 3]> = o
                .grid
                .voxels()
                .map(|(g, _)| g)
                .filter(|g| (-SHELL..=SHELL).any(|dx| (-SHELL + dx.abs()..=SHELL - dx.abs()).any(|dy| { let r = SHELL - dx.abs() - dy.abs(); (-r..=r).any(|dz| !solid([g[0] + dx, g[1] + dy, g[2] + dz])) })))
                .collect();
            want.sort_unstable();
            let have = p.bridge_ref().engine_voxel_cells(c).ok_or_else(|| format!("object {k}: its collider {c} is gone"))?;
            if have != want {
                let missing = want.iter().filter(|g| have.binary_search(g).is_err()).count();
                let extra = have.iter().filter(|g| want.binary_search(g).is_err()).count();
                let first = want.iter().find(|g| have.binary_search(g).is_err()).or(have.iter().find(|g| want.binary_search(g).is_err()));
                return Err(format!("object {k}: its collider has {} cells, its shell {}: {missing} missing, {extra} too many (first {first:?})", have.len(), want.len()));
            }
            checked += 1;
        }
        // An object with cells and no collider: its shell is not empty, so it should have one.
        for (k, o) in level.objects.iter().enumerate() {
            if o.as_ref().is_some_and(|o| o.grid.count() > 0) && self.colliders.get(k).copied().flatten().is_none() && self.physics && !self.bodies.contains_key(&k) && self.objects.get(k).is_some_and(|v| v.is_some()) {
                return Err(format!("object {k} has {} cells and no collider", o.as_ref().unwrap().grid.count()));
            }
        }
        Ok(checked)
    }

    /// A level of solid blocks of palette entry 1 made here, no file (the checks' and the tests'): each block's corner
    /// cell, its size in cells, its terrain tile if it is one, and whether it is an anchor. Voxels of 0.1 m, the mesh
    /// route, the level's physics on.
    pub fn of_blocks(blocks: &[([i32; 3], [i32; 3], Option<(i32, i32)>, bool)], w: &mut World, ph: &mut Option<PhysicsWorld>) -> VoxelWorld {
        let mut v = VoxelWorld { route: Some(Route::Mesh), size: 0.1, palette: vec![0, 0], physics: true, max_bodies: MAX_BODIES, density: vec![1000.0; 2], floor: -100.0, ..Default::default() };
        let mut level = Level::new(gravity_for(0.1, TICK_RATE));
        for (n, &(at, size, tile, anchor)) in blocks.iter().enumerate() {
            let mut grid = VoxelGrid::default();
            for x in 0..size[0] {
                for y in 0..size[1] {
                    for z in 0..size[2] {
                        grid.set(x, y, z, 1);
                    }
                }
            }
            let k = level.add_object(Rc::new(grid), Placement::new(at, 0, [0, 0, 0]), anchor) as usize;
            let corner = Vec3::new(at[0] as f32, at[1] as f32, at[2] as f32) * 0.1;
            v.objects.push(Some(VoxelObject { name: format!("block{n}"), world: Mat4::from_translation(corner), corner: Vec3::ZERO, off: [0; 3], tile, lo: corner, hi: corner + Vec3::new(size[0] as f32, size[1] as f32, size[2] as f32) * 0.1, shared: None, parts: vec![], flying: None }));
            if let Some(t) = tile {
                v.tiles.insert(t, k);
            }
        }
        level.settle();
        v.level = Some(level);
        for k in 0..blocks.len() {
            let g = v.make_geometry(w, k, None).expect("a block's mesh");
            v.place(w, k, &g).expect("a block's object");
        }
        v.colliders = vec![None; blocks.len()];
        let p = ph.get_or_insert_with(Default::default);
        // (The level's fixed body first, as a loaded level has.)
        let lb = v.id();
        p.bridge().engine_level_body(lb).unwrap();
        v.level_body = Some(lb);
        let (mut full, mut shapes) = (HashMap::new(), HashMap::new());
        for k in 0..blocks.len() {
            v.collide(p, k, &mut full, &mut shapes);
        }
        v
    }

    /// The voxel bodies' determinism script (tools/physics-determinism.ts runs it natively and in wasm32, in the
    /// renderer's graph): a tower on two terrain tiles is undercut on one side, so what is above the cut is a body
    /// that lands on the stump off its balance and topples; an explosion where it lies carves it as it is turned (its
    /// cells' centres through the body's pose) and cuts it in bodies; the ground under them is carved. The level's
    /// hash and the physics world's, folded every 20 ticks and after every edit.
    pub fn check_script() -> u64 {
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::of_blocks(&[([0, 0, 0], [64, 8, 64], Some((0, 0)), true), ([64, 0, 0], [64, 8, 64], Some((1, 0)), true), ([52, 8, 28], [6, 40, 6], None, false)], &mut w, &mut ph);
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let fold = |h: &mut u64, v: &VoxelWorld, ph: &Option<PhysicsWorld>| {
            for x in [v.hash().unwrap_or(0), ph.as_ref().map_or(0, |p| p.hash()), v.bodies() as u64] {
                *h = (*h ^ x).wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        let run = |h: &mut u64, v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>, ticks: u32| {
            for t in 0..ticks {
                crate::api::physics_advance(ph, v, w, 1.0 / 60.0);
                w.take_changes();
                if t % 20 == 19 {
                    fold(h, v, ph);
                }
            }
        };
        let edit = |h: &mut u64, v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>, shape: ShapeM| {
            v.edit(w, ph, &EditOp { shape, kind: EditKind::Carve([256; 4]) }).expect("an edit");
            w.take_changes();
            fold(h, v, ph);
        };
        // The undercut: a ball out of the tower's foot on its +x side, then a cut through what is left of that level:
        // 1,260 voxels come down on a stump with a hollow in it.
        edit(&mut h, &mut v, &mut w, &mut ph, ShapeM::Sphere([5.9, 1.1, 3.1], 0.45));
        edit(&mut h, &mut v, &mut w, &mut ph, ShapeM::Box([5.0, 1.0, 2.6], [5.9, 1.2, 3.6]));
        run(&mut h, &mut v, &mut w, &mut ph, 240);
        // Where the body stands now: an explosion into its foot (it is carved through its pose, and goes over), then
        // a pit in the ground under it.
        let at = ph.as_ref().and_then(|p| v.body_list(p).as_array().and_then(|l| l.iter().max_by_key(|b| b["voxels"].as_u64()).map(|b| [0, 1, 2].map(|k| b["position"][k].as_f64().unwrap_or(0.0) as f32)))).unwrap_or([5.5, 1.0, 3.1]);
        edit(&mut h, &mut v, &mut w, &mut ph, ShapeM::Sphere([at[0] + 0.3, at[1] + 0.3, at[2] + 0.3], 0.5));
        run(&mut h, &mut v, &mut w, &mut ph, 120);
        edit(&mut h, &mut v, &mut w, &mut ph, ShapeM::Sphere([at[0], 0.8, at[2]], 0.9));
        run(&mut h, &mut v, &mut w, &mut ph, 240);
        // Step 10 (a): the largest body painted dense on one side from the world, then grown by a block of its own on
        // that side (mass by cell: it settles another way).
        v.density.push(7800.0);
        v.palette.push(0);
        // (Its middle: the centre of its grid's bounds, through its pose.)
        let (b, at) = ph
            .as_ref()
            .and_then(|p| {
                let x = v.bodies.values().max_by_key(|x| x.grid.count())?;
                let (at, rot, _) = p.bridge_ref().engine_pose(x.body)?;
                let (lo, hi) = x.grid.bounds()?;
                let mid = (Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) + Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32)) * 0.5 * v.size;
                Some((x.body, Mat4::from_rotation_translation(Quat::from_array(rot), Vec3::from(at)).transform_point3(mid).to_array()))
            })
            .unwrap_or((0, [5.5, 1.0, 3.1]));
        v.edit(&mut w, &mut ph, &EditOp { shape: ShapeM::Sphere([at[0] + 0.3, at[1], at[2]], 0.7), kind: EditKind::Paint(2) }).expect("a paint");
        w.take_changes();
        fold(&mut h, &v, &ph);
        if v.bodies.values().any(|x| x.body == b) {
            v.edit_body(&mut w, &mut ph, b, &EditOp { shape: ShapeM::Box([at[0] + 0.2, at[1] - 0.3, at[2] - 0.3], [at[0] + 0.8, at[1] + 0.3, at[2] + 0.3]), kind: EditKind::Add(2) }).expect("an add");
            w.take_changes();
        }
        fold(&mut h, &v, &ph);
        run(&mut h, &mut v, &mut w, &mut ph, 180);
        h
    }

    /// A voxel grid as a rigid body at a pose: the objects that draw it (its mesh in its own space, placed by the
    /// pose, driven: a move is a transform), the body and its voxel collider. Its mass is its voxels' (each palette
    /// entry's density), spread evenly over its cells. At `max_bodies` one goes first, or this part is debris (below).
    /// A grid the physics world refuses (no cell, a cell too far) is not drawn either: its voxels are
    /// debris.
    fn add_body(&mut self, w: &mut World, p: &mut PhysicsWorld, k: usize, grid: VoxelGrid, position: Vec3, rotation: Quat, velocity: ([f32; 3], [f32; 3])) -> Result<()> {
        let size = self.size;
        let debris = |s: &mut Self, grid: &VoxelGrid| s.keep(grid.voxels().map(|(g, m)| (position + rotation * ((Vec3::new(g[0] as f32, g[1] as f32, g[2] as f32) + 0.5) * size), m)));
        // At the cap, one goes to make room, if one may: never a body this edit carved or made (the remainder of a
        // body being cut, the parts it is cut into, the other parts of one explosion); of the rest the one that
        // matters least: asleep before awake, then the fewest voxels, then the oldest. It goes only if it has no
        // more voxels than the part that would take its place; else that part is debris itself (a chip cut off at
        // the cap does not make a bigger body vanish).
        let count = grid.count();
        // (A part whose cells span more than a body may is debris, as one the physics refuses.)
        if !grid.bounds().is_some_and(|(lo, hi)| body_fits(lo, hi.map(|v| v - 1))) {
            debris(self, &grid);
            return Ok(());
        }
        while self.bodies.len() >= self.max_bodies.max(1) {
            let least = self
                .bodies
                .iter()
                .filter(|(k, _)| !self.this_edit.contains(k))
                .map(|(&k, b)| (!p.bridge_ref().engine_pose(b.body).is_none_or(|s| s.2), b.grid.count(), k))
                .min();
            match least {
                Some((_, n, k)) if n <= count => self.retire(w, p, k, true, "cap"),
                _ => {
                    debris(self, &grid);
                    self.capped += 1;
                    return Ok(());
                }
            }
        }
        let cells: Vec<[i32; 3]> = grid.voxels().map(|(g, _)| g).collect();
        // (The collider carries no mass: the body's is its cells', each at its own density: `body_mass`.)
        let density = 0.0;
        let (body, collider) = (self.id(), self.id());
        let made = p.bridge().engine_body(body, &BodyDesc { position: position.to_array(), rotation: rotation.to_array(), linvel: velocity.0, angvel: velocity.1, ..Default::default() }).and_then(|_| {
            let d = ColliderDesc { density, friction: FRICTION, ..ColliderDesc::new(physics_core::Shape::Voxels { size, cells }) };
            p.bridge().engine_collider(collider, Some(body), &d)
        });
        if let Err(e) = made {
            eprintln!("voxels: a loose part is no body ({e}): its voxels are debris");
            p.bridge().engine_remove(Some(body), None);
            debris(self, &grid);
            return Ok(());
        }
        if let Err(e) = self.weigh(p, body, &grid) {
            eprintln!("voxels: a body's mass was not set ({e})");
        }
        self.this_edit.insert(k);
        let parts = self.body_parts(w, k, &grid, position, rotation)?;
        if self.objects.len() <= k {
            self.objects.resize_with(k + 1, || None);
        }
        let mut v = VoxelObject { name: format!("part{k}"), world: Mat4::from_rotation_translation(rotation, position), corner: Vec3::ZERO, off: [0; 3], tile: None, lo: Vec3::ZERO, hi: Vec3::ZERO, shared: None, parts, flying: None };
        v.bounds(&grid, size);
        self.objects[k] = Some(v);
        self.bodies.insert(k, VoxelBody { body, collider, grid, pose: (position.to_array(), rotation.to_array()) });
        Ok(())
    }

    /// The world objects that draw a body's grid at a pose: new ones, driven (added in place, moved as instances).
    fn body_parts(&mut self, w: &mut World, k: usize, grid: &VoxelGrid, position: Vec3, rotation: Quat) -> Result<Vec<(u8, u32, u32)>> {
        let size = self.size;
        let mut parts = vec![];
        let place = |w: &mut World, id: u32, kind: Kind, m: u8| -> Result<()> {
            w.set_object(id, format!("part{k}#{m}"), None, kind, true, false)?;
            w.set_transform(id, position, rotation, Vec3::ONE)?;
            w.set_driven(id)
        };
        match self.route {
            Some(Route::Boxes) => {
                let l = layout(grid, Granularity::Object, [0.0; 3], size);
                if !l.boxes.is_empty() {
                    let (gid, id) = (self.id(), self.id());
                    w.set_proc_geometry(gid, self.function.clone(), l.boxes, l.data)?;
                    place(w, id, Kind::Procedural { geometry: gid, materials: self.palette.clone() }, 0)?;
                    parts.push((0, id, gid));
                }
            }
            _ => {
                let mut mesh = greedy_mesh(grid, [0.0; 3], size, &mut |_, _, _| false);
                if self.vertex_colors {
                    mesh.parts = voxel_core::merge_by_class(mesh.parts, &self.class_of, &self.entry_color);
                }
                for part in mesh.parts {
                    let (gid, id, m) = (self.id(), self.id(), part.material);
                    w.set_geometry(gid, part_geometry(part, Vec3::ZERO))?;
                    place(w, id, Kind::Mesh { geometry: gid, material: self.palette.get(m as usize).copied().unwrap_or(0), own_blas: true, materials: vec![] }, m)?;
                    parts.push((m, id, gid));
                }
            }
        }
        Ok(parts)
    }

    /// Voxels kept as debris for scene code (`LevelDesc::debris`): world centre and palette entry, within the caps.
    fn keep(&mut self, voxels: impl Iterator<Item = (Vec3, u8)>) {
        if !self.keep_debris {
            return;
        }
        for (c, m) in voxels.take(EDIT_DEBRIS) {
            if self.debris.len() / 4 >= MAX_DEBRIS {
                break;
            }
            self.debris.extend([c.x, c.y, c.z, m as f32]);
        }
    }

    /// Body `k` gone: its objects, its body and collider. `debris`: its voxels are kept as debris where they are (it
    /// was retired for the cap; one that left the world leaves nothing).
    fn retire(&mut self, w: &mut World, p: &mut PhysicsWorld, k: usize, debris: bool, reason: &'static str) {
        let Some(b) = self.bodies.remove(&k) else { return };
        if self.gone.len() >= MAX_GONE {
            self.gone.drain(..MAX_GONE / 2);
        }
        self.gone.push((b.body, reason));
        if debris {
            let m = Mat4::from_rotation_translation(Quat::from_array(b.pose.1), Vec3::from(b.pose.0));
            let size = self.size;
            self.keep(b.grid.voxels().map(|(g, mat)| (m.transform_point3((Vec3::new(g[0] as f32, g[1] as f32, g[2] as f32) + 0.5) * size), mat)));
            self.retired += 1;
        }
        p.bridge().engine_remove(Some(b.body), None);
        if let Some(o) = self.objects.get_mut(k).and_then(|o| o.take()) {
            for part in o.parts {
                w.remove(part.1);
                w.drop_geometry(part.2);
            }
        }
    }

    /// After the physics world's ticks: every voxel body's objects where the body is now (a body that did not move
    /// costs a comparison); a body the world took out (it fell off the level) goes with its objects. Returns how many
    /// moved and how many went.
    pub fn sync_bodies(&mut self, w: &mut World, p: &mut PhysicsWorld) -> (usize, usize) {
        let (mut moved, mut gone) = (0, vec![]);
        // (A body far under the level's lowest cell got through the ground: it goes.)
        let floor = self.floor;
        for (&k, b) in self.bodies.iter_mut() {
            match p.bridge_ref().engine_pose(b.body) {
                None => gone.push(k),
                Some((at, ..)) if at[1] < floor => gone.push(k),
                Some((at, rot, _)) if (at, rot) != b.pose => {
                    b.pose = (at, rot);
                    let (t, r) = (Vec3::from(at), Quat::from_array(rot));
                    if let Some(o) = self.objects.get_mut(k).and_then(|o| o.as_mut()) {
                        o.world = Mat4::from_rotation_translation(r, t);
                        for part in &o.parts {
                            w.place_driven(part.1, t, r);
                        }
                    }
                    moved += 1;
                }
                Some(_) => {}
            }
        }
        for &k in &gone {
            self.retire(w, p, k, false, "fell");
        }
        (moved, gone.len())
    }

    /// A carve reaches the bodies too: of each body the shape meets, the voxels whose centres are inside it (each
    /// palette entry within its hardness's share of the radius) go. What is left of a body stays the body (its
    /// collider loses the cells, its mesh is made again); what the carve cut off it is a body of its own, moving as
    /// it did (parts under `MIN_PIECE` voxels: debris); a body with nothing left is gone. Returns the voxels carved
    /// and the bodies carved.
    fn carve_bodies(&mut self, w: &mut World, p: &mut PhysicsWorld, shape: ShapeM, radii: [i32; 4], only: Option<usize>) -> Result<(usize, usize)> {
        let size = self.size;
        let hardness = self.level.as_ref().map_or([0u8; 256], |l| l.hardness);
        let shape = level_shape(shape, size);
        let (centre, reach) = shape_reach(shape);
        let inside = |q: Vec3, scale: f32| shape_contains(shape, q, scale);
        let (mut voxels, mut carved) = (0usize, 0usize);
        let keys: Vec<usize> = self.bodies.keys().copied().filter(|&k| only.is_none_or(|o| o == k)).collect();
        for k in keys {
            // (A body retired for the cap by a part cut off an earlier one in this same carve is no longer there.)
            let Some(b) = self.bodies.get(&k) else { continue };
            let Some((at, rot, _)) = p.bridge_ref().engine_pose(b.body) else { continue };
            let m = Mat4::from_rotation_translation(Quat::from_array(rot), Vec3::from(at));
            let centre_of = |g: [i32; 3]| m.transform_point3((Vec3::new(g[0] as f32, g[1] as f32, g[2] as f32) + 0.5) * size);
            let Some((lo, hi)) = b.grid.bounds() else { continue };
            // (The grid's bounds are its bricks': from `lo` up to `hi`, not through it.)
            let (lo, hi) = (Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32), Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32));
            let (mid, half) = ((lo + hi) * 0.5 * size, ((hi - lo) * 0.5 * size).length());
            if m.transform_point3(mid).distance(centre) > reach + half {
                continue;
            }
            let gone: Vec<([i32; 3], u8)> = b.grid.voxels().filter(|&(g, mat)| { let r = radii[(hardness[mat as usize] & 3) as usize]; r > 0 && inside(centre_of(g), r as f32 / 256.0) }).collect();
            if gone.is_empty() {
                continue;
            }
            voxels += gone.len();
            carved += 1;
            // (What is left of it is not retired for the cap while this carve makes the parts it cut off.)
            self.this_edit.insert(k);
            self.keep(gone.iter().map(|&(g, mat)| (centre_of(g), mat)));
            let b = self.bodies.get_mut(&k).unwrap();
            for &(g, _) in &gone {
                b.grid.take(g[0], g[1], g[2]);
            }
            let mut cleared: Vec<[i32; 3]> = gone.iter().map(|g| g.0).collect();
            // What is left, in six-connected parts: the largest stays this body.
            let mut parts = components(&b.grid);
            parts.sort_by(|x, y| y.len().cmp(&x.len()).then(x[0].cmp(&y[0])));
            let velocity = p.bridge_ref().engine_velocity_of(b.body).unwrap_or(([0.0; 3], [0.0; 3]));
            let mut loose: Vec<Vec<([i32; 3], u8)>> = vec![];
            for part in parts.into_iter().skip(1) {
                let with: Vec<([i32; 3], u8)> = part.iter().map(|&g| (g, b.grid.take(g[0], g[1], g[2]))).collect();
                cleared.extend(part);
                loose.push(with);
            }
            let (collider, left) = (b.collider, b.grid.count());
            if left == 0 {
                self.retire(w, p, k, false, "carved");
            } else {
                let _ = collider;
                self.redraw_body(w, p, k, &cleared, &[], Vec3::from(at), Quat::from_array(rot))?;
            }
            for part in loose {
                if part.len() < MIN_PIECE {
                    self.keep(part.iter().map(|&(g, mat)| (centre_of(g), mat)));
                    continue;
                }
                // Its own grid from its lowest corner, placed where that corner is.
                let lo = [0, 1, 2].map(|a| part.iter().map(|(g, _)| g[a]).min().unwrap());
                let mut grid = VoxelGrid::default();
                for &(g, mat) in &part {
                    grid.set(g[0] - lo[0], g[1] - lo[1], g[2] - lo[2], mat);
                }
                let corner = m.transform_point3(Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) * size);
                let n = self.objects.len().max(self.level.as_ref().map_or(0, |l| l.objects.len()));
                // (The level's next object would take this index: it gets a hole there.)
                if let Some(l) = self.level.as_mut() {
                    l.objects.resize_with(n + 1, || None);
                }
                self.add_body(w, p, n, grid, corner, Quat::from_array(rot), velocity)?;
            }
        }
        Ok((voxels, carved))
    }

    /// A body's mass properties from its cells, each a cube of its palette entry's density (parry's voxel mass is the
    /// same sum at one density): mass, centre of mass and inertia about it in the body's frame, summed in the grid's
    /// fixed order in f64 (the same bits everywhere), then set on the body (its collider carries none).
    fn weigh(&self, p: &mut PhysicsWorld, body: u32, grid: &VoxelGrid) -> std::result::Result<(), String> {
        let s = self.size as f64;
        let v = s * s * s;
        let cell = |m: u8| self.density.get(m as usize).copied().unwrap_or(DENSITY[0]) as f64 * v;
        let (mut mass, mut first) = (0.0f64, [0.0f64; 3]);
        for (g, m) in grid.voxels() {
            let (mm, c) = (cell(m), [0, 1, 2].map(|a| (g[a] as f64 + 0.5) * s));
            mass += mm;
            for a in 0..3 {
                first[a] += mm * c[a];
            }
        }
        if mass <= 0.0 {
            return Err("no cell".into());
        }
        let com = first.map(|f| f / mass);
        // About the centre of mass: each cube's own (m s² / 6 on the diagonal) and the parallel axes' (m (|d|² I - d dᵀ)).
        let mut i = [0.0f64; 6];
        for (g, m) in grid.voxels() {
            let mm = cell(m);
            let d = [0, 1, 2].map(|a| (g[a] as f64 + 0.5) * s - com[a]);
            let (own, dd) = (mm * s * s / 6.0, d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
            i[0] += own + mm * (dd - d[0] * d[0]);
            i[1] += own + mm * (dd - d[1] * d[1]);
            i[2] += own + mm * (dd - d[2] * d[2]);
            i[3] -= mm * d[0] * d[1];
            i[4] -= mm * d[0] * d[2];
            i[5] -= mm * d[1] * d[2];
        }
        p.bridge().engine_mass(body, mass as f32, com.map(|c| c as f32), i.map(|c| c as f32))
    }

    /// Body `k` after its cells changed (`cleared` out of its grid, `filled` in): its collider edited by them, its mesh
    /// made again as new objects in the old ones' place (a driven object given another geometry would rebuild the
    /// scene; one removed and one added are both in place), its mass from its cells.
    fn redraw_body(&mut self, w: &mut World, p: &mut PhysicsWorld, k: usize, cleared: &[[i32; 3]], filled: &[[i32; 3]], at: Vec3, rot: Quat) -> Result<()> {
        let (body, collider) = (self.bodies[&k].body, self.bodies[&k].collider);
        if !(cleared.is_empty() && filled.is_empty()) {
            if let Err(e) = p.bridge().engine_edit_voxels(collider, cleared, filled) {
                eprintln!("voxels: a body's collider was not edited ({e})");
            }
        }
        // (A body whose collider the physics took out is not kept drawn and weighed without it: its voxels are debris.)
        if p.bridge_ref().engine_voxels(collider).is_none() {
            self.retire(w, p, k, true, "carved");
            return Ok(());
        }
        let old = std::mem::take(&mut self.objects[k].as_mut().unwrap().parts);
        for part in old {
            w.remove(part.1);
            w.drop_geometry(part.2);
        }
        let grid = self.bodies[&k].grid.clone();
        let parts = self.body_parts(w, k, &grid, at, rot)?;
        let o = self.objects[k].as_mut().unwrap();
        o.parts = parts;
        o.bounds(&grid, self.size);
        if let Err(e) = self.weigh(p, body, &grid) {
            eprintln!("voxels: a body's mass was not set ({e})");
        }
        Ok(())
    }

    /// Paint reaching bodies (all, or `only` one): of each body the shape meets, the cells on its surface (a face
    /// open to the outside of its grid, as the level's paint) whose centres are inside take `m`. Collider unchanged,
    /// mesh and mass again. Returns the voxels painted and the bodies.
    fn paint_bodies(&mut self, w: &mut World, p: &mut PhysicsWorld, shape: ShapeM, m: u8, only: Option<usize>) -> Result<(usize, usize)> {
        let size = self.size;
        let shape = level_shape(shape, size);
        let (centre, reach) = shape_reach(shape);
        let (mut voxels, mut painted) = (0, 0);
        let keys: Vec<usize> = self.bodies.keys().copied().filter(|&k| only.is_none_or(|o| o == k)).collect();
        for k in keys {
            let b = &self.bodies[&k];
            let Some((at, rot, _)) = p.bridge_ref().engine_pose(b.body) else { continue };
            let pose = Mat4::from_rotation_translation(Quat::from_array(rot), Vec3::from(at));
            if !reaches(&b.grid, pose, size, centre, reach) {
                continue;
            }
            let grid = &b.grid;
            let open = |g: [i32; 3]| DIRS6.iter().any(|d| grid.get(g[0] + d[0], g[1] + d[1], g[2] + d[2]) == 0);
            let these: Vec<[i32; 3]> = grid.voxels().filter(|&(g, mat)| mat != m && open(g) && shape_contains(shape, pose.transform_point3((Vec3::new(g[0] as f32, g[1] as f32, g[2] as f32) + 0.5) * size), 1.0)).map(|(g, _)| g).collect();
            if these.is_empty() {
                continue;
            }
            let b = self.bodies.get_mut(&k).unwrap();
            for g in &these {
                b.grid.set(g[0], g[1], g[2], m);
            }
            voxels += these.len();
            painted += 1;
            self.redraw_body(w, p, k, &[], &[], Vec3::from(at), Quat::from_array(rot))?;
        }
        Ok((voxels, painted))
    }

    /// Voxels added to body `k`: the empty cells of its grid whose centres are inside the shape (world metres, as the
    /// level takes it), at most `MAX_BODY_ADD` of them and 64 times that looked at. Collider, mesh and mass again. Returns the voxels.
    fn add_to_body(&mut self, w: &mut World, p: &mut PhysicsWorld, k: usize, shape: ShapeM, m: u8) -> Result<usize> {
        let size = self.size;
        let shape = level_shape(shape, size);
        let b = &self.bodies[&k];
        let Some((at, rot, _)) = p.bridge_ref().engine_pose(b.body) else { return Ok(0) };
        let pose = Mat4::from_rotation_translation(Quat::from_array(rot), Vec3::from(at));
        // The shape's world box, into the grid's cells (through the inverse pose: the eight corners' bounds).
        let (lo, hi) = shape_box(shape);
        let inv = pose.inverse();
        let corners = (0..8).map(|c| inv.transform_point3(Vec3::new([lo.x, hi.x][c & 1], [lo.y, hi.y][(c >> 1) & 1], [lo.z, hi.z][c >> 2])) / size);
        let (mut a, mut z) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for c in corners {
            (a, z) = (a.min(c), z.max(c));
        }
        if !(a.is_finite() && z.is_finite()) {
            return Err(anyhow!("voxelEdit: the shape is not finite"));
        }
        let (ga, gz) = (a.floor().as_ivec3(), z.ceil().as_ivec3());
        // (Its cells are looked at one by one: at most 64 times what may be added, 2^22, a box of 16 m at 0.1 m.)
        // (In f64: three sides of up to 2^32 cells overflow an i64.)
        let n = (0..3).map(|i| (gz[i] as f64 - ga[i] as f64 + 1.0).max(0.0)).product::<f64>();
        if n > ((MAX_BODY_ADD as u64) << 6) as f64 {
            return Err(anyhow!("voxelBodyEdit: the shape is too large for an add on a body (at most {} cells across its bounds in the body's grid)", MAX_BODY_ADD << 6));
        }
        let mut filled = vec![];
        for x in ga.x..=gz.x {
            for y in ga.y..=gz.y {
                for z in ga.z..=gz.z {
                    if b.grid.get(x, y, z) == 0 && shape_contains(shape, pose.transform_point3((Vec3::new(x as f32, y as f32, z as f32) + 0.5) * size), 1.0) {
                        filled.push([x, y, z]);
                        if filled.len() > MAX_BODY_ADD {
                            return Err(anyhow!("voxelBodyEdit: at most {MAX_BODY_ADD} voxels can be added to a body at once"));
                        }
                    }
                }
            }
        }
        if filled.is_empty() {
            return Ok(0);
        }
        // What the body would be: refused before anything changes if it is too large, or its cells too far apart.
        if b.grid.count() + filled.len() > MAX_BODY_CELLS {
            return Err(anyhow!("voxelBodyEdit: a body grows to at most {MAX_BODY_CELLS} voxels"));
        }
        let (mut lo, mut hi) = b.grid.bounds().map_or(([i32::MAX; 3], [i32::MIN; 3]), |(lo, hi)| (lo, hi.map(|v| v - 1)));
        for g in &filled {
            for a in 0..3 {
                (lo[a], hi[a]) = (lo[a].min(g[a]), hi[a].max(g[a]));
            }
        }
        if !body_fits(lo, hi) {
            return Err(anyhow!("voxelBodyEdit: a body's voxels span at most {MAX_BODY_SIDE} cells a side and {MAX_BODY_BOX} in all"));
        }
        // The collider first: if the physics refuses the cells, nothing changes.
        let collider = b.collider;
        if let Err(e) = p.bridge().engine_edit_voxels(collider, &[], &filled) {
            if p.bridge_ref().engine_voxels(collider).is_none() {
                self.retire(w, p, k, true, "carved");
            }
            return Err(anyhow!("voxelBodyEdit: {e}"));
        }
        let b = self.bodies.get_mut(&k).unwrap();
        for g in &filled {
            b.grid.set(g[0], g[1], g[2], m);
        }
        self.redraw_body(w, p, k, &[], &[], Vec3::from(at), Quat::from_array(rot))?;
        Ok(filled.len())
    }

    /// An edit of one voxel body alone (scene code's `voxelBodyEdit`: a voxel object's `add`, `paint`, `carve`): the
    /// shape in world metres; `explode` and `carve` cut it (parts cut off are bodies of their own), `paint` its
    /// surface, `add` fills its empty cells in the shape.
    pub fn edit_body(&mut self, w: &mut World, ph: &mut Option<PhysicsWorld>, body: u32, op: &EditOp) -> Result<serde_json::Value> {
        let k = self.bodies.iter().find(|(_, b)| b.body == body).map(|(&k, _)| k).ok_or_else(|| anyhow!("voxelBodyEdit: no voxel body {body}"))?;
        let p = ph.as_mut().ok_or_else(|| anyhow!("voxelBodyEdit: no physics world"))?;
        if let EditKind::Add(m) | EditKind::Paint(m) = op.kind {
            if m as usize >= self.palette.len() {
                return Err(anyhow!("voxelBodyEdit: material {m} is not in the level's palette ({} entries)", self.palette.len()));
            }
        }
        self.ticks.0 += 1;
        self.this_edit.clear();
        let (voxels, n) = match op.kind {
            EditKind::Carve(radii) => self.carve_bodies(w, p, op.shape, radii, Some(k))?,
            EditKind::Paint(m) => self.paint_bodies(w, p, op.shape, m, Some(k))?,
            EditKind::Add(m) => {
                let n = self.add_to_body(w, p, k, op.shape, m)?;
                (n, (n > 0) as usize)
            }
        };
        Ok(serde_json::json!({ "voxels": voxels, "bodies": n, "gone": !self.bodies.contains_key(&k) }))
    }

    /// A body's contacts reported to scene code or not (its collider's events: `__engine.physics("contacts")`).
    pub fn body_contacts(&mut self, ph: &mut Option<PhysicsWorld>, body: u32, on: bool) -> Result<()> {
        let b = self.bodies.values().find(|b| b.body == body).ok_or_else(|| anyhow!("voxelContacts: no voxel body {body}"))?;
        let p = ph.as_mut().ok_or_else(|| anyhow!("voxelContacts: no physics world"))?;
        p.bridge().engine_events(b.collider, on).map_err(|e| anyhow!(e))
    }

    /// The bodies gone since the last call, with why: [[body, "cap" | "carved" | "fell"], ...].
    pub fn take_gone(&mut self) -> serde_json::Value {
        serde_json::Value::Array(std::mem::take(&mut self.gone).into_iter().map(|(b, r)| serde_json::json!([b, r])).collect())
    }

    /// Voxel bodies now.
    pub fn bodies(&self) -> usize {
        self.bodies.len()
    }

    /// The bodies as scene code may name them: the physics world's body id of each (`__engine.physics` calls take
    /// it), its voxels, whether it sleeps, and where it is.
    pub fn body_list(&self, p: &PhysicsWorld) -> serde_json::Value {
        serde_json::Value::Array(self.bodies.values().filter_map(|b| p.bridge_ref().engine_pose(b.body).map(|(at, rot, asleep)| serde_json::json!({ "body": b.body, "collider": b.collider, "voxels": b.grid.count(), "asleep": asleep, "position": at, "rotation": rot }))).collect())
    }

    /// Scene code's `voxelDebris`: the debris kept since the last call (x, y, z, palette index per voxel).
    pub fn take_debris(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.debris)
    }

    /// Scene code's `voxelFrame`, once a frame: the level ticked for the time that passed (60 ticks a second, at most
    /// `MAX_TICKS` a call), the falling objects written again where they are now, the landed ones where they rest.
    pub fn frame(&mut self, w: &mut World, dt: f64) -> Result<serde_json::Value> {
        if self.route.is_none() && self.pending.is_some() {
            self.queue(Queued::Frame(dt))?;
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
                    w.drop_geometry(p.2);
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
        for p in &old {
            if !geometry.iter().any(|g| g.0 == p.0) {
                w.remove(p.1);
            }
        }
        // Its old geometries are let go, unless another object (of the same file) still draws them.
        for p in old {
            if !geometry.iter().any(|g| g.1 == p.2) {
                w.drop_geometry(p.2);
            }
        }
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
        let mut h = self.level.as_ref()?.hash();
        // The bodies' voxels (where the bodies are is the physics world's hash): in their order, each grid's cells
        // in the grid's.
        for (&k, b) in &self.bodies {
            for x in [k as u32, b.grid.count() as u32].into_iter().chain(b.grid.voxels().flat_map(|(g, m)| [g[0] as u32, g[1] as u32, g[2] as u32, m as u32])) {
                for byte in x.to_le_bytes() {
                    h = (h ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        Some(h)
    }
}

/// Where a level object's collider stands, its cells being the object's own: the rotation of its quarter turns about
/// y and the translation that puts the cube of its cell g on the cube of world cell `place.to_world(g)` (a turned
/// cube's lowest corner is not the turned corner: `voxel_core::Placement`).
/// Whether cells spanning `lo` ..= `hi` fit a voxel body (`MAX_BODY_SIDE`, `MAX_BODY_BOX`).
fn body_fits(lo: [i32; 3], hi: [i32; 3]) -> bool {
    let side = |a: usize| hi[a] as i64 - lo[a] as i64 + 1;
    (0..3).all(|a| (1..=MAX_BODY_SIDE as i64).contains(&side(a))) && side(0) * side(1) * side(2) <= MAX_BODY_BOX
}

/// The six face neighbours.
const DIRS6: [[i32; 3]; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];

/// A shape as the level's edits take it (voxel_core `Shape::checked`): a radius within 0 and `MAX_RADIUS` voxels (NaN:
/// 0), a box as given.
fn level_shape(shape: ShapeM, size: f32) -> ShapeM {
    let cap = voxel_core::MAX_RADIUS as f32 * size;
    let r = |r: f32| if r.is_nan() { 0.0 } else { r.clamp(0.0, cap) };
    match shape {
        ShapeM::Sphere(c, x) => ShapeM::Sphere(c, r(x)),
        ShapeM::Capsule(a, b, x) => ShapeM::Capsule(a, b, r(x)),
        other => other,
    }
}

/// Whether a point (world metres) is inside the shape, its radius scaled by `scale` (a class's share; a box whole).
fn shape_contains(shape: ShapeM, q: Vec3, scale: f32) -> bool {
    match shape {
        ShapeM::Sphere(c, r) => q.distance_squared(Vec3::from(c)) <= (r * scale) * (r * scale),
        ShapeM::Box(a, b) => {
            let (c, h) = ((Vec3::from(a) + Vec3::from(b)) * 0.5, (Vec3::from(b) - Vec3::from(a)).abs() * 0.5);
            (q - c).abs().cmple(h).all()
        }
        ShapeM::Capsule(a, b, r) => {
            let (a, ab) = (Vec3::from(a), Vec3::from(b) - Vec3::from(a));
            let t = if ab.length_squared() > 0.0 { ((q - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            q.distance_squared(a + ab * t) <= (r * scale) * (r * scale)
        }
    }
}

/// A sphere about the shape: its centre and radius.
fn shape_reach(shape: ShapeM) -> (Vec3, f32) {
    match shape {
        ShapeM::Sphere(c, r) => (Vec3::from(c), r),
        ShapeM::Box(a, b) => ((Vec3::from(a) + Vec3::from(b)) * 0.5, (Vec3::from(b) - Vec3::from(a)).length() * 0.5),
        ShapeM::Capsule(a, b, r) => ((Vec3::from(a) + Vec3::from(b)) * 0.5, (Vec3::from(b) - Vec3::from(a)).length() * 0.5 + r),
    }
}

/// The shape's world box.
fn shape_box(shape: ShapeM) -> (Vec3, Vec3) {
    match shape {
        ShapeM::Sphere(c, r) => (Vec3::from(c) - r, Vec3::from(c) + r),
        ShapeM::Box(a, b) => (Vec3::from(a).min(Vec3::from(b)), Vec3::from(a).max(Vec3::from(b))),
        ShapeM::Capsule(a, b, r) => (Vec3::from(a).min(Vec3::from(b)) - r, Vec3::from(a).max(Vec3::from(b)) + r),
    }
}

/// Whether a body's grid at its pose can reach the sphere about a shape (its bricks' bounding sphere against it).
fn reaches(grid: &VoxelGrid, pose: Mat4, size: f32, centre: Vec3, reach: f32) -> bool {
    let Some((lo, hi)) = grid.bounds() else { return false };
    // (The grid's bounds are its bricks': from `lo` up to `hi`, not through it.)
    let (lo, hi) = (Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32), Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32));
    let (mid, half) = ((lo + hi) * 0.5 * size, ((hi - lo) * 0.5 * size).length());
    pose.transform_point3(mid).distance(centre) <= reach + half
}

fn collider_pose(place: &Placement, size: f32) -> ([f32; 3], [f32; 4]) {
    const S: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let (rotation, corner) = match place.turn & 3 {
        0 => ([0.0, 0.0, 0.0, 1.0], [0, 0, 0]),
        1 => ([0.0, S, 0.0, S], [0, 0, -1]),
        2 => ([0.0, 1.0, 0.0, 0.0], [-1, 0, -1]),
        _ => ([0.0, S, 0.0, -S], [-1, 0, 0]),
    };
    ([0, 1, 2].map(|a| (place.t[a] - corner[a]) as f32 * size), rotation)
}

/// A grid's solid cells in six-connected parts, each part's cells in the grid's order (so the parts and their order
/// are the same on every machine).
fn components(grid: &VoxelGrid) -> Vec<Vec<[i32; 3]>> {
    let cells: Vec<[i32; 3]> = grid.voxels().map(|(g, _)| g).collect();
    let index: HashMap<[i32; 3], usize> = cells.iter().enumerate().map(|(i, &c)| (c, i)).collect();
    let mut part = vec![usize::MAX; cells.len()];
    let mut out: Vec<Vec<[i32; 3]>> = vec![];
    for start in 0..cells.len() {
        if part[start] != usize::MAX {
            continue;
        }
        let id = out.len();
        let mut members = vec![start];
        part[start] = id;
        let mut next = 0;
        while next < members.len() {
            let c = cells[members[next]];
            next += 1;
            for d in [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]] {
                if let Some(&j) = index.get(&[c[0] + d[0], c[1] + d[1], c[2] + d[2]]) {
                    if part[j] == usize::MAX {
                        part[j] = id;
                        members.push(j);
                    }
                }
            }
        }
        members.sort_unstable();
        out.push(members.into_iter().map(|i| cells[i]).collect());
    }
    out
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
        level_with(w, &mut None, false)
    }

    /// The same with the level's physics on (`ph` gets the world) or off (as before step 8: parts fall through the
    /// lattice).
    fn level_with(w: &mut World, ph: &mut Option<PhysicsWorld>, physics: bool) -> VoxelWorld {
        let mut v = VoxelWorld::default();
        let d: LevelDesc = serde_json::from_value(serde_json::json!({
            "route": "mesh", "debris": true, "occupancy": 0.2, "physics": physics,
            "palette": [{ "name": "none", "color": [0, 0, 0], "roughness": 1, "metalness": 0 }, { "name": "black", "color": [0, 0, 0], "roughness": 1, "metalness": 0 },
                        { "name": "post", "color": [1, 1, 1], "roughness": 1, "metalness": 0, "hardness": "hard" }],
            "objects": [{ "name": "slab", "uri": "tests/render/data/voxel-slab.tpg", "position": [0, 0, 0], "anchor": true },
                        { "name": "post", "uri": "tests/render/data/voxel-post.tpg", "position": [0.0, 0.4, 0.0] },
                        { "name": "post-above", "uri": "tests/render/data/voxel-post.tpg", "position": [0.0, 2.0, 0.0], "turn": 1 }],
        }))
        .unwrap();
        v.request(d).unwrap();
        let r = v.realize(w, ph, &assets()).unwrap().unwrap();
        assert_eq!(r["colliders"].as_u64(), Some(if physics { 3 } else { 0 }), "{r}");
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
            let e = v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "carve", "box": [-0.5, 0.8, -0.5, 0.5, 1.0, 0.5] }))).unwrap();
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
            let e = v.explode(&mut w, &mut None, Vec3::new(0.0, 0.6, 0.0), 0.5).unwrap();
            assert!(e["voxels"].as_u64().unwrap() > 100, "{e}");
            // (It took the stump's foot: what stood on it falls into the crater. Edits reach static voxels only: let it
            // land.)
            for _ in 0..60 {
                v.frame(&mut w, 1.0 / 60.0).unwrap();
            }
            assert_eq!(v.frame(&mut w, 1.0 / 60.0).unwrap()["batches"], 0);
            // Paint and added voxels.
            let e = v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "paint", "sphere": [0.0, 2.0, 0.0, 0.4], "material": 1 }))).unwrap();
            assert!(e["voxels"].as_u64().unwrap() > 0 && e["pieces"] == 0, "{e}");
            let e = v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "add", "box": [0.5, 0.4, 0.5, 0.7, 0.6, 0.7], "material": 2 }))).unwrap();
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

    /// Step 8: the level collides and what comes loose is a rigid body. The level's objects are fixed voxel colliders
    /// (a physics ray meets the slab and the posts); the cut that dropped the upper part through the lattice now
    /// makes it a body, which lands on the stump 0.2 m lower, rests and sleeps, its objects where it is; an
    /// explosion under it carves the colliders it stood on and it falls on; a carve through the body itself cuts it
    /// in two bodies. The same commands twice end in the same voxels and the same physics bits.
    #[test]
    fn the_level_collides_and_loose_parts_are_bodies() {
        use physics_core::bridge::ENGINE_IDS;
        let run = || {
            let mut w = World::default();
            let mut ph: Option<PhysicsWorld> = None;
            let mut v = level_with(&mut w, &mut ph, true);
            assert!(ph.is_some(), "the level made the physics world");
            let call = |ph: &mut Option<PhysicsWorld>, w: &mut World, op: &str, a: serde_json::Value| ph.as_mut().unwrap().call(w, op, &a);
            let step = |v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>, frames: u32| {
                for _ in 0..frames {
                    assert!(crate::api::physics_advance(ph, v, w, 1.0 / 60.0));
                    assert_eq!(v.frame(w, 1.0 / 60.0).unwrap()["batches"], 0, "nothing falls through the lattice");
                }
            };
            // A ray down beside the posts meets the slab's top (y = 0.4); over them, the upper post's (y = 3.6).
            let hit = call(&mut ph, &mut w, "ray", serde_json::json!({ "origin": [0.6, 5.0, 0.6], "direction": [0, -1, 0], "max": 10 })).unwrap();
            assert!(hit["collider"].as_u64().unwrap() >= ENGINE_IDS as u64 && (hit["point"][1].as_f64().unwrap() - 0.4).abs() < 1e-4, "{hit}");
            let hit = call(&mut ph, &mut w, "ray", serde_json::json!({ "origin": [0.0, 5.0, 0.0], "direction": [0, -1, 0], "max": 10 })).unwrap();
            assert!((hit["point"][1].as_f64().unwrap() - 3.6).abs() < 1e-4, "{hit}");
            // The cut.
            let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [-0.5, 0.8, -0.5, 0.5, 1.0, 0.5] }))).unwrap();
            assert_eq!((e["voxels"].as_u64(), e["pieces"].as_u64(), e["piece_voxels"].as_u64(), e["bodies"].as_u64()), (Some(32), Some(1), Some(16 * (10 + 16)), Some(1)), "{e}");
            assert_eq!((v.bodies(), v.level.as_ref().unwrap().falling()), (1, 0));
            let part = *v.bodies.keys().next().unwrap();
            let objects: Vec<u32> = v.objects[part].as_ref().unwrap().parts.iter().map(|p| p.1).collect();
            assert!(!objects.is_empty() && objects.iter().all(|id| w.objects[id].driven));
            let at0 = w.objects[&objects[0]].position;
            let y0 = at0.y;
            assert!((at0 - Vec3::new(-0.2, 1.0, -0.2)).length() < 1e-5, "the part's lowest corner is the cut's top: {at0}");
            let list = v.body_list(ph.as_ref().unwrap());
            let body = list[0]["body"].as_u64().unwrap();
            assert_eq!((list.as_array().unwrap().len(), list[0]["voxels"].as_u64(), list[0]["asleep"].as_bool()), (1, Some(416), Some(false)), "{list}");
            // 416 voxels of a hard entry: 2,400 kg/m³ x 0.001 m³ each.
            let state = call(&mut ph, &mut w, "state", serde_json::json!({ "body": body })).unwrap();
            assert!((state["mass"].as_f64().unwrap() - 416.0 * 2.4).abs() < 0.05, "{state}");
            step(&mut v, &mut w, &mut ph, 180);
            let at = w.objects[&objects[0]].position;
            assert!((y0 - at.y - 0.2).abs() < 5e-3 && (at.x - at0.x).abs() < 5e-3 && (at.z - at0.z).abs() < 5e-3, "it fell {} m to {at:?}", y0 - at.y);
            assert!(v.body_list(ph.as_ref().unwrap())[0]["asleep"] == true);
            // Scene code names the body in its calls, and cannot remove it or make one in the engine's ids.
            assert!(call(&mut ph, &mut w, "remove", serde_json::json!({ "body": body })).is_err());
            assert!(call(&mut ph, &mut w, "body", serde_json::json!({ "id": body + 100 })).is_err());
            call(&mut ph, &mut w, "impulse", serde_json::json!({ "body": body, "impulse": [0, 5, 0] })).unwrap();
            step(&mut v, &mut w, &mut ph, 120);
            // The stump and the slab under it carved away: the colliders lose those cells and the part falls on, to the
            // crater's floor (the posts are hard: a cut through everything takes them).
            let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [-0.5, 0.2, -0.5, 0.5, 0.8, 0.5] }))).unwrap();
            assert!(e["voxels"].as_u64().unwrap() >= 16 * 4 && e["pieces"] == 0, "{e}");
            step(&mut v, &mut w, &mut ph, 180);
            let at = w.objects[&objects[0]].position;
            assert!((at.y - 0.2).abs() < 5e-3, "on the slab's carved floor: {at:?}");
            let hit = call(&mut ph, &mut w, "ray", serde_json::json!({ "origin": [0.05, 0.75, 5.0], "direction": [0, 0, -1], "max": 10 })).unwrap();
            assert_eq!(hit["body"].as_u64(), Some(body), "a ray from the side meets the part: {hit}");
            // A cut through the body, 0.2 m thick at its middle: two bodies, each with its own objects; the first
            // one's objects are new ones (its mesh changed).
            let before = v.bodies[&part].grid.count();
            let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [-0.5, 1.4, -0.5, 0.5, 1.6, 0.5] }))).unwrap();
            assert_eq!((e["bodies_carved"].as_u64(), e["body_voxels"].as_u64(), e["bodies"].as_u64()), (Some(1), Some(32), Some(2)), "{e}");
            let cells: usize = v.bodies.values().map(|b| b.grid.count()).sum();
            assert_eq!(cells + 32, before);
            assert!(v.objects[part].as_ref().unwrap().parts.iter().all(|p| !objects.contains(&p.1) && w.objects[&p.1].driven));
            assert!(objects.iter().all(|id| !w.objects.contains_key(id)));
            step(&mut v, &mut w, &mut ph, 240);
            let list = v.body_list(ph.as_ref().unwrap());
            assert!(list.as_array().unwrap().iter().all(|b| b["asleep"] == true), "{list}");
            // The upper one came down the 0.2 m of the cut onto the lower.
            let top = v.bodies.keys().copied().max().unwrap();
            let y = w.objects[&v.objects[top].as_ref().unwrap().parts[0].1].position.y;
            assert!((y - (0.2 + 1.2)).abs() < 1e-2, "{y}");
            // A carve that takes a body whole: gone, with its objects.
            let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [-1.0, 1.3, -1.0, 1.0, 9.0, 1.0] }))).unwrap();
            assert_eq!(e["bodies"].as_u64(), Some(1), "{e}");
            assert!(v.objects[top].is_none());
            (v.hash().unwrap(), ph.as_ref().unwrap().hash(), v.take_debris().len())
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert!(a.2 > 0);
    }

    /// The solid world cells of a level within `SHELL` steps of one that is not (under the bedrock, y < 0, counts as
    /// solid when `bedrock`): what the colliders must hold, by brute force.
    fn shell_by_brute_force(v: &VoxelWorld, bedrock: bool) -> usize {
        let level = v.level.as_ref().unwrap();
        let solid: std::collections::HashSet<[i32; 3]> = level.objects.iter().flatten().flat_map(|o| o.grid.voxels().map(|(g, _)| o.place.to_world(g))).collect();
        let is = |c: [i32; 3]| solid.contains(&c) || (bedrock && c[1] < 0);
        solid
            .iter()
            .filter(|c| {
                (-SHELL..=SHELL).any(|dx| (-SHELL + dx.abs()..=SHELL - dx.abs()).any(|dy| { let r = SHELL - dx.abs() - dy.abs(); (-r..=r).any(|dz| !is([c[0] + dx, c[1] + dy, c[2] + dz])) }))
            })
            .count()
    }

    fn collider_cells(v: &VoxelWorld, ph: &Option<PhysicsWorld>) -> usize {
        v.colliders.iter().flatten().map(|&c| ph.as_ref().unwrap().bridge_ref().engine_voxels(c).unwrap_or(0)).sum()
    }

    /// A level object's collider is the shell of its cells near a surface, not its volume, and edits keep it so: a
    /// block of 32³ cells has the four layers under its faces; a pit carved into it brings the pit's walls and floor
    /// in (a ray finds the floor, which no collider held before), and what the colliders hold is what brute force
    /// says, before and after. Terrain tiles side by side cover each other's sides, down to the bedrock, until one is
    /// carved at the edge they share.
    #[test]
    fn a_collider_is_the_shell_of_its_cells() {
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::of_blocks(&[([0, 0, 0], [32, 32, 32], None, true)], &mut w, &mut ph);
        assert_eq!(collider_cells(&v, &ph), 32 * 32 * 32 - 24 * 24 * 24);
        assert_eq!(collider_cells(&v, &ph), shell_by_brute_force(&v, false));
        let ray = |ph: &mut Option<PhysicsWorld>, w: &mut World, x: f32, z: f32| ph.as_mut().unwrap().call(w, "ray", &serde_json::json!({ "origin": [x, 9.0, z], "direction": [0, -1, 0], "max": 20 })).unwrap()["point"][1].as_f64().unwrap();
        assert!((ray(&mut ph, &mut w, 1.65, 1.65) - 3.2).abs() < 1e-4);
        // A pit 0.8 m wide, 1.2 m deep, in the middle of the top.
        let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [1.2, 2.0, 1.2, 2.0, 3.3, 2.0] }))).unwrap();
        assert_eq!(e["voxels"].as_u64(), Some(8 * 8 * 12), "{e}");
        assert_eq!(collider_cells(&v, &ph), shell_by_brute_force(&v, false));
        assert!((ray(&mut ph, &mut w, 1.65, 1.65) - 2.0).abs() < 1e-4, "the pit's floor");
        // Deeper, through cells no collider held, and out through the bottom: a shaft.
        v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [1.4, -0.1, 1.4, 1.8, 2.1, 1.8] }))).unwrap();
        assert_eq!(collider_cells(&v, &ph), shell_by_brute_force(&v, false));
        // Two terrain tiles (64 x 64 cells, 16 deep) side by side on the bedrock: the top four layers and the outer
        // sides; nothing of the side they share, nor of their undersides.
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::of_blocks(&[([0, 0, 0], [64, 16, 64], Some((0, 0)), true), ([64, 0, 0], [64, 16, 64], Some((1, 0)), true)], &mut w, &mut ph);
        assert_eq!(collider_cells(&v, &ph), shell_by_brute_force(&v, true));
        assert_eq!(collider_cells(&v, &ph), 128 * 64 * 16 - 120 * 56 * 12);
        // A trench in the second tile along the edge they share, six cells deep: the first tile's side is bare there,
        // and its collider has those cells now.
        let before = ph.as_ref().unwrap().bridge_ref().engine_voxels(v.colliders[0].unwrap()).unwrap();
        v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [6.4, 1.0, 1.0, 6.8, 1.7, 5.0] }))).unwrap();
        assert_eq!(collider_cells(&v, &ph), shell_by_brute_force(&v, true));
        assert!(ph.as_ref().unwrap().bridge_ref().engine_voxels(v.colliders[0].unwrap()).unwrap() > before, "the first tile's side");
        // A ray along the trench's floor from inside the second tile meets the first tile's bared side.
        let hit = ph.as_mut().unwrap().call(&mut w, "ray", &serde_json::json!({ "origin": [6.75, 1.25, 3.05], "direction": [-1, 0, 0], "max": 5 })).unwrap();
        assert!((hit["point"][0].as_f64().unwrap() - 6.4).abs() < 1e-4 && hit["collider"].as_u64() == v.colliders[0].map(|c| c as u64), "{hit}");
        // A crater that crosses the border by one cell (the review of #24): the second tile loses a few cells of its
        // own, and its shell must still be made whole over all the first tile's crater bared. (It was not: rays
        // across the crater went through the whole second tile, to its far side.)
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::of_blocks(&[([0, 0, 0], [64, 64, 64], Some((0, 0)), true), ([64, 0, 0], [64, 64, 64], Some((1, 0)), true)], &mut w, &mut ph);
        let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "sphere": [0.47, 3.05, 3.05, 6.0] }))).unwrap();
        assert!(e["touched"].as_u64() == Some(2), "both tiles lose cells: {e}");
        assert_eq!(collider_cells(&v, &ph), shell_by_brute_force(&v, true));
        for (y, z, wall) in [(3.05, 3.05, 0.5), (4.05, 3.05, 0.4), (4.15, 3.05, 0.4), (3.05, 4.05, 0.4), (2.05, 3.05, 0.4)] {
            let hit = ph.as_mut().unwrap().call(&mut w, "ray", &serde_json::json!({ "origin": [6.0, y, z], "direction": [1, 0, 0], "max": 50 })).unwrap();
            assert!((hit["distance"].as_f64().unwrap() - wall).abs() < 1e-3, "a ray along +x from (6.0, {y}, {z}) meets the crater's wall {wall} m away: {hit}");
        }
    }

    /// A collider's pose puts every cell of the object where the level has it, whatever its turn.
    #[test]
    fn a_colliders_pose_is_the_objects_placement() {
        for turn in 0..4 {
            let place = Placement::new([7, -3, 12], turn, [2, 0, -5]);
            let (at, rot) = collider_pose(&place, 0.1);
            let m = Mat4::from_rotation_translation(Quat::from_array(rot), Vec3::from(at));
            for g in [[0, 0, 0], [1, 0, 0], [0, 0, 1], [5, 9, -4], [-3, 2, 6]] {
                let w = place.to_world(g);
                let centre = m.transform_point3((Vec3::new(g[0] as f32, g[1] as f32, g[2] as f32) + 0.5) * 0.1);
                assert!((centre - (Vec3::new(w[0] as f32, w[1] as f32, w[2] as f32) + 0.5) * 0.1).length() < 1e-5, "turn {turn}, cell {g:?}: {centre} for world cell {w:?}");
            }
        }
    }

    /// The voxel bodies' script ends in the recorded bits (tools/physics-determinism.ts: in wasm32 too), and in it a
    /// tower toppled, was cut in bodies and they came to rest.
    #[test]
    fn voxel_bodies_step_to_the_recorded_bits() {
        assert_eq!(format!("{:016x}", VoxelWorld::check_script()), format!("{:016x}", EXPECTED_SCRIPT));
    }

    /// tests/render/voxel-cut-body.ts on the CPU: the cut is asked for in setup, before the level is loaded, so it is
    /// made at the load, where the part it frees becomes a body; an offline render then settles the physics to its
    /// scene time (2 s), and the part stands on the stump. And what scene code gets back from a ray is enough to act
    /// on the body: the hit's `body` is the id the physics calls take (an impulse through it moves the part).
    #[test]
    fn a_cut_made_in_setup_frees_a_body_at_the_load_and_a_rays_hit_names_it() {
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::default();
        let d: LevelDesc = serde_json::from_value(serde_json::json!({
            "route": "mesh",
            "palette": [{ "name": "none", "color": [0, 0, 0], "roughness": 1, "metalness": 0 }, { "name": "black", "color": [0, 0, 0], "roughness": 1, "metalness": 0 },
                        { "name": "glow", "color": [1, 1, 1], "roughness": 1, "metalness": 0, "emissive": 5 }],
            "objects": [{ "name": "slab", "uri": "tests/render/data/voxel-slab.tpg", "position": [0, 0, 0], "anchor": true },
                        { "name": "post", "uri": "tests/render/data/voxel-post.tpg", "position": [0.0, 0.4, 0.0] },
                        { "name": "post-above", "uri": "tests/render/data/voxel-post.tpg", "position": [0.0, 2.0, 0.0], "turn": 1 }],
        }))
        .unwrap();
        v.request(d).unwrap();
        assert_eq!(v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [-0.5, 0.8, -0.5, 0.5, 1.0, 0.5] }))).unwrap()["queued"], true);
        // (Ticks asked for in setup after the cut are the lattice's: on a level that collides they move nothing. The
        // body is where the cut left it until the engine steps the physics world.)
        for _ in 0..60 {
            assert_eq!(v.frame(&mut w, 1.0 / 60.0).unwrap()["queued"], true);
        }
        let r = v.realize(&mut w, &mut ph, &assets()).unwrap().unwrap();
        assert_eq!((r["explosions"].as_u64(), r["frames"].as_u64(), v.bodies()), (Some(1), Some(60), 1), "{r}");
        let spawned = w.objects[&v.objects[*v.bodies.keys().next().unwrap()].as_ref().unwrap().parts[0].1].position;
        assert!((spawned - Vec3::new(-0.2, 1.0, -0.2)).length() < 1e-4, "{spawned}");
        // (The two posts are one file: one shape between their colliders until the cut gave the lower one its own.)
        assert_eq!(r["colliders"].as_u64(), Some(3));
        crate::api::physics_settle(&mut ph, &mut v, &mut w, 120);
        let part = *v.bodies.keys().next().unwrap();
        let at = w.objects[&v.objects[part].as_ref().unwrap().parts[0].1].position;
        assert!((at - Vec3::new(-0.2, 0.8, -0.2)).length() < 5e-3, "{at}");
        // A ray from the side at the part: its hit's body, straight into an impulse.
        let p = ph.as_mut().unwrap();
        let hit = p.call(&mut w, "ray", &serde_json::json!({ "origin": [0.05, 2.05, 5.0], "direction": [0, 0, -1], "max": 10 })).unwrap();
        let body = hit["body"].as_u64().expect("the ray meets the part");
        assert_eq!(Some(body), v.body_list(p)[0]["body"].as_u64());
        p.call(&mut w, "impulse", &serde_json::json!({ "body": body, "impulse": [0, 2000, 0] })).unwrap();
        let up = p.call(&mut w, "state", &serde_json::json!({ "body": body })).unwrap()["linvel"][1].as_f64().unwrap();
        assert!((up - 2000.0 / (416.0 * 0.9)).abs() < 0.05, "416 voxels of a soft entry (900 kg/m³) take 2,000 N s as {up} m/s");
        for _ in 0..12 {
            crate::api::physics_advance(&mut ph, &mut v, &mut w, 1.0 / 60.0);
        }
        assert!(w.objects[&v.objects[part].as_ref().unwrap().parts[0].1].position.y > 1.2, "the part went up with it");
    }

    /// A carve that cuts bodies at the cap (the review of #24, and before it the benchmark level's `shatter` with a cap
    /// of 128, which ended the renderer when a body retired mid-carve was looked up: gpu-runs-8.md L7). What the carve
    /// is cutting is not retired for its own chips: every body keeps its id and the larger part, the chips are debris.
    /// And what goes at the cap otherwise: the least of the rest (asleep, then fewest voxels, then oldest), and only
    /// for a part at least as large.
    #[test]
    fn a_carve_that_passes_the_cap_while_cutting_bodies() {
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        // A slab and six bars standing on it in a row, each cut loose in turn: six bodies; the cap is six. Beside
        // them a thick column and a thin post, cut loose at the end.
        let mut blocks = vec![([0, 0, 0], [64, 4, 64], Some((0, 0)), true)];
        blocks.extend((0..6).map(|k| ([8 + 8 * k, 4, 8], [4, 24, 4], None, false)));
        blocks.extend([([8, 4, 40], [8, 30, 8], None, false), ([40, 4, 40], [2, 8, 2], None, false)]);
        let mut v = VoxelWorld::of_blocks(&blocks, &mut w, &mut ph);
        v.max_bodies = 6;
        v.keep_debris = true;
        let cut = |v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>, lo: [f32; 3], hi: [f32; 3]| v.edit(w, ph, &EditOp { shape: ShapeM::Box(lo, hi), kind: EditKind::Carve([256; 4]) }).unwrap();
        for k in 0..6 {
            let x = 0.8 + 0.8 * k as f32;
            cut(&mut v, &mut w, &mut ph, [x - 0.1, 0.5, 0.7], [x + 0.5, 0.7, 1.3]);
        }
        assert_eq!(v.bodies(), 6);
        let settle = |v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>| {
            for _ in 0..180 {
                crate::api::physics_advance(ph, v, w, 1.0 / 60.0);
            }
        };
        settle(&mut v, &mut w, &mut ph);
        let ids: Vec<usize> = v.bodies.keys().copied().collect();
        // Cuts across all six at once: each body in two, the larger part kept as the body, the chip debris.
        let mut carved = 0;
        for y in [1.0, 1.6, 2.2] {
            let e = cut(&mut v, &mut w, &mut ph, [0.0, y, 0.0], [6.4, y + 0.1, 3.0]);
            carved += e["bodies_carved"].as_u64().unwrap();
            assert!(v.bodies() <= 6, "{} bodies", v.bodies());
        }
        assert!(carved >= 12, "{carved} bodies carved");
        assert_eq!(v.bodies.keys().copied().collect::<Vec<_>>(), ids, "every body is the one it was");
        assert_eq!(v.retired, 0, "none retired for a chip");
        assert!(v.capped >= 12, "{} chips debris", v.capped);
        assert!(v.bodies.values().all(|b| b.grid.count() < 336 && b.grid.count() >= 64), "{:?}", v.bodies.values().map(|b| b.grid.count()).collect::<Vec<_>>());
        // Every body there is has its objects, and no object is left of one that went.
        let parts: usize = v.bodies.keys().map(|&k| v.objects[k].as_ref().unwrap().parts.len()).sum();
        assert_eq!(v.world_objects(), 1 + 6 + 2 + parts);
        settle(&mut v, &mut w, &mut ph);
        assert!(!v.take_debris().is_empty());
        // The thick column cut loose: larger than any body, so the least of them goes (all asleep: the fewest voxels,
        // then the oldest).
        let least = v.bodies.iter().map(|(&k, b)| (b.grid.count(), k)).min().unwrap().1;
        cut(&mut v, &mut w, &mut ph, [0.7, 0.5, 3.9], [1.7, 0.7, 4.9]);
        assert_eq!((v.bodies(), v.retired), (6, 1));
        assert!(!v.bodies.contains_key(&least), "the least body went");
        // The thin post cut loose: 2 x 2 x 5 cells, smaller than every body: debris, and no body goes for it.
        let (before, capped) = (v.bodies.keys().copied().collect::<Vec<_>>(), v.capped);
        cut(&mut v, &mut w, &mut ph, [3.9, 0.5, 3.9], [4.3, 0.7, 4.3]);
        assert_eq!(v.bodies.keys().copied().collect::<Vec<_>>(), before);
        assert_eq!((v.retired, v.capped), (1, capped + 1));
    }

    /// Step 10 (a): a voxel body weighed cell by cell; paint reaching it from the world and from its own edit; voxels
    /// added to it alone; carved alone; its contacts reported when asked; gone, with why; and scene code's colliders
    /// refused on it.
    #[test]
    fn voxel_bodies_are_weighed_painted_and_grown_cell_by_cell() {
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::of_blocks(&[([0, 0, 0], [64, 4, 64], Some((0, 0)), true), ([28, 4, 28], [4, 24, 4], None, false), ([8, 4, 8], [4, 24, 4], None, false)], &mut w, &mut ph);
        // A third palette entry of 8,000 kg/m³.
        v.palette.push(0);
        v.density.push(8000.0);
        for x in [2.7f32, 0.7] {
            v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [x, 0.5, x, x + 0.6, 0.7, x + 0.6] }))).unwrap();
        }
        assert_eq!(v.bodies(), 2);
        let (k, b) = v.bodies.iter().find(|(_, b)| b.grid.count() == 336 && p_pose(&ph, b.body)[0] > 2.0).map(|(&k, b)| (k, b.body)).unwrap();
        let mass = |ph: &mut Option<PhysicsWorld>, w: &mut World| ph.as_mut().unwrap().call(w, "state", &serde_json::json!({ "body": b })).unwrap()["mass"].as_f64().unwrap();
        assert!((mass(&mut ph, &mut w) - 336.0).abs() < 1e-2, "336 cells of 1 l at 1,000 kg/m³");
        // Paint from the world over the bar's upper half: its surface there (the outer ring of each layer) is dense.
        let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "paint", "material": 2, "box": [2.5, 1.75, 2.5, 3.5, 3.0, 3.5] }))).unwrap();
        assert_eq!((e["bodies_painted"].as_u64(), e["body_voxels"].as_u64()), (Some(1), Some(12 * 11 + 4)), "{e}");
        let painted = 12 * 11 + 4;
        assert!((mass(&mut ph, &mut w) - (336.0 + painted as f64 * 7.0)).abs() < 1e-1, "{} kg", mass(&mut ph, &mut w));
        // The other bar is untouched by it.
        assert!(v.bodies.iter().filter(|(&kk, _)| kk != k).all(|(_, o)| o.grid.voxels().all(|(_, m)| m == 1)));
        // A slab added to this body alone, on its top: 4 x 4 cells more; its collider has them.
        let top = p_pose(&ph, b)[1] + 2.1;
        let e = v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "add", "material": 1, "box": [2.7, top + 0.01, 2.7, 3.1, top + 0.09, 3.1] }))).unwrap();
        assert_eq!(e["voxels"].as_u64(), Some(16), "{e}");
        let grid_cells = v.bodies[&k].grid.count();
        assert_eq!(ph.as_ref().unwrap().bridge_ref().engine_voxels(v.bodies[&k].collider), Some(grid_cells));
        assert!((mass(&mut ph, &mut w) - (352.0 + painted as f64 * 7.0)).abs() < 1e-1);
        // Scene code cannot put a collider of its own on it.
        let r = ph.as_mut().unwrap().call(&mut w, "collider", &serde_json::json!({ "id": 7, "body": b, "shape": { "type": "sphere", "radius": 0.2 } }));
        assert!(r.is_err(), "{r:?}");
        // Its contacts, asked for: thrown up, it lands, and the landing is reported.
        v.body_contacts(&mut ph, b, true).unwrap();
        ph.as_mut().unwrap().call(&mut w, "velocity", &serde_json::json!({ "body": b, "linvel": [0, 3, 0] })).unwrap();
        for _ in 0..90 {
            crate::api::physics_advance(&mut ph, &mut v, &mut w, 1.0 / 60.0);
        }
        let c = ph.as_mut().unwrap().call(&mut w, "contacts", &serde_json::json!({})).unwrap();
        assert!(c.as_array().unwrap().iter().any(|c| c["started"] == true && (c["bodyA"].as_u64() == Some(b as u64) || c["bodyB"].as_u64() == Some(b as u64))), "{c}");
        // Carved away whole, alone: gone, "carved"; the other body stays.
        let e = v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "carve", "box": [1.5, -1.0, 1.5, 4.5, 5.0, 4.5] }))).unwrap();
        assert_eq!(e["gone"], true, "{e}");
        assert_eq!(v.bodies(), 1);
        assert_eq!(v.take_gone(), serde_json::json!([[b, "carved"]]));
        assert!(v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "paint", "material": 1, "box": [0, 0, 0, 1, 1, 1] }))).is_err());
    }

    /// The review of #25: adds that would make a body huge or far-flung are refused before anything changes (grid,
    /// collider and mass stay as they were); a body grows to `MAX_BODY_CELLS` and no further; a body is never left
    /// without its collider; redraws let their old geometries go (the level's edits too); a material outside the
    /// palette is refused.
    #[test]
    fn a_body_is_not_grown_too_large_or_too_far() {
        let (mut w, mut ph, mut v) = one_body();
        let (k, b) = v.bodies.iter().map(|(&k, b)| (k, b.body)).next().unwrap();
        let cells = |v: &VoxelWorld, ph: &Option<PhysicsWorld>| (v.bodies[&k].grid.count(), ph.as_ref().unwrap().bridge_ref().engine_voxels(v.bodies[&k].collider));
        let at = p_pose(&ph, b);
        let before = cells(&v, &ph);
        // Far: 120 m out on every axis (it took 2.4 GB), 2e8 m out on one (it ran out of memory), 101 km (past the
        // physics' cell range: the mesh had the cells, the collider not).
        for far in [[120.0f32, 120.0, 120.0], [2e8, 0.0, 0.0], [101_000.0, 0.0, 0.0]] {
            let c = [at[0] + far[0], at[1] + far[1], at[2] + far[2]];
            let t = Instant::now();
            let e = v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "add", "material": 1, "box": [c[0], c[1], c[2], c[0] + 0.3, c[1] + 0.3, c[2] + 0.3] })));
            assert!(e.is_err(), "{far:?}: {e:?}");
            assert!(t.elapsed().as_millis() < 50, "{far:?}: refused after {} ms", t.elapsed().as_millis());
            assert_eq!(cells(&v, &ph), before, "{far:?}: nothing changed");
        }
        // A material the palette does not have.
        assert!(v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "paint", "material": 255, "box": [0, 0, 0, 9, 9, 9] }))).is_err());
        // Grown by adds of 64,000 voxels next to it, again and again: refused at `MAX_BODY_CELLS`, collider and grid
        // alike all the way.
        let mut x = at[0] + 0.4;
        let (mut grown, mut refused) = (0, false);
        for _ in 0..40 {
            match v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "add", "material": 1, "box": [x, at[1], at[2], x + 3.99, at[1] + 3.99, at[2] + 3.99] }))) {
                Ok(e) => grown += e["voxels"].as_u64().unwrap(),
                Err(e) => {
                    assert!(format!("{e}").contains("at most"), "{e}");
                    refused = true;
                    break;
                }
            }
            x += 4.0;
            let (g, c) = cells(&v, &ph);
            assert_eq!(Some(g), c, "grid and collider");
        }
        assert!(refused && v.bodies[&k].grid.count() <= MAX_BODY_CELLS, "{grown} grown, {} cells", v.bodies[&k].grid.count());
        // Its redraws let their geometries go: 200 paints back and forth leave the world's geometries as they were.
        let geometries = w.geometries.len() + w.proc_geometries.len();
        for n in 0..200 {
            v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "paint", "material": 1 + (n % 2), "box": [at[0] - 1.0, at[1] - 1.0, at[2] - 1.0, at[0] + 1.0, at[1] + 4.0, at[2] + 1.0] }))).unwrap();
        }
        assert!(w.geometries.len() + w.proc_geometries.len() <= geometries + 2, "{} geometries for {geometries}", w.geometries.len());
        // And so do the level's edits: 200 small pits in the slab.
        let geometries = w.geometries.len();
        for n in 0..200 {
            let x = 0.3 + (n % 20) as f32 * 0.25;
            v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "sphere": [x, 0.4, 0.3 + (n / 20) as f32 * 0.25, 0.08] }))).unwrap();
        }
        assert!(w.geometries.len() <= geometries + 4, "{} geometries for {geometries}", w.geometries.len());
    }

    /// Contacts of a listening body: a start once per pair, even when an edit takes its collider out and puts it back;
    /// an end when the body goes (the world's own end of a collider it no longer has was lost).
    #[test]
    fn a_bodys_contacts_start_once_and_end_when_it_goes() {
        let (mut w, mut ph, mut v) = one_body();
        let b = *v.bodies.values().map(|x| &x.body).next().unwrap();
        v.body_contacts(&mut ph, b, true).unwrap();
        let mut take = |ph: &mut Option<PhysicsWorld>, w: &mut World| ph.as_mut().unwrap().call(w, "contacts", &serde_json::json!({})).unwrap().as_array().unwrap().iter().filter(|c| c["bodyA"].as_u64() == Some(b as u64) || c["bodyB"].as_u64() == Some(b as u64)).map(|c| c["started"].as_bool().unwrap()).collect::<Vec<_>>();
        let step = |n: usize, v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>| (0..n).for_each(|_| { crate::api::physics_advance(ph, v, w, 1.0 / 60.0); });
        ph.as_mut().unwrap().call(&mut w, "velocity", &serde_json::json!({ "body": b, "linvel": [0, 2, 0] })).unwrap();
        step(90, &mut v, &mut w, &mut ph);
        let landed = take(&mut ph, &mut w);
        assert!(landed.contains(&true), "it landed: {landed:?}");
        // Resting on the stump, a voxel added and one carved (each takes its collider out and puts it back): no start again.
        let at = p_pose(&ph, b);
        v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "add", "material": 1, "box": [at[0] + 0.41, at[1] + 1.0, at[2] + 0.1, at[0] + 0.49, at[1] + 1.08, at[2] + 0.18] }))).unwrap();
        v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "carve", "box": [at[0] + 0.41, at[1] + 1.0, at[2] + 0.1, at[0] + 0.49, at[1] + 1.08, at[2] + 0.18] }))).unwrap();
        step(30, &mut v, &mut w, &mut ph);
        let edited = take(&mut ph, &mut w);
        assert!(!edited.contains(&true), "no second start: {edited:?}");
        // Carved away whole: its contacts end.
        v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "carve", "box": [at[0] - 1.0, at[1] - 1.0, at[2] - 1.0, at[0] + 2.0, at[1] + 4.0, at[2] + 2.0] }))).unwrap();
        assert_eq!(v.bodies(), 0);
        let gone = take(&mut ph, &mut w);
        assert!(!gone.is_empty() && gone.iter().all(|s| !s), "its contacts end: {gone:?}");
    }

    /// The level's colliders hang on its fixed body (step 10 c1): a scene body hangs from the level by a rope (2 m,
    /// held within 2 cm); to a ray and to contacts the level is still on no body (`body` null: `engineColliders` with no
    /// `body`, as before); a body resting on it falls asleep.
    #[test]
    fn the_level_has_a_fixed_body_that_joints_hold_to() {
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::of_blocks(&[([0, 0, 0], [64, 4, 64], Some((0, 0)), true)], &mut w, &mut ph);
        let level = v.level_body().expect("a level body");
        assert!(level >= physics_core::bridge::ENGINE_IDS);
        let p = ph.as_mut().unwrap();
        assert_eq!(p.bridge().engine_level(), Some(level));
        // A ray down at the slab: the level, on no body.
        let hit = p.call(&mut w, "ray", &serde_json::json!({ "origin": [3.0, 5.0, 3.0], "direction": [0, -1, 0] })).unwrap();
        assert!(hit["collider"].as_u64().is_some() && hit["body"].is_null(), "{hit}");
        // A box resting on it, listening: its contact's other end is on no body; it falls asleep.
        p.call(&mut w, "body", &serde_json::json!({ "id": 1, "position": [1.0, 0.6, 1.0] })).unwrap();
        p.call(&mut w, "collider", &serde_json::json!({ "id": 1, "body": 1, "shape": { "type": "box", "halfExtents": [0.2, 0.2, 0.2] }, "events": true })).unwrap();
        // A ball hanging from the level by a rope 2 m long, from a point 3 m above the slab's top.
        p.call(&mut w, "body", &serde_json::json!({ "id": 2, "position": [4.0, 3.4, 4.0] })).unwrap();
        p.call(&mut w, "collider", &serde_json::json!({ "id": 2, "body": 2, "shape": { "type": "sphere", "radius": 0.1 } })).unwrap();
        p.call(&mut w, "joint", &serde_json::json!({ "id": 3, "a": level, "b": 2, "type": "rope", "length": 2.0, "anchor1": [4.0, 3.4, 4.0] })).unwrap();
        for _ in 0..240 {
            crate::api::physics_advance(&mut ph, &mut v, &mut w, 1.0 / 60.0);
        }
        let p = ph.as_mut().unwrap();
        let ball = p.call(&mut w, "state", &serde_json::json!({ "body": 2 })).unwrap();
        let y = ball["position"][1].as_f64().unwrap();
        assert!((3.4 - y - 2.0).abs() < 0.02, "hangs 2 m below its anchor: {ball}");
        let contacts = p.call(&mut w, "contacts", &serde_json::json!({})).unwrap();
        let mine: Vec<_> = contacts.as_array().unwrap().iter().filter(|c| c["bodyA"].as_u64() == Some(1) || c["bodyB"].as_u64() == Some(1)).collect();
        assert!(!mine.is_empty() && mine.iter().all(|c| c["bodyA"].is_null() || c["bodyB"].is_null()), "the level's end on no body: {contacts}");
        let crate_ = p.call(&mut w, "state", &serde_json::json!({ "body": 1 })).unwrap();
        assert_eq!(crate_["sleeping"], serde_json::json!(true), "{crate_}");
    }

    /// The same body as one more check: a slab and a bar cut loose (336 voxels of 1,000 kg/m³).
    fn one_body() -> (World, Option<PhysicsWorld>, VoxelWorld) {
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::of_blocks(&[([0, 0, 0], [64, 4, 64], Some((0, 0)), true), ([28, 4, 28], [4, 24, 4], None, false)], &mut w, &mut ph);
        v.palette.push(0);
        v.density.push(8000.0);
        v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [2.7, 0.5, 2.7, 3.3, 0.7, 3.3] }))).unwrap();
        assert_eq!(v.bodies(), 1);
        (w, ph, v)
    }

    /// A body of two densities weighed cell by cell is what parry itself makes of the same cubes (each a cuboid of its
    /// density, placed, summed): mass, centre of mass and principal inertia.
    #[test]
    fn a_body_of_two_densities_weighs_what_parry_says() {
        use physics_core::rapier3d::parry::{mass_properties::MassProperties, math::{Pose, Vector}};
        let (mut w, mut ph, mut v) = one_body();
        let (k, b) = v.bodies.iter().map(|(&k, b)| (k, b.body)).next().unwrap();
        // One side of the bar dense (8,000 kg/m³), the rest 1,000: an off-centre, non-diagonal case.
        let at = p_pose(&ph, b);
        v.edit_body(&mut w, &mut ph, b, &op(serde_json::json!({ "op": "paint", "material": 2, "box": [at[0] - 1.0, at[1] - 1.0, at[2] - 1.0, at[0] + 0.15, at[1] + 1.2, at[2] + 0.15] }))).unwrap();
        let grid = &v.bodies[&k].grid;
        assert!(grid.voxels().any(|(_, m)| m == 2) && grid.voxels().any(|(_, m)| m == 1));
        let s = v.size;
        let parry = grid.voxels().fold(MassProperties::default(), |acc, (g, m)| {
            let c = MassProperties::from_cuboid(v.density[m as usize], Vector::splat(s / 2.0));
            acc + c.transform_by(&Pose::from_translation(Vector::new((g[0] as f32 + 0.5) * s, (g[1] as f32 + 0.5) * s, (g[2] as f32 + 0.5) * s)))
        });
        let (mass, com, inertia) = ph.as_ref().unwrap().bridge_ref().engine_mass_properties(b).unwrap();
        let (pc, pi) = (parry.local_com, parry.principal_inertia());
        let rel = |a: f32, b: f32| (a - b).abs() / b.abs().max(1e-6);
        assert!(rel(mass, parry.mass()) < 1e-5, "mass {mass} for parry's {}", parry.mass());
        assert!((Vector::from(com) - pc).length() < 1e-5, "centre of mass {com:?} for parry's {pc:?}");
        let (mut ours, mut theirs) = (inertia, [pi.x, pi.y, pi.z]);
        ours.sort_by(f32::total_cmp);
        theirs.sort_by(f32::total_cmp);
        assert!((0..3).all(|a| rel(ours[a], theirs[a]) < 1e-4), "principal inertia {ours:?} for parry's {theirs:?}");
    }

    fn p_pose(ph: &Option<PhysicsWorld>, body: u32) -> [f32; 3] {
        ph.as_ref().unwrap().bridge_ref().engine_pose(body).unwrap().0
    }

    /// What scene code asks of a level before it is loaded is kept, and counted: past `MAX_QUEUED` an edit or a frame
    /// is refused (the review of #24: a setup looping on edits grew the queue without bound, outside the script
    /// engine's memory limit).
    #[test]
    fn edits_before_the_load_are_bounded() {
        let mut v = VoxelWorld::default();
        let d: LevelDesc = serde_json::from_value(serde_json::json!({ "route": "mesh", "palette": [{ "name": "none", "color": [0, 0, 0], "roughness": 1, "metalness": 0 }], "objects": [] })).unwrap();
        v.request(d).unwrap();
        let (mut w, mut ph) = (World::default(), None);
        for k in 0..MAX_QUEUED {
            let r = if k % 2 == 0 { v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [0, 0, 0, 1, 1, 1] }))) } else { v.frame(&mut w, 1.0 / 60.0) };
            assert_eq!(r.unwrap()["queued"], true);
        }
        let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [0, 0, 0, 1, 1, 1] }))).unwrap_err();
        assert!(format!("{e}").contains("at most 4096"), "{e}");
        assert!(v.frame(&mut w, 1.0 / 60.0).is_err());
    }

    /// A carve takes a body's voxels as it takes the level's (the review of #24): a radius clamped to `MAX_RADIUS`
    /// voxels and to 0 below, and a box whole for every class it reaches, not shrunk by the class's share.
    #[test]
    fn a_carve_takes_bodies_as_it_takes_the_level() {
        let one = || {
            let mut w = World::default();
            let mut ph: Option<PhysicsWorld> = None;
            let mut v = VoxelWorld::of_blocks(&[([0, 0, 0], [64, 4, 64], Some((0, 0)), true), ([28, 4, 28], [4, 24, 4], None, false)], &mut w, &mut ph);
            v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [2.7, 0.5, 2.7, 3.3, 0.7, 3.3] }))).unwrap();
            assert_eq!(v.bodies(), 1);
            (w, ph, v)
        };
        let body_voxels = |v: &VoxelWorld| v.bodies.values().map(|b| b.grid.count()).sum::<usize>();
        // A sphere of 600 m centred 500 m away: 12.8 m as the level takes it, which does not reach.
        let (mut w, mut ph, mut v) = one();
        if let Ok(e) = v.edit(&mut w, &mut ph, &EditOp { shape: ShapeM::Sphere([500.0, 0.0, 0.0], 600.0), kind: EditKind::Carve([256; 4]) }) {
            assert_eq!((e["voxels"].as_u64(), e["body_voxels"].as_u64().unwrap_or(0)), (Some(0), 0), "{e}");
        }
        assert_eq!(body_voxels(&v), 336);
        // A negative radius at the body: nothing, as in the level.
        let (mut w, mut ph, mut v) = one();
        let e = v.edit(&mut w, &mut ph, &EditOp { shape: ShapeM::Sphere([3.0, 1.5, 3.0], -2.0), kind: EditKind::Carve([256; 4]) }).unwrap();
        assert_eq!(e["body_voxels"].as_u64().unwrap_or(0), 0, "{e}");
        assert_eq!(body_voxels(&v), 336);
        // A box over the body's upper half with every class at half its share: the half, whole.
        let (mut w, mut ph, mut v) = one();
        let e = v.edit(&mut w, &mut ph, &EditOp { shape: ShapeM::Box([2.5, 1.75, 2.5], [3.5, 3.0, 3.5]), kind: EditKind::Carve([128; 4]) }).unwrap();
        assert_eq!(e["body_voxels"].as_u64(), Some(16 * 11), "{e}");
    }

    /// The cap on bodies: past it a sleeping body as large as the new one goes, its voxels debris; and a body the physics world took
    /// out goes with its objects.
    #[test]
    fn voxel_bodies_are_bounded() {
        let mut w = World::default();
        let mut ph: Option<PhysicsWorld> = None;
        let mut v = VoxelWorld::default();
        // Four posts on a slab, each cut loose in turn; at most two bodies.
        let at = |k: usize| ([-0.4, 0.4][k & 1], [-0.4, 0.4][k >> 1]);
        let posts: Vec<serde_json::Value> = (0..4).map(|k| serde_json::json!({ "name": format!("post{k}"), "uri": "tests/render/data/voxel-post.tpg", "position": [at(k).0, 0.4, at(k).1] })).collect();
        let mut objects = vec![serde_json::json!({ "name": "slab", "uri": "tests/render/data/voxel-slab.tpg", "position": [0, 0, 0], "anchor": true })];
        objects.extend(posts);
        let d: LevelDesc = serde_json::from_value(serde_json::json!({
            "route": "mesh", "debris": true, "maxBodies": 2,
            "palette": [{ "name": "none", "color": [0, 0, 0], "roughness": 1, "metalness": 0 }, { "name": "black", "color": [0, 0, 0], "roughness": 1, "metalness": 0 },
                        { "name": "post", "color": [1, 1, 1], "roughness": 1, "metalness": 0, "density": 500 }],
            "objects": objects,
        }))
        .unwrap();
        v.request(d).unwrap();
        v.realize(&mut w, &mut ph, &assets()).unwrap();
        let mut seen = 0;
        for k in 0..4 {
            let (x, z) = at(k);
            let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [x - 0.3, 0.5, z - 0.3, x + 0.3, 0.7, z + 0.3] }))).unwrap();
            assert_eq!(e["pieces"], 1, "{e}");
            seen += 1;
            for _ in 0..120 {
                crate::api::physics_advance(&mut ph, &mut v, &mut w, 1.0 / 60.0);
            }
            assert_eq!(v.bodies(), seen.min(2), "after {seen} cuts");
        }
        assert_eq!(v.retired, 2);
        assert!(v.take_debris().len() / 4 > 2 * 100, "the retired bodies' voxels are debris");
        assert_eq!(v.world_objects(), 1 + 4 + 2, "the slab, four stumps, two parts");
        // A body the physics world no longer has (it took it out: one that fell off the level, `Physics::step`): at the
        // next sync it is gone with its objects, and nothing of it is kept.
        let body = v.body_list(ph.as_ref().unwrap())[0]["body"].as_u64().unwrap() as u32;
        ph.as_mut().unwrap().bridge().engine_remove(Some(body), None);
        crate::api::physics_advance(&mut ph, &mut v, &mut w, 1.0 / 60.0);
        assert!(v.take_debris().is_empty());
        assert_eq!((v.bodies(), v.retired, v.world_objects()), (1, 2, 1 + 4 + 1), "{}", v.body_list(ph.as_ref().unwrap()));
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
            "route": "mesh", "physics": false,
            "palette": [{ "name": "none", "color": [0, 0, 0], "roughness": 1, "metalness": 0 }, { "name": "ground", "color": [0.5, 0.5, 0.5], "roughness": 1, "metalness": 0 }],
            "objects": [tile(0), tile(1), tile(2)],
        }))
        .unwrap();
        v.request(d).unwrap();
        v.realize(&mut w, &mut None, &assets()).unwrap();
        assert_eq!((v.tiles.len(), v.world_objects()), (3, 3));
        // Everything of tile 0 (x -0.8 .. 0.8 m) and tile 1's voxels up to its own x = 0 (world 6.4 m).
        let e = v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "carve", "box": [-1.0, -1.0, -1.0, 6.4, 1.0, 1.0] }))).unwrap();
        assert_eq!((e["removed"].as_u64(), e["touched"].as_u64()), (Some(1), Some(1)), "{e}");
        assert_eq!(e["voxels"].as_u64(), Some(1024 + 512));
        assert_eq!((v.tiles.len(), v.world_objects()), (2, 2));
        assert!(!v.tiles.contains_key(&(0, 0)) && v.objects[0].is_none());
        // Tile 1's left edge again (its neighbour there is gone), and tile 2 whole: tile 1 is meshed again for both.
        let e = v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "carve", "box": [6.4, -1.0, -1.0, 6.5, 1.0, 1.0] }))).unwrap();
        assert_eq!((e["removed"].as_u64(), e["touched"].as_u64(), e["voxels"].as_u64()), (Some(0), Some(1), Some(64)), "{e}");
        let e = v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "carve", "box": [11.0, -1.0, -1.0, 14.0, 1.0, 1.0] }))).unwrap();
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
        assert!(v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "carve", "sphere": [0, 0, 0, 1] }))).is_err());
        let mut v = level(&mut w);
        // A box of a kilometre is refused; a huge radius is cut down to the largest (12.8 m).
        assert!(v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "carve", "box": [-500, -500, -500, 500, 500, 500] }))).is_err());
        let e = v.edit(&mut w, &mut None, &op(serde_json::json!({ "op": "explode", "sphere": [0, 0, 0, 1e9] }))).unwrap();
        assert!(e["voxels"].as_u64().unwrap() > 0);
    }

    /// Step 8 on the benchmark level, on the CPU (no GPU; a measurement):
    ///   VOXEL_PHYSICS=../out/bundles/voxel-level-destruction-mesh.js cargo test --release --lib voxel_level_physics -- --ignored --nocapture
    /// The level's colliders (how many, their cells, the time to build them, their memory); explosions of 0.5 to 4 m
    /// at a building's foot (the edit's parts: the level's carve, the colliders, the meshes, the bodies); then a cut
    /// under the whole building (it comes loose as bodies) and the physics' ticks after each: step and sync ms a
    /// frame, the bodies and how many sleep.
    #[test]
    #[ignore = "a measurement: VOXEL_PHYSICS=<bundle.js>"]
    fn voxel_level_physics() {
        let Ok(bundle) = std::env::var("VOXEL_PHYSICS") else { return println!("VOXEL_PHYSICS=<bundle.js>") };
        let assets = assets();
        let host = crate::script::ScriptHost::new(&std::fs::read_to_string(&bundle).unwrap()).unwrap();
        host.setup("r", None).unwrap();
        let (mut v, mut w, mut ph) = (host.voxels.borrow_mut(), host.world.borrow_mut(), host.physics.borrow_mut());
        let t = Instant::now();
        let r = v.realize(&mut w, &mut ph, &assets).unwrap().unwrap();
        println!("level: {} objects, {} triangles; realize {:.2} s; colliders: {} of {} cells in {:.0} ms (of it finding the cells {:.0}), {:.1} MiB (parry's count)", r["objects"], r["triangles"], t.elapsed().as_secs_f64(), r["colliders"], r["collider_cells"], r["colliders_ms"].as_f64().unwrap(), r["shell_ms"].as_f64().unwrap(), ph.as_ref().unwrap().bridge_ref().voxel_bytes() as f64 / 1048576.0);
        w.take_changes();
        {
            let level = v.level.as_ref().unwrap();
            let tiles: Vec<usize> = (0..v.objects.len()).filter(|&k| v.objects[k].as_ref().is_some_and(|o| o.tile.is_some())).collect();
            let cells: usize = tiles.iter().map(|&k| level.objects[k].as_ref().unwrap().grid.count()).sum();
            let aligned = tiles.iter().filter(|&&k| { let (o, l) = (v.objects[k].as_ref().unwrap(), level.objects[k].as_ref().unwrap()); l.place.turn == 0 && o.off.iter().all(|x| x % 8 == 0) && l.place.t.iter().all(|x| x.rem_euclid(8) == 0) }).count();
            let o = v.objects[tiles[0]].as_ref().unwrap();
            let l = level.objects[tiles[0]].as_ref().unwrap();
            println!("terrain: {} tiles of {} cells in all ({} aligned to the world's bricks); the first: off {:?}, placed at {:?} turn {}, cells {:?}..{:?}", tiles.len(), cells, aligned, o.off, l.place.t, l.place.turn, l.lo, l.hi);
            let houses: BTreeMap<String, usize> = v.objects.iter().flatten().filter(|o| o.tile.is_none()).fold(BTreeMap::new(), |mut m, o| { *m.entry(o.name.split('-').next().unwrap_or("").to_string()).or_default() += 1; m });
            println!("objects by kind: {houses:?}");
            let anchors = (0..v.objects.len()).filter(|&k| v.objects[k].as_ref().is_some_and(|o| o.tile.is_none()) && level.objects[k].as_ref().is_some_and(|o| o.anchor)).count();
            println!("anchors that are no tiles: {anchors}");
        }
        // The building with the most voxels, and the ground under its middle.
        let level = v.level.as_ref().unwrap();
        let (k, cells) = (0..v.objects.len()).filter(|&k| v.objects[k].as_ref().is_some_and(|o| o.name.starts_with("house") && o.tile.is_none())).map(|k| (k, level.objects[k].as_ref().map_or(0, |o| o.grid.count()))).max_by_key(|x| x.1).expect("a house");
        let (lo, hi) = (v.objects[k].as_ref().unwrap().lo, v.objects[k].as_ref().unwrap().hi);
        println!("the largest building part: {} ({cells} voxels), {:.1} x {:.1} x {:.1} m at {:?}", v.objects[k].as_ref().unwrap().name, hi.x - lo.x, hi.y - lo.y, hi.z - lo.z, lo);
        let frames = |v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>, n: usize, what: &str| {
            let (mut step, mut sync, mut worst) = (vec![], vec![], 0.0f64);
            for _ in 0..n {
                let t = Instant::now();
                crate::api::physics_advance(ph, v, w, 1.0 / 60.0);
                let all = t.elapsed().as_secs_f64() * 1e3;
                let s = ph.as_mut().unwrap().take_stats();
                step.push(s.step_ms);
                sync.push(all - s.step_ms);
                worst = worst.max(all);
                w.take_changes();
            }
            step.sort_by(f64::total_cmp);
            sync.sort_by(f64::total_cmp);
            let list = v.body_list(ph.as_ref().unwrap());
            let asleep = list.as_array().unwrap().iter().filter(|b| b["asleep"] == true).count();
            println!("  {what}: {n} frames: step p50 {:.3} p95 {:.3} ms, placing p50 {:.3} ms, a frame at worst {:.2} ms; {} bodies, {asleep} asleep", step[n / 2], step[n * 95 / 100], sync[n / 2], worst, v.bodies());
        };
        frames(&mut v, &mut w, &mut ph, 60, "the level at rest");
        let foot = Vec3::new(lo.x + 0.3, lo.y + 1.0, (lo.z + hi.z) * 0.5);
        for (n, r) in [0.5f32, 1.0, 2.0, 4.0].into_iter().enumerate() {
            let at = foot + Vec3::new(0.0, 0.0, (n as f32 - 1.5) * (hi.z - lo.z) * 0.2);
            let t = Instant::now();
            let e = v.explode(&mut w, &mut ph, at, r).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            println!("explosion of {r} m: {ms:.2} ms in all: carve {:.2}, colliders {:.2}, meshes {:.2}, parts {:.2} (meshes and bodies {:.2}), bodies carved {:.2}; {} voxels, {} objects touched, {} parts of {} voxels; bodies now {}", e["carve_ms"].as_f64().unwrap(), e["colliders_ms"].as_f64().unwrap(), e["rebuild_ms"].as_f64().unwrap(), e["pieces_ms"].as_f64().unwrap(), e["piece_mesh_ms"].as_f64().unwrap(), e["bodies_ms"].as_f64().unwrap_or(0.0), e["voxels"], e["touched"], e["pieces"], e["piece_voxels"], e["bodies"]);
            w.take_changes();
            frames(&mut v, &mut w, &mut ph, 120, "after it");
        }
        // The cut (the bench's `fell`): a slab 1 m thick through everything, half a metre above the building's foot,
        // 30 x 30 m about its middle: the buildings, trees, lamps and cars there lose their footing.
        let t = Instant::now();
        let (cx, cz) = ((lo.x + hi.x) * 0.5, (lo.z + hi.z) * 0.5);
        let e = v.edit(&mut w, &mut ph, &op(serde_json::json!({ "op": "carve", "box": [cx - 15.0, lo.y + 0.5, cz - 15.0, cx + 15.0, lo.y + 1.5, cz + 15.0] }))).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        println!("the cut under the building: {ms:.2} ms in all: carve {:.2}, colliders {:.2}, meshes {:.2}, parts {:.2}; {} voxels, {} objects touched, {} parts of {} voxels; bodies now {}", e["carve_ms"].as_f64().unwrap(), e["colliders_ms"].as_f64().unwrap(), e["rebuild_ms"].as_f64().unwrap(), e["pieces_ms"].as_f64().unwrap(), e["voxels"], e["touched"], e["pieces"], e["piece_voxels"], e["bodies"]);
        let mut sizes: Vec<usize> = v.bodies.values().map(|b| b.grid.count()).collect();
        sizes.sort_unstable();
        println!("  bodies by voxels: the largest {:?}, the median {:?}, {} in all voxels; voxel shapes now {:.1} MiB", sizes.last(), sizes.get(sizes.len() / 2), sizes.iter().sum::<usize>(), ph.as_ref().unwrap().bridge_ref().voxel_bytes() as f64 / 1048576.0);
        w.take_changes();
        frames(&mut v, &mut w, &mut ph, 60, "the first second");
        frames(&mut v, &mut w, &mut ph, 240, "the next four");
        frames(&mut v, &mut w, &mut ph, 300, "the next five");
        // An explosion into what fell.
        let t = Instant::now();
        let e = v.explode(&mut w, &mut ph, Vec3::new((lo.x + hi.x) * 0.5, lo.y + 2.0, (lo.z + hi.z) * 0.5), 3.0).unwrap();
        println!("an explosion of 3 m into it: {:.2} ms in all: carve {:.2}, colliders {:.2}, bodies carved {:.2} ({} bodies, {} voxels); bodies now {}", t.elapsed().as_secs_f64() * 1e3, e["carve_ms"].as_f64().unwrap(), e["colliders_ms"].as_f64().unwrap(), e["bodies_ms"].as_f64().unwrap_or(0.0), e["bodies_carved"], e["body_voxels"], e["bodies"]);
        w.take_changes();
        frames(&mut v, &mut w, &mut ph, 300, "five seconds after");
        println!("retired for the cap: {}", v.retired);
    }

    /// The trees of the benchmark level cut at the trunk (the destruction game's `topple`): what each crown does as a
    /// body, a second at a time: its speed, its spin, whether it sleeps; and the step's time.
    #[test]
    #[ignore = "a measurement: VOXEL_PHYSICS=<bundle.js>"]
    fn voxel_level_topple() {
        let Ok(bundle) = std::env::var("VOXEL_PHYSICS") else { return println!("VOXEL_PHYSICS=<bundle.js>") };
        let assets = assets();
        let host = crate::script::ScriptHost::new(&std::fs::read_to_string(&bundle).unwrap()).unwrap();
        host.setup("r", None).unwrap();
        let (mut v, mut w, mut ph) = (host.voxels.borrow_mut(), host.world.borrow_mut(), host.physics.borrow_mut());
        v.realize(&mut w, &mut ph, &assets).unwrap().unwrap();
        w.take_changes();
        // (The game's eight trunks with seed 1, as its `trunks` gives them; an explosion of 0.6 m at 0.8 m up each.)
        let trunks = [[-100.6f32, 9.3, -92.0], [-103.9, 9.1, -93.0], [-86.7, 9.8, -91.1], [-105.9, 9.2, -88.8], [-89.3, 10.0, -88.1], [-105.8, 9.2, -86.5], [-88.1, 10.1, -87.0], [-83.9, 10.0, -90.7]];
        for t in trunks {
            let at = Vec3::new(t[0], t[1] + 0.8, t[2]);
            let e = v.explode(&mut w, &mut ph, at, 0.6).unwrap();
            println!("a trunk cut at {at:?}: {} voxels carved, {} parts of {} voxels", e["voxels"], e["pieces"], e["piece_voxels"]);
            w.take_changes();
            for _ in 0..30 {
                crate::api::physics_advance(&mut ph, &mut v, &mut w, 1.0 / 60.0);
                w.take_changes();
            }
        }
        for second in 1..=20 {
            let mut step = vec![];
            for _ in 0..60 {
                crate::api::physics_advance(&mut ph, &mut v, &mut w, 1.0 / 60.0);
                step.push(ph.as_mut().unwrap().take_stats().step_ms);
                w.take_changes();
            }
            step.sort_by(f64::total_cmp);
            let p = ph.as_ref().unwrap();
            let each: Vec<String> = v.bodies.values().map(|b| {
                let (at, _, asleep) = p.bridge_ref().engine_pose(b.body).unwrap();
                let (lin, ang) = p.bridge_ref().engine_velocity_of(b.body).unwrap();
                format!("{}{} y {:.2} v {:.3} w {:.3}", b.grid.count(), if asleep { " asleep" } else { "" }, at[1], Vec3::from(lin).length(), Vec3::from(ang).length())
            }).collect();
            println!("  second {second}: step p50 {:.3} ms; {}", step[30], each.join(" | "));
        }
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

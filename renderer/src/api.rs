//! The engine API scene code calls (`globalThis.__engine`, packages/scene/src/engine.ts): what each call does to the
//! engine's world, apart from how the call arrives. Natively it arrives from the embedded QuickJS (script.rs
//! `install_engine`), in a browser from the page's own JavaScript through the wasm module's exports (renderer/web):
//! both hand a call's descriptor over as JSON and its arrays as slices, and both get the same world.

use crate::desc;
use crate::physics::PhysicsWorld;
use crate::voxel::VoxelWorld;
use crate::world::{Geometry, Kind, Settings, Texture, TextureSource, World};
use glam::{Quat, Vec3};
use serde_json::Value;
use std::sync::Arc;

/// A call that failed: `Type` for an argument of the wrong shape (a `TypeError` to scene code), `Other` for what the
/// engine refused (an `Error` with its message).
pub enum Error {
    Type(String),
    Other(anyhow::Error),
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Error {
        Error::Other(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Type(m) => write!(f, "{m}"),
            Error::Other(e) => write!(f, "{e:#}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A descriptor as the engine's own type.
pub fn parse<T: serde::de::DeserializeOwned>(what: &str, v: &Value) -> Result<T> {
    serde_json::from_value(v.clone()).map_err(|e| Error::Type(format!("engine.{what}: {e}")))
}

/// Physics (renderer/src/physics.rs): one call, `physics(op, args)`, answering in JSON. The world is made by the first
/// call; the engine steps it, scene code cannot.
pub fn physics(p: &mut Option<PhysicsWorld>, w: &mut World, op: &str, args: &Value) -> Result<String> {
    Ok(p.get_or_insert_with(Default::default).call(w, op, args)?.to_string())
}

/// The poses of the objects that bodies drive, by the slot the binding gave each body (7 floats: position, rotation
/// xyzw, as scene code reads an object's): one copy for all a frame reads. Empty without a world.
pub fn physics_poses(p: &Option<PhysicsWorld>) -> &[f32] {
    p.as_ref().map_or(&[], |p| p.poses())
}

/// The voxel engine's level (renderer/src/voxel.rs): loaded once the session knows the asset root.
pub fn voxel_level(v: &mut VoxelWorld, d: &Value) -> Result<()> {
    let d: crate::voxel::LevelDesc = parse("voxelLevel", d)?;
    Ok(v.request(d)?)
}

/// An explosion: what it did, as JSON.
pub fn voxel_explode(v: &mut VoxelWorld, w: &mut World, p: &mut Option<PhysicsWorld>, x: f64, y: f64, z: f64, r: f64, push: Option<f64>) -> Result<String> {
    // (A push out of range or not finite is refused before anything changes: `VoxelWorld::explode`.)
    let push = push.map_or(crate::voxel::DEFAULT_PUSH, |p| if p.is_finite() && p.abs() <= f32::MAX as f64 { p as f32 } else { f32::NAN });
    Ok(v.explode(w, p, Vec3::new(x as f32, y as f32, z as f32), r as f32, push)?.to_string())
}

/// An edit of the level (`voxelEdit({ op, sphere | box | capsule, material })`; voxel.rs `EditOp`).
pub fn voxel_edit(v: &mut VoxelWorld, w: &mut World, p: &mut Option<PhysicsWorld>, d: &Value) -> Result<String> {
    let op = crate::voxel::EditOp::parse(d)?;
    Ok(v.edit(w, p, &op)?.to_string())
}

/// An edit of one voxel body alone (`voxelBodyEdit(body, { op, sphere | box | capsule, material })`: a voxel object's
/// `add`, `paint`, `carve`): what it did, as JSON.
pub fn voxel_body_edit(v: &mut VoxelWorld, w: &mut World, p: &mut Option<PhysicsWorld>, body: f64, d: &Value) -> Result<String> {
    let op = crate::voxel::EditOp::parse(d)?;
    let body = whole(body).ok_or_else(|| Error::Type("voxelBodyEdit: body must be a body id".into()))?;
    Ok(v.edit_body(w, p, body, &op)?.to_string())
}

/// A voxel body's contacts reported or not (`voxelContacts(body, on)`).
pub fn voxel_contacts(v: &mut VoxelWorld, p: &mut Option<PhysicsWorld>, body: f64, on: bool) -> Result<()> {
    let body = whole(body).ok_or_else(|| Error::Type("voxelContacts: body must be a body id".into()))?;
    Ok(v.body_contacts(p, body, on)?)
}

/// The voxel bodies gone since the last call (`voxelGone()`): [[body, "cap" | "carved" | "fell"], ...].
pub fn voxel_gone(v: &mut VoxelWorld) -> String {
    v.take_gone().to_string()
}

/// A body id as scene code hands it: a whole number below 2^32.
fn whole(x: f64) -> Option<u32> {
    (x.is_finite() && x >= 0.0 && x <= u32::MAX as f64 && x.fract() == 0.0).then_some(x as u32)
}

/// The voxel bodies (loose parts the physics world moves): each one's body id for `physics` calls, its voxels,
/// whether it sleeps, where it is. "[]" without any.
pub fn voxel_bodies(v: &VoxelWorld, p: &Option<PhysicsWorld>) -> String {
    p.as_ref().map_or("[]".into(), |p| v.body_list(p).to_string())
}

/// A frame of the physics world (`dt` seconds: the whole ticks that fit), and the voxel bodies' objects placed
/// where the bodies are. False without a world.
pub fn physics_advance(p: &mut Option<PhysicsWorld>, v: &mut VoxelWorld, w: &mut World, dt: f64) -> bool {
    let Some(p) = p.as_mut() else { return false };
    p.advance(w, dt);
    let t = web_time::Instant::now();
    v.sync_bodies(w, p);
    p.add_sync_ms(t.elapsed().as_secs_f64() * 1e3);
    true
}

/// The physics world `ticks` ticks on in all (an offline render at a scene time), the voxel bodies' objects with it.
pub fn physics_settle(p: &mut Option<PhysicsWorld>, v: &mut VoxelWorld, w: &mut World, ticks: u32) {
    if let Some(p) = p.as_mut() {
        p.settle(w, ticks);
        v.sync_bodies(w, p);
    }
}

/// The level's hash (16 hex digits; "" before a level): the same commands give the same hash.
pub fn voxel_hash(v: &VoxelWorld) -> String {
    v.hash().map_or(String::new(), |h| format!("{h:016x}"))
}

pub fn voxel_frame(v: &mut VoxelWorld, w: &mut World, dt: f64) -> Result<String> {
    Ok(v.frame(w, dt)?.to_string())
}

/// (`groups`: triples of u32; `colors`: per vertex.)
pub fn geometry(w: &mut World, id: u32, p: &[f32], n: &[f32], uv: &[f32], idx: &[u32], groups: Option<&[u32]>, colors: Option<&[f32]>) -> Result<()> {
    let groups = groups.map_or(vec![], |g| g.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect());
    let g = Geometry { positions: p.to_vec(), normals: n.to_vec(), uvs: uv.to_vec(), indices: idx.to_vec(), groups, colors: colors.map_or(vec![], |c| c.to_vec()) };
    Ok(w.set_geometry(id, g)?)
}

pub fn texture(w: &mut World, id: u32, j: &Value, data: Option<&[u8]>) -> Result<()> {
    let pair = |k: &str, def: f32| -> [f32; 2] { [0, 1].map(|i| j[k][i].as_f64().map_or(def, |x| x as f32)) };
    let source = match (data, j["uri"].as_str()) {
        (Some(bytes), _) if j["size"].is_array() => {
            let (width, height) = (j["size"][0].as_u64().unwrap_or(0) as u32, j["size"][1].as_u64().unwrap_or(0) as u32);
            if bytes.len() != width as usize * height as usize * 4 {
                return Err(Error::Type(format!("engine.texture: {} bytes are not {width}x{height} RGBA8 pixels", bytes.len())));
            }
            TextureSource::Pixels { width, height, rgba: Arc::new(bytes.to_vec()) }
        }
        (Some(bytes), _) => TextureSource::Encoded(Arc::new(bytes.to_vec())),
        (None, Some(uri)) => TextureSource::File(uri.to_string()),
        (None, None) => return Err(Error::Type("engine.texture: needs a uri or data".into())),
    };
    let t = Texture { source, srgb: j["srgb"].as_bool().unwrap_or(true), wrap: j["wrap"].as_bool().unwrap_or(true), repeat: pair("repeat", 1.0), offset: pair("offset", 0.0) };
    w.set_texture(id, t)?;
    Ok(())
}

/// Density grids (ROADMAP § 6 step 5b): a grid (its size, channels, program and data).
pub fn density_grid(w: &mut World, id: u32, j: &Value, data: Option<&[f32]>) -> Result<()> {
    let n = |k: usize| j["size"][k].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32;
    let program = j["program"]["id"].as_str().map(|id| (id.to_string(), j["program"]["params"].as_array().map_or(vec![], |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())));
    Ok(w.set_grid(id, [n(0), n(1), n(2)], j["channels"].as_u64().unwrap_or(1) as u32, data, program)?)
}

pub fn material(w: &mut World, id: u32, d: &Value) -> Result<()> {
    let m: desc::Material = parse("material", d)?;
    w.set_material(id, m);
    Ok(())
}

pub fn object(w: &mut World, id: u32, j: &Value) -> Result<()> {
    let num = |k: &str| j[k].as_u64().map(|x| x as u32);
    // `body`: a rigid body will drive it (physics.rs). A mesh is then an instance of its own BLAS, so one added while
    // the game runs is placed in place and its moves are its instance's transform.
    let body = j["body"].as_bool().unwrap_or(false);
    let kind = match j["kind"].as_str().unwrap_or("group") {
        "mesh" => {
            let materials: Vec<u32> = j["materials"].as_array().map_or(vec![], |a| a.iter().filter_map(|v| v.as_u64().map(|x| x as u32)).collect());
            Kind::Mesh { geometry: num("geometry").unwrap_or(0), material: num("material").unwrap_or(0), own_blas: j["dynamicGeometry"].as_bool().unwrap_or(false) || (body && materials.len() <= 1), materials }
        }
        "model" => Kind::Model { uri: j["uri"].as_str().unwrap_or_default().to_string(), nodes: serde_json::from_value(j["nodes"].clone()).ok(), animation: serde_json::from_value(j["animation"].clone()).ok(), functions: serde_json::from_value(j["functions"].clone()).unwrap_or_default() },
        "light" => Kind::Light(serde_json::from_value(j["light"].clone()).map_err(|e| Error::Type(format!("engine.object light: {e}")))?),
        "camera" => Kind::Camera(serde_json::from_value(j["camera"].clone()).map_err(|e| Error::Type(format!("engine.object camera: {e}")))?),
        "volume" => Kind::Volume {
            function: j["volume"]["id"].as_str().unwrap_or_default().to_string(),
            params: j["volume"]["params"].as_array().map_or(vec![], |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()),
            shape: if j["shape"].as_str() == Some("sphere") { 1 } else { 0 },
            majorant: j["majorant"].as_f64().unwrap_or(0.0) as f32,
            g: j["g"].as_f64().unwrap_or(0.0) as f32,
            homogeneous: j["homogeneous"].as_bool().unwrap_or(false),
            animated: j["animated"].as_bool().unwrap_or(false),
            grid: num("grid"),
        },
        "instanced" => Kind::Instanced {
            geometry: num("geometry").unwrap_or(0),
            procedural: j["procedural"].as_bool().unwrap_or(false),
            materials: j["materials"].as_array().map_or(vec![], |a| a.iter().filter_map(|v| v.as_u64().map(|x| x as u32)).collect()),
            capacity: num("count").unwrap_or(0),
        },
        "procedural" => Kind::Procedural {
            geometry: num("geometry").unwrap_or(0),
            materials: j["materials"].as_array().map_or(vec![], |a| a.iter().filter_map(|v| v.as_u64().map(|x| x as u32)).collect()),
        },
        "gpuParticles" => Kind::GpuParticles {
            capacity: num("capacity").unwrap_or(0).min(1 << 21),
            program: j["program"]["id"].as_str().unwrap_or_default().to_string(),
            params: j["program"]["params"].as_array().map_or(vec![], |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()),
            material: num("material").unwrap_or(0),
            emitter_area: j["emitterArea"].as_f64().unwrap_or(0.0) as f32,
            rate: (j["rate"].as_f64().unwrap_or(0.0) as f32).max(0.0),
            collide: crate::world::ParticleCollide {
                plane: j["collide"]["plane"].as_f64().map(|v| v as f32),
                voxels: j["collide"]["voxels"].as_bool().unwrap_or(false),
                restitution: (j["collide"]["restitution"].as_f64().unwrap_or(0.3) as f32).clamp(0.0, 1.0),
                friction: (j["collide"]["friction"].as_f64().unwrap_or(0.4) as f32).clamp(0.0, 1.0),
            },
        },
        "particles" => Kind::Particles {
            count: num("count").unwrap_or(0),
            emitter: j["emitter"]["id"].as_str().unwrap_or_default().to_string(),
            params: j["emitter"]["params"].as_array().map_or(vec![], |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()),
            material: num("material").unwrap_or(0),
            emitter_area: j["emitterArea"].as_f64().unwrap_or(0.0) as f32,
        },
        _ => Kind::Group,
    };
    w.set_object(id, j["name"].as_str().unwrap_or_default().to_string(), num("parent"), kind, j["castShadow"].as_bool().unwrap_or(true), j["movable"].as_bool().unwrap_or(false))?;
    if body {
        w.set_driven(id)?;
    }
    Ok(())
}

/// (Ten numbers: position, rotation xyzw, scale.)
pub fn transform(w: &mut World, id: u32, t: &[f32]) -> Result<()> {
    if t.len() < 10 {
        return Err(Error::Type("engine.transform: needs 10 numbers".into()));
    }
    let q = Quat::from_xyzw(t[3], t[4], t[5], t[6]);
    Ok(w.set_transform(id, Vec3::new(t[0], t[1], t[2]), if q.length_squared() > 0.0 { q.normalize() } else { Quat::IDENTITY }, Vec3::new(t[7], t[8], t[9]))?)
}

pub fn custom_data(w: &mut World, id: u32, v: &[f64]) -> Result<()> {
    let mut c = [0.0f32; 4];
    for (k, x) in v.iter().take(4).enumerate() {
        c[k] = *x as f32;
    }
    Ok(w.set_custom(id, c)?)
}

/// (None: no sway.)
pub fn sway(w: &mut World, id: u32, d: Option<&Value>) -> Result<()> {
    let s: Option<crate::world::Sway> = d.map(|d| parse("sway", d)).transpose()?;
    Ok(w.set_sway(id, s)?)
}

pub fn light(w: &mut World, id: u32, d: &Value) -> Result<()> {
    let l: desc::Light = parse("light", d)?;
    Ok(w.set_light(id, l)?)
}

pub fn camera(w: &mut World, id: u32, d: &Value) -> Result<()> {
    let c: desc::Camera = parse("camera", d)?;
    Ok(w.set_camera_desc(id, c)?)
}

/// (None: no animation.)
pub fn animation(w: &mut World, id: u32, d: Option<&Value>) -> Result<()> {
    let a: Option<desc::Animation> = d.map(|d| parse("animation", d)).transpose()?;
    Ok(w.set_animation(id, a)?)
}

/// Removes an object and what is under it; their bodies go with them (a body asleep would never find out its object
/// went; the object's id may come again).
pub fn remove(w: &mut World, p: &mut Option<PhysicsWorld>, id: u32) {
    let gone = w.remove(id);
    if let Some(p) = p.as_mut() {
        p.objects_removed(&gone);
    }
}

pub fn environment(w: &mut World, d: &Value) -> Result<()> {
    let env: desc::Environment = parse("environment", d)?;
    w.set_environment(env);
    Ok(())
}

pub fn settings(w: &mut World, j: &Value) {
    let s = Settings {
        name: j["name"].as_str().unwrap_or("scene").to_string(),
        preset: j["preset"].as_str().unwrap_or("cinematic").to_string(),
        overrides: Some(j["overrides"].clone()).filter(|o| !o.is_null()),
        width: j["width"].as_u64().unwrap_or(1280) as u32,
        height: j["height"].as_u64().unwrap_or(720) as u32,
        time: j["time"].as_f64().unwrap_or(0.0) as f32,
        camera: j["camera"].as_u64().map(|c| c as u32),
    };
    w.set_settings(s);
}

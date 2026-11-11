//! The scene runtime of a render node. A game bundle (`tools/bundle-game.ts`: game code + the ThreeRT runtime + the
//! node host) runs in an embedded QuickJS engine in the renderer process, and drives the engine directly: the native
//! functions installed as `globalThis.__engine` create and edit the engine's world (world.rs) as the scene code's
//! binding (packages/scene/src/engine.ts) calls them. No scene document crosses between the two. `LiveCore::commit`
//! then applies what changed (GPU tables in place, or a rebuild).
//!
//! Sandbox: the engine has no I/O at all (no fs, network, timers or module loader); the host adds only a log sink, a
//! monotonic clock and the engine API. Memory is capped and every call runs under a time budget (the interrupt
//! handler aborts it).

use crate::desc;
use crate::world::{Geometry, Kind, Settings, Texture, TextureSource, World};
use anyhow::{anyhow, Result};
use glam::{Quat, Vec3};
use rquickjs::{CatchResultExt, Context, Ctx, Exception, Function, Module, Object, Runtime, TypedArray, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Web APIs the runtime and games use that QuickJS lacks (all pure).
const PRELUDE: &str = r#"
globalThis.console = {
  log: (...a) => __host_log("info", a.map(String).join(" ")),
  info: (...a) => __host_log("info", a.map(String).join(" ")),
  warn: (...a) => __host_log("warn", a.map(String).join(" ")),
  error: (...a) => __host_log("error", a.map(String).join(" ")),
  debug: () => {},
};
globalThis.performance = { now: () => __host_now() };
globalThis.structuredClone = (v) => (v === undefined ? undefined : JSON.parse(JSON.stringify(v)));
globalThis.TextEncoder = class TextEncoder {
  get encoding() { return "utf-8"; }
  encode(s = "") {
    s = String(s);
    const out = [];
    for (let i = 0; i < s.length; i++) {
      let c = s.charCodeAt(i);
      if (c >= 0xd800 && c < 0xdc00 && i + 1 < s.length) {
        const d = s.charCodeAt(i + 1);
        if (d >= 0xdc00 && d < 0xe000) { c = 0x10000 + ((c - 0xd800) << 10) + (d - 0xdc00); i++; }
      }
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return new Uint8Array(out);
  }
};
globalThis.atob = (s) => {
  const A = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  s = String(s).replace(/[\s=]/g, "");
  let out = "", buf = 0, bits = 0;
  for (let i = 0; i < s.length; i++) {
    const v = A.indexOf(s[i]);
    if (v < 0) throw new Error("atob: invalid character");
    buf = (buf << 6) | v; bits += 6;
    if (bits >= 8) { bits -= 8; out += String.fromCharCode((buf >> bits) & 255); }
  }
  return out;
};
"#;

/// Where the game's `console.*` lines go besides stderr (the client's log panel and dev server): `set_log_sink`.
static LOG_SINK: std::sync::RwLock<Option<fn(&str, &str)>> = std::sync::RwLock::new(None);

/// Also hands every game log line (`level`, `text`) to `sink` (one per process; None removes it).
pub fn set_log_sink(sink: Option<fn(&str, &str)>) {
    if let Ok(mut s) = LOG_SINK.write() {
        *s = sink;
    }
}

/// The game code a session runs: a one-script bundle (tools/bundle-game.ts), or a game module on the runtime module
/// (`ScriptHost::from_modules`, the cloud's layout; the clients' playground code).
pub enum GameCode<'a> {
    Bundle(&'a str),
    Modules { game: &'a str, runtime: &'a str },
}

/// Time budgets per entry point (ms): building a scene may take a while, events and frames may not.
const SETUP_MS: u64 = 120_000;
const EVENT_MS: u64 = 1_000;
const FRAME_MS: u64 = 50;

pub struct ScriptHost {
    _rt: Runtime,
    ctx: Context,
    /// Abort deadline in ms since `epoch` (0 = none), checked by the interrupt handler.
    deadline: Arc<AtomicU64>,
    epoch: Instant,
    /// `describe()`: handlers, keys, whether the game has a frame script.
    pub info: serde_json::Value,
    /// The engine's world the scene code edits through `__engine`.
    pub world: Rc<RefCell<World>>,
    /// The voxel engine's objects (`__engine.voxelLevel` / `voxelExplode`; renderer/src/voxel.rs).
    pub voxels: Rc<RefCell<crate::voxel::VoxelWorld>>,
    /// The physics world (`__engine.physics`; renderer/src/physics.rs): None until scene code makes a physics call.
    pub physics: Rc<RefCell<Option<crate::physics::PhysicsWorld>>>,
}

pub struct EventOut {
    pub log: Vec<String>,
    pub seq: i64,
    pub ms: f64,
}

fn host<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    ctx.globals().get("__threert")
}

/// A JS value as JSON (descriptor objects of the engine API).
fn to_json(v: &Value) -> rquickjs::Result<serde_json::Value> {
    use rquickjs::Type;
    Ok(match v.type_of() {
        Type::Bool => serde_json::Value::Bool(v.as_bool().unwrap_or(false)),
        Type::Int => serde_json::json!(v.as_int().unwrap_or(0)),
        Type::Float => serde_json::Number::from_f64(v.as_float().unwrap_or(0.0)).map_or(serde_json::Value::Null, serde_json::Value::Number),
        Type::String => serde_json::Value::String(v.as_string().map(|s| s.to_string()).transpose()?.unwrap_or_default()),
        Type::Array => {
            let a = v.as_array().expect("array");
            serde_json::Value::Array(a.iter::<Value>().map(|x| x.and_then(|x| to_json(&x))).collect::<rquickjs::Result<_>>()?)
        }
        Type::Object => {
            let o = v.as_object().expect("object");
            let mut m = serde_json::Map::new();
            for k in o.keys::<String>() {
                let k = k?;
                let x: Value = o.get(&k)?;
                if !x.is_undefined() {
                    m.insert(k, to_json(&x)?);
                }
            }
            serde_json::Value::Object(m)
        }
        _ => serde_json::Value::Null,
    })
}

fn parse<T: serde::de::DeserializeOwned>(ctx: &Ctx, what: &str, v: &Value) -> rquickjs::Result<T> {
    serde_json::from_value(to_json(v)?).map_err(|e| Exception::throw_type(ctx, &format!("engine.{what}: {e}")))
}

fn fail(ctx: &Ctx, e: anyhow::Error) -> rquickjs::Error {
    Exception::throw_message(ctx, &format!("{e:#}"))
}

/// The engine API (`globalThis.__engine`), see packages/scene/src/engine.ts.
fn install_engine<'js>(ctx: &Ctx<'js>, world: &Rc<RefCell<World>>, voxels: &Rc<RefCell<crate::voxel::VoxelWorld>>, physics: &Rc<RefCell<Option<crate::physics::PhysicsWorld>>>) -> rquickjs::Result<()> {
    let e = Object::new(ctx.clone())?;
    // Physics (renderer/src/physics.rs): one call, `physics(op, args)`, answering in JSON. The world is made by the
    // first call; the engine steps it, scene code cannot.
    let (p, w) = (physics.clone(), world.clone());
    e.set("physics", Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, args: Value<'js>| -> rquickjs::Result<String> {
        let a: serde_json::Value = if args.is_undefined() || args.is_null() { serde_json::Value::Null } else { parse(&ctx, "physics", &args)? };
        let out = p.borrow_mut().get_or_insert_with(Default::default).call(&mut w.borrow_mut(), &op, &a).map_err(|e| fail(&ctx, e))?;
        Ok(out.to_string())
    })?)?;
    // The poses of the objects that bodies drive, by the slot the binding gave each body (7 floats: position, rotation
    // xyzw, as scene code reads an object's): one copy for all a frame reads. Empty without a world.
    let p = physics.clone();
    e.set("physicsPoses", Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<TypedArray<'js, f32>> { TypedArray::new(ctx, p.borrow().as_ref().map_or(vec![], |p| p.poses().to_vec())) })?)?;
    // The voxel engine's level and explosions (renderer/src/voxel.rs): the level is loaded natively once the session
    // knows the asset root; an explosion returns what it did as JSON.
    let v = voxels.clone();
    e.set("voxelLevel", Function::new(ctx.clone(), move |ctx: Ctx, d: Value| -> rquickjs::Result<()> {
        let d: crate::voxel::LevelDesc = parse(&ctx, "voxelLevel", &d)?;
        v.borrow_mut().request(d).map_err(|e| fail(&ctx, e))
    })?)?;
    let (v, w) = (voxels.clone(), world.clone());
    e.set("voxelExplode", Function::new(ctx.clone(), move |ctx: Ctx, x: f64, y: f64, z: f64, r: f64| -> rquickjs::Result<String> {
        let out = v.borrow_mut().explode(&mut w.borrow_mut(), glam::Vec3::new(x as f32, y as f32, z as f32), r as f32).map_err(|e| fail(&ctx, e))?;
        Ok(out.to_string())
    })?)?;
    // An edit of the level (`voxelEdit({ op, sphere | box | capsule, material })`; voxel.rs `EditOp`), and the level's
    // hash (16 hex digits; "" before a level): the same commands give the same hash.
    let (v, w) = (voxels.clone(), world.clone());
    e.set("voxelEdit", Function::new(ctx.clone(), move |ctx: Ctx<'js>, d: Value<'js>| -> rquickjs::Result<String> {
        let j: serde_json::Value = parse(&ctx, "voxelEdit", &d)?;
        let op = crate::voxel::EditOp::parse(&j).map_err(|e| fail(&ctx, e))?;
        let out = v.borrow_mut().edit(&mut w.borrow_mut(), &op).map_err(|e| fail(&ctx, e))?;
        Ok(out.to_string())
    })?)?;
    let v = voxels.clone();
    e.set("voxelHash", Function::new(ctx.clone(), move || -> String { v.borrow().hash().map_or(String::new(), |h| format!("{h:016x}")) })?)?;
    let v = voxels.clone();
    e.set("voxelDebris", Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<TypedArray<'js, f32>> { TypedArray::new(ctx, v.borrow_mut().take_debris()) })?)?;
    let (v, w) = (voxels.clone(), world.clone());
    e.set("voxelFrame", Function::new(ctx.clone(), move |ctx: Ctx, dt: f64| -> rquickjs::Result<String> {
        let out = v.borrow_mut().frame(&mut w.borrow_mut(), dt).map_err(|e| fail(&ctx, e))?;
        Ok(out.to_string())
    })?)?;
    let w = world.clone();
    // (…, groups?, colors?): the optional arguments as the rest (rquickjs takes seven parameters at most).
    e.set("geometry", Function::new(ctx.clone(), move |ctx: Ctx<'js>, id: u32, p: TypedArray<f32>, n: TypedArray<f32>, uv: TypedArray<f32>, idx: TypedArray<u32>, rest: rquickjs::function::Rest<Value<'js>>| -> rquickjs::Result<()> {
        let opt = |k: usize| rest.0.get(k).filter(|v| !v.is_null() && !v.is_undefined());
        let groups = match opt(0) {
            Some(v) => AsRef::<[u32]>::as_ref(&TypedArray::<u32>::from_value(v.clone())?).chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
            None => vec![],
        };
        let colors = match opt(1) {
            Some(v) => AsRef::<[f32]>::as_ref(&TypedArray::<f32>::from_value(v.clone())?).to_vec(),
            None => vec![],
        };
        let g = Geometry { positions: AsRef::<[f32]>::as_ref(&p).to_vec(), normals: AsRef::<[f32]>::as_ref(&n).to_vec(), uvs: AsRef::<[f32]>::as_ref(&uv).to_vec(), indices: AsRef::<[u32]>::as_ref(&idx).to_vec(), groups, colors };
        w.borrow_mut().set_geometry(id, g).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("texture", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value, data: Option<TypedArray<u8>>| -> rquickjs::Result<()> {
        let j = to_json(&d)?;
        let pair = |k: &str, def: f32| -> [f32; 2] { [0, 1].map(|i| j[k][i].as_f64().map_or(def, |x| x as f32)) };
        let source = match (data, j["uri"].as_str()) {
            (Some(bytes), _) if j["size"].is_array() => {
                let (width, height) = (j["size"][0].as_u64().unwrap_or(0) as u32, j["size"][1].as_u64().unwrap_or(0) as u32);
                let rgba = AsRef::<[u8]>::as_ref(&bytes).to_vec();
                if rgba.len() != width as usize * height as usize * 4 {
                    return Err(Exception::throw_type(&ctx, &format!("engine.texture: {} bytes are not {width}x{height} RGBA8 pixels", rgba.len())));
                }
                TextureSource::Pixels { width, height, rgba: Arc::new(rgba) }
            }
            (Some(bytes), _) => TextureSource::Encoded(Arc::new(AsRef::<[u8]>::as_ref(&bytes).to_vec())),
            (None, Some(uri)) => TextureSource::File(uri.to_string()),
            (None, None) => return Err(Exception::throw_type(&ctx, "engine.texture: needs a uri or data")),
        };
        let t = Texture { source, srgb: j["srgb"].as_bool().unwrap_or(true), wrap: j["wrap"].as_bool().unwrap_or(true), repeat: pair("repeat", 1.0), offset: pair("offset", 0.0) };
        w.borrow_mut().set_texture(id, t).map_err(|e| fail(&ctx, e))?;
        Ok(())
    })?)?;
    let w = world.clone();
    e.set("shader", Function::new(ctx.clone(), move |name: String, code: String, helpers: String| w.borrow_mut().set_shader(&name, code, helpers))?)?;
    let w = world.clone();
    e.set("particleFunction", Function::new(ctx.clone(), move |name: String, code: String, helpers: String| w.borrow_mut().set_particle_fn(&name, code, helpers))?)?;
    let w = world.clone();
    e.set("particleProgram", Function::new(ctx.clone(), move |name: String, emit: String, update: String, helpers: String| w.borrow_mut().set_particle_program(&name, emit, update, helpers))?)?;
    let w = world.clone();
    e.set("particlesSpawn", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, data: TypedArray<f32>| -> rquickjs::Result<()> { w.borrow_mut().spawn_particles(id, data.as_ref()).map_err(|e| fail(&ctx, e)) })?)?;
    // Density grids (ROADMAP § 6 step 5b): a grid program's WGSL, a grid (its size, channels, program and data), and
    // the sources the game gives a volume's grid program.
    let w = world.clone();
    e.set("gridProgram", Function::new(ctx.clone(), move |name: String, code: String, helpers: String| w.borrow_mut().set_grid_program(&name, code, helpers))?)?;
    let w = world.clone();
    e.set("densityGrid", Function::new(ctx.clone(), move |ctx: Ctx<'js>, id: u32, d: Value<'js>, data: Value<'js>| -> rquickjs::Result<()> {
        let j: serde_json::Value = parse(&ctx, "densityGrid", &d)?;
        let n = |k: usize| j["size"][k].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32;
        let program = j["program"]["id"].as_str().map(|id| (id.to_string(), j["program"]["params"].as_array().map_or(vec![], |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())));
        let data = if data.is_null() || data.is_undefined() { None } else { Some(TypedArray::<f32>::from_value(data)?) };
        w.borrow_mut().set_grid(id, [n(0), n(1), n(2)], j["channels"].as_u64().unwrap_or(1) as u32, data.as_ref().map(|d| d.as_ref()), program).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("gridSources", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, data: TypedArray<f32>| -> rquickjs::Result<()> { w.borrow_mut().add_grid_sources(id, data.as_ref()).map_err(|e| fail(&ctx, e)) })?)?;
    let w = world.clone();
    e.set("volumeFunction", Function::new(ctx.clone(), move |name: String, code: String, helpers: String| w.borrow_mut().set_volume_fn(&name, code, helpers))?)?;
    let w = world.clone();
    e.set("geometryFunction", Function::new(ctx.clone(), move |name: String, code: String, helpers: String| w.borrow_mut().set_geometry_fn(&name, code, helpers))?)?;
    let w = world.clone();
    e.set("procedural", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, function: String, boxes: TypedArray<f32>, data: TypedArray<u32>| -> rquickjs::Result<()> {
        w.borrow_mut().set_proc_geometry(id, function, AsRef::<[f32]>::as_ref(&boxes).to_vec(), AsRef::<[u32]>::as_ref(&data).to_vec()).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("proceduralAsset", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, function: String, uri: String| -> rquickjs::Result<()> {
        w.borrow_mut().set_proc_asset(id, function, uri).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("material", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> {
        let m: desc::Material = parse(&ctx, "material", &d)?;
        w.borrow_mut().set_material(id, m);
        Ok(())
    })?)?;
    let w = world.clone();
    e.set("object", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> {
        let j = to_json(&d)?;
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
            "light" => Kind::Light(serde_json::from_value(j["light"].clone()).map_err(|e| Exception::throw_type(&ctx, &format!("engine.object light: {e}")))?),
            "camera" => Kind::Camera(serde_json::from_value(j["camera"].clone()).map_err(|e| Exception::throw_type(&ctx, &format!("engine.object camera: {e}")))?),
            "volume" => Kind::Volume {
                function: j["volume"]["id"].as_str().unwrap_or_default().to_string(),
                params: j["volume"]["params"].as_array().map_or(vec![], |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()),
                shape: if j["shape"].as_str() == Some("sphere") { 1 } else { 0 },
                majorant: j["majorant"].as_f64().unwrap_or(0.0) as f32,
                g: j["g"].as_f64().unwrap_or(0.0) as f32,
                homogeneous: j["homogeneous"].as_bool().unwrap_or(false),
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
        let mut w = w.borrow_mut();
        w.set_object(id, j["name"].as_str().unwrap_or_default().to_string(), num("parent"), kind, j["castShadow"].as_bool().unwrap_or(true), j["movable"].as_bool().unwrap_or(false)).map_err(|e| fail(&ctx, e))?;
        if body {
            w.set_driven(id).map_err(|e| fail(&ctx, e))?;
        }
        Ok(())
    })?)?;
    let w = world.clone();
    e.set("transform", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, t: TypedArray<f32>| -> rquickjs::Result<()> {
        let t: &[f32] = t.as_ref();
        if t.len() < 10 {
            return Err(Exception::throw_type(&ctx, "engine.transform: needs 10 numbers"));
        }
        let q = Quat::from_xyzw(t[3], t[4], t[5], t[6]);
        w.borrow_mut().set_transform(id, Vec3::new(t[0], t[1], t[2]), if q.length_squared() > 0.0 { q.normalize() } else { Quat::IDENTITY }, Vec3::new(t[7], t[8], t[9])).map_err(|e| fail(&ctx, e))
    })?)?;
    // An instanced mesh's instances: count drawn, then matrices (16 numbers each), colours (3) and per-instance data
    // (4) for its whole capacity, each null to keep what it had (world.rs `set_instances` checks them).
    let w = world.clone();
    e.set("instances", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, count: u32, m: Option<TypedArray<f32>>, c: Option<TypedArray<f32>>, d: Option<TypedArray<f32>>| -> rquickjs::Result<()> {
        let v = |a: Option<TypedArray<f32>>| a.map(|a| AsRef::<[f32]>::as_ref(&a).to_vec());
        w.borrow_mut().set_instances(id, count, v(m), v(c), v(d)).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("visible", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, v: bool| w.borrow_mut().set_visible(id, v).map_err(|e| fail(&ctx, e)))?)?;
    let w = world.clone();
    e.set("power", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, p: f64| w.borrow_mut().set_power(id, p as f32).map_err(|e| fail(&ctx, e)))?)?;
    let w = world.clone();
    e.set("customData", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, v: Vec<f64>| {
        let mut c = [0.0f32; 4];
        for (k, x) in v.iter().take(4).enumerate() {
            c[k] = *x as f32;
        }
        w.borrow_mut().set_custom(id, c).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("sway", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> {
        let s: Option<crate::world::Sway> = if d.is_null() || d.is_undefined() { None } else { Some(parse(&ctx, "sway", &d)?) };
        w.borrow_mut().set_sway(id, s).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("light", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> {
        let l: desc::Light = parse(&ctx, "light", &d)?;
        w.borrow_mut().set_light(id, l).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("camera", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> {
        let c: desc::Camera = parse(&ctx, "camera", &d)?;
        w.borrow_mut().set_camera_desc(id, c).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("animation", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> {
        let a: Option<desc::Animation> = if d.is_null() || d.is_undefined() { None } else { Some(parse(&ctx, "animation", &d)?) };
        w.borrow_mut().set_animation(id, a).map_err(|e| fail(&ctx, e))
    })?)?;
    let w = world.clone();
    let p = physics.clone();
    e.set("remove", Function::new(ctx.clone(), move |id: u32| {
        let gone = w.borrow_mut().remove(id);
        // (A body asleep would never find out its object went; the object's id may come again.)
        if let Some(p) = p.borrow_mut().as_mut() {
            p.objects_removed(&gone);
        }
    })?)?;
    let w = world.clone();
    e.set("environment", Function::new(ctx.clone(), move |ctx: Ctx, d: Value| -> rquickjs::Result<()> {
        let env: desc::Environment = parse(&ctx, "environment", &d)?;
        w.borrow_mut().set_environment(env);
        Ok(())
    })?)?;
    let w = world.clone();
    e.set("settings", Function::new(ctx.clone(), move |d: Value| -> rquickjs::Result<()> {
        let j = to_json(&d)?;
        let s = Settings {
            name: j["name"].as_str().unwrap_or("scene").to_string(),
            preset: j["preset"].as_str().unwrap_or("cinematic").to_string(),
            overrides: Some(j["overrides"].clone()).filter(|o| !o.is_null()),
            width: j["width"].as_u64().unwrap_or(1280) as u32,
            height: j["height"].as_u64().unwrap_or(720) as u32,
            time: j["time"].as_f64().unwrap_or(0.0) as f32,
            camera: j["camera"].as_u64().map(|c| c as u32),
        };
        w.borrow_mut().set_settings(s);
        Ok(())
    })?)?;
    ctx.globals().set("__engine", e)
}

/// Entry of a game given as ES modules: the game (or a scene module) on the node host of the runtime module.
const MODULE_ENTRY: &str = r#"
import game from "game.js";
import { defineGame, installNodeHost } from "threert-runtime.js";
installNodeHost(typeof game === "function" ? defineGame({ state: {}, setup: () => game(), handlers: {} }) : game);
"#;

impl ScriptHost {
    /// Loads a game bundle: one script, game + runtime (tools/bundle-game.ts); it installs `globalThis.__threert`.
    pub fn new(bundle: &str) -> Result<ScriptHost> {
        Self::load(|ctx| ctx.eval::<(), _>(bundle))
    }

    /// Loads a game as ES modules: `game.js` (agent code, importing `./threert-runtime.js`) and the runtime module
    /// (out/threert-runtime.js, `node tools/build-runtime.ts`).
    pub fn from_modules(game: &str, runtime: &str) -> Result<ScriptHost> {
        Self::load(|ctx| {
            Module::declare(ctx.clone(), "threert-runtime.js", runtime)?;
            Module::declare(ctx.clone(), "game.js", game)?;
            Module::evaluate(ctx.clone(), "threert-entry.js", MODULE_ENTRY)?.finish::<()>()
        })
    }

    fn load(code: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<()>) -> Result<ScriptHost> {
        let rt = Runtime::new()?;
        rt.set_memory_limit(1 << 30);
        rt.set_max_stack_size(4 << 20);
        let deadline = Arc::new(AtomicU64::new(0));
        let epoch = Instant::now();
        let d = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            let limit = d.load(Ordering::Relaxed);
            limit != 0 && epoch.elapsed().as_millis() as u64 > limit
        })));
        let ctx = Context::full(&rt)?;
        let world = Rc::new(RefCell::new(World::default()));
        let voxels = Rc::new(RefCell::new(crate::voxel::VoxelWorld::default()));
        let mut h = ScriptHost { _rt: rt, ctx, deadline, epoch, info: serde_json::Value::Null, world, voxels, physics: Default::default() };
        let (w, v, ph) = (h.world.clone(), h.voxels.clone(), h.physics.clone());
        let info: String = h.call(SETUP_MS, |ctx| {
            let g = ctx.globals();
            g.set("__host_log", Function::new(ctx.clone(), |level: String, msg: String| {
                eprintln!("[game{}] {msg}", if level == "info" { String::new() } else { format!(" {level}") });
                if let Some(sink) = LOG_SINK.read().ok().and_then(|s| *s) {
                    sink(&level, &msg);
                }
            })?)?;
            g.set("__host_now", Function::new(ctx.clone(), move || epoch.elapsed().as_secs_f64() * 1000.0)?)?;
            install_engine(ctx, &w, &v, &ph)?;
            ctx.eval::<(), _>(PRELUDE)?;
            code(ctx)?;
            host(ctx)?.get::<_, Function>("describe")?.call(())
        })?;
        h.info = serde_json::from_str(&info)?;
        Ok(h)
    }

    /// Runs `f` in the engine under a time budget; JS exceptions become errors with their message and stack.
    fn call<T>(&self, budget_ms: u64, f: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<T>) -> Result<T> {
        self.deadline.store(self.epoch.elapsed().as_millis() as u64 + budget_ms, Ordering::Relaxed);
        let r = self.ctx.with(|ctx| f(&ctx).catch(&ctx).map_err(|e| anyhow!("{e}")));
        self.deadline.store(0, Ordering::Relaxed);
        r
    }

    fn call_host<T: for<'js> rquickjs::FromJs<'js>>(&self, budget_ms: u64, name: &str, args: impl for<'js> rquickjs::function::IntoArgs<'js>) -> Result<T> {
        self.call(budget_ms, |ctx| host(ctx)?.get::<_, Function>(name)?.call(args))
    }

    pub fn has_frame(&self) -> bool {
        self.info["frame"].as_bool().unwrap_or(false)
    }

    /// Handler bound to a key ("R", "Space"), if any.
    pub fn handler_for_key(&self, key: &str) -> Option<String> {
        self.info["keys"][key].as_str().map(String::from)
    }

    /// The game handles events of this type: with a handler (a transaction on the state) or a world handler (`on`:
    /// it changes the scene on this node; packages/gameplay/src/world.ts).
    pub fn has_handler(&self, name: &str) -> bool {
        ["handlers", "on"].iter().any(|k| self.info[k].as_array().is_some_and(|h| h.iter().any(|x| x == name)))
    }

    /// The node host runs every frame (`frame`): the game has a frame script, or world handlers (they read the
    /// frame clock), or the scene has a physics world (its contacts and the bodies that left the world are delivered
    /// to their listeners once a frame, whoever registered them: a frame script, a world handler or setup). A scene
    /// with none of them pays nothing.
    pub fn frames(&self) -> bool {
        self.has_frame() || self.info["on"].as_array().is_some_and(|h| !h.is_empty()) || self.physics.borrow().is_some()
    }

    /// Builds and binds the scene (deterministic for a room id / seed); returns the time it took, ms.
    pub fn setup(&self, room: &str, seed: Option<u32>) -> Result<f64> {
        let t = Instant::now();
        let _: String = self.call_host(SETUP_MS, "setup", (room, seed))?;
        Ok(t.elapsed().as_secs_f64() * 1000.0)
    }

    /// One client event on this node's own store (local play: the node is the authority).
    pub fn event(&self, kind: &str, payload: &serde_json::Value) -> Result<EventOut> {
        let t = Instant::now();
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64);
        let json: String = self.call_host(EVENT_MS, "event", (kind, payload.to_string(), ts))?;
        let v: serde_json::Value = serde_json::from_str(&json)?;
        Ok(EventOut { log: serde_json::from_value(v["log"].clone()).unwrap_or_default(), seq: v["seq"].as_i64().unwrap_or(-1), ms: t.elapsed().as_secs_f64() * 1000.0 })
    }

    /// State writes another authority (the room) committed.
    pub fn commit(&self, writes: &serde_json::Value, seq: i64) -> Result<()> {
        self.call_host::<()>(EVENT_MS, "commit", (writes.to_string(), seq))
    }

    /// Replaces the game state (a render of a room's current state).
    pub fn set_state(&self, state: &serde_json::Value, seq: i64) -> Result<()> {
        self.call_host::<()>(EVENT_MS, "setState", (state.to_string(), seq))
    }

    /// The committed game state and its sequence number.
    pub fn state(&self) -> Result<(serde_json::Value, i64)> {
        let s: String = self.call_host(EVENT_MS, "state", ())?;
        let v: serde_json::Value = serde_json::from_str(&s)?;
        Ok((v["state"].clone(), v["seq"].as_i64().unwrap_or(0)))
    }

    /// Sets the frame script's clock to `t` (s) and shows the scene settled at that moment (an offline render at a scene
    /// time; frames from there advance it as before).
    /// Bundles built before `seek` existed have none: nothing to do then.
    pub fn seek(&self, t: f32) -> Result<()> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64);
        self.call(FRAME_MS, |ctx| match host(ctx)?.get::<_, Option<Function>>("seek")? {
            Some(f) => f.call((t as f64, now)),
            None => Ok(()),
        })
    }

    /// A frame of the physics world, if scene code made one: its ticks, and the objects its bodies drive placed
    /// (renderer/src/physics.rs). True if there is a world (the caller commits what moved).
    pub fn advance_physics(&self, dt: f32) -> bool {
        match self.physics.borrow_mut().as_mut() {
            Some(p) => {
                p.advance(&mut self.world.borrow_mut(), dt as f64);
                true
            }
            None => false,
        }
    }

    /// The physics world `ticks` ticks on (an offline render at a scene time).
    pub fn settle_physics(&self, ticks: u32) {
        if let Some(p) = self.physics.borrow_mut().as_mut() {
            p.settle(&mut self.world.borrow_mut(), ticks);
        }
    }

    /// Runs the frame script for one rendered frame.
    pub fn frame(&self, dt: f32) -> Result<()> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64);
        self.call_host::<()>(FRAME_MS, "frame", (dt as f64, now))
    }

    /// The live scene's objects (editor outliner), JSON.
    pub fn outline(&self) -> Result<String> {
        self.call_host(EVENT_MS, "outline", ())
    }

    /// One object's editable properties, JSON.
    pub fn inspect(&self, name: &str) -> Result<String> {
        self.call_host(EVENT_MS, "inspect", (name,))
    }

    /// Edits one property of a live object (`value` JSON).
    pub fn edit(&self, name: &str, prop: &str, value: &serde_json::Value) -> Result<()> {
        self.call_host::<()>(EVENT_MS, "edit", (name, prop, value.to_string()))
    }
}

/// A game running on this node with its live view: the scene built in the embedded runtime, events (local authority)
/// and the frame script committed to the renderer. Shared by the native window (`play`), the headless runner and the
/// playground.
pub struct GameSession {
    pub host: ScriptHost,
    pub seq: i64,
    pub setup_ms: f64,
    /// Frame script + commit cost of the last frame, ms.
    pub frame_ms: f32,
    /// Of `frame_ms`, the frame script's own JavaScript (the rest: the physics' ticks, and applying what both changed,
    /// `LiveCore::commit`: `commit_ms`).
    pub js_ms: f32,
    pub commit_ms: f32,
    /// Last handler log line (window title).
    pub status: String,
    /// The frame script's clock (`f.time`).
    clock: GameClock,
    /// When the last click arrived, until its first frame is drawn (`click_shown`; THREERT_CLICK_LOG=1).
    clicked: Option<Instant>,
    /// Whether the last event's commit rebuilt the scene.
    rebuilt: bool,
}

/// THREERT_CLICK_LOG=1: a click's chain timed on stderr (`click: ...`): the pick, the script (handler, frame script,
/// binding; of which the physics world's calls), the commit, and the first frame after it.
fn click_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("THREERT_CLICK_LOG").is_ok_and(|v| v == "1"))
}

/// The frame script's clock: the gameplay FrameRunner adds each frame's dt clamped to [0, 0.1] s, in f64. While a
/// frame script runs, the scene time is this clock, whatever advanced the scene between its frames (the playground
/// animates the scene while editing, and runs the frame script only in Play).
#[derive(Default)]
struct GameClock(f64);

impl GameClock {
    fn tick(&mut self, dt: f32) -> f32 {
        self.0 += (dt as f64).clamp(0.0, 0.1);
        self.0 as f32
    }
}

impl GameSession {
    /// Loads the bundle and builds the scene in the runtime.
    fn load(bundle: &std::path::Path, room: &str) -> Result<GameSession> {
        Self::load_code(GameCode::Bundle(&std::fs::read_to_string(bundle).map_err(|e| anyhow!("{}: {e}", bundle.display()))?), room)
    }

    /// Loads game code and builds the scene in the runtime, without a live view yet: a client can fetch the files the
    /// scene reads (`files`) before `start_loaded` / `open_loaded` build the GPU scene from them.
    pub fn load_code(code: GameCode, room: &str) -> Result<GameSession> {
        let host = match code {
            GameCode::Bundle(b) => ScriptHost::new(b)?,
            GameCode::Modules { game, runtime } => ScriptHost::from_modules(game, runtime)?,
        };
        let setup_ms = host.setup(room, None)?;
        host.world.borrow_mut().take_changes();
        eprintln!("game: setup {setup_ms:.0} ms in the node runtime, {} objects, handlers {}, frame script {}", host.world.borrow().objects.len(), host.info["handlers"], host.has_frame());
        Ok(GameSession { host, seq: 0, setup_ms, frame_ms: 0.0, js_ms: 0.0, commit_ms: 0.0, status: String::new(), clock: GameClock::default(), clicked: None, rebuilt: false })
    }

    /// Loads the bundle, builds the scene in the runtime and opens a live view of it. Models and file textures resolve
    /// against `assets`.
    pub fn start(bundle: &std::path::Path, room: &str, assets: &std::path::Path, g: crate::gpu::Gpu, presets: &std::path::Path, w: u32, h: u32) -> Result<(GameSession, crate::live::LiveCore)> {
        Self::load(bundle, room)?.start_loaded(assets, g, presets, w, h)
    }

    /// Loads the bundle into an existing live view (history cleared, camera from the scene).
    pub fn open(bundle: &std::path::Path, room: &str, assets: &std::path::Path, core: &mut crate::live::LiveCore) -> Result<GameSession> {
        Self::load(bundle, room)?.open_loaded(assets, core)
    }

    /// Files the built scene reads from the asset root (models, file textures, procedural assets).
    pub fn files(&self) -> Vec<String> {
        self.host.world.borrow().files()
    }

    /// Opens the first live view of a loaded session (`load_code`).
    pub fn start_loaded(self, assets: &std::path::Path, g: crate::gpu::Gpu, presets: &std::path::Path, w: u32, h: u32) -> Result<(GameSession, crate::live::LiveCore)> {
        self.realize_voxels(assets)?;
        let core = crate::live::LiveCore::new(g, &self.host.world.borrow(), assets, presets, w, h)?;
        Ok((self, core))
    }

    /// Shows a loaded session (`load_code`) in an existing live view (history cleared, camera from the scene).
    pub fn open_loaded(self, assets: &std::path::Path, core: &mut crate::live::LiveCore) -> Result<GameSession> {
        self.realize_voxels(assets)?;
        core.open(&self.host.world.borrow(), assets)?;
        core.seq = 0;
        Ok(self)
    }

    /// Loads the voxel level the scene code asked for (`voxelLevel`) now that the asset root is known. The scene build
    /// that follows includes it, so its changes are dropped here (as the setup's are, `load`).
    fn realize_voxels(&self, assets: &std::path::Path) -> Result<()> {
        let mut w = self.host.world.borrow_mut();
        if let Some(r) = self.host.voxels.borrow_mut().realize(&mut w, assets)? {
            eprintln!("voxels: {r}");
            w.take_changes();
        }
        Ok(())
    }

    /// An editor edit of the live scene (see the node host's `edit`), applied to the view.
    pub fn edit(&mut self, core: &mut crate::live::LiveCore, name: &str, path: &str, value: &serde_json::Value) -> Result<bool> {
        self.host.edit(name, path, value)?;
        self.commit(core)
    }

    /// Applies what the scene code changed to the live view.
    pub fn commit(&mut self, core: &mut crate::live::LiveCore) -> Result<bool> {
        core.commit(&mut self.host.world.borrow_mut())
    }

    /// Runs a handler (and its follow-ups); the frame script updates the scene to the new state.
    pub fn event(&mut self, core: &mut crate::live::LiveCore, kind: &str, payload: serde_json::Value) -> Result<EventOut> {
        let e = self.host.event(kind, &payload)?;
        let t = Instant::now();
        let rebuilt = self.commit(core)?;
        self.rebuilt = rebuilt;
        self.seq = e.seq;
        core.seq = e.seq;
        for l in &e.log {
            eprintln!("game: {l} · {:.1} ms handler, {:.2} ms {}", e.ms, t.elapsed().as_secs_f64() * 1000.0, if rebuilt { "rebuild" } else { "commit" });
        }
        if let Some(l) = e.log.last() {
            self.status = l.clone();
        }
        Ok(e)
    }

    /// A click at (u, v) of the image: picks the object under it and sends `click {target, point, player}`, with
    /// `instanceId` for an instance of an instanced mesh (three.js's intersection field).
    pub fn click(&mut self, core: &mut crate::live::LiveCore, u: f32, v: f32) -> Result<Option<EventOut>> {
        if !self.host.has_handler("click") {
            return Ok(None);
        }
        let t0 = Instant::now();
        let Some((target, p, sub)) = core.pick(u, v) else { return Ok(None) };
        let pick_ms = t0.elapsed().as_secs_f64() * 1e3;
        let r = |x: f32| (x * 1e4).round() / 1e4;
        // (`ray`: the view ray through the click, for a `Raycaster` in scene code: `raycaster.set(origin, direction)`.)
        let (o, d) = core.ray(u, v);
        let mut payload = serde_json::json!({ "target": target, "point": [r(p.x), r(p.y), r(p.z)], "player": "you", "ray": { "origin": o.to_array(), "direction": d.to_array() } });
        if let Some(i) = sub {
            payload["instanceId"] = i.into();
        }
        if !click_log() {
            return self.event(core, "click", payload).map(Some);
        }
        if let Some(p) = self.host.physics.borrow_mut().as_mut() {
            p.take_call_ms();
        }
        let t1 = Instant::now();
        let e = self.event(core, "click", payload)?;
        let all = t1.elapsed().as_secs_f64() * 1e3;
        let physics = self.host.physics.borrow_mut().as_mut().map_or(0.0, |p| p.take_call_ms());
        eprintln!("click: pick {pick_ms:.3} ms, script {:.3} ms (the physics world's calls {physics:.3} ms), commit {:.3} ms ({})", e.ms, all - e.ms, if self.rebuilt { "rebuild" } else { "in place" });
        self.clicked = Some(t0);
        Ok(Some(e))
    }

    /// A frame was drawn: the first one after a click is timed (THREERT_CLICK_LOG=1).
    pub fn click_shown(&mut self) {
        if let Some(t) = self.clicked.take() {
            eprintln!("click: first frame submitted {:.3} ms after the click", t.elapsed().as_secs_f64() * 1e3);
        }
    }

    /// Runs the frame script and applies what it changed; true if that rebuilt the scene. A frame script (or its
    /// sync) that throws does not stop the world: the frame's physics ticks are made and what changed is committed all
    /// the same, then the error is returned (the window and `live` print it and go on).
    pub fn frame(&mut self, core: &mut crate::live::LiveCore, dt: f32) -> Result<bool> {
        if !self.host.frames() {
            // Wind sway and the physics follow the scene time with or without a frame script.
            let swayed = self.host.world.borrow_mut().apply_sway(core.scene_time());
            if self.host.advance_physics(dt) || swayed {
                return self.commit(core);
            }
            return Ok(false);
        }
        let t = Instant::now();
        // (A frame script's clock is the game's; a game without one keeps the view's own time, as before.)
        if self.host.has_frame() {
            core.set_time(self.clock.tick(dt));
        }
        self.host.world.borrow_mut().apply_sway(core.scene_time());
        let scripted = self.host.frame(dt);
        self.js_ms = t.elapsed().as_secs_f32() * 1000.0;
        // The physics' ticks after the frame script (its impulses and targets act in this frame's ticks).
        self.host.advance_physics(dt);
        let tc = Instant::now();
        let rebuilt = self.commit(core)?;
        self.commit_ms = tc.elapsed().as_secs_f32() * 1000.0;
        self.frame_ms = t.elapsed().as_secs_f32() * 1000.0;
        scripted?;
        Ok(rebuilt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A hand-written bundle: what the binding (packages/scene/src/engine.ts) does, reduced to engine calls.
    const GAME: &str = r#"
        let power = 1, frames = 0;
        const quad = (id) => __engine.geometry(id, new Float32Array([0,0,0, 1,0,0, 0,1,0]), new Float32Array([0,0,1, 0,0,1, 0,0,1]), new Float32Array([0,0, 1,0, 0,1]), new Uint32Array([0,1,2]));
        globalThis.__threert = {
            describe: () => JSON.stringify({ handlers: ["click"], frame: true, keys: { L: "click" } }),
            setup: (room, seed) => {
                quad(1);
                __engine.texture(2, { srgb: true, wrap: true, repeat: [1, 1], offset: [0, 0] }, new Uint8Array([1, 2, 3]).subarray(1));
                __engine.material(3, { name: "m", color: [1, 1, 1], opacity: 1, metalness: 0, roughness: 0.5, emissive: [1, 1, 1], emissiveIntensity: 10, doubleSided: false, map: 2 });
                __engine.object(4, { name: "sign", parent: null, castShadow: true, kind: "mesh", geometry: 1, material: 3 });
                __engine.object(5, { name: "cam", parent: null, castShadow: false, kind: "camera", camera: { fov: 50, near: 0.1, far: 100, exposure: { mode: "manual", ev100: 0 } } });
                __engine.transform(5, new Float32Array([0, 0, 5, 0, 0, 0, 1, 1, 1, 1]));
                __engine.environment({ sky: { type: "color", color: [0, 0, 0], intensity: 0, skyLightIntensity: 1 }, wetness: new TextEncoder().encode("é€").length / 10 });
                __engine.settings({ name: room + seed, preset: "cinematic", width: 64, height: 36, time: 0, camera: 5 });
                return "{}";
            },
            event: (type, payload, ts) => { power = 0; __engine.power(4, power); return JSON.stringify({ log: [type + " " + JSON.parse(payload).target], seq: 1 }); },
            commit: () => {},
            setState: () => {},
            frame: (dt) => { frames++; },
            outline: () => "[]", inspect: () => "{}", edit: () => {},
        };
        globalThis.spin = () => { for (;;) {} };
        globalThis.bad = () => __engine.material(9, { name: 7 });
    "#;

    #[test]
    fn game_clock_matches_the_frame_runner() {
        // packages/gameplay/test/frame.test.ts: the frame clock adds each dt clamped to [0, 0.1] s.
        let mut c = GameClock::default();
        let t: Vec<f32> = [1.0 / 60.0, 0.5, -1.0, 1.0 / 60.0].into_iter().map(|dt| c.tick(dt)).collect();
        let f = |x: f64| x as f32;
        assert_eq!(t, [f(1.0 / 60.0), f(1.0 / 60.0 + 0.1), f(1.0 / 60.0 + 0.1), f(1.0 / 60.0 + 0.1 + 1.0 / 60.0)]);
    }

    #[test]
    fn a_bundle_drives_the_engine() {
        let h = ScriptHost::new(GAME).unwrap();
        assert!(h.has_frame());
        assert_eq!(h.handler_for_key("L").as_deref(), Some("click"));
        h.setup("room", Some(7)).unwrap();
        {
            let mut w = h.world.borrow_mut();
            assert_eq!(w.settings.name, "room7");
            assert_eq!(w.environment.wetness, 0.5, "TextEncoder in the prelude");
            assert!(matches!(&w.textures[&2].source, TextureSource::Encoded(b) if b.as_slice() == [2, 3]), "typed-array views keep their offset");
            assert_eq!(w.objects[&4].name, "sign");
            assert_eq!(w.objects[&5].position, Vec3::new(0.0, 0.0, 5.0));
            assert!(w.take_changes().structural);
        }
        let e = h.event("click", &serde_json::json!({ "target": "sign" })).unwrap();
        assert_eq!(e.log, vec!["click sign"]);
        let c = h.world.borrow_mut().take_changes();
        assert!(!c.structural && c.powers.contains(&4), "power is a parameter change");
        h.frame(0.5).unwrap();
    }

    #[test]
    fn a_game_as_es_modules() {
        // The cloud's layout: agent code importing "./threert-runtime.js", the runtime as its own module.
        let runtime = r#"
            export const defineGame = (g) => g;
            export function installNodeHost(game) {
                globalThis.__threert = { describe: () => JSON.stringify({ handlers: Object.keys(game.handlers), frame: false, keys: {} }), setup: () => { game.setup(); return "{}"; } };
            }
            export const box = (id) => __engine.geometry(id, new Float32Array(9), new Float32Array(9), new Float32Array(6), new Uint32Array([0, 1, 2]));
        "#;
        let game = r#"
            import { box } from "./threert-runtime.js";
            export default { handlers: { hit() {} }, setup() { box(1); } };
        "#;
        let h = ScriptHost::from_modules(game, runtime).unwrap();
        assert_eq!(h.info["handlers"], serde_json::json!(["hit"]));
        h.setup("r", None).unwrap();
        assert!(h.world.borrow().geometries.contains_key(&1));
        let scene = "export default function () { __engine.geometry(2, new Float32Array(9), new Float32Array(9), new Float32Array(6), new Uint32Array([0, 1, 2])); }";
        let h = ScriptHost::from_modules(scene, &runtime.replace("const defineGame = (g) => g", "const defineGame = (g) => ({ ...g, handlers: {}, setup: g.setup })")).unwrap();
        assert_eq!(h.info["handlers"], serde_json::json!([]), "a scene module is a game without handlers");
        assert!(ScriptHost::from_modules("import x from './nope.js';", runtime).is_err());
    }

    /// A frame whose script throws (as the binding's sync does once when the engine took out a body whose state went
    /// non-finite, or a `physics` cannot be made a body): the error is returned, and the frame's physics tick and
    /// commit are made all the same. (Skipped without a GPU.)
    #[test]
    fn a_frame_script_that_throws_does_not_stop_the_world() {
        let g = match crate::gpu::Gpu::new() {
            Ok(g) => g,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        // The game above with a weightless body that drives the sign at 6 m/s, and a frame script that throws in its
        // second frame.
        let game = GAME.replace("return \"{}\";", "__engine.physics('body', { id: 1, object: 4, gravityScale: 0, linvel: [0, 0, 6] }); __engine.physics('collider', { id: 1, body: 1, shape: { type: 'sphere', radius: 0.5 } }); return \"{}\";").replace("frame: (dt) => { frames++; },", "frame: (dt) => { frames++; if (frames === 2) throw new Error('the sync threw'); },").replace(", map: 2 }", " }");
        // (Without the sign's texture: its bytes are not an image.)
        assert!(game.contains("__engine.physics") && game.contains("the sync threw") && !game.contains("map: 2"));
        let bundle = std::env::temp_dir().join(format!("threert-frame-throws-{}.js", std::process::id()));
        std::fs::write(&bundle, game).unwrap();
        let started = GameSession::start(&bundle, "room", std::path::Path::new("."), g, &crate::default_presets_dir(), 64, 36);
        std::fs::remove_file(&bundle).ok();
        let (mut s, mut core) = started.unwrap();
        let z = |s: &GameSession| s.host.world.borrow().objects[&4].position.z;
        let ticks = |s: &GameSession| s.host.physics.borrow_mut().as_mut().unwrap().take_stats().ticks;
        s.frame(&mut core, 1.0 / 60.0).unwrap();
        assert_eq!(ticks(&s), 1);
        assert!((z(&s) - 0.1).abs() < 1e-4, "{}", z(&s));
        let e = s.frame(&mut core, 1.0 / 60.0).unwrap_err();
        assert!(format!("{e:#}").contains("the sync threw"), "{e:#}");
        assert_eq!(ticks(&s), 1, "the tick of the frame that threw");
        assert!((z(&s) - 0.2).abs() < 1e-4, "{}", z(&s));
        let c = s.host.world.borrow_mut().take_changes();
        assert!(c.moved.is_empty() && !c.structural, "and its commit");
        s.frame(&mut core, 1.0 / 60.0).unwrap();
        assert!((z(&s) - 0.3).abs() < 1e-4, "{}", z(&s));
    }

    /// The input path knows a game's world handlers (`on`) as it knows its handlers, and the node host runs every
    /// frame for a game that has them, or whose scene has a physics world, though it has no frame script.
    #[test]
    fn world_handlers_and_the_frames_of_a_game_without_a_frame_script() {
        let plain = GAME.replace("handlers: [\"click\"], frame: true", "handlers: [\"hit\"], frame: false");
        assert!(plain.contains("frame: false"));
        let h = ScriptHost::new(&plain).unwrap();
        assert!(h.has_handler("hit") && !h.has_handler("click") && !h.has_frame() && !h.frames(), "a scene with no script to run a frame for");
        let on = plain.replace("handlers: [\"hit\"],", "handlers: [\"hit\"], on: [\"click\"],");
        let h = ScriptHost::new(&on).unwrap();
        assert!(h.has_handler("click") && h.has_handler("hit") && !h.has_handler("drag"));
        assert!(!h.has_frame() && h.frames(), "world handlers read the frame clock");
        // A physics world (made by the scene's first physics call): its events go to their listeners once a frame.
        let bodies = plain.replace("return \"{}\";", "__engine.physics('gravity', { gravity: [0, -9.81, 0] }); return \"{}\";");
        let h = ScriptHost::new(&bodies).unwrap();
        assert!(!h.frames());
        h.setup("room", None).unwrap();
        assert!(h.frames());
    }

    /// A game without a frame script whose scene has bodies: the node host's `frame` runs every frame all the same
    /// (it delivers the contacts and the bodies that left the world to their listeners), after which the physics
    /// ticks; a scene with neither has no script run for it. (Skipped without a GPU.)
    #[test]
    fn a_scene_with_bodies_gets_its_frames_without_a_frame_script() {
        for (physics, expected) in [(true, 3), (false, 0)] {
            let g = match crate::gpu::Gpu::new() {
                Ok(g) => g,
                Err(e) => return eprintln!("skipped: {e}"),
            };
            let mut game = GAME.replace("handlers: [\"click\"], frame: true", "handlers: [\"click\"], frame: false").replace(", map: 2 }", " }");
            if physics {
                game = game.replace("return \"{}\";", "__engine.physics('body', { id: 1, object: 4, gravityScale: 0, linvel: [0, 0, 6] }); __engine.physics('collider', { id: 1, body: 1, shape: { type: 'sphere', radius: 0.5 } }); return \"{}\";");
            }
            let bundle = std::env::temp_dir().join(format!("threert-no-frame-script-{}-{physics}.js", std::process::id()));
            std::fs::write(&bundle, game).unwrap();
            let started = GameSession::start(&bundle, "room", std::path::Path::new("."), g, &crate::default_presets_dir(), 64, 36);
            std::fs::remove_file(&bundle).ok();
            let (mut s, mut core) = started.unwrap();
            for _ in 0..3 {
                s.frame(&mut core, 1.0 / 60.0).unwrap();
            }
            let frames: i32 = s.host.call(100, |ctx| ctx.eval("frames")).unwrap();
            assert_eq!(frames, expected, "physics {physics}");
            if physics {
                let z = s.host.world.borrow().objects[&4].position.z;
                assert!((z - 0.3).abs() < 1e-4, "three ticks at 6 m/s: {z}");
            }
        }
    }

    #[test]
    fn budgets_and_errors() {
        let h = ScriptHost::new(GAME).unwrap();
        let t = Instant::now();
        let err = h.call(100, |ctx| ctx.eval::<(), _>("spin()")).unwrap_err();
        assert!(t.elapsed().as_millis() < 2000, "interrupted");
        assert!(format!("{err}").contains("interrupted"), "{err}");
        let err = h.call(100, |ctx| ctx.eval::<(), _>("null.x")).unwrap_err();
        assert!(format!("{err}").contains("cannot read property"), "{err}");
        let err = h.call(100, |ctx| ctx.eval::<(), _>("bad()")).unwrap_err();
        assert!(format!("{err}").contains("engine.material"), "{err}");
        // still usable afterwards
        assert!(h.setup("r", None).is_ok());
    }
}

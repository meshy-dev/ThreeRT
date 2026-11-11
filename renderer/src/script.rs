//! The scene runtime of a render node. A game bundle (`tools/bundle-game.ts`: game code + the ThreeRT runtime + the
//! node host) runs in an embedded QuickJS engine in the renderer process, and drives the engine directly: the native
//! functions installed as `globalThis.__engine` create and edit the engine's world (world.rs) as the scene code's
//! binding (packages/scene/src/engine.ts) calls them. No scene document crosses between the two. `LiveCore::commit`
//! then applies what changed (GPU tables in place, or a rebuild).
//!
//! Sandbox: the engine has no I/O at all (no fs, network, timers or module loader); the host adds only a log sink, a
//! monotonic clock and the engine API. Memory is capped and every call runs under a time budget (the interrupt
//! handler aborts it).

use crate::world::World;
use anyhow::{anyhow, Result};
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

/// Time budgets per entry point (ms): building a scene may take a while, events and frames may not.
const SETUP_MS: u64 = 120_000;
const EVENT_MS: u64 = 1_000;
const FRAME_MS: u64 = 50;

/// The most engine time one entry into the script (a frame, an event, setup) is lent, µs: past it the time the engine
/// spends in voxel calls counts against the script's budget again. 250 ms: the largest single edit measured on the
/// benchmark level, the 30 x 30 m cut, takes 115 ms of engine time, so two of those fit; a frame that spends more in
/// the engine than that is a hitch of a quarter of a second whatever the script does, and a script that keeps asking
/// is then ended at its budget like one that spins.
const LEND_US: u64 = 250_000;

/// The script's clock for one entry: its deadline in µs since the host's epoch (0: none), checked by the interrupt
/// handler, and the engine time lent to it so far.
struct Budget {
    epoch: Instant,
    deadline: AtomicU64,
    lent: AtomicU64,
}

impl Budget {
    /// Past the deadline (with what was lent): the script's time is up.
    fn up(&self) -> bool {
        let d = self.deadline.load(Ordering::Relaxed);
        d != 0 && self.epoch.elapsed().as_micros() as u64 > d
    }
}

/// The engine's time inside a voxel call is not the script's: when the call returns (or throws), the deadline moves
/// by the time it took, as it was, up to `LEND_US` in all for the entry. (A cut through a hundred voxel bodies takes
/// 50 ms and more; without this the script that asked for it would be ended at its next statement, the rest of its
/// frame lost.) A script that spins, or loops on cheap calls, is ended as before.
///
/// A voxel call made when the time is already up throws "interrupted" at once: the script engine looks at its
/// deadline only every so many of its own operations, and a loop whose every turn is a call of some milliseconds
/// makes few of those (it ran for seconds past its budget before).
struct Lent<'a>(&'a Budget, Instant);

impl<'a> Lent<'a> {
    fn new(ctx: &Ctx<'_>, budget: &'a Budget) -> rquickjs::Result<Self> {
        if budget.up() {
            return Err(fail(ctx, anyhow!("interrupted (the script's time is up)")));
        }
        Ok(Lent(budget, Instant::now()))
    }
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        let took = self.1.elapsed().as_micros() as u64;
        let give = took.min(LEND_US.saturating_sub(self.0.lent.load(Ordering::Relaxed)));
        // (0: no deadline is set, and none is made.)
        if give > 0 && self.0.deadline.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |d| (d != 0).then_some(d + give)).is_ok() {
            self.0.lent.fetch_add(give, Ordering::Relaxed);
        }
    }
}

pub struct ScriptHost {
    _rt: Runtime,
    ctx: Context,
    /// The deadline of the entry running now and the engine time lent to it (`Budget`).
    budget: Arc<Budget>,
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

/// A failed engine call as the exception scene code catches (`api::Error`).
fn thrown(ctx: &Ctx, e: crate::api::Error) -> rquickjs::Error {
    match e {
        crate::api::Error::Type(m) => Exception::throw_type(ctx, &m),
        crate::api::Error::Other(e) => fail(ctx, e),
    }
}

/// The engine API (`globalThis.__engine`), see packages/scene/src/engine.ts: each call's arguments out of QuickJS,
/// then what the call does (api.rs: the same code a browser's exports run, renderer/web).
fn install_engine<'js>(ctx: &Ctx<'js>, world: &Rc<RefCell<World>>, voxels: &Rc<RefCell<crate::voxel::VoxelWorld>>, physics: &Rc<RefCell<Option<crate::physics::PhysicsWorld>>>, budget: &Arc<Budget>) -> rquickjs::Result<()> {
    use crate::api;
    let e = Object::new(ctx.clone())?;
    // Physics (renderer/src/physics.rs): one call, `physics(op, args)`, answering in JSON. The world is made by the
    // first call; the engine steps it, scene code cannot.
    let (p, w) = (physics.clone(), world.clone());
    e.set("physics", Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, args: Value<'js>| -> rquickjs::Result<String> {
        let a: serde_json::Value = if args.is_undefined() || args.is_null() { serde_json::Value::Null } else { parse(&ctx, "physics", &args)? };
        api::physics(&mut p.borrow_mut(), &mut w.borrow_mut(), &op, &a).map_err(|e| thrown(&ctx, e))
    })?)?;
    // The poses of the objects that bodies drive, by the slot the binding gave each body (7 floats: position, rotation
    // xyzw, as scene code reads an object's): one copy for all a frame reads. Empty without a world.
    let p = physics.clone();
    e.set("physicsPoses", Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<TypedArray<'js, f32>> { TypedArray::new(ctx, api::physics_poses(&p.borrow()).to_vec()) })?)?;
    // The voxel engine's level and explosions (renderer/src/voxel.rs): the level is loaded natively once the session
    // knows the asset root; an explosion returns what it did as JSON.
    let v = voxels.clone();
    e.set("voxelLevel", Function::new(ctx.clone(), move |ctx: Ctx, d: Value| -> rquickjs::Result<()> { api::voxel_level(&mut v.borrow_mut(), &to_json(&d)?).map_err(|e| thrown(&ctx, e)) })?)?;
    let (v, w, p, d) = (voxels.clone(), world.clone(), physics.clone(), budget.clone());
    e.set("voxelExplode", Function::new(ctx.clone(), move |ctx: Ctx, x: f64, y: f64, z: f64, r: f64| -> rquickjs::Result<String> { let _lent = Lent::new(&ctx, &d)?; api::voxel_explode(&mut v.borrow_mut(), &mut w.borrow_mut(), &mut p.borrow_mut(), x, y, z, r).map_err(|e| thrown(&ctx, e)) })?)?;
    // An edit of the level (`voxelEdit({ op, sphere | box | capsule, material })`; voxel.rs `EditOp`), and the level's
    // hash (16 hex digits; "" before a level): the same commands give the same hash.
    let (v, w, p, dl) = (voxels.clone(), world.clone(), physics.clone(), budget.clone());
    e.set("voxelEdit", Function::new(ctx.clone(), move |ctx: Ctx<'js>, d: Value<'js>| -> rquickjs::Result<String> { let _lent = Lent::new(&ctx, &dl)?; api::voxel_edit(&mut v.borrow_mut(), &mut w.borrow_mut(), &mut p.borrow_mut(), &to_json(&d)?).map_err(|e| thrown(&ctx, e)) })?)?;
    // The voxel bodies (ROADMAP § 6 step 8), as JSON: [{ body, collider, voxels, asleep, position, rotation }]. `body`
    // is the physics world's id: `physics("impulse", { body, ... })` and the other calls take it.
    let (v, p) = (voxels.clone(), physics.clone());
    e.set("voxelBodies", Function::new(ctx.clone(), move || -> String { api::voxel_bodies(&v.borrow(), &p.borrow()) })?)?;
    // One voxel body's edit (its object's `add` / `paint` / `carve`), its contacts on or off, the bodies gone since
    // the last call (step 10: voxel bodies as objects).
    let (v, w, p, d) = (voxels.clone(), world.clone(), physics.clone(), budget.clone());
    e.set("voxelBodyEdit", Function::new(ctx.clone(), move |ctx: Ctx<'js>, body: f64, d2: Value<'js>| -> rquickjs::Result<String> { let _lent = Lent::new(&ctx, &d)?; api::voxel_body_edit(&mut v.borrow_mut(), &mut w.borrow_mut(), &mut p.borrow_mut(), body, &to_json(&d2)?).map_err(|e| thrown(&ctx, e)) })?)?;
    let (v, p) = (voxels.clone(), physics.clone());
    e.set("voxelContacts", Function::new(ctx.clone(), move |ctx: Ctx, body: f64, on: bool| -> rquickjs::Result<()> { api::voxel_contacts(&mut v.borrow_mut(), &mut p.borrow_mut(), body, on).map_err(|e| thrown(&ctx, e)) })?)?;
    let v = voxels.clone();
    e.set("voxelGone", Function::new(ctx.clone(), move || -> String { api::voxel_gone(&mut v.borrow_mut()) })?)?;
    let v = voxels.clone();
    e.set("voxelHash", Function::new(ctx.clone(), move || -> String { api::voxel_hash(&v.borrow()) })?)?;
    let v = voxels.clone();
    e.set("voxelDebris", Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<TypedArray<'js, f32>> { TypedArray::new(ctx, v.borrow_mut().take_debris()) })?)?;
    let (v, w, d) = (voxels.clone(), world.clone(), budget.clone());
    e.set("voxelFrame", Function::new(ctx.clone(), move |ctx: Ctx, dt: f64| -> rquickjs::Result<String> { let _lent = Lent::new(&ctx, &d)?; api::voxel_frame(&mut v.borrow_mut(), &mut w.borrow_mut(), dt).map_err(|e| thrown(&ctx, e)) })?)?;
    let w = world.clone();
    // (…, groups?, colors?): the optional arguments as the rest (rquickjs takes seven parameters at most).
    e.set("geometry", Function::new(ctx.clone(), move |ctx: Ctx<'js>, id: u32, p: TypedArray<f32>, n: TypedArray<f32>, uv: TypedArray<f32>, idx: TypedArray<u32>, rest: rquickjs::function::Rest<Value<'js>>| -> rquickjs::Result<()> {
        let opt = |k: usize| rest.0.get(k).filter(|v| !v.is_null() && !v.is_undefined());
        let groups = opt(0).map(|v| TypedArray::<u32>::from_value(v.clone())).transpose()?;
        let colors = opt(1).map(|v| TypedArray::<f32>::from_value(v.clone())).transpose()?;
        api::geometry(&mut w.borrow_mut(), id, p.as_ref(), n.as_ref(), uv.as_ref(), idx.as_ref(), groups.as_ref().map(|g| g.as_ref()), colors.as_ref().map(|c| c.as_ref())).map_err(|e| thrown(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("texture", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value, data: Option<TypedArray<u8>>| -> rquickjs::Result<()> {
        api::texture(&mut w.borrow_mut(), id, &to_json(&d)?, data.as_ref().map(|d| d.as_ref())).map_err(|e| thrown(&ctx, e))
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
        let data = if data.is_null() || data.is_undefined() { None } else { Some(TypedArray::<f32>::from_value(data)?) };
        api::density_grid(&mut w.borrow_mut(), id, &j, data.as_ref().map(|d| d.as_ref())).map_err(|e| thrown(&ctx, e))
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
    e.set("material", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> { api::material(&mut w.borrow_mut(), id, &to_json(&d)?).map_err(|e| thrown(&ctx, e)) })?)?;
    let w = world.clone();
    e.set("object", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> { api::object(&mut w.borrow_mut(), id, &to_json(&d)?).map_err(|e| thrown(&ctx, e)) })?)?;
    let w = world.clone();
    e.set("transform", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, t: TypedArray<f32>| -> rquickjs::Result<()> { api::transform(&mut w.borrow_mut(), id, t.as_ref()).map_err(|e| thrown(&ctx, e)) })?)?;
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
    e.set("customData", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, v: Vec<f64>| api::custom_data(&mut w.borrow_mut(), id, &v).map_err(|e| thrown(&ctx, e)))?)?;
    let w = world.clone();
    e.set("sway", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> {
        let j = if d.is_null() || d.is_undefined() { None } else { Some(to_json(&d)?) };
        api::sway(&mut w.borrow_mut(), id, j.as_ref()).map_err(|e| thrown(&ctx, e))
    })?)?;
    let w = world.clone();
    e.set("light", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> { api::light(&mut w.borrow_mut(), id, &to_json(&d)?).map_err(|e| thrown(&ctx, e)) })?)?;
    let w = world.clone();
    e.set("camera", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> { api::camera(&mut w.borrow_mut(), id, &to_json(&d)?).map_err(|e| thrown(&ctx, e)) })?)?;
    let w = world.clone();
    e.set("animation", Function::new(ctx.clone(), move |ctx: Ctx, id: u32, d: Value| -> rquickjs::Result<()> {
        let j = if d.is_null() || d.is_undefined() { None } else { Some(to_json(&d)?) };
        api::animation(&mut w.borrow_mut(), id, j.as_ref()).map_err(|e| thrown(&ctx, e))
    })?)?;
    let w = world.clone();
    let p = physics.clone();
    e.set("remove", Function::new(ctx.clone(), move |id: u32| api::remove(&mut w.borrow_mut(), &mut p.borrow_mut(), id))?)?;
    let w = world.clone();
    e.set("environment", Function::new(ctx.clone(), move |ctx: Ctx, d: Value| -> rquickjs::Result<()> { api::environment(&mut w.borrow_mut(), &to_json(&d)?).map_err(|e| thrown(&ctx, e)) })?)?;
    let w = world.clone();
    e.set("settings", Function::new(ctx.clone(), move |d: Value| -> rquickjs::Result<()> {
        api::settings(&mut w.borrow_mut(), &to_json(&d)?);
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
        let epoch = Instant::now();
        let budget = Arc::new(Budget { epoch, deadline: AtomicU64::new(0), lent: AtomicU64::new(0) });
        let b = budget.clone();
        rt.set_interrupt_handler(Some(Box::new(move || b.up())));
        let ctx = Context::full(&rt)?;
        let world = Rc::new(RefCell::new(World::default()));
        let voxels = Rc::new(RefCell::new(crate::voxel::VoxelWorld::default()));
        let mut h = ScriptHost { _rt: rt, ctx, budget, epoch, info: serde_json::Value::Null, world, voxels, physics: Default::default() };
        let (w, v, ph, d2) = (h.world.clone(), h.voxels.clone(), h.physics.clone(), h.budget.clone());
        let info: String = h.call(SETUP_MS, |ctx| {
            let g = ctx.globals();
            g.set("__host_log", Function::new(ctx.clone(), |level: String, msg: String| eprintln!("[game{}] {msg}", if level == "info" { String::new() } else { format!(" {level}") }))?)?;
            g.set("__host_now", Function::new(ctx.clone(), move || epoch.elapsed().as_secs_f64() * 1000.0)?)?;
            install_engine(ctx, &w, &v, &ph, &d2)?;
            ctx.eval::<(), _>(PRELUDE)?;
            code(ctx)?;
            host(ctx)?.get::<_, Function>("describe")?.call(())
        })?;
        h.info = serde_json::from_str(&info)?;
        Ok(h)
    }

    /// Runs `f` in the engine under a time budget; JS exceptions become errors with their message and stack.
    fn call<T>(&self, budget_ms: u64, f: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<T>) -> Result<T> {
        self.budget.lent.store(0, Ordering::Relaxed);
        self.budget.deadline.store(self.epoch.elapsed().as_micros() as u64 + budget_ms * 1000, Ordering::Relaxed);
        let r = self.ctx.with(|ctx| f(&ctx).catch(&ctx).map_err(|e| anyhow!("{e}")));
        self.budget.deadline.store(0, Ordering::Relaxed);
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
        crate::api::physics_advance(&mut self.physics.borrow_mut(), &mut self.voxels.borrow_mut(), &mut self.world.borrow_mut(), dt as f64)
    }

    /// The physics world `ticks` ticks on (an offline render at a scene time).
    pub fn settle_physics(&self, ticks: u32) {
        crate::api::physics_settle(&mut self.physics.borrow_mut(), &mut self.voxels.borrow_mut(), &mut self.world.borrow_mut(), ticks);
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
        let host = ScriptHost::new(&std::fs::read_to_string(bundle).map_err(|e| anyhow!("{}: {e}", bundle.display()))?)?;
        let setup_ms = host.setup(room, None)?;
        host.world.borrow_mut().take_changes();
        eprintln!("game: setup {setup_ms:.0} ms in the node runtime, {} objects, handlers {}, frame script {}", host.world.borrow().objects.len(), host.info["handlers"], host.has_frame());
        Ok(GameSession { host, seq: 0, setup_ms, frame_ms: 0.0, js_ms: 0.0, commit_ms: 0.0, status: String::new(), clock: GameClock::default(), clicked: None, rebuilt: false })
    }

    /// Loads the bundle, builds the scene in the runtime and opens a live view of it. Models and file textures resolve
    /// against `assets`.
    pub fn start(bundle: &std::path::Path, room: &str, assets: &std::path::Path, g: crate::gpu::Gpu, presets: &std::path::Path, w: u32, h: u32) -> Result<(GameSession, crate::live::LiveCore)> {
        let s = Self::load(bundle, room)?;
        s.realize_voxels(assets)?;
        let core = crate::live::LiveCore::new(g, &s.host.world.borrow(), assets, presets, w, h)?;
        Ok((s, core))
    }

    /// Loads the bundle into an existing live view (history cleared, camera from the scene).
    pub fn open(bundle: &std::path::Path, room: &str, assets: &std::path::Path, core: &mut crate::live::LiveCore) -> Result<GameSession> {
        let s = Self::load(bundle, room)?;
        s.realize_voxels(assets)?;
        core.open(&s.host.world.borrow(), assets)?;
        core.seq = 0;
        Ok(s)
    }

    /// Loads the voxel level the scene code asked for (`voxelLevel`) now that the asset root is known. The scene build
    /// that follows includes it, so its changes are dropped here (as the setup's are, `load`).
    fn realize_voxels(&self, assets: &std::path::Path) -> Result<()> {
        let mut w = self.host.world.borrow_mut();
        if let Some(r) = self.host.voxels.borrow_mut().realize(&mut w, &mut self.host.physics.borrow_mut(), assets)? {
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
    use crate::world::TextureSource;
    use glam::Vec3;

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

    /// The engine's time in a voxel edit is not the script's: a frame's worth of script that spends 150 ms in edits
    /// (three times its budget) goes on to its end, and one that spins after its edits is still ended.
    #[test]
    fn the_engines_time_in_a_voxel_call_is_not_the_scripts() {
        let h = ScriptHost::new(GAME).unwrap();
        *h.voxels.borrow_mut() = crate::voxel::VoxelWorld::of_blocks(&[([0, 0, 0], [64, 16, 64], Some((0, 0)), true)], &mut h.world.borrow_mut(), &mut h.physics.borrow_mut());
        let edits = "let t = __host_now(), n = 0; while (__host_now() - t < 150) { __engine.voxelEdit({ op: 'carve', box: [0.5, 0.5, 0.5, 5.5, 1.5, 5.5] }); __engine.voxelEdit({ op: 'add', box: [0.5, 0.5, 0.5, 5.5, 1.5, 5.5], material: 1 }); n++; }";
        let n: i32 = h.call(FRAME_MS, |ctx| ctx.eval(format!("{edits} let s = 0; for (let i = 0; i < 100000; i++) s += i; n"))).unwrap();
        assert!(n >= 1);
        let t = Instant::now();
        let err = h.call(FRAME_MS, |ctx| ctx.eval::<(), _>(format!("{{ {edits} }} spin()"))).unwrap_err();
        assert!(format!("{err}").contains("interrupted"), "{err}");
        assert!(t.elapsed().as_millis() < 2000, "{} ms", t.elapsed().as_millis());
    }

    /// The engine's time is lent as it was, not rounded up, and at most `LEND_US` an entry (the review of #24: each
    /// call moved the deadline by a millisecond at least, so a frame script looping on cheap voxel calls, or on ones
    /// that throw, ran for ever). Each loop is ended within its budget, and real edits past what may be lent are too.
    #[test]
    fn a_script_looping_on_voxel_calls_is_ended() {
        let h = ScriptHost::new(GAME).unwrap();
        // Without a level: voxelFrame does nothing, voxelEdit and voxelExplode throw.
        for body in ["__engine.voxelFrame(0)", "try { __engine.voxelEdit({}) } catch (e) {}", "try { __engine.voxelExplode(0, 0, 0, 1) } catch (e) {}"] {
            let t = Instant::now();
            let err = h.call(FRAME_MS, |ctx| ctx.eval::<(), _>(format!("for (;;) {{ {body}; }}"))).unwrap_err();
            assert!(format!("{err}").contains("interrupted"), "{body}: {err}");
            assert!(t.elapsed().as_millis() < 200, "{body}: ended after {} ms", t.elapsed().as_millis());
        }
        // With a level, edits that each take real time, for ever: lent up to 250 ms, then the budget's 50.
        *h.voxels.borrow_mut() = crate::voxel::VoxelWorld::of_blocks(&[([0, 0, 0], [64, 16, 64], Some((0, 0)), true)], &mut h.world.borrow_mut(), &mut h.physics.borrow_mut());
        let t = Instant::now();
        let err = h.call(FRAME_MS, |ctx| ctx.eval::<(), _>("for (;;) { __engine.voxelEdit({ op: 'carve', box: [0.5, 0.5, 0.5, 5.5, 1.5, 5.5] }); __engine.voxelEdit({ op: 'add', box: [0.5, 0.5, 0.5, 5.5, 1.5, 5.5], material: 1 }); }")).unwrap_err();
        assert!(format!("{err}").contains("interrupted"), "{err}");
        let ms = t.elapsed().as_millis();
        assert!((250..800).contains(&ms), "ended after {ms} ms (50 of script and at most 250 lent)");
    }

    /// An id scene code read out of an answer goes back into a call: a ray's hit on a voxel body, its `body` into an
    /// impulse. (Parsed out of the answer the id is a double in the script engine, and comes back as `1073741837.0`:
    /// the bridge reads a whole number as the id it is.)
    #[test]
    fn an_id_a_script_read_from_a_hit_goes_into_a_call() {
        let h = ScriptHost::new(GAME).unwrap();
        // A slab and a bar standing on it, cut loose: 336 voxels of 1,000 kg/m³.
        let mut v = crate::voxel::VoxelWorld::of_blocks(&[([0, 0, 0], [64, 4, 64], Some((0, 0)), true), ([28, 4, 28], [4, 24, 4], None, false)], &mut h.world.borrow_mut(), &mut h.physics.borrow_mut());
        v.edit(&mut h.world.borrow_mut(), &mut h.physics.borrow_mut(), &crate::voxel::EditOp::parse(&serde_json::json!({ "op": "carve", "box": [2.7, 0.5, 2.7, 3.3, 0.7, 3.3] })).unwrap()).unwrap();
        assert_eq!(v.bodies(), 1);
        *h.voxels.borrow_mut() = v;
        let up: f64 = h.call(100, |ctx| ctx.eval(r#"
            const hit = JSON.parse(__engine.physics("ray", { origin: [3.05, 1.55, 10], direction: [0, 0, -1], max: 20 }));
            if (hit.body !== JSON.parse(__engine.voxelBodies())[0].body) throw new Error("the hit is not the body: " + JSON.stringify(hit));
            __engine.physics("impulse", { body: hit.body, impulse: [0, 500, 0] });
            JSON.parse(__engine.physics("state", { body: hit.body })).linvel[1]
        "#)).unwrap();
        assert!((up - 500.0 / 336.0).abs() < 0.05, "{up} m/s");
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

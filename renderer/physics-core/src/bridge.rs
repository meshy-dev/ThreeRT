//! Between scene code and the world (ROADMAP § 6 step 7): the calls a game's binding makes (bodies, colliders, joints,
//! impulses, forces, queries, contacts; JSON in, JSON out), and the scene objects the bodies drive. The engine owns a
//! `Bridge`, steps it at the fixed tick and lets it place the objects of the bodies that moved; scene code never steps
//! it and never sees the world. The scene is whatever implements `Scene` (the renderer's world natively; the browser's
//! the same way), so this layer has no GPU, file system, clock or thread in it either: it builds for wasm32.
//!
//! Ids of bodies, colliders and joints are the caller's (the binding chooses them, as it does for geometries and
//! objects), so creating one needs no answer. A body that drives an object may take a `slot`: `poses()` is then the
//! table of every such object's position and rotation as the last sync left them, one copy for a whole frame's reads.

use crate::{limits, BodyDesc, BodyId, BodyKind, ColliderDesc, ColliderId, Hit, JointDesc, JointId, Limits, Motor, Physics, Shape, TICK};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The most a session may hold (game code is untrusted), and the most ticks one frame makes up.
pub const MAX_BODIES: usize = 20_000;
pub const MAX_COLLIDERS: usize = 40_000;
pub const MAX_JOINTS: usize = 40_000;
pub const MAX_MESH_VERTICES: usize = 1 << 20;
pub const MAX_HULL_POINTS: usize = 4096;
pub const MAX_TICKS: u32 = 5;
/// The most hits one ray reports.
pub const MAX_HITS: usize = 256;
/// Ids from here up are the engine's own bodies and colliders (the voxel world's: ROADMAP § 6 step 8). Scene code
/// cannot make or remove one, and can name one wherever a call takes a body or a collider (an impulse, a velocity, a
/// joint's end, a query's `skip`); hits and contacts report them like any other.
pub const ENGINE_IDS: u32 = 1 << 30;
/// The most bodies and colliders the engine itself makes (a level's objects are a collider each).
pub const MAX_ENGINE_BODIES: usize = 20_000;
pub const MAX_ENGINE_COLLIDERS: usize = 40_000;
/// Floats a slot of `poses()` holds: position xyz, rotation xyzw.
pub const POSE: usize = 7;

/// The scene whose objects bodies drive, by the engine's object and geometry ids.
pub trait Scene {
    /// An object's position and rotation (xyzw) in the world; None if there is no such object.
    fn world_pose(&self, object: u32) -> Option<([f32; 3], [f32; 4])>;
    /// Places an object at a pose of the world, its scale kept. Returns the position and rotation it has for scene code
    /// (in its parent's space); None if the object is gone.
    fn place(&mut self, object: u32, position: [f32; 3], rotation: [f32; 4]) -> Option<([f32; 3], [f32; 4])>;
    /// A body moves the object from now on (the renderer keeps such an object apart, so a move is cheap).
    fn drive(&mut self, object: u32);
    /// A geometry's positions (three numbers a vertex) and triangle indices.
    fn geometry(&self, id: u32) -> Option<(&[f32], &[u32])>;
}

pub struct Bridge {
    core: Physics,
    /// Caller id -> the world's, and back for what queries and contacts report.
    bodies: BTreeMap<u32, BodyId>,
    body_ids: BTreeMap<BodyId, u32>,
    colliders: BTreeMap<u32, ColliderId>,
    /// The world's collider -> the caller's id of it and of its body.
    collider_ids: BTreeMap<ColliderId, (u32, Option<u32>)>,
    /// The voxel level's fixed body (`engine_level_body`): its colliders hang on it, so a joint can hold to the level;
    /// to queries and contacts they are on no body, as the level's colliders always were (`hit.object` is
    /// `engineColliders`, with no `body`).
    level_body: Option<u32>,
    /// Scene code's joints the engine let go of since the last `broken` call: their anchor's material carved away, or
    /// an engine body they held to gone (step 10 c1).
    broken: Vec<u32>,
    /// The pairs of colliders (the caller's ids, lower first) touching as reported, with their bodies: a start is
    /// reported once (a collider edited in place, taken out and put back, starts again without having ended), and a
    /// collider that goes ends its pairs (`ended`; the world's own end events of a collider it no longer has are lost).
    touching: BTreeMap<(u32, u32), (Option<u32>, Option<u32>)>,
    /// Ends of pairs whose collider went, for the next `contacts`.
    ended: Vec<Value>,
    joints: BTreeMap<u32, JointId>,
    /// Body (the world's id) -> the scene object it drives and its slot in `poses`.
    drives: BTreeMap<BodyId, (u32, Option<u32>)>,
    /// The objects that have a body and the slots that are taken (an object has one body, a slot one object).
    driven: std::collections::BTreeSet<u32>,
    slots: std::collections::BTreeSet<u32>,
    /// `POSE` floats a slot: the driven objects' poses as scene code reads them.
    poses: Vec<f32>,
    /// Bodies awake after any tick since the last sync, and those awake at the last sync (one that fell asleep in the
    /// first tick after it still moved in that tick).
    moved: Vec<BodyId>,
    awake: Vec<BodyId>,
    /// Bodies with forces: cleared once a frame's ticks are done.
    forced: Vec<BodyId>,
    /// A tick was made since the last sync.
    ticked: bool,
    /// Time not yet stepped, s.
    behind: f64,
    /// Bodies the world took out after a tick (`Physics::step`), by the caller's id, with the object they drove,
    /// whether they left the world (or went non-finite) and why: those not yet reported by "faults", and (for
    /// "state") the last `MAX_FAULTS` of them.
    faults: Vec<(u32, Option<u32>, bool, &'static str)>,
    faulted: BTreeMap<u32, &'static str>,
}

/// The most faults "faults" holds until it is called, and "state" remembers.
pub const MAX_FAULTS: usize = 1024;

/// A joint's description in a call (`joint`, `joint_set`).
fn joint_desc(a: &Value) -> Result<JointDesc, String> {
    let (anchor1, anchor2) = (vec3(a, "anchor1", [0.0; 3])?, vec3(a, "anchor2", [0.0; 3])?);
    let limits = match &a["limits"] {
        Value::Null => None,
        v => Some(nums::<2>(v).map(|l| Limits { min: l[0], max: l[1] }).ok_or("physics: limits must be [min, max]")?),
    };
    let motor = match &a["motor"] {
        Value::Null => None,
        m => Some(Motor { velocity: num(m, "velocity", 0.0)?, damping: num(m, "damping", 1e4)?, max_force: num(m, "maxForce", f32::MAX)? }),
    };
    Ok(match a["type"].as_str() {
        Some("fixed") => JointDesc::Fixed { anchor1, anchor2 },
        Some("revolute") => JointDesc::Revolute { anchor1, anchor2, axis: vec3(a, "axis", [0.0, 1.0, 0.0])?, limits, motor },
        Some("prismatic") => JointDesc::Prismatic { anchor1, anchor2, axis: vec3(a, "axis", [1.0, 0.0, 0.0])?, limits, motor },
        Some("spherical") => JointDesc::Spherical { anchor1, anchor2 },
        Some("rope") => JointDesc::Rope { anchor1, anchor2, length: num(a, "length", f32::NAN)? },
        other => return Err(format!("physics: joint type {other:?} is none of fixed, revolute, prismatic, spherical, rope")),
    })
}

fn nums<const N: usize>(v: &Value) -> Option<[f32; N]> {
    let a = v.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0f32; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().filter(|x| x.is_finite())? as f32;
    }
    Some(out)
}

fn vec3(a: &Value, key: &str, default: [f32; 3]) -> Result<[f32; 3], String> {
    match &a[key] {
        Value::Null => Ok(default),
        v => nums::<3>(v).ok_or_else(|| format!("physics: {key} must be three finite numbers")),
    }
}

fn quat(a: &Value, key: &str) -> Result<[f32; 4], String> {
    match &a[key] {
        Value::Null => Ok([0.0, 0.0, 0.0, 1.0]),
        v => nums::<4>(v).ok_or_else(|| format!("physics: {key} must be a quaternion (x, y, z, w)")),
    }
}

fn num(a: &Value, key: &str, default: f32) -> Result<f32, String> {
    match &a[key] {
        Value::Null => Ok(default),
        v => v.as_f64().filter(|x| x.is_finite()).map(|x| x as f32).ok_or_else(|| format!("physics: {key} must be a finite number")),
    }
}

/// A whole number from 0 to `u32::MAX`, however the caller's JSON wrote it. (A script engine has one kind of number:
/// an id it read out of an answer, a hit's `body`, comes back as `1073741837.0`.)
fn whole(v: &Value) -> Option<u32> {
    v.as_u64().or_else(|| v.as_f64().filter(|x| *x >= 0.0 && *x <= u32::MAX as f64 && x.fract() == 0.0).map(|x| x as u64)).filter(|&x| x <= u32::MAX as u64).map(|x| x as u32)
}

fn id(a: &Value, key: &str) -> Result<u32, String> {
    whole(&a[key]).ok_or_else(|| format!("physics: {key} must be an id"))
}

fn opt_id(a: &Value, key: &str) -> Result<Option<u32>, String> {
    if a[key].is_null() {
        Ok(None)
    } else {
        id(a, key).map(Some)
    }
}

impl Default for Bridge {
    fn default() -> Self {
        Bridge {
            core: Physics::default(),
            bodies: BTreeMap::new(),
            body_ids: BTreeMap::new(),
            colliders: BTreeMap::new(),
            collider_ids: BTreeMap::new(),
            level_body: None,
            broken: vec![],
            touching: BTreeMap::new(),
            ended: vec![],
            joints: BTreeMap::new(),
            drives: BTreeMap::new(),
            driven: Default::default(),
            slots: Default::default(),
            poses: vec![],
            moved: vec![],
            awake: vec![],
            forced: vec![],
            ticked: false,
            behind: 0.0,
            faults: vec![],
            faulted: BTreeMap::new(),
        }
    }
}

impl Bridge {
    /// A shape from its description: `{type: "box", halfExtents}`, `{type: "sphere", radius}`, `{type: "capsule",
    /// halfHeight, radius}` (along y), `{type: "convexHull", points: [x, y, z, ...]}`, `{type: "trimesh", vertices: [...],
    /// indices: [...]}`; or the last two from a geometry the scene holds: `{type: "convexHull" | "trimesh", geometry: id,
    /// scale?: [x, y, z]}`.
    fn shape(scene: &impl Scene, s: &Value) -> Result<Shape, String> {
        let floats = |key: &str| -> Result<Vec<[f32; 3]>, String> {
            let a = s[key].as_array().filter(|a| a.len() % 3 == 0 && a.len() / 3 <= MAX_MESH_VERTICES).ok_or_else(|| format!("physics: shape {key} must be numbers, three a point, at most {MAX_MESH_VERTICES} points"))?;
            a.chunks(3).map(|c| nums::<3>(&Value::Array(c.to_vec())).ok_or_else(|| format!("physics: shape {key} must be finite numbers"))).collect()
        };
        let geometry = |hull: bool| -> Result<(Vec<[f32; 3]>, Vec<[u32; 3]>), String> {
            let gid = id(s, "geometry")?;
            let (positions, indices) = scene.geometry(gid).ok_or_else(|| format!("physics: unknown geometry {gid}"))?;
            let k = vec3(s, "scale", [1.0; 3])?;
            if positions.len() / 3 > MAX_MESH_VERTICES || (!hull && indices.len() / 3 > limits::MAX_MESH_TRIANGLES) {
                return Err(format!("physics: geometry {gid} has more than {MAX_MESH_VERTICES} vertices or {} triangles", limits::MAX_MESH_TRIANGLES));
            }
            let mut points: Vec<[f32; 3]> = positions.chunks_exact(3).map(|p| [p[0] * k[0], p[1] * k[1], p[2] * k[2]]).collect();
            if hull && points.len() > MAX_HULL_POINTS {
                // Every n-th vertex: a hull of a dense mesh needs few of them.
                let step = points.len().div_ceil(MAX_HULL_POINTS);
                points = points.into_iter().step_by(step).collect();
            }
            Ok((points, indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect()))
        };
        Ok(match s["type"].as_str() {
            Some("box") => Shape::Box(nums::<3>(&s["halfExtents"]).ok_or("physics: a box needs halfExtents [x, y, z]")?),
            Some("sphere") => Shape::Sphere(num(s, "radius", f32::NAN)?),
            Some("capsule") => Shape::Capsule { half_height: num(s, "halfHeight", f32::NAN)?, radius: num(s, "radius", f32::NAN)? },
            Some("convexHull") if !s["geometry"].is_null() => Shape::ConvexHull(geometry(true)?.0),
            Some("convexHull") => {
                let p = floats("points")?;
                if p.len() > MAX_HULL_POINTS {
                    return Err(format!("physics: a convex hull takes at most {MAX_HULL_POINTS} points"));
                }
                Shape::ConvexHull(p)
            }
            Some("trimesh") if !s["geometry"].is_null() => {
                let (vertices, indices) = geometry(false)?;
                Shape::Trimesh { vertices, indices }
            }
            Some("trimesh") => {
                let vertices = floats("vertices")?;
                let idx = s["indices"].as_array().filter(|a| a.len() % 3 == 0 && a.len() / 3 <= limits::MAX_MESH_TRIANGLES).ok_or_else(|| format!("physics: a trimesh needs indices, three a triangle, at most {} triangles", limits::MAX_MESH_TRIANGLES))?;
                // (An index past u32 is refused, not cut: 4294967296 is not vertex 0.)
                let index = whole;
                let indices = idx.chunks(3).map(|t| Some([index(&t[0])?, index(&t[1])?, index(&t[2])?])).collect::<Option<Vec<_>>>().ok_or("physics: trimesh indices must be whole numbers below 2^32")?;
                Shape::Trimesh { vertices, indices }
            }
            other => return Err(format!("physics: shape type {other:?} is none of box, sphere, capsule, convexHull, trimesh")),
        })
    }

    /// One call from scene code. Returns its answer (a state, hits, contacts) as JSON; null for a command.
    pub fn call(&mut self, scene: &mut impl Scene, op: &str, a: &Value) -> Result<Value, String> {
        Ok(match op {
            "gravity" => {
                self.core.set_gravity(vec3(a, "gravity", [0.0, -9.81, 0.0])?)?;
                Value::Null
            }
            // {id, kind, object?, slot?, position?, rotation?, linvel?, angvel?, linearDamping?, angularDamping?,
            //  gravityScale?, canSleep?, ccd?}: a body; with `object`, it drives that scene object (and starts where it
            //  is, unless a position is given), and with `slot` the object's pose is in `poses()` there.
            "body" => {
                if self.core.bodies() >= MAX_BODIES {
                    return Err(format!("physics: more than {MAX_BODIES} bodies"));
                }
                let bid = id(a, "id")?;
                if bid >= ENGINE_IDS {
                    return Err(format!("physics: ids from {ENGINE_IDS} up are the engine's"));
                }
                if self.bodies.contains_key(&bid) {
                    return Err(format!("physics: body {bid} exists"));
                }
                let kind = match a["kind"].as_str() {
                    None | Some("dynamic") => BodyKind::Dynamic,
                    Some("fixed") => BodyKind::Fixed,
                    Some("kinematic") => BodyKind::Kinematic,
                    Some(other) => return Err(format!("physics: body kind \"{other}\" is none of dynamic, fixed, kinematic")),
                };
                let object = opt_id(a, "object")?;
                let slot = opt_id(a, "slot")?;
                let (mut position, mut rotation) = ([0.0; 3], [0.0, 0.0, 0.0, 1.0]);
                if let Some(o) = object {
                    (position, rotation) = scene.world_pose(o).ok_or_else(|| format!("physics: unknown object {o}"))?;
                    if self.driven.contains(&o) {
                        return Err(format!("physics: object {o} already has a body"));
                    }
                }
                if let Some(s) = slot {
                    if object.is_none() || s as usize >= MAX_BODIES || self.slots.contains(&s) {
                        return Err(format!("physics: slot {s} is taken, past {MAX_BODIES}, or of a body without an object"));
                    }
                }
                let d = BodyDesc {
                    kind,
                    position: vec3(a, "position", position)?,
                    rotation: if a["rotation"].is_null() { rotation } else { quat(a, "rotation")? },
                    linvel: vec3(a, "linvel", [0.0; 3])?,
                    angvel: vec3(a, "angvel", [0.0; 3])?,
                    linear_damping: num(a, "linearDamping", 0.0)?,
                    angular_damping: num(a, "angularDamping", 0.0)?,
                    gravity_scale: num(a, "gravityScale", 1.0)?,
                    can_sleep: a["canSleep"].as_bool().unwrap_or(true),
                    ccd: a["ccd"].as_bool().unwrap_or(false),
                };
                let b = self.core.add_body(&d)?;
                self.bodies.insert(bid, b);
                self.body_ids.insert(b, bid);
                self.faulted.remove(&bid);
                if let Some(o) = object {
                    if kind != BodyKind::Fixed {
                        scene.drive(o);
                    }
                    self.drives.insert(b, (o, slot));
                    self.driven.insert(o);
                    if let Some(s) = slot {
                        self.slots.insert(s);
                        let end = (s as usize + 1) * POSE;
                        if self.poses.len() < end {
                            self.poses.resize(end, 0.0);
                        }
                    }
                    // (Where the body is: its rotation normalized, as the world keeps it.)
                    self.place_at_body(scene, b);
                }
                Value::Null
            }
            // {id, body?, shape, position?, rotation?, density? | mass?, friction?, restitution?, sensor?, events?}: a
            // collider on a body, or a static one in the world.
            "collider" => {
                if self.colliders.len() >= MAX_COLLIDERS {
                    return Err(format!("physics: more than {MAX_COLLIDERS} colliders"));
                }
                let cid = id(a, "id")?;
                if cid >= ENGINE_IDS {
                    return Err(format!("physics: ids from {ENGINE_IDS} up are the engine's"));
                }
                if self.colliders.contains_key(&cid) {
                    return Err(format!("physics: collider {cid} exists"));
                }
                let owner = opt_id(a, "body")?;
                // (The engine's voxel bodies are edited cell by cell, cut in parts and weighed by their cells: a
                // collider of scene code's on one would ride along in no cell. Join a body of its own instead.)
                if owner.is_some_and(|b| b >= ENGINE_IDS) {
                    return Err("physics: a collider cannot be put on one of the engine's bodies (join a body of your own to it with a joint)".into());
                }
                let body = match owner {
                    Some(b) => Some(*self.bodies.get(&b).ok_or_else(|| format!("physics: unknown body {b}"))?),
                    None => None,
                };
                let d = ColliderDesc {
                    shape: Self::shape(scene, &a["shape"])?,
                    position: vec3(a, "position", [0.0; 3])?,
                    rotation: quat(a, "rotation")?,
                    density: num(a, "density", 1000.0)?,
                    mass: if a["mass"].is_null() { None } else { Some(num(a, "mass", 0.0)?) },
                    friction: num(a, "friction", 0.5)?,
                    restitution: num(a, "restitution", 0.0)?,
                    sensor: a["sensor"].as_bool().unwrap_or(false),
                    events: a["events"].as_bool().unwrap_or(false),
                };
                let c = self.core.add_collider(body, &d)?;
                self.colliders.insert(cid, c);
                self.collider_ids.insert(c, (cid, owner));
                Value::Null
            }
            // {id, a, b, type, anchor1?, anchor2?, axis?, limits?: [min, max], motor?: {velocity, damping, maxForce},
            //  length?}: a joint between two bodies.
            "joint" => {
                if self.joints.len() >= MAX_JOINTS {
                    return Err(format!("physics: more than {MAX_JOINTS} joints"));
                }
                let jid = id(a, "id")?;
                if self.joints.contains_key(&jid) {
                    return Err(format!("physics: joint {jid} exists"));
                }
                let d = joint_desc(a)?;
                let (ba, bb) = (self.body(a, "a")?, self.body(a, "b")?);
                let j = self.core.add_joint(ba, bb, &d)?;
                self.joints.insert(jid, j);
                Value::Null
            }
            // The joints the engine let go of since the last call: [id]: their anchor's material carved away, or an engine
            // body they held to gone (a voxel body retired, carved away, fallen). Scene code's ids; gone from the world.
            "broken" => json!(std::mem::take(&mut self.broken)),
            // {id, type, limits?, motor?, length?}: a joint's parameters changed where it is (a hinge's or a slider's
            // limits and motor, a rope's length); its bodies, anchors and axis stay as made (`Physics::set_joint`).
            "joint_set" => {
                let jid = id(a, "id")?;
                let &j = self.joints.get(&jid).ok_or_else(|| format!("physics: unknown joint {jid}"))?;
                self.core.set_joint(j, &joint_desc(a)?)?;
                Value::Null
            }
            "remove" => {
                // (All three read before any is removed: a call that is refused changes nothing.)
                let (body, collider, joint) = (opt_id(a, "body")?, opt_id(a, "collider")?, opt_id(a, "joint")?);
                if body.is_some_and(|b| b >= ENGINE_IDS) || collider.is_some_and(|c| c >= ENGINE_IDS) {
                    return Err("physics: the engine's bodies and colliders are not scene code's to remove (a voxel object's go with its voxels)".into());
                }
                if let Some(b) = body.and_then(|b| self.bodies.get(&b).copied()) {
                    self.forget_body(b);
                }
                if let Some(c) = collider.and_then(|c| self.colliders.remove(&c)) {
                    self.core.remove_collider(c);
                    self.unmap(c);
                }
                if let Some(j) = joint.and_then(|j| self.joints.remove(&j)) {
                    self.core.remove_joint(j);
                }
                Value::Null
            }
            // {body, impulse?, point?, torque?}: an impulse at the centre of mass, or at a point of the world; an angular
            // one.
            "impulse" => {
                let b = self.body(a, "body")?;
                let (j, t) = (vec3(a, "impulse", [0.0; 3])?, vec3(a, "torque", [0.0; 3])?);
                if a["point"].is_null() {
                    self.core.apply_impulse(b, j, t)?;
                } else {
                    self.core.apply_impulse_at(b, j, vec3(a, "point", [0.0; 3])?, t)?;
                }
                Value::Null
            }
            // {body, force?, torque?}: a force and a torque through the ticks of this frame (until the next `sync`).
            "force" => {
                let b = self.body(a, "body")?;
                self.core.add_force(b, vec3(a, "force", [0.0; 3])?, vec3(a, "torque", [0.0; 3])?)?;
                if !self.forced.contains(&b) {
                    self.forced.push(b);
                }
                Value::Null
            }
            // {body, linvel?, angvel?}: either velocity set, the other kept.
            "velocity" => {
                let b = self.body(a, "body")?;
                let opt = |key: &str| -> Result<Option<[f32; 3]>, String> { if a[key].is_null() { Ok(None) } else { vec3(a, key, [0.0; 3]).map(Some) } };
                self.core.set_velocity(b, opt("linvel")?, opt("angvel")?)?;
                Value::Null
            }
            "pose" => {
                let b = self.body(a, "body")?;
                let (p, q) = (vec3(a, "position", [0.0; 3])?, quat(a, "rotation")?);
                self.core.set_pose(b, p, q)?;
                self.place_at_body(scene, b);
                Value::Null
            }
            // Where a kinematic body is after the next tick.
            "target" => {
                self.core.set_kinematic_target(self.body(a, "body")?, vec3(a, "position", [0.0; 3])?, quat(a, "rotation")?)?;
                Value::Null
            }
            // {body}: scene code moved the body's object. The body goes where the object is: at once (its velocity
            // kept), or, a kinematic one, through the next tick, pushing what is in its way.
            "follow" => {
                let b = self.body(a, "body")?;
                if let Some((p, q)) = self.drives.get(&b).and_then(|&(o, _)| scene.world_pose(o)) {
                    if self.core.is_kinematic(b) {
                        self.core.set_kinematic_target(b, p, q)?;
                        self.note_pose(scene, b);
                    } else {
                        self.core.set_pose(b, p, q)?;
                        self.place_at_body(scene, b);
                    }
                }
                Value::Null
            }
            // {body}: its pose, velocities, sleeping and mass; for a body the world took out after a tick, {removed: why}.
            "state" => {
                if let Some(why) = id(a, "body").ok().filter(|b| !self.bodies.contains_key(b)).and_then(|b| self.faulted.get(&b)) {
                    return Ok(json!({ "removed": why }));
                }
                let b = self.body(a, "body")?;
                match (self.core.pose(b), self.core.velocity(b)) {
                    (Some((p, q)), Some((v, av))) => json!({ "position": p, "rotation": q, "linvel": v, "angvel": av, "sleeping": self.core.sleeping(b), "mass": self.core.mass(b) }),
                    _ => Value::Null,
                }
            }
            // Queries, against the world as the calls left it (`Physics::fresh`: with or without a tick since).
            // {origin, direction, max?, skip?, all?}: the first hit (or
            // null), or with `all` every hit, nearest first.
            "ray" => {
                let (o, d, max, skip) = (vec3(a, "origin", [0.0; 3])?, vec3(a, "direction", [0.0; 3])?, num(a, "max", 1e4)?, self.skip(a)?);
                if a["all"].as_bool().unwrap_or(false) {
                    Value::Array(self.core.cast_ray_all(o, d, max, skip, MAX_HITS)?.into_iter().map(|h| self.hit(Some(h))).collect())
                } else {
                    self.hit(self.core.cast_ray(o, d, max, skip)?)
                }
            }
            // The colliders a shape overlaps: [{collider, body, object}].
            "overlap" => {
                let s = Self::shape(scene, &a["shape"])?;
                let hits = self.core.overlap(&s, vec3(a, "position", [0.0; 3])?, quat(a, "rotation")?, self.skip(a)?)?;
                Value::Array(hits.iter().filter_map(|c| self.collider_ids.get(c)).map(|&(c, b)| json!({ "collider": c, "body": b, "object": self.object_of(b) })).collect())
            }
            "sweep" => {
                let s = Self::shape(scene, &a["shape"])?;
                let h = self.core.sweep(&s, vec3(a, "position", [0.0; 3])?, quat(a, "rotation")?, vec3(a, "direction", [0.0; 3])?, num(a, "max", 1e4)?, self.skip(a)?)?;
                self.hit(h)
            }
            // The contacts begun and ended since the last call (colliders with `events`), in the order they happened:
            // [{a, b, bodyA, bodyB, started}].
            "contacts" => {
                let mut out = std::mem::take(&mut self.ended);
                for c in self.core.take_contacts() {
                    let (Some(&(a, ba)), Some(&(b, bb))) = (self.collider_ids.get(&c.a), self.collider_ids.get(&c.b)) else { continue };
                    let (a, b, ba, bb) = if a <= b { (a, b, ba, bb) } else { (b, a, bb, ba) };
                    // (A start of a pair touching already, or an end of one that is not: not reported again.)
                    let new = if c.started { self.touching.insert((a, b), (ba, bb)).is_none() } else { self.touching.remove(&(a, b)).is_some() };
                    if new {
                        out.push(json!({ "a": a, "b": b, "bodyA": ba, "bodyB": bb, "started": c.started }));
                    }
                }
                Value::Array(out)
            }
            // The world's hash and ticks (16 hex digits): the same calls at the same ticks give the same one.
            "hash" => json!({ "hash": format!("{:016x}", self.core.hash()), "ticks": self.core.ticks(), "bodies": self.core.bodies(), "contactsDropped": self.core.contacts_dropped() }),
            // The bodies the world took out after a tick since the last call (`Physics::step`), with the object each
            // drove and why: [{body, object, kind, reason}]. `kind` is "leftWorld" for a body past the world's edge (it
            // fell off the level: nothing is wrong) and "nonFinite" for one whose pose, velocity or shape went
            // non-finite (an error). Their colliders and joints went with them; "state" on one answers {removed: why}.
            "faults" => Value::Array(std::mem::take(&mut self.faults).into_iter().map(|(b, o, left, why)| json!({ "body": b, "object": o, "kind": if left { "leftWorld" } else { "nonFinite" }, "reason": why })).collect()),
            other => return Err(format!("physics: unknown call \"{other}\"")),
        })
    }

    /// A body of the engine's own (id from `ENGINE_IDS` up): it drives no scene object through the bridge (the engine
    /// places what it stands for: `engine_pose`).
    pub fn engine_body(&mut self, id: u32, d: &BodyDesc) -> Result<(), String> {
        if id < ENGINE_IDS || self.bodies.contains_key(&id) {
            return Err(format!("physics: engine body {id} exists or is below {ENGINE_IDS}"));
        }
        if self.bodies.range(ENGINE_IDS..).count() >= MAX_ENGINE_BODIES {
            return Err(format!("physics: more than {MAX_ENGINE_BODIES} engine bodies"));
        }
        let b = self.core.add_body(d)?;
        self.bodies.insert(id, b);
        self.body_ids.insert(b, id);
        self.faulted.remove(&id);
        Ok(())
    }

    /// A collider of the engine's own, on one of its bodies or fixed in the world.
    pub fn engine_collider(&mut self, id: u32, body: Option<u32>, d: &ColliderDesc) -> Result<(), String> {
        if id < ENGINE_IDS || self.colliders.contains_key(&id) {
            return Err(format!("physics: engine collider {id} exists or is below {ENGINE_IDS}"));
        }
        if self.colliders.range(ENGINE_IDS..).count() >= MAX_ENGINE_COLLIDERS {
            return Err(format!("physics: more than {MAX_ENGINE_COLLIDERS} engine colliders"));
        }
        let owner = match body {
            Some(b) => Some(*self.bodies.get(&b).ok_or_else(|| format!("physics: unknown body {b}"))?),
            None => None,
        };
        let c = self.core.add_collider(owner, d)?;
        self.colliders.insert(id, c);
        // (The level's colliders are on no body to queries and contacts.)
        self.collider_ids.insert(c, (id, body.filter(|&b| Some(b) != self.level_body)));
        Ok(())
    }

    /// The voxel level's fixed body (at the origin, unturned): the engine hangs the level's colliders on it
    /// (`engine_collider` with this body) so that joints can hold to the level, as Rapier hangs static colliders on a
    /// fixed body. To queries and contacts its colliders are on no body (the level has no `body`). A level loaded
    /// again makes it anew (the old one goes, with its colliders and joints).
    pub fn engine_level_body(&mut self, id: u32) -> Result<(), String> {
        if let Some(old) = self.level_body.take() {
            self.engine_remove(Some(old), None);
        }
        self.engine_body(id, &BodyDesc { kind: BodyKind::Fixed, ..Default::default() })?;
        self.level_body = Some(id);
        Ok(())
    }

    /// The joints on an engine body (step 10 c1): each one's world id, whether the body is its first, and its anchor
    /// in the body's space; in id order.
    pub fn engine_joints_of(&self, body: u32) -> Vec<(JointId, bool, [f32; 3])> {
        self.bodies.get(&body).map_or(vec![], |&b| self.core.joints_of(b))
    }

    /// One end of a joint moved from engine body `from` to `to` (its anchor's material went into another body), the same
    /// joint to scene code.
    pub fn engine_rebind_joint(&mut self, joint: JointId, from: u32, to: u32) -> Result<(), String> {
        let (f, t) = (*self.bodies.get(&from).ok_or("physics: unknown body")?, *self.bodies.get(&to).ok_or("physics: unknown body")?);
        self.core.rebind_joint(joint, f, t)
    }

    /// A joint let go of by the engine (its anchor's material carved away): gone from the world, reported to scene code
    /// as broken (`broken`).
    pub fn engine_break_joint(&mut self, joint: JointId) {
        if let Some(id) = self.joints.iter().find(|(_, &v)| v == joint).map(|(&k, _)| k) {
            self.joints.remove(&id);
            self.note_broken(id);
        }
        self.core.remove_joint(joint);
    }

    /// A broken joint noted for `broken` (the newest `MAX_JOINTS` kept: nobody asking does not make it grow).
    fn note_broken(&mut self, id: u32) {
        if self.broken.len() >= MAX_JOINTS {
            self.broken.remove(0);
        }
        self.broken.push(id);
    }

    /// The level's fixed body, if a level with colliders is loaded.
    pub fn engine_level(&self) -> Option<u32> {
        self.level_body
    }

    /// An engine voxel collider's cells changed (`Physics::edit_voxels`). Returns the cells it has; with none left
    /// it is gone, its id with it.
    pub fn engine_edit_voxels(&mut self, id: u32, clear: &[[i32; 3]], fill: &[[i32; 3]]) -> Result<usize, String> {
        let c = *self.colliders.get(&id).filter(|_| id >= ENGINE_IDS).ok_or_else(|| format!("physics: unknown engine collider {id}"))?;
        let left = self.core.edit_voxels(c, clear, fill);
        if !self.core.has_collider(c) {
            self.colliders.remove(&id);
            self.unmap(c);
        }
        left
    }

    /// An engine body with its colliders and joints, or an engine collider, gone. (Unknown ids: nothing.)
    pub fn engine_remove(&mut self, body: Option<u32>, collider: Option<u32>) {
        if let Some(b) = body.filter(|&b| b >= ENGINE_IDS).and_then(|b| self.bodies.get(&b).copied()) {
            // (Scene code's joints on it go with it: broken.)
            for (j, _, _) in self.core.joints_of(b) {
                if let Some(id) = self.joints.iter().find(|(_, &v)| v == j).map(|(&k, _)| k) {
                    self.note_broken(id);
                }
            }
            self.forget_body(b);
        }
        if let Some(c) = collider.filter(|&c| c >= ENGINE_IDS).and_then(|c| self.colliders.remove(&c)) {
            self.core.remove_collider(c);
            self.unmap(c);
        }
    }

    /// Where a body is (any body, by its caller's id): position, rotation, and whether it sleeps. None: no such body
    /// (never made, removed, or taken out by the world after a tick: `faults`).
    pub fn engine_pose(&self, id: u32) -> Option<([f32; 3], [f32; 4], bool)> {
        let b = *self.bodies.get(&id)?;
        self.core.pose(b).map(|(p, q)| (p, q, self.core.sleeping(b)))
    }

    /// A body's velocities set (the engine's: a part that comes loose keeps moving as what it broke from did).
    pub fn engine_velocity(&mut self, id: u32, linvel: [f32; 3], angvel: [f32; 3]) -> Result<(), String> {
        let b = *self.bodies.get(&id).ok_or_else(|| format!("physics: unknown body {id}"))?;
        self.core.set_velocity(b, Some(linvel), Some(angvel))
    }

    /// A body's linear and angular velocity.
    pub fn engine_velocity_of(&self, id: u32) -> Option<([f32; 3], [f32; 3])> {
        self.core.velocity(*self.bodies.get(&id)?)
    }

    /// An engine body's mass properties as the world has them (`Physics::mass_properties`).
    pub fn engine_mass_properties(&self, id: u32) -> Option<(f32, [f32; 3], [f32; 3])> {
        self.core.mass_properties(*self.bodies.get(&id).filter(|_| id >= ENGINE_IDS)?)
    }

    /// An engine body's mass properties (`Physics::set_mass_properties`).
    pub fn engine_mass(&mut self, id: u32, mass: f32, com: [f32; 3], inertia: [f32; 6]) -> Result<(), String> {
        let b = *self.bodies.get(&id).filter(|_| id >= ENGINE_IDS).ok_or_else(|| format!("physics: unknown engine body {id}"))?;
        self.core.set_mass_properties(b, mass, com, inertia)
    }

    /// An engine collider's contacts reported or not (`Physics::set_events`).
    pub fn engine_events(&mut self, id: u32, on: bool) -> Result<(), String> {
        let c = *self.colliders.get(&id).filter(|_| id >= ENGINE_IDS).ok_or_else(|| format!("physics: unknown engine collider {id}"))?;
        self.core.set_events(c, on)
    }

    /// The memory of the world's voxel shapes, bytes (`Physics::voxel_bytes`).
    pub fn voxel_bytes(&self) -> usize {
        self.core.voxel_bytes()
    }

    /// A voxel collider's solid cells, counted (None: none, or no voxel shape).
    pub fn engine_voxels(&self, id: u32) -> Option<usize> {
        self.core.voxels(*self.colliders.get(&id)?)
    }

    /// An engine voxel collider's cells, sorted (`Physics::voxel_cells`).
    pub fn engine_voxel_cells(&self, id: u32) -> Option<Vec<[i32; 3]>> {
        self.core.voxel_cells(*self.colliders.get(&id)?)
    }

    /// The world's body of the caller's id in `a[key]`.
    fn body(&self, a: &Value, key: &str) -> Result<BodyId, String> {
        let b = id(a, key)?;
        self.bodies.get(&b).copied().ok_or_else(|| format!("physics: unknown body {b}"))
    }

    /// A query's `skip`: the body whose colliders it passes through.
    fn skip(&self, a: &Value) -> Result<Option<BodyId>, String> {
        Ok(opt_id(a, "skip")?.and_then(|b| self.bodies.get(&b).copied()))
    }

    /// The scene object the body of caller id `b` drives.
    fn object_of(&self, b: Option<u32>) -> Option<u32> {
        self.drives.get(self.bodies.get(&b?)?).map(|d| d.0)
    }

    /// Removes a body (the world's id) with its colliders and joints, and their ids.
    /// A collider's ids forgotten (it is gone from the world), and the pairs it touched ended for scene code.
    fn unmap(&mut self, c: ColliderId) -> Option<(u32, Option<u32>)> {
        let ids = self.collider_ids.remove(&c)?;
        let gone: Vec<(u32, u32)> = self.touching.keys().copied().filter(|&(a, b)| a == ids.0 || b == ids.0).collect();
        for k in gone {
            let (ba, bb) = self.touching.remove(&k).unwrap();
            // (Held as the world's own contacts are, up to `MAX_CONTACTS`: nobody may take them.)
            if self.ended.len() >= crate::world::limits::MAX_CONTACTS {
                self.ended.remove(0);
            }
            self.ended.push(json!({ "a": k.0, "b": k.1, "bodyA": ba, "bodyB": bb, "started": false }));
        }
        Some(ids)
    }

    fn forget_body(&mut self, b: BodyId) {
        let gone = self.core.remove_body(b);
        self.forget_ids(b, &gone);
    }

    /// Forgets the ids of a body the world no longer has (the world's id), of the colliders that went with it and of
    /// its joints.
    fn forget_ids(&mut self, b: BodyId, colliders: &[ColliderId]) {
        for c in colliders {
            if let Some((id, _)) = self.unmap(*c) {
                self.colliders.remove(&id);
            }
        }
        if let Some((o, slot)) = self.drives.remove(&b) {
            self.driven.remove(&o);
            if let Some(s) = slot {
                self.slots.remove(&s);
            }
        }
        if let Some(id) = self.body_ids.remove(&b) {
            self.bodies.remove(&id);
        }
        // (Its joints went with it: those the world no longer has.)
        if !self.joints.is_empty() {
            let core = &self.core;
            self.joints.retain(|_, j| core.has_joint(*j));
        }
    }

    fn hit(&self, h: Option<Hit>) -> Value {
        match h {
            Some(h) => json!({
                "collider": self.collider_ids.get(&h.collider).map(|c| c.0), "body": h.body.and_then(|b| self.body_ids.get(&b)).filter(|&&b| Some(b) != self.level_body), "object": h.body.and_then(|b| self.drives.get(&b)).map(|d| d.0),
                "distance": h.distance, "point": h.point, "normal": h.normal,
            }),
            None => Value::Null,
        }
    }

    /// The object body `b` drives, placed at the body's pose, and its slot of `poses` written. False if the object is
    /// gone (the body goes with it).
    fn place(&mut self, scene: &mut impl Scene, b: BodyId, position: [f32; 3], rotation: [f32; 4]) -> bool {
        let Some(&(o, slot)) = self.drives.get(&b) else { return true };
        let Some((p, q)) = scene.place(o, position, rotation) else {
            self.forget_body(b);
            return false;
        };
        if let Some(s) = slot {
            let at = s as usize * POSE;
            self.poses[at..at + 3].copy_from_slice(&p);
            self.poses[at + 3..at + POSE].copy_from_slice(&q);
        }
        true
    }

    /// The object body `b` drives placed where the body is (`place`).
    fn place_at_body(&mut self, scene: &mut impl Scene, b: BodyId) {
        if let Some((p, q)) = self.core.pose(b) {
            self.place(scene, b, p, q);
        }
    }

    /// The scene removed these objects: the bodies that drove them go too (an awake body would find out when it next
    /// placed its object; a sleeping one never would, and the object's id may be used again).
    pub fn objects_removed(&mut self, objects: &[u32]) {
        if self.driven.is_empty() {
            return;
        }
        let doomed: Vec<BodyId> = objects.iter().filter(|o| self.driven.contains(o)).filter_map(|o| self.drives.iter().find(|(_, d)| d.0 == *o).map(|(&b, _)| b)).collect();
        for b in doomed {
            self.forget_body(b);
        }
    }

    /// A kinematic body's object was moved by scene code: its slot of `poses` is where the object is now.
    fn note_pose(&mut self, scene: &mut impl Scene, b: BodyId) {
        if let Some(&(o, Some(s))) = self.drives.get(&b) {
            if let Some((p, q)) = scene.world_pose(o).and_then(|(p, q)| scene.place(o, p, q)) {
                let at = s as usize * POSE;
                self.poses[at..at + 3].copy_from_slice(&p);
                self.poses[at + 3..at + POSE].copy_from_slice(&q);
            }
        }
    }

    /// The driven objects' poses by slot (`POSE` floats each: position, rotation xyzw, in the object's parent's space),
    /// as of the last sync. A slot no body holds keeps what it had.
    pub fn poses(&self) -> &[f32] {
        &self.poses
    }

    /// One tick.
    pub fn tick(&mut self) {
        self.core.step();
        for f in self.core.take_faults() {
            self.fault(f);
        }
        self.core.awake(&mut self.moved);
        // (Made unique once it holds the bodies a few times over: an offline settle makes up to 36,000 ticks before
        // its one sync, and 20,000 awake bodies would add 720 million entries.)
        if self.moved.len() > 4 * self.core.bodies() + 1024 {
            self.moved.sort_unstable();
            self.moved.dedup();
        }
        self.ticked = true;
    }

    /// What the world took out after a tick: its ids forgotten, and the body reported ("faults", "state").
    fn fault(&mut self, f: crate::Fault) {
        let Some(b) = f.body else {
            self.forget_colliders(&f.colliders);
            return;
        };
        let caller = self.body_ids.get(&b).copied();
        let object = self.drives.get(&b).map(|d| d.0);
        self.forget_ids(b, &f.colliders);
        let Some(caller) = caller else { return };
        if self.faults.len() == MAX_FAULTS {
            self.faults.remove(0);
        }
        self.faults.push((caller, object, f.left_world, f.reason));
        if self.faulted.len() == MAX_FAULTS {
            self.faulted.pop_first();
        }
        self.faulted.insert(caller, f.reason);
    }

    fn forget_colliders(&mut self, colliders: &[ColliderId]) {
        for c in colliders {
            if let Some((id, _)) = self.unmap(*c) {
                self.colliders.remove(&id);
            }
        }
    }

    /// After a frame's ticks (none, at a frame rate above the tick rate): places the objects of the bodies that moved
    /// in them, where they are now, and ends the frame's forces. Returns how many bodies moved.
    pub fn sync(&mut self, scene: &mut impl Scene) -> usize {
        let mut n = 0;
        if std::mem::take(&mut self.ticked) {
            // Those awake after any of the ticks, and those awake before the first (one that came to rest and fell
            // asleep in a tick moved in it).
            let mut moved = std::mem::take(&mut self.moved);
            moved.append(&mut self.awake);
            moved.sort_unstable();
            moved.dedup();
            for &b in &moved {
                if let Some((p, q)) = self.core.pose(b) {
                    self.place(scene, b, p, q);
                }
            }
            n = moved.len();
            self.core.awake(&mut self.awake);
        }
        // A force lasts the ticks of the frame it was applied in: scene code applies it again every frame it acts, at
        // any frame rate.
        for b in std::mem::take(&mut self.forced) {
            self.core.clear_forces(b);
        }
        n
    }

    /// The whole ticks a frame of `dt` seconds makes (at most `MAX_TICKS`: a longer frame slows the world down rather
    /// than stepping without bound).
    pub fn due(&mut self, dt: f64) -> u32 {
        self.behind += if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        // (A frame of exactly one tick is one tick, whatever its f32 rounds to.)
        let due = ((self.behind + 1e-6) / TICK as f64).floor() as u32;
        // (What a frame owing more than `MAX_TICKS` leaves is dropped; one owing exactly that many keeps its remainder,
        // or a steady 12 fps would lose a fifth of its time.)
        let ticks = due.min(MAX_TICKS);
        self.behind = if due > MAX_TICKS { 0.0 } else { (self.behind - ticks as f64 * TICK as f64).max(0.0) };
        ticks
    }

    /// The ticks left until the world has made `ticks` in all (an offline render at a scene time: the world as it is
    /// after that long; at most 36,000). A world already past that makes none: it cannot go back.
    pub fn due_until(&self, ticks: u32) -> u32 {
        ticks.min(36_000).saturating_sub(self.core.ticks().min(u32::MAX as u64) as u32)
    }

    pub fn bodies(&self) -> usize {
        self.core.bodies()
    }

    /// The world's hash (`Physics::hash`).
    pub fn hash(&self) -> u64 {
        self.core.hash()
    }
}

#[cfg(test)]
#[path = "robustness.rs"]
mod robustness;

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene of objects with a position, a rotation and maybe a parent (parents here only move, they do not turn).
    #[derive(Default)]
    struct Objects {
        at: BTreeMap<u32, ([f32; 3], [f32; 4], Option<u32>)>,
        driven: Vec<u32>,
        geometries: BTreeMap<u32, (Vec<f32>, Vec<u32>)>,
        placed: usize,
    }

    impl Objects {
        fn offset(&self, o: u32) -> [f32; 3] {
            match self.at.get(&o).and_then(|x| x.2) {
                Some(p) => {
                    let (a, b) = (self.at[&p].0, self.offset(p));
                    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
                }
                None => [0.0; 3],
            }
        }
    }

    impl Scene for Objects {
        fn world_pose(&self, o: u32) -> Option<([f32; 3], [f32; 4])> {
            let (p, q, _) = *self.at.get(&o)?;
            let d = self.offset(o);
            Some(([p[0] + d[0], p[1] + d[1], p[2] + d[2]], q))
        }
        fn place(&mut self, o: u32, p: [f32; 3], q: [f32; 4]) -> Option<([f32; 3], [f32; 4])> {
            let d = self.offset(o);
            let e = self.at.get_mut(&o)?;
            (e.0, e.1) = ([p[0] - d[0], p[1] - d[1], p[2] - d[2]], q);
            self.placed += 1;
            Some((e.0, e.1))
        }
        fn drive(&mut self, o: u32) {
            self.driven.push(o);
        }
        fn geometry(&self, id: u32) -> Option<(&[f32], &[u32])> {
            self.geometries.get(&id).map(|(p, i)| (p.as_slice(), i.as_slice()))
        }
    }

    const Q: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn scene() -> Objects {
        let mut s = Objects::default();
        // 1: the ground (top at y = 0), 2: a box 2 m up, 3: a group at (10, 0, 0), 4: a box under it.
        s.at.insert(1, ([0.0, -0.5, 0.0], Q, None));
        s.at.insert(2, ([0.0, 2.0, 0.0], Q, None));
        s.at.insert(3, ([10.0, 0.0, 0.0], Q, None));
        s.at.insert(4, ([0.0, 1.0, 0.0], Q, Some(3)));
        s
    }

    fn run(b: &mut Bridge, s: &mut Objects, ticks: u32) -> usize {
        (0..ticks).map(|_| {
            b.tick();
            b.sync(s)
        })
        .sum()
    }

    fn ground(b: &mut Bridge, s: &mut Objects) {
        b.call(s, "body", &json!({ "id": 1, "kind": "fixed", "object": 1 })).unwrap();
        b.call(s, "collider", &json!({ "id": 1, "body": 1, "shape": { "type": "box", "halfExtents": [20, 0.5, 20] } })).unwrap();
    }

    /// A body drives its object: the box falls onto the ground and rests on it, the object where the body is; one under
    /// a parent is placed in the parent's space; `poses` holds them by slot; nothing is placed once they sleep.
    #[test]
    fn bodies_drive_objects() {
        let (mut b, mut s) = (Bridge::default(), scene());
        ground(&mut b, &mut s);
        for (id, slot) in [(2, 0), (4, 3)] {
            b.call(&mut s, "body", &json!({ "id": id, "object": id, "slot": slot })).unwrap();
            b.call(&mut s, "collider", &json!({ "id": id, "body": id, "shape": { "type": "box", "halfExtents": [0.25, 0.25, 0.25] } })).unwrap();
        }
        assert_eq!(s.driven, vec![2, 4], "the fixed ground is not driven");
        assert_eq!(b.poses().len(), 4 * POSE);
        assert_eq!(b.poses()[..3], [0.0, 2.0, 0.0]);
        assert_eq!(b.poses()[3 * POSE..3 * POSE + 3], [0.0, 1.0, 0.0], "in its parent's space");
        let moved = run(&mut b, &mut s, 240);
        assert!(moved > 60 && moved < 400, "placed while they moved only: {moved}");
        let rest = s.at[&2].0;
        // (A flat drop of 2 m ends within a centimetre of where it began sideways: the solver's own noise.)
        assert!(rest[0].abs() < 1e-2 && (rest[1] - 0.25).abs() < 3e-3 && rest[2].abs() < 1e-2, "{rest:?}");
        let under = s.at[&4].0;
        assert!((under[0]).abs() < 1e-2 && (under[1] - 0.25).abs() < 3e-3, "local to the group at x = 10: {under:?}");
        assert_eq!(b.poses()[..3], rest);
        assert_eq!(b.poses()[3 * POSE..3 * POSE + 3], under);
        let st = b.call(&mut s, "state", &json!({ "body": 4 })).unwrap();
        assert!((st["position"][0].as_f64().unwrap() - 10.0).abs() < 1e-2 && st["sleeping"] == true, "{st}");
        // Asleep: ticks place nothing.
        let before = s.placed;
        assert_eq!(run(&mut b, &mut s, 30), 0);
        assert_eq!(s.placed, before);
        // An impulse wakes it; it moves again. (The id as a script engine hands back one it read from an answer: a
        // whole number written as a double. One that is not whole, or negative, or past 2^32, is no id.)
        b.call(&mut s, "impulse", &json!({ "body": 2.0, "impulse": [0, 300, 0] })).unwrap();
        assert!(run(&mut b, &mut s, 10) >= 10);
        assert!(s.at[&2].0[1] > 0.5);
        for bad in [json!(2.5), json!(-2.0), json!(4294967296.0), json!("2"), json!(f64::MAX)] {
            let e = b.call(&mut s, "impulse", &json!({ "body": bad, "impulse": [0, 300, 0] })).unwrap_err();
            assert!(e.contains("must be an id"), "{bad}: {e}");
        }
    }

    /// Scene code moves a body's object: a dynamic body is there at once with its velocity; a kinematic one goes there
    /// through the next tick and pushes what is in its way.
    #[test]
    fn bodies_follow_objects_scene_code_moved() {
        let (mut b, mut s) = (Bridge::default(), scene());
        ground(&mut b, &mut s);
        b.call(&mut s, "body", &json!({ "id": 2, "object": 2, "slot": 0, "linvel": [1, 0, 0] })).unwrap();
        b.call(&mut s, "collider", &json!({ "id": 2, "body": 2, "shape": { "type": "sphere", "radius": 0.25 } })).unwrap();
        s.at.get_mut(&2).unwrap().0 = [3.0, 5.0, 0.0];
        b.call(&mut s, "follow", &json!({ "body": 2 })).unwrap();
        let st = b.call(&mut s, "state", &json!({ "body": 2 })).unwrap();
        assert_eq!((st["position"][0].as_f64(), st["position"][1].as_f64(), st["linvel"][0].as_f64()), (Some(3.0), Some(5.0), Some(1.0)));
        assert_eq!(b.poses()[..3], [3.0, 5.0, 0.0]);
        // A kinematic paddle (object 4, under the group) pushed into a resting ball.
        let mut s = scene();
        let mut b = Bridge::default();
        ground(&mut b, &mut s);
        s.at.get_mut(&2).unwrap().0 = [10.0, 0.25, 1.0];
        s.at.get_mut(&4).unwrap().0 = [0.0, 0.25, 0.0];
        b.call(&mut s, "body", &json!({ "id": 2, "object": 2 })).unwrap();
        b.call(&mut s, "collider", &json!({ "id": 2, "body": 2, "shape": { "type": "sphere", "radius": 0.25 } })).unwrap();
        b.call(&mut s, "body", &json!({ "id": 4, "object": 4, "kind": "kinematic", "slot": 1 })).unwrap();
        b.call(&mut s, "collider", &json!({ "id": 4, "body": 4, "shape": { "type": "box", "halfExtents": [0.5, 0.25, 0.25] } })).unwrap();
        for k in 1..=30 {
            s.at.get_mut(&4).unwrap().0 = [0.0, 0.25, k as f32 * 0.02];
            b.call(&mut s, "follow", &json!({ "body": 4 })).unwrap();
            assert_eq!(b.poses()[POSE + 2], k as f32 * 0.02, "what scene code wrote is what it reads");
            run(&mut b, &mut s, 1);
        }
        let st = b.call(&mut s, "state", &json!({ "body": 4 })).unwrap();
        assert!((st["position"][2].as_f64().unwrap() - 0.6).abs() < 1e-4 && (st["position"][0].as_f64().unwrap() - 10.0).abs() < 1e-4, "{st}");
        assert!(s.at[&2].0[2] > 1.05, "pushed: {:?}", s.at[&2].0);
    }

    /// Queries report the caller's ids and the scene objects (a collider is seen as soon as it is added, before any
    /// tick); a force acts through one frame's ticks only; contacts come in the order they happened with their bodies.
    #[test]
    fn queries_forces_and_contacts() {
        let (mut b, mut s) = (Bridge::default(), scene());
        ground(&mut b, &mut s);
        // (Weightless, so it stays where the queries expect it.)
        b.call(&mut s, "body", &json!({ "id": 7, "object": 2, "gravityScale": 0 })).unwrap();
        b.call(&mut s, "collider", &json!({ "id": 70, "body": 7, "shape": { "type": "box", "halfExtents": [0.25, 0.25, 0.25] }, "events": true })).unwrap();
        // (A query sees what the calls made, before any tick, and the same after one.)
        let before = b.call(&mut s, "ray", &json!({ "origin": [0, 10, 0], "direction": [0, -1, 0] })).unwrap();
        run(&mut b, &mut s, 1);
        let h = b.call(&mut s, "ray", &json!({ "origin": [0, 10, 0], "direction": [0, -1, 0] })).unwrap();
        assert_eq!((before["collider"].as_u64(), before["distance"].as_f64().map(|d| (d * 1e4).round())), (Some(70), Some(77500.0)), "before a tick: {before}");
        assert_eq!((h["collider"].as_u64(), h["body"].as_u64(), h["object"].as_u64()), (Some(70), Some(7), Some(2)));
        assert!((h["distance"].as_f64().unwrap() - 7.75).abs() < 1e-4 && h["normal"][1] == 1.0);
        let all = b.call(&mut s, "ray", &json!({ "origin": [0, 10, 0], "direction": [0, -1, 0], "all": true })).unwrap();
        assert_eq!(all.as_array().unwrap().iter().map(|h| h["object"].as_u64().unwrap()).collect::<Vec<_>>(), vec![2, 1], "nearest first");
        let skipped = b.call(&mut s, "ray", &json!({ "origin": [0, 10, 0], "direction": [0, -1, 0], "skip": 7 })).unwrap();
        assert_eq!(skipped["object"].as_u64(), Some(1));
        assert!(b.call(&mut s, "ray", &json!({ "origin": [5, 10, 0], "direction": [0, 1, 0] })).unwrap().is_null());
        let o = b.call(&mut s, "overlap", &json!({ "shape": { "type": "sphere", "radius": 0.5 }, "position": [0, 2.2, 0] })).unwrap();
        assert_eq!(o, json!([{ "collider": 70, "body": 7, "object": 2 }]));
        let w = b.call(&mut s, "sweep", &json!({ "shape": { "type": "sphere", "radius": 0.5 }, "position": [0, 6, 0], "direction": [0, -1, 0] })).unwrap();
        assert!((w["distance"].as_f64().unwrap() - 3.25).abs() < 1e-3 && w["object"] == 2, "{w}");
        // A force of 600 N on its 125 kg through one frame's tick: 0.08 m/s more; the next frame has no force.
        let m = b.call(&mut s, "state", &json!({ "body": 7 })).unwrap()["mass"].as_f64().unwrap();
        assert!((m - 125.0).abs() < 1e-3, "0.5 m cube of water: {m}");
        b.call(&mut s, "body", &json!({ "id": 8, "position": [50, 0, 0] })).unwrap();
        b.call(&mut s, "collider", &json!({ "id": 80, "body": 8, "shape": { "type": "sphere", "radius": 0.5 }, "mass": 3 })).unwrap();
        assert_eq!(b.call(&mut s, "state", &json!({ "body": 8 })).unwrap()["mass"].as_f64(), Some(3.0));
        b.call(&mut s, "remove", &json!({ "body": 8 })).unwrap();
        b.call(&mut s, "force", &json!({ "body": 7, "force": [0, 600, 0] })).unwrap();
        run(&mut b, &mut s, 1);
        let up = b.call(&mut s, "state", &json!({ "body": 7 })).unwrap()["linvel"][1].as_f64().unwrap();
        assert!((up - 600.0 / 125.0 / 60.0).abs() < 1e-4, "{up}");
        run(&mut b, &mut s, 1);
        let then = b.call(&mut s, "state", &json!({ "body": 7 })).unwrap()["linvel"][1].as_f64().unwrap();
        assert!((then - up).abs() < 1e-6, "{then}");
        // A frame without a tick (a frame rate above the tick rate) ends its force too: none is left for the next.
        b.call(&mut s, "force", &json!({ "body": 7, "force": [0, 600, 0] })).unwrap();
        assert_eq!(b.sync(&mut s), 0);
        run(&mut b, &mut s, 1);
        assert!((b.call(&mut s, "state", &json!({ "body": 7 })).unwrap()["linvel"][1].as_f64().unwrap() - up).abs() < 1e-6);
        // Sent down, it lands: one contact started, between the ground's collider (1, of body 1) and its own. (Its
        // other velocity is kept by a call that sets one.)
        b.call(&mut s, "velocity", &json!({ "body": 7, "angvel": [0, 0, 0] })).unwrap();
        assert_eq!(b.call(&mut s, "state", &json!({ "body": 7 })).unwrap()["linvel"][1].as_f64().unwrap(), then);
        b.call(&mut s, "velocity", &json!({ "body": 7, "linvel": [0, -3, 0] })).unwrap();
        run(&mut b, &mut s, 60);
        assert_eq!(b.call(&mut s, "contacts", &Value::Null).unwrap(), json!([{ "a": 1, "b": 70, "bodyA": 1, "bodyB": 7, "started": true }]));
        assert_eq!(b.call(&mut s, "contacts", &Value::Null).unwrap(), json!([]));
        // An impulse at a corner turns it.
        b.call(&mut s, "impulse", &json!({ "body": 7, "impulse": [0, 100, 0], "point": [0.25, 0.0, 0.25] })).unwrap();
        run(&mut b, &mut s, 1);
        let av = b.call(&mut s, "state", &json!({ "body": 7 })).unwrap()["angvel"].clone();
        assert!(av[0].as_f64().unwrap().abs() > 1.0 && av[2].as_f64().unwrap().abs() > 1.0, "{av}");
    }

    /// Shapes from a geometry of the scene; what a call refuses and why; limits; removal; an object that is gone takes
    /// its body with it.
    #[test]
    fn shapes_errors_and_removal() {
        let (mut b, mut s) = (Bridge::default(), scene());
        // A unit tetrahedron and a 2 m square of two triangles.
        s.geometries.insert(1, (vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], vec![0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3]));
        s.geometries.insert(2, (vec![-1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0], vec![0, 2, 1, 0, 3, 2]));
        b.call(&mut s, "collider", &json!({ "id": 1, "shape": { "type": "trimesh", "geometry": 2, "scale": [5, 1, 5] } })).unwrap();
        b.call(&mut s, "body", &json!({ "id": 2, "object": 2 })).unwrap();
        b.call(&mut s, "collider", &json!({ "id": 2, "body": 2, "shape": { "type": "convexHull", "geometry": 1, "scale": [0.5, 0.5, 0.5] } })).unwrap();
        run(&mut b, &mut s, 240);
        let y = s.at[&2].0[1];
        assert!(y > -0.01 && y < 0.3, "the hull rests on the scaled trimesh: {y}");
        let h = b.call(&mut s, "ray", &json!({ "origin": [4.5, 3, 4.5], "direction": [0, -1, 0] })).unwrap();
        assert!((h["distance"].as_f64().unwrap() - 3.0).abs() < 1e-4 && h["collider"] == 1 && h["body"].is_null() && h["object"].is_null(), "a static collider: {h}");
        for (op, args, why) in [
            ("body", json!({ "id": 2 }), "exists"),
            ("body", json!({ "id": 3, "object": 99 }), "unknown object"),
            ("body", json!({ "id": 3, "object": 2 }), "already has a body"),
            ("body", json!({ "id": 3, "kind": "soft" }), "none of dynamic"),
            ("body", json!({ "id": 3, "position": [0, "x", 0] }), "three finite numbers"),
            ("body", json!({ "id": 3, "slot": 0 }), "without an object"),
            ("collider", json!({ "id": 2, "shape": { "type": "sphere", "radius": 1 } }), "exists"),
            ("collider", json!({ "id": 3, "body": 9, "shape": { "type": "sphere", "radius": 1 } }), "unknown body"),
            ("collider", json!({ "id": 3, "shape": { "type": "sphere", "radius": -1 } }), "radius must be from 1e-3 to 1e4"),
            ("collider", json!({ "id": 3, "shape": { "type": "cone" } }), "none of box"),
            ("collider", json!({ "id": 3, "shape": { "type": "trimesh", "geometry": 9 } }), "unknown geometry"),
            ("joint", json!({ "id": 1, "a": 2, "b": 9, "type": "fixed" }), "unknown body"),
            ("joint", json!({ "id": 1, "a": 2, "b": 2, "type": "weld" }), "none of fixed"),
            ("impulse", json!({ "body": 9 }), "unknown body"),
            ("step", json!({}), "unknown call"),
        ] {
            let e = b.call(&mut s, op, &args).unwrap_err();
            assert!(e.contains(why), "{op} {args}: {e}");
        }
        // Removing a body takes its collider; the ids are free again.
        b.call(&mut s, "remove", &json!({ "body": 2 })).unwrap();
        assert_eq!(b.bodies(), 0);
        b.call(&mut s, "body", &json!({ "id": 2, "object": 2, "slot": 0 })).unwrap();
        b.call(&mut s, "collider", &json!({ "id": 2, "body": 2, "shape": { "type": "sphere", "radius": 0.2 } })).unwrap();
        // The object is removed from the scene while its body moves: the body goes at the next sync.
        b.call(&mut s, "impulse", &json!({ "body": 2, "impulse": [0, 50, 0] })).unwrap();
        s.at.remove(&2);
        run(&mut b, &mut s, 2);
        assert_eq!(b.bodies(), 0);
        assert!(b.call(&mut s, "state", &json!({ "body": 2 })).is_err());
        // The limit on bodies.
        for k in 0..MAX_BODIES as u32 {
            b.call(&mut s, "body", &json!({ "id": 100 + k, "kind": "fixed" })).unwrap();
        }
        assert!(b.call(&mut s, "body", &json!({ "id": 5 })).unwrap_err().contains("more than"));
    }

    /// Frames make whole ticks: 60 a second whatever the frame rate, at most `MAX_TICKS` a frame; an offline render
    /// asks for a tick count once.
    #[test]
    fn ticks_due() {
        let mut b = Bridge::default();
        assert_eq!((0..60).map(|_| b.due(1.0 / 60.0)).sum::<u32>(), 60);
        assert_eq!((0..120).map(|_| b.due(1.0 / 120.0)).sum::<u32>(), 60);
        assert_eq!((0..30).map(|_| b.due(1.0 / 30.0)).sum::<u32>(), 60);
        assert_eq!(b.due(10.0), MAX_TICKS);
        assert_eq!((b.due(f64::NAN), b.due(0.0), b.due(-1.0)), (0, 0, 0), "a long frame leaves nothing behind");
        assert_eq!(b.due_until(120), 120);
        for _ in 0..50 {
            b.tick();
        }
        assert_eq!((b.due_until(120), b.due_until(40), b.due_until(1_000_000)), (70, 0, 35_950));
    }
}

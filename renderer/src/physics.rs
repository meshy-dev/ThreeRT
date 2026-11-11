//! Physics in the renderer (ROADMAP § 6 step 7): the game session's physics world, owned and stepped by the engine at
//! a fixed tick. Everything about it is `physics_core::bridge::Bridge` (the world on Rapier, scene code's calls, the
//! objects that bodies drive; it has no GPU in it and builds for wasm32); here is the scene it drives, the renderer's
//! `World`, and the frame's timing. Scene code reaches it through `__engine.physics(op, args)` and reads the driven
//! objects' poses through `__engine.physicsPoses()`; it never steps it. After a frame's ticks the bodies that moved
//! place their objects: a driven mesh or model is an instance, so a move is its TLAS transform, no triangle (live.rs
//! `move_instances`), and nothing of it passes through the script. A session that never makes a physics call has no
//! world and pays nothing.

use crate::world::World;
use anyhow::{anyhow, Result};
use glam::{Mat4, Quat, Vec3};
use physics_core::bridge::{Bridge, Scene};
use serde_json::Value;
use web_time::Instant;

/// A frame's physics, for the frame log: the ticks stepped and their time, the time to place the driven objects, the
/// bodies and how many of them moved.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub ticks: u32,
    pub step_ms: f64,
    pub sync_ms: f64,
    pub bodies: usize,
    pub moved: usize,
}

impl Scene for World {
    fn world_pose(&self, object: u32) -> Option<([f32; 3], [f32; 4])> {
        self.objects.contains_key(&object).then(|| {
            let (_, r, t) = self.world_matrix(object).to_scale_rotation_translation();
            (t.to_array(), r.to_array())
        })
    }

    /// Under a parent, in the parent's space.
    fn place(&mut self, object: u32, position: [f32; 3], rotation: [f32; 4]) -> Option<([f32; 3], [f32; 4])> {
        let parent = self.objects.get(&object)?.parent;
        let (mut t, mut r) = (Vec3::from(position), Quat::from_array(rotation));
        if let Some(p) = parent {
            let local = self.world_matrix(p).inverse() * Mat4::from_rotation_translation(r, t);
            let (_, lr, lt) = local.to_scale_rotation_translation();
            (t, r) = (lt, lr);
        }
        // (A parent scaled to nothing has no inverse: the object stays where it is.)
        if !(t.is_finite() && r.is_finite()) {
            let o = &self.objects[&object];
            return Some((o.position.to_array(), o.rotation.to_array()));
        }
        self.place_driven(object, t, r);
        Some((t.to_array(), r.to_array()))
    }

    fn drive(&mut self, object: u32) {
        let _ = self.set_driven(object);
    }

    fn geometry(&self, id: u32) -> Option<(&[f32], &[u32])> {
        self.geometries.get(&id).map(|g| (g.positions.as_slice(), g.indices.as_slice()))
    }
}

#[derive(Default)]
pub struct PhysicsWorld {
    bridge: Bridge,
    stats: Stats,
    /// Time in scene code's calls since `take_call_ms` (a click's chain, THREERT_CLICK_LOG).
    call_ms: f64,
}

impl PhysicsWorld {
    /// One engine call from scene code (`Bridge::call`). Returns its answer as JSON.
    pub fn call(&mut self, w: &mut World, op: &str, a: &Value) -> Result<Value> {
        let t = Instant::now();
        let r = self.bridge.call(w, op, a).map_err(|e| anyhow!(e));
        self.call_ms += t.elapsed().as_secs_f64() * 1e3;
        r
    }

    /// The bridge itself, for the engine's own bodies and colliders (the voxel world's: voxel.rs).
    pub fn bridge(&mut self) -> &mut Bridge {
        &mut self.bridge
    }

    pub fn bridge_ref(&self) -> &Bridge {
        &self.bridge
    }

    /// The scene removed these objects: their bodies go (`Bridge::objects_removed`).
    pub fn objects_removed(&mut self, objects: &[u32]) {
        self.bridge.objects_removed(objects);
    }

    pub fn take_call_ms(&mut self) -> f64 {
        std::mem::take(&mut self.call_ms)
    }

    /// The driven objects' poses by slot (`Bridge::poses`).
    pub fn poses(&self) -> &[f32] {
        self.bridge.poses()
    }

    /// `ticks` ticks, then the moved bodies' objects placed.
    fn step(&mut self, w: &mut World, ticks: u32) {
        let t0 = Instant::now();
        for _ in 0..ticks {
            self.bridge.tick();
        }
        let t1 = Instant::now();
        let moved = self.bridge.sync(w);
        self.stats.ticks += ticks;
        self.stats.step_ms += (t1 - t0).as_secs_f64() * 1e3;
        self.stats.sync_ms += t1.elapsed().as_secs_f64() * 1e3;
        self.stats.bodies = self.bridge.bodies();
        self.stats.moved += moved;
    }

    /// A frame of `dt` seconds: the whole ticks that fit (`Bridge::due`).
    pub fn advance(&mut self, w: &mut World, dt: f64) {
        let ticks = self.bridge.due(dt);
        self.step(w, ticks);
    }

    /// Steps until the world has made `ticks` ticks in all (an offline render at a scene time; `Bridge::due_until`).
    /// Settling is not a frame: with no tick due it does nothing, so the forces scene code applied (in setup, before a
    /// live run's first frame) still act in the first frame's ticks instead of ending in an empty sync.
    pub fn settle(&mut self, w: &mut World, ticks: u32) {
        let ticks = self.bridge.due_until(ticks);
        if ticks > 0 {
            self.step(w, ticks);
        }
    }

    /// The frame's figures, and a fresh count for the next.
    pub fn take_stats(&mut self) -> Stats {
        std::mem::take(&mut self.stats)
    }

    /// The world's hash (`Physics::hash`).
    pub fn hash(&self) -> u64 {
        self.bridge.hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Kind;
    use serde_json::json;

    fn world() -> World {
        let mut w = World::default();
        w.set_geometry(1, crate::world::Geometry { positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0], uvs: vec![0.0; 6], indices: vec![0, 1, 2], ..Default::default() }).unwrap();
        w
    }

    /// The renderer's world as the bridge's scene: a body drives an object under a turned, scaled parent to where the
    /// body is in the world, the object's own scale kept; the object is movable and driven from then on; a session's
    /// frames make whole ticks and the frame's figures count them.
    #[test]
    fn bodies_drive_the_worlds_objects() {
        let mut w = world();
        let group = Kind::Group;
        w.set_object(1, "group".into(), None, group, true, false).unwrap();
        w.set_transform(1, Vec3::new(4.0, 0.0, 0.0), Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::splat(2.0)).unwrap();
        w.set_object(2, "box".into(), Some(1), Kind::Mesh { geometry: 1, material: 1, own_blas: false, materials: vec![] }, true, false).unwrap();
        w.set_transform(2, Vec3::new(0.0, 1.0, 0.5), Quat::IDENTITY, Vec3::splat(0.5)).unwrap();
        let at = w.world_matrix(2).transform_point3(Vec3::ZERO);
        assert!((at - Vec3::new(5.0, 2.0, 0.0)).length() < 1e-5, "{at}");
        let mut p = PhysicsWorld::default();
        p.call(&mut w, "body", &json!({ "id": 1, "object": 2, "slot": 0, "gravityScale": 0, "linvel": [0, 0, 6] })).unwrap();
        p.call(&mut w, "collider", &json!({ "id": 1, "body": 1, "shape": { "type": "sphere", "radius": 0.5 } })).unwrap();
        assert!(w.objects[&2].driven && w.objects[&2].movable);
        p.advance(&mut w, 0.5);
        let s = p.take_stats();
        assert_eq!((s.ticks, s.bodies, s.moved), (5, 1, 1), "at most five ticks a frame");
        let now = w.world_matrix(2).transform_point3(Vec3::ZERO);
        assert!((now - Vec3::new(5.0, 2.0, 0.5)).length() < 1e-4, "5 ticks at 6 m/s: {now}");
        assert_eq!(w.objects[&2].scale, Vec3::splat(0.5));
        // What scene code reads: the object's own position, in the group's space.
        let local = w.objects[&2].position;
        assert_eq!(p.poses()[..3], local.to_array());
        assert!((local - Vec3::new(-0.25, 1.0, 0.5)).length() < 1e-4, "{local}");
        p.settle(&mut w, 65);
        assert_eq!(p.take_stats().ticks, 60);
        // The object removed from the world: its body goes when it would next be placed.
        w.remove(2);
        p.advance(&mut w, 1.0 / 60.0);
        assert_eq!(p.take_stats().bodies, 0);
    }

    /// The renderer's build of the physics steps to the recorded bits, as physics-core's own build and its wasm32
    /// build do (tools/physics-determinism.ts): features that the renderer's other dependencies turn on in the shared
    /// crates (num-traits' and glam's `std`) do not bring the platform's math functions into it.
    #[test]
    fn the_renderers_physics_steps_to_the_recorded_bits() {
        use physics_core::check;
        assert_eq!(format!("{:016x}", check::script(600)), format!("{:016x}", check::EXPECTED));
        assert_eq!(format!("{:016x}", check::cases()), format!("{:016x}", check::EXPECTED_CASES));
    }

    /// Two events with no tick between them: the body the first one's script added is in the engine once that script
    /// returned (the binding's sync), and the second one's ray meets it, though the world has not stepped since.
    #[test]
    fn a_query_sees_a_body_added_since_the_last_tick() {
        let mut w = world();
        w.set_object(1, "box".into(), None, Kind::Mesh { geometry: 1, material: 1, own_blas: false, materials: vec![] }, true, false).unwrap();
        w.set_transform(1, Vec3::new(0.0, 2.0, 0.0), Quat::IDENTITY, Vec3::ONE).unwrap();
        let mut p = PhysicsWorld::default();
        let ray = json!({ "origin": [0, 9, 0], "direction": [0, -1, 0] });
        p.call(&mut w, "gravity", &json!({ "gravity": [0, -9.81, 0] })).unwrap();
        p.advance(&mut w, 1.0 / 60.0);
        assert!(p.call(&mut w, "ray", &ray).unwrap().is_null());
        // The first event's sync.
        p.call(&mut w, "body", &json!({ "id": 1, "object": 1, "slot": 0 })).unwrap();
        p.call(&mut w, "collider", &json!({ "id": 1, "body": 1, "shape": { "type": "box", "halfExtents": [0.5, 0.5, 0.5] } })).unwrap();
        // The second event's query, no `advance` between.
        let hit = p.call(&mut w, "ray", &ray).unwrap();
        assert_eq!((hit["object"].as_u64(), hit["body"].as_u64()), (Some(1), Some(1)), "{hit}");
        assert!((hit["distance"].as_f64().unwrap() - 6.5).abs() < 1e-4, "{hit}");
        assert_eq!(p.take_stats().ticks, 1);
    }

    /// A force applied before a live run (in setup) acts in its first frame's tick: settling at time 0 makes no tick
    /// and does not end it (a frame without a tick does).
    #[test]
    fn settling_with_no_tick_keeps_the_forces() {
        let mut w = world();
        w.set_object(1, "box".into(), None, Kind::Mesh { geometry: 1, material: 1, own_blas: false, materials: vec![] }, true, false).unwrap();
        let mut p = PhysicsWorld::default();
        p.call(&mut w, "body", &json!({ "id": 1, "object": 1, "slot": 0, "gravityScale": 0 })).unwrap();
        p.call(&mut w, "collider", &json!({ "id": 1, "body": 1, "shape": { "type": "sphere", "radius": 0.5 }, "mass": 2 })).unwrap();
        p.call(&mut w, "force", &json!({ "body": 1, "force": [0, 120, 0] })).unwrap();
        p.settle(&mut w, 0);
        assert_eq!(p.take_stats().ticks, 0);
        p.advance(&mut w, 1.0 / 60.0);
        let v = p.call(&mut w, "state", &json!({ "body": 1 })).unwrap()["linvel"][1].as_f64().unwrap();
        assert!((v - 120.0 / 2.0 / 60.0).abs() < 1e-4, "120 N on 2 kg through one tick: {v}");
    }
}

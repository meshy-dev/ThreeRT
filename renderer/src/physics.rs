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
/// bodies and how many of them moved; and the times the step's wait kicked the physics' threads (`in_pool`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub ticks: u32,
    pub step_ms: f64,
    pub sync_ms: f64,
    pub bodies: usize,
    pub moved: usize,
    pub kicks: u32,
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
        let bridge = &mut self.bridge;
        #[allow(unused_mut)]
        let mut tick = move || {
            for _ in 0..ticks {
                bridge.tick();
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.stats.kicks += in_pool(tick);
        }
        #[cfg(target_arch = "wasm32")]
        tick();
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

    /// Time the engine spent placing what its own bodies stand for (the voxel bodies' objects: voxel.rs
    /// `sync_bodies`), counted with the frame's sync.
    pub fn add_sync_ms(&mut self, ms: f64) {
        self.stats.sync_ms += ms;
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

/// The threads the physics world is stepped on natively (Rapier's parallel solver: physics-core's `parallel`): a pool
/// of the physics' own, not the process's, which has every core for the renderer's own work. **4 threads, or the
/// machine's performance cores when it has fewer**; `THREERT_PHYSICS_THREADS` sets another count (1 to 16), and 1 (or
/// 0, or "off") is sequential: the solver on one thread, which costs what the sequential build does. Measured
/// (docs/VOXEL.md "Voxel bodies"): 4 threads take a third of the sequential step's time, 8 gain nothing on the frame,
/// and with little moving the pool costs 0.03 ms. The bits do not depend on the count (the recorded hashes at 1 to
/// 15 threads on this machine: ROADMAP § 6 step 8 says what was checked and what was not). wasm32 has no threads
/// here: there the world is stepped where it is called.
#[cfg(not(target_arch = "wasm32"))]
fn pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let threads = threads_from(std::env::var("THREERT_PHYSICS_THREADS").ok().as_deref(), performance_cores());
        rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|k| format!("physics-{k}")).build().expect("the physics' threads")
    })
}

/// How long a step is waited for before the physics' threads are kicked, and again after each kick.
#[cfg(not(target_arch = "wasm32"))]
const KICK: std::time::Duration = std::time::Duration::from_micros(500);

/// Runs `f` (a frame's ticks) on one of the physics' threads and waits for it; returns the kicks it took.
///
/// **Why not `pool().install(f)`**, which is what this was: Rapier 0.36.1's step hangs there, about once in 10^5
/// steps of a small world stepped back to back (a fuzz of 1,500 edits and calls: one run in fifty). Its step hands the
/// broad phase's tree optimisation to `rayon::spawn` and later waits for it with a channel's `recv`
/// (`physics_pipeline/solve.rs`, `mod.rs` `join_deferred_bvh_optimize`). Inside a pool the spawn goes onto the
/// stepping worker's own deque, and the `recv` blocks that worker outside rayon, so only another worker can run the
/// job: one that steals it. rayon does not promise that another worker comes: waking for a job in a worker's deque is
/// best effort (the push's index store and the pusher's read of the sleep counters have no store-to-load barrier
/// between them; crossbeam-deque stores the index relaxed), because in rayon's own waits the pusher runs the job
/// itself in the end. When all the other workers happen to be falling asleep at that instant they all miss it, and
/// the step never returns. Rapier's own `configure_thread_pool` steps the same way and has the same hole.
///
/// So the caller, which only waits anyway, **kicks**: while the step has not returned, every `KICK` it injects an
/// empty job into the pool. An injected job always wakes a sleeping worker (that path is the one `install` itself
/// rests on), and a worker that wakes looks through every deque before it sleeps again: it takes the stranded job.
/// A step that hung costs `KICK` once; a long step gets a few empty jobs, which change nothing (the bits do not
/// depend on which thread runs what). With one thread Rapier runs that pass inline and there is nothing to strand.
/// `THREERT_PHYSICS_KICK=off` waits without kicking, to reproduce the hang (the fuzz, `examples/physics_fuzz.rs`,
/// finds it in a few dozen runs).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn in_pool(f: impl FnOnce() + Send) -> u32 {
    use std::sync::{Condvar, Mutex, PoisonError};
    static KICKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let pool = pool();
    if pool.current_num_threads() < 2 || !*KICKS.get_or_init(|| std::env::var("THREERT_PHYSICS_KICK").map_or(true, |v| v != "off" && v != "0")) {
        pool.install(f);
        return 0;
    }
    // Set when `f` has returned, or unwound: the wait ends either way, and the scope then carries the panic on.
    struct Done<'a>(&'a (Mutex<bool>, Condvar));
    impl Drop for Done<'_> {
        fn drop(&mut self) {
            *self.0 .0.lock().unwrap_or_else(PoisonError::into_inner) = true;
            self.0 .1.notify_one();
        }
    }
    let done = (Mutex::new(false), Condvar::new());
    let mut kicks = 0;
    pool.in_place_scope(|s| {
        s.spawn(|_| {
            let _done = Done(&done);
            f();
        });
        let mut finished = done.0.lock().unwrap_or_else(PoisonError::into_inner);
        while !*finished {
            let (guard, wait) = done.1.wait_timeout(finished, KICK).unwrap_or_else(PoisonError::into_inner);
            finished = guard;
            if !*finished && wait.timed_out() {
                kicks += 1;
                pool.spawn(|| {});
            }
        }
    });
    kicks
}

/// The pool's size: what `THREERT_PHYSICS_THREADS` says (1 to 16; 0 and "off": 1, sequential; anything else is not
/// taken), else 4 or the performance cores when fewer.
#[cfg(not(target_arch = "wasm32"))]
fn threads_from(env: Option<&str>, cores: usize) -> usize {
    match env.map(str::trim) {
        Some("off") | Some("0") => 1,
        Some(v) => v.parse::<usize>().ok().filter(|n| (1..=16).contains(n)).unwrap_or_else(|| threads_from(None, cores)),
        None => cores.clamp(1, 4),
    }
}

/// The machine's performance cores: on Apple silicon the first performance level's physical cores (the efficiency
/// cores are left to the rest of the process); elsewhere the cores the process may use.
#[cfg(not(target_arch = "wasm32"))]
fn performance_cores() -> usize {
    #[cfg(target_os = "macos")]
    {
        let (mut n, mut size) = (0i32, std::mem::size_of::<i32>());
        // SAFETY: the name is a C string, and the value is read into an i32 whose size is passed with it.
        let read = unsafe { libc::sysctlbyname(c"hw.perflevel0.physicalcpu".as_ptr(), (&mut n as *mut i32).cast(), &mut size, std::ptr::null_mut(), 0) };
        if read == 0 && n > 0 {
            return n as usize;
        }
    }
    std::thread::available_parallelism().map_or(1, |n| n.get())
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

    /// The physics' threads: 4, the performance cores when fewer, what the environment says when it says a count;
    /// 1 (0, "off") is sequential.
    #[test]
    fn the_physics_threads() {
        assert_eq!([threads_from(None, 5), threads_from(None, 12), threads_from(None, 2), threads_from(None, 0)], [4, 4, 2, 1]);
        assert_eq!([threads_from(Some("8"), 5), threads_from(Some("1"), 5), threads_from(Some("0"), 5), threads_from(Some("off"), 5), threads_from(Some(" 2 "), 5)], [8, 1, 1, 1, 2]);
        assert_eq!([threads_from(Some("64"), 5), threads_from(Some("many"), 2), threads_from(Some(""), 5)], [4, 2, 4]);
        assert!(performance_cores() >= 1);
        assert_eq!(pool().current_num_threads(), threads_from(std::env::var("THREERT_PHYSICS_THREADS").ok().as_deref(), performance_cores()));
    }

    /// The renderer's build of the physics steps to the recorded bits, as physics-core's own build and its wasm32
    /// build do (tools/physics-determinism.ts): features that the renderer's other dependencies turn on in the shared
    /// crates (num-traits' and glam's `std`) do not bring the platform's math functions into it.
    #[test]
    fn the_renderers_physics_steps_to_the_recorded_bits() {
        use physics_core::check;
        assert_eq!(format!("{:016x}", check::script(600)), format!("{:016x}", check::EXPECTED));
        assert_eq!(format!("{:016x}", check::cases()), format!("{:016x}", check::EXPECTED_CASES));
        // And stepped as the renderer steps its world: on one of the physics' threads, with their count. (Called
        // from here the checks above run their parallel parts on the process's pool, whatever its size.)
        let (mut script, mut cases) = (0, 0);
        in_pool(|| (script, cases) = (check::script(600), check::cases()));
        assert_eq!(format!("{script:016x}"), format!("{:016x}", check::EXPECTED), "on the physics' threads");
        assert_eq!(format!("{cases:016x}"), format!("{:016x}", check::EXPECTED_CASES), "on the physics' threads");
    }

    /// A step's wait ends when the step unwinds, and the panic comes through; the pool steps again after it.
    #[test]
    fn a_step_that_panics_is_not_waited_for() {
        let r = std::panic::catch_unwind(|| in_pool(|| panic!("in the step")));
        assert!(r.is_err());
        let mut ran = false;
        in_pool(|| ran = true);
        assert!(ran);
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

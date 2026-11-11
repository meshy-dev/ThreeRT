//! Two CPU checks of the engine's own path through the voxel bodies and the physics (ROADMAP § 6 step 8), **with a
//! watchdog**: a step that does not return is a failure, not a wait.
//!
//! - **The fuzz** (`--runs`): a small voxel level, and per iteration a random edit (carve, explosion, added voxels,
//!   paint; with wild numbers after the first third) or one of scene code's physics calls (on the voxel bodies too),
//!   then one to four frames' steps (`api::physics_advance`: on the physics' threads, as the renderer steps). After
//!   every iteration: each voxel body has its engine body, its collider's cells are its grid's, it is somewhere
//!   finite, and the cap holds.
//! - **The stress** (`--stress`): a slab and six bars cut loose, kept moving, stepped back to back that many times.
//!   This is where the physics' step hung (src/physics.rs `in_pool` has the cause: Rapier waits on a channel, inside
//!   a rayon worker, for a job only another worker can take, and rayon does not promise one comes): with a tree
//!   optimisation spawned in nearly every step and the next step microseconds after the last, the unguarded step
//!   hangs after 30,000 to 400,000 steps (five runs of five; `THREERT_PHYSICS_KICK=off` brings it back). The fuzz
//!   found it first, in one run of fifty.
//!
//! The same place for `--limit` seconds (30) ends the process with exit 3 and says where.
//!
//!   cargo run --release --example physics_fuzz -- [--runs 20] [--from 1] [--iterations 1500] [--many]
//!                                                  [--stress 2000000] [--limit 30]
//!
//! `--many`: 480 bars cut loose at once and again, so more loose parts than the cap holds. Exit 0: every run ended and
//! every check held. `kicks` counts the times a step's wait woke the physics' threads (`in_pool`): in the stress,
//! whose steps take microseconds, each is a step that would have hung (or a thread the system held back).
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use threert_render::api;
use threert_render::physics::PhysicsWorld;
use threert_render::voxel::{EditOp, VoxelWorld, MAX_BODIES};
use threert_render::world::World;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

/// Where the fuzz is (seed in the high half, iteration in the low), for the watchdog.
static AT: AtomicU64 = AtomicU64::new(0);

/// One run; returns the kicks its steps took.
fn run(seed: u64, iterations: u32, many: bool) -> u32 {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut w = World::default();
    let mut ph: Option<PhysicsWorld> = None;
    // Two terrain tiles side by side and bars standing on them (a few of them anchors).
    let mut blocks = vec![([0, 0, 0], [64, 6, 64], Some((0, 0)), true), ([64, 0, 0], [64, 6, 64], Some((1, 0)), true)];
    if many {
        for i in 0..24 {
            for j in 0..20 {
                blocks.push(([2 + i * 5, 6, 2 + j * 3], [2, 8 + r.below(12) as i32, 2], None, false));
            }
        }
    } else {
        for i in 0..10 {
            for j in 0..5 {
                blocks.push(([6 + i * 11, 6, 6 + j * 11], [2 + r.below(4) as i32, 6 + r.below(20) as i32, 2 + r.below(4) as i32], None, r.chance(0.1)));
            }
        }
    }
    let mut v = VoxelWorld::of_blocks(&blocks, &mut w, &mut ph);
    if many {
        // Every bar cut loose at its foot, then again: more loose parts than the cap holds.
        for y in [0.7, 1.1] {
            let _ = v.edit(&mut w, &mut ph, &EditOp::parse(&json!({ "op": "carve", "box": [0.0, y, 0.0, 12.8, y + 0.1, 6.4] })).unwrap());
            w.take_changes();
        }
    }
    let weird = |r: &mut Rng| -> f64 { [0.0, -1.0, 1e-9, 1e9, 1e30, -1e30, 12.9, 300.0, 0.05, 1e5, 104857.6][r.below(11) as usize] };
    let pos = |r: &mut Rng, wild: bool| -> f64 {
        if wild && r.chance(0.15) {
            weird(r)
        } else {
            r.range(-1.0, 13.5)
        }
    };
    let (mut edits, mut refused, mut calls_refused, mut most, mut kicks) = (0u32, 0u32, 0u32, 0usize, 0u32);
    let mut advance = |v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>| {
        api::physics_advance(ph, v, w, 1.0 / 60.0);
        w.take_changes();
        kicks += ph.as_mut().map_or(0, |p| p.take_stats().kicks);
    };
    for it in 0..iterations {
        AT.store(seed << 32 | it as u64, Ordering::Relaxed);
        let wild = it > iterations / 3;
        match r.below(10) {
            0..=4 => {
                let op = ["carve", "carve", "explode", "add", "paint"][r.below(5) as usize];
                let wild = wild && op != "add";
                let rad = if wild && r.chance(0.2) { weird(&mut r) } else { r.range(0.1, 1.5) };
                let d = match if many && r.chance(0.5) { 3 } else { r.below(3) } {
                    3 => {
                        let y = r.range(0.5, 2.5);
                        json!({ "op": "carve", "box": [r.range(0.0, 6.0), y, 0.0, r.range(6.0, 12.8), y + 0.1, 6.4] })
                    }
                    0 => json!({ "op": op, "material": 1, "sphere": [pos(&mut r, wild), r.range(0.0, 3.0), pos(&mut r, wild) * 0.5, rad] }),
                    1 => {
                        let (x, y, z) = (pos(&mut r, wild), r.range(0.0, 3.0), pos(&mut r, wild) * 0.5);
                        let far = if wild && r.chance(0.2) { weird(&mut r) } else { r.range(-0.5, 3.0) };
                        json!({ "op": op, "material": 1, "box": [x, y, z, x + far, y + r.range(-0.2, 1.0), z + r.range(0.0, 3.0)] })
                    }
                    _ => json!({ "op": op, "material": 1, "capsule": [pos(&mut r, wild), r.range(0.0, 3.0), pos(&mut r, wild) * 0.5, pos(&mut r, wild), r.range(0.0, 3.0), pos(&mut r, wild) * 0.5, rad.min(3.0)] }),
                };
                match EditOp::parse(&d).and_then(|e| v.edit(&mut w, &mut ph, &e)) {
                    Ok(_) => edits += 1,
                    Err(_) => refused += 1,
                }
            }
            5 => {
                let radius = if wild && r.chance(0.3) { [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e300, -3.0][r.below(5) as usize] } else { r.range(0.2, 2.0) };
                let _ = api::voxel_explode(&mut v, &mut w, &mut ph, pos(&mut r, wild), r.range(0.0, 3.0), pos(&mut r, wild) * 0.5, radius);
            }
            _ => {
                // Scene code's physics calls, on the voxel bodies too.
                let list = v.body_list(ph.as_ref().unwrap());
                let ids: Vec<u64> = list.as_array().unwrap().iter().map(|b| b["body"].as_u64().unwrap()).collect();
                let cols: Vec<u64> = list.as_array().unwrap().iter().map(|b| b["collider"].as_u64().unwrap()).collect();
                let body = if ids.is_empty() || r.chance(0.2) { 1 + r.below(6) } else { ids[r.below(ids.len() as u64) as usize] };
                let big = |r: &mut Rng| if r.chance(0.1) { 1e6 } else { r.range(-300.0, 300.0) };
                let (op, args) = match r.below(12) {
                    0 => ("impulse", json!({ "body": body, "impulse": [big(&mut r), big(&mut r), big(&mut r)], "point": [r.range(0.0, 12.0), 1.0, 3.0] })),
                    1 => ("velocity", json!({ "body": body, "linvel": [r.range(-1e4, 1e4), r.range(-50.0, 50.0), 0.0], "angvel": [r.range(-1e3, 1e3), 0.0, 0.0] })),
                    2 => ("pose", json!({ "body": body, "position": [pos(&mut r, false), r.range(-300.0, 20.0), 3.0], "rotation": [r.unit(), r.unit(), r.unit(), r.unit() + 0.01] })),
                    3 => ("body", json!({ "id": 1 + r.below(6), "position": [r.range(0.0, 12.0), r.range(1.0, 6.0), 3.0], "kind": (["dynamic", "kinematic", "fixed"][r.below(3) as usize]) })),
                    4 => ("collider", json!({ "id": 10 + r.below(12), "body": body, "shape": { "type": "box", "halfExtents": [r.range(0.05, 2.0), 0.2, 0.2] }, "mass": if r.chance(0.3) { 1e7 } else { 10.0 }, "events": true })),
                    5 => ("joint", json!({ "id": 30 + r.below(8), "a": body, "b": if ids.is_empty() { 2 } else { ids[r.below(ids.len() as u64) as usize] }, "type": (["fixed", "revolute", "spherical", "rope", "prismatic"][r.below(5) as usize]), "length": 1.0, "limits": [-0.5, 0.5] })),
                    6 => ("remove", json!({ "body": if r.chance(0.5) { body } else { 1 + r.below(6) }, "collider": if r.chance(0.5) && !cols.is_empty() { cols[0] } else { 10 + r.below(12) }, "joint": 30 + r.below(8) })),
                    7 => ("force", json!({ "body": body, "force": [big(&mut r) * 100.0, 0.0, 0.0], "torque": [0.0, big(&mut r) * 100.0, 0.0] })),
                    8 => ("ray", json!({ "origin": [r.range(0.0, 12.0), 8.0, r.range(0.0, 6.0)], "direction": [r.range(-0.2, 0.2), -1.0, 0.0], "all": r.chance(0.5), "skip": body })),
                    9 => ("target", json!({ "body": body, "position": [1.0, 2.0, 3.0] })),
                    10 => ("contacts", json!({})),
                    _ => ("overlap", json!({ "shape": { "type": "sphere", "radius": r.range(0.1, 3.0) }, "position": [r.range(0.0, 12.0), 1.0, 3.0] })),
                };
                if ph.as_mut().unwrap().call(&mut w, op, &args).is_err() {
                    calls_refused += 1;
                }
            }
        }
        w.take_changes();
        for _ in 0..r.below(4) {
            advance(&mut v, &mut w, &mut ph);
        }
        // What must hold (after a sync: a body the world took out is retired at the next sync).
        advance(&mut v, &mut w, &mut ph);
        let p = ph.as_ref().unwrap();
        let list = v.body_list(p);
        let list = list.as_array().unwrap();
        most = most.max(list.len());
        assert_eq!(list.len(), v.bodies(), "seed {seed}, iteration {it}: a voxel body without an engine body");
        assert!(list.len() <= MAX_BODIES, "seed {seed}, iteration {it}: {} bodies", list.len());
        for b in list {
            let (id, collider, voxels) = (b["body"].as_u64().unwrap() as u32, b["collider"].as_u64().unwrap() as u32, b["voxels"].as_u64().unwrap() as usize);
            assert!(voxels > 0, "seed {seed}, iteration {it}: body {id} has no voxel");
            assert_eq!(p.bridge_ref().engine_voxels(collider), Some(voxels), "seed {seed}, iteration {it}: body {id}'s collider cells are not its grid's");
            assert!(b["position"].as_array().unwrap().iter().all(|x| x.as_f64().is_some_and(f64::is_finite)), "seed {seed}, iteration {it}: body {id} at {}", b["position"]);
        }
    }
    println!(
        "seed {seed}: {iterations} iterations, edits {edits} (refused {refused}), calls refused {calls_refused}, most bodies {most}, retired {}, kicks {kicks}, hash {:016x}/{:016x}",
        v.retired,
        v.hash().unwrap_or(0),
        ph.as_ref().unwrap().hash()
    );
    kicks
}

/// A small world stepped back to back: a slab and six bars cut loose, one of them thrown about every step (so the
/// broad phase has a tree to tidy, which is the job that was stranded). Returns the kicks.
fn stress(steps: u64) -> u32 {
    let mut w = World::default();
    let mut ph: Option<PhysicsWorld> = None;
    let mut blocks = vec![([0, 0, 0], [64, 4, 64], Some((0, 0)), true)];
    blocks.extend((0..6).map(|k| ([8 + 8 * k, 4, 8], [4, 12, 4], None, false)));
    let mut v = VoxelWorld::of_blocks(&blocks, &mut w, &mut ph);
    v.edit(&mut w, &mut ph, &EditOp::parse(&json!({ "op": "carve", "box": [0.0, 0.5, 0.0, 6.4, 0.6, 6.4] })).unwrap()).unwrap();
    let ids: Vec<u64> = v.body_list(ph.as_ref().unwrap()).as_array().unwrap().iter().map(|b| b["body"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 6);
    let (mut r, mut kicks) = (Rng(0x9E37_79B9_7F4A_7C15), 0);
    for n in 0..steps {
        AT.store(u64::MAX << 32 | (n & 0xffff_ffff), Ordering::Relaxed);
        let body = ids[r.below(6) as usize];
        ph.as_mut().unwrap().call(&mut w, "velocity", &json!({ "body": body, "linvel": [r.range(-2.0, 2.0), 3.0, r.range(-2.0, 2.0)] })).unwrap();
        api::physics_advance(&mut ph, &mut v, &mut w, 1.0 / 60.0);
        w.take_changes();
        kicks += ph.as_mut().unwrap().take_stats().kicks;
    }
    assert_eq!(v.bodies(), 6, "the six bars are still bodies");
    kicks
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let number = |name: &str, default: u64| args.iter().position(|a| a == name).and_then(|k| args.get(k + 1)).and_then(|v| v.parse().ok()).unwrap_or(default);
    let (runs, from, iterations, steps, limit) = (number("--runs", 20), number("--from", 1), number("--iterations", 1500) as u32, number("--stress", 2_000_000), number("--limit", 30));
    let many = args.iter().any(|a| a == "--many");
    // The watchdog: the same place for `limit` seconds is a step (or an edit) that does not return.
    std::thread::spawn(move || {
        let (mut last, mut since) = (u64::MAX, std::time::Instant::now());
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let at = AT.load(Ordering::Relaxed);
            if at != last {
                (last, since) = (at, std::time::Instant::now());
            } else if since.elapsed().as_secs() >= limit {
                let (seed, n) = (at >> 32, at & 0xffff_ffff);
                eprintln!("physics fuzz: STUCK: {} has not returned in {limit} s", if seed == u32::MAX as u64 { format!("the stress's step {n} (modulo 2^32)") } else { format!("seed {seed}, iteration {n}") });
                std::process::exit(3);
            }
        }
    });
    let t = std::time::Instant::now();
    let kicks: u32 = (from..from + runs).map(|seed| run(seed, iterations, many)).sum();
    if runs > 0 {
        println!("physics fuzz: {runs} runs of {iterations} iterations in {:.1} s, every check held, no step hung; kicks {kicks}", t.elapsed().as_secs_f64());
    }
    if steps > 0 {
        let t = std::time::Instant::now();
        let kicks = stress(steps);
        println!("physics stress: {steps} steps back to back in {:.1} s ({:.1} us a step), none hung; kicks {kicks}", t.elapsed().as_secs_f64(), t.elapsed().as_secs_f64() * 1e6 / steps as f64);
    }
}

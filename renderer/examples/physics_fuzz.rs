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
//! - **The bits** (`--bits`): the physics' recorded hashes (physics_core::check: the script and the 1,500 random
//!   worlds) and the voxel adapter's (`VoxelWorld::check_script`), stepped on the physics' own threads as the
//!   renderer steps (`in_pool`, `THREERT_PHYSICS_THREADS`). tools/physics-determinism.ts holds the parallel build to
//!   the sequential one stepping from a thread outside any pool; this is the path the renderer takes.
//!
//! The same place for `--limit` seconds (30) ends the process with exit 3 and says where.
//!
//!   cargo run --release --example physics_fuzz -- [--bits] [--runs 20] [--from 1] [--iterations 1500] [--many]
//!                                                  [--stress 2000000] [--limit 30]
//!
//! `--many`: 480 bars cut loose at once and again, so more loose parts than the cap holds. Exit 0: every run ended and
//! every check held. `kicks` counts the times a step's wait woke the physics' threads (`in_pool`): in the stress,
//! whose steps take microseconds, each is a step that would have hung (or a thread the system held back).
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use threert_render::api;
use threert_render::physics::PhysicsWorld;
use threert_render::voxel::{EditOp, VoxelWorld, MAX_BODIES, MAX_BODY_CELLS};
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

/// How often (iterations) a run checks the level's colliders against their shells by brute force (`--shells`).
const SHELLS: u32 = 50;

/// Where the fuzz is (seed in the high half, iteration in the low), for the watchdog.
static AT: AtomicU64 = AtomicU64::new(0);

/// One run; returns the kicks its steps took.
fn run(seed: u64, iterations: u32, many: bool, shells: u32) -> u32 {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut w = World::default();
    let mut ph: Option<PhysicsWorld> = None;
    // Two terrain tiles side by side, 1.6 m deep (deep enough to have cells under their shells), and bars standing on
    // them (a few of them anchors).
    let mut blocks = vec![([0, 0, 0], [64, 16, 64], Some((0, 0)), true), ([64, 0, 0], [64, 16, 64], Some((1, 0)), true)];
    if many {
        for i in 0..24 {
            for j in 0..20 {
                blocks.push(([2 + i * 5, 16, 2 + j * 3], [2, 8 + r.below(12) as i32, 2], None, false));
            }
        }
    } else {
        for i in 0..10 {
            for j in 0..5 {
                blocks.push(([6 + i * 11, 16, 6 + j * 11], [2 + r.below(4) as i32, 6 + r.below(20) as i32, 2 + r.below(4) as i32], None, r.chance(0.1)));
            }
        }
    }
    let mut v = VoxelWorld::of_blocks(&blocks, &mut w, &mut ph);
    if many {
        // Every bar cut loose at its foot, then again: more loose parts than the cap holds.
        for y in [1.7, 2.1] {
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
    let mut broken_seen = 0u32;
    let mut moved_seen = 0u32;
    let (mut body_edits, mut checked) = (0u32, 0usize);
    let mut advance = |v: &mut VoxelWorld, w: &mut World, ph: &mut Option<PhysicsWorld>| {
        api::physics_advance(ph, v, w, 1.0 / 60.0);
        w.take_changes();
        kicks += ph.as_mut().map_or(0, |p| p.take_stats().kicks);
    };
    for it in 0..iterations {
        AT.store(seed << 32 | it as u64, Ordering::Relaxed);
        let wild = it > iterations / 3;
        match r.below(12) {
            0..=4 => {
                let op = ["carve", "carve", "explode", "add", "paint"][r.below(5) as usize];
                let wild = wild && op != "add";
                let rad = if wild && r.chance(0.2) { weird(&mut r) } else { r.range(0.1, 1.5) };
                let d = match if many && r.chance(0.5) { 3 } else { r.below(3) } {
                    3 => {
                        let y = r.range(1.5, 3.5);
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
            // One body's own edit (a voxel object's addVoxels / paint / carve), around where it is.
            10 => {
                let list = v.body_list(ph.as_ref().unwrap());
                if let Some(b) = list.as_array().unwrap().get(r.below(list.as_array().unwrap().len().max(1) as u64) as usize) {
                    let (id, at) = (b["body"].as_u64().unwrap() as u32, [0, 1, 2].map(|k| b["position"][k].as_f64().unwrap()));
                    // Near it mostly; now and then far (up to 10^8 m out: the review of #25 grew a body's mesh over
                    // that), with a wild size, or grown again and again beside it.
                    let off = if r.chance(0.1) { [1e2, 1e5, 1e8][r.below(3) as usize] * if r.chance(0.5) { 1.0 } else { -1.0 } } else { 0.0 };
                    let c = [at[0] + r.range(-0.5, 1.0) + off, at[1] + r.range(-0.5, 1.5), at[2] + r.range(-0.5, 1.0) + off * r.range(0.0, 1.0)];
                    let rad = if wild && r.chance(0.2) { weird(&mut r) } else { r.range(0.05, 0.8) };
                    let op = ["add", "paint", "carve"][r.below(3) as usize];
                    let material = if r.chance(0.05) { 255 } else { 1 };
                    let d = if r.chance(0.5) { json!({ "op": op, "material": material, "sphere": [c[0], c[1], c[2], rad] }) } else { json!({ "op": op, "material": material, "box": [c[0], c[1], c[2], c[0] + rad, c[1] + rad, c[2] + rad] }) };
                    let grow = if op == "add" && r.chance(0.02) { 4 } else { 1 };
                    for g in 0..grow {
                        let d = if grow > 1 { json!({ "op": "add", "material": 1, "box": [at[0] + 0.4 + 3.2 * g as f64, at[1], at[2], at[0] + 3.59 + 3.2 * g as f64, at[1] + 3.19, at[2] + 3.19] }) } else { d.clone() };
                        match EditOp::parse(&d).and_then(|e| v.edit_body(&mut w, &mut ph, id, &e)) {
                            Ok(_) => body_edits += 1,
                            Err(_) => refused += 1,
                        }
                    }
                }
            }
            // A body's contacts on or off.
            11 => {
                let list = v.body_list(ph.as_ref().unwrap());
                if let Some(b) = list.as_array().unwrap().first() {
                    let _ = v.body_contacts(&mut ph, b["body"].as_u64().unwrap() as u32, r.chance(0.7));
                }
            }
            5 => {
                let radius = if wild && r.chance(0.3) { [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e300, -3.0][r.below(5) as usize] } else { r.range(0.2, 2.0) };
                // (Its push too, step 10 c1: none, sane, or wild.)
                let push = match r.below(4) { 0 => None, 1 => Some(r.range(0.0, 2000.0)), 2 => Some([f64::NAN, f64::INFINITY, -1.0, 1e9, 1e300][r.below(5) as usize]), _ => Some(10_000.0) };
                let _ = api::voxel_explode(&mut v, &mut w, &mut ph, pos(&mut r, wild), r.range(0.0, 3.0), pos(&mut r, wild) * 0.5, radius, push);
            }
            _ => {
                // Scene code's physics calls, on the voxel bodies too.
                let list = v.body_list(ph.as_ref().unwrap());
                let ids: Vec<u64> = list.as_array().unwrap().iter().map(|b| b["body"].as_u64().unwrap()).collect();
                let cols: Vec<u64> = list.as_array().unwrap().iter().map(|b| b["collider"].as_u64().unwrap()).collect();
                // (The world's anchor, the voxel level's body, now and then too: `pose` must refuse it, `skip` of it
                // skips nothing, the rest leave a fixed body as it is.)
                let anchor = v.level_body().map(|l| l as u64);
                let body = match anchor {
                    Some(a) if r.chance(0.1) => a,
                    _ if ids.is_empty() || r.chance(0.2) => 1 + r.below(6),
                    _ => ids[r.below(ids.len() as u64) as usize],
                };
                let big = |r: &mut Rng| if r.chance(0.1) { 1e6 } else { r.range(-300.0, 300.0) };
                let (op, args) = match r.below(14) {
                    0 => ("impulse", json!({ "body": body, "impulse": [big(&mut r), big(&mut r), big(&mut r)], "point": [r.range(0.0, 12.0), 1.0, 3.0] })),
                    1 => ("velocity", json!({ "body": body, "linvel": [r.range(-1e4, 1e4), r.range(-50.0, 50.0), 0.0], "angvel": [r.range(-1e3, 1e3), 0.0, 0.0] })),
                    2 => ("pose", json!({ "body": body, "position": [pos(&mut r, false), r.range(-300.0, 20.0), 3.0], "rotation": [r.unit(), r.unit(), r.unit(), r.unit() + 0.01] })),
                    3 => ("body", json!({ "id": 1 + r.below(6), "position": [r.range(0.0, 12.0), r.range(1.0, 6.0), 3.0], "kind": (["dynamic", "kinematic", "fixed"][r.below(3) as usize]) })),
                    4 => ("collider", json!({ "id": 10 + r.below(12), "body": body, "shape": { "type": "box", "halfExtents": [r.range(0.05, 2.0), 0.2, 0.2] }, "mass": if r.chance(0.3) { 1e7 } else { 10.0 }, "events": true })),
                    // (Joints to the level's anchor too: step 10 c1.)
                    5 => ("joint", json!({ "id": 30 + r.below(8), "a": if r.chance(0.3) { anchor.unwrap_or(body) } else { body }, "b": if ids.is_empty() { 2 } else { ids[r.below(ids.len() as u64) as usize] }, "type": (["fixed", "revolute", "spherical", "rope", "prismatic"][r.below(5) as usize]), "length": 1.0, "limits": [-0.5, 0.5] })),
                    6 => ("remove", json!({ "body": if r.chance(0.5) { body } else { 1 + r.below(6) }, "collider": if r.chance(0.5) && !cols.is_empty() { cols[0] } else { 10 + r.below(12) }, "joint": 30 + r.below(8) })),
                    7 => ("force", json!({ "body": body, "force": [big(&mut r) * 100.0, 0.0, 0.0], "torque": [0.0, big(&mut r) * 100.0, 0.0] })),
                    8 => ("ray", json!({ "origin": [r.range(0.0, 12.0), 8.0, r.range(0.0, 6.0)], "direction": [r.range(-0.2, 0.2), -1.0, 0.0], "all": r.chance(0.5), "skip": body })),
                    9 => ("target", json!({ "body": body, "position": [1.0, 2.0, 3.0] })),
                    10 => ("contacts", json!({})),
                    // What the engine did to the joints (anchors carried off or carved, step 10 c1).
                    12 => ("joint_changes", json!({})),
                    // A joint's parameters changed in place (step 10 c1): any kind named, sane or wild numbers.
                    11 => {
                        let wild_num = |r: &mut Rng| if r.chance(0.2) { [f64::NAN, f64::INFINITY, -1.0, 1e30, 0.0][r.below(5) as usize] } else { r.range(-3.0, 3.0) };
                        let (lo, hi) = (wild_num(&mut r), wild_num(&mut r));
                        let mut a = json!({ "id": 30 + r.below(9), "type": (["fixed", "revolute", "spherical", "rope", "prismatic", "weld"][r.below(6) as usize]), "length": wild_num(&mut r), "limits": [lo, hi], "motor": { "velocity": wild_num(&mut r), "damping": wild_num(&mut r) * 1e4, "maxForce": wild_num(&mut r) * 1e3 } });
                        // (Now and then naming what makes the joint: refused.)
                        if r.chance(0.1) {
                            a[["a", "b", "anchor1", "anchor2", "axis"][r.below(5) as usize]] = json!([0.0, 1.0, 0.0]);
                        }
                        ("joint_set", a)
                    }
                    _ => ("overlap", json!({ "shape": { "type": "sphere", "radius": r.range(0.1, 3.0) }, "position": [r.range(0.0, 12.0), 1.0, 3.0] })),
                };
                match ph.as_mut().unwrap().call(&mut w, op, &args) {
                    Err(_) => calls_refused += 1,
                    Ok(v) if op == "joint_changes" => {
                        broken_seen += v["broken"].as_array().map_or(0, |a| a.len()) as u32;
                        moved_seen += v["moved"].as_array().map_or(0, |a| a.len()) as u32;
                    }
                    Ok(_) if op == "pose" && anchor == Some(body) => panic!("pose of the world's anchor was taken"),
                    Ok(_) => {}
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
        // The bodies said to be gone are gone, with a reason.
        for g in v.take_gone().as_array().unwrap() {
            let id = g[0].as_u64().unwrap();
            assert!(["cap", "carved", "fell"].contains(&g[1].as_str().unwrap()), "seed {seed}, iteration {it}: {g}");
            assert!(!list.iter().any(|b| b["body"].as_u64() == Some(id)), "seed {seed}, iteration {it}: body {id} said gone and listed");
        }
        // The level's colliders against their shells by brute force, every `shells` iterations and at the end.
        if shells > 0 && (it % shells == shells - 1 || it + 1 == iterations) {
            checked += v.check_shells(p).unwrap_or_else(|e| panic!("seed {seed}, iteration {it}: {e}"));
        }
        assert_eq!(list.len(), v.bodies(), "seed {seed}, iteration {it}: a voxel body without an engine body");
        // (The world's anchor where it was made, whatever was called on it.)
        if let Some(a) = v.level_body() {
            assert_eq!(p.bridge_ref().engine_pose(a).map(|x| (x.0, x.1)), Some(([0.0; 3], [0.0, 0.0, 0.0, 1.0])), "seed {seed}, iteration {it}: the anchor moved");
        }
        assert!(list.len() <= MAX_BODIES, "seed {seed}, iteration {it}: {} bodies", list.len());
        for b in list {
            let (id, collider, voxels) = (b["body"].as_u64().unwrap() as u32, b["collider"].as_u64().unwrap() as u32, b["voxels"].as_u64().unwrap() as usize);
            assert!(voxels > 0, "seed {seed}, iteration {it}: body {id} has no voxel");
            assert!(voxels <= MAX_BODY_CELLS.max(1 << 24), "seed {seed}, iteration {it}: body {id} has {voxels} voxels");
            assert_eq!(p.bridge_ref().engine_voxels(collider), Some(voxels), "seed {seed}, iteration {it}: body {id}'s collider cells are not its grid's");
            assert!(b["position"].as_array().unwrap().iter().all(|x| x.as_f64().is_some_and(f64::is_finite)), "seed {seed}, iteration {it}: body {id} at {}", b["position"]);
        }
    }
    println!(
        "seed {seed}: {iterations} iterations, edits {edits} (refused {refused}), body edits {body_edits}, calls refused {calls_refused}, joints broken {broken_seen}, moved {moved_seen}, most bodies {most}, retired {}, shells checked {checked}, kicks {kicks}, hash {:016x}/{:016x}",
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
    // (The level's colliders against brute force every this many iterations, and at a run's end; 0: never.)
    let shells = number("--shells", SHELLS as u64) as u32;
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
    if args.iter().any(|a| a == "--bits") {
        use physics_core::check;
        let t = std::time::Instant::now();
        let (mut script, mut cases) = (0, 0);
        threert_render::physics::in_pool(|| (script, cases) = (check::script(600), check::cases()));
        let voxels = threert_render::voxel::VoxelWorld::check_script();
        let threads = std::env::var("THREERT_PHYSICS_THREADS").unwrap_or_default();
        println!("physics bits on the physics' threads (THREERT_PHYSICS_THREADS={threads:?}): script {script:016x} (recorded {:016x}), cases {cases:016x} ({:016x}), voxel bodies {voxels:016x} ({:016x}); {:.1} s", check::EXPECTED, check::EXPECTED_CASES, threert_render::voxel::EXPECTED_SCRIPT, t.elapsed().as_secs_f64());
        assert_eq!((script, cases, voxels), (check::EXPECTED, check::EXPECTED_CASES, threert_render::voxel::EXPECTED_SCRIPT), "the recorded hashes");
    }
    let t = std::time::Instant::now();
    let kicks: u32 = (from..from + runs).map(|seed| run(seed, iterations, many, shells)).sum();
    if runs > 0 {
        println!("physics fuzz: {runs} runs of {iterations} iterations in {:.1} s, every check held, no step hung; kicks {kicks}", t.elapsed().as_secs_f64());
    }
    if steps > 0 {
        let t = std::time::Instant::now();
        let kicks = stress(steps);
        println!("physics stress: {steps} steps back to back in {:.1} s ({:.1} us a step), none hung; kicks {kicks}", t.elapsed().as_secs_f64(), t.elapsed().as_secs_f64() * 1e6 / steps as f64);
    }
}

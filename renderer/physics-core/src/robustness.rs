//! What a game script can do to the physics (it is untrusted, and calls `__engine.physics` directly, not only through
//! the scene binding): each test is one finding of the review of ROADMAP § 6 step 7, through `Bridge::call` as a
//! script's calls go, and fails (most by a panic in a tick) without its fix.

use super::*;
use crate::limits::*;

/// Objects at a pose each (no parents), recording where they were placed.
#[derive(Default)]
struct Objects {
    at: BTreeMap<u32, ([f32; 3], [f32; 4])>,
    placed: Vec<(u32, [f32; 3], [f32; 4])>,
}

impl Scene for Objects {
    fn world_pose(&self, o: u32) -> Option<([f32; 3], [f32; 4])> {
        self.at.get(&o).copied()
    }
    fn place(&mut self, o: u32, p: [f32; 3], q: [f32; 4]) -> Option<([f32; 3], [f32; 4])> {
        let e = self.at.get_mut(&o)?;
        *e = (p, q);
        self.placed.push((o, p, q));
        Some((p, q))
    }
    fn drive(&mut self, _: u32) {}
    fn geometry(&self, _: u32) -> Option<(&[f32], &[u32])> {
        None
    }
}

fn world() -> (Bridge, Objects) {
    let mut s = Objects::default();
    for o in 1..=64 {
        s.at.insert(o, ([o as f32, 2.0, 0.0], [0.0, 0.0, 0.0, 1.0]));
    }
    (Bridge::default(), s)
}

fn ok(b: &mut Bridge, s: &mut Objects, op: &str, a: Value) -> Value {
    b.call(s, op, &a).unwrap_or_else(|e| panic!("{op} {a}: {e}"))
}

fn run(b: &mut Bridge, s: &mut Objects, ticks: u32) {
    for _ in 0..ticks {
        b.tick();
        b.sync(s);
    }
}

fn sphere(r: f64) -> Value {
    json!({ "type": "sphere", "radius": r })
}

/// A1, the entry: the inputs the review found getting a body disabled by Rapier's quarantine (and a joint on it then
/// panicking a tick: interaction_groups.rs unreachable), and the rest of what has no bound, are refused with the
/// reason; the world is as it was, and ticks on.
#[test]
fn numbers_out_of_bounds_are_refused() {
    let (mut b, mut s) = world();
    ok(&mut b, &mut s, "body", json!({ "id": 1, "position": [0, 2, 0] }));
    ok(&mut b, &mut s, "collider", json!({ "id": 1, "body": 1, "shape": sphere(0.3) }));
    ok(&mut b, &mut s, "body", json!({ "id": 2, "kind": "fixed", "position": [0, 4, 0] }));
    let open = json!({ "type": "trimesh", "vertices": [1.4, 3.0, -2.2, -2.0, 0.8, 2.0, -0.4, 0.8, 1.5, 1.2, 0.07, -2.5], "indices": [0, 1, 2, 1, 2, 3] });
    for (op, a, why) in [
        ("collider", json!({ "id": 9, "body": 1, "shape": open }), "fixed bodies"),
        ("velocity", json!({ "body": 1, "angvel": [1e20, 0, 0] }), "angular velocity"),
        ("velocity", json!({ "body": 1, "angvel": [3e38, 3e38, 3e38] }), "angular velocity"),
        ("velocity", json!({ "body": 1, "linvel": [1e5, 0, 0] }), "linear velocity"),
        ("collider", json!({ "id": 9, "body": 1, "shape": sphere(0.3), "mass": 1e-38 }), "mass"),
        ("collider", json!({ "id": 9, "body": 1, "shape": sphere(0.3), "density": 3e38 }), "density"),
        ("collider", json!({ "id": 9, "body": 1, "shape": sphere(0.3), "position": [1e20, 0, 0] }), "position"),
        ("collider", json!({ "id": 9, "body": 1, "shape": sphere(0.3), "friction": -1 }), "friction"),
        ("collider", json!({ "id": 9, "body": 1, "shape": sphere(0.3), "restitution": 2 }), "restitution"),
        ("body", json!({ "id": 3, "gravityScale": 3e38 }), "gravity scale"),
        ("body", json!({ "id": 3, "position": [1e20, 0, 0] }), "position"),
        ("body", json!({ "id": 3, "linearDamping": 1e30 }), "damping"),
        ("body", json!({ "id": 3, "rotation": [0, 0, 0, 0] }), "non-zero quaternion"),
        ("pose", json!({ "body": 1, "position": [3e38, 3e38, -3e38] }), "position"),
        ("target", json!({ "body": 1, "position": [1e30, 0, 0] }), "target"),
        ("impulse", json!({ "body": 1, "impulse": [3e38, 0, 0], "point": [0, 3e38, 0] }), "impulse"),
        ("impulse", json!({ "body": 1, "torque": [1e30, 0, 0] }), "angular impulse"),
        ("force", json!({ "body": 1, "force": [3e38, 0, 0], "torque": [3e38, 0, 0] }), "force"),
        ("gravity", json!({ "gravity": [0, -1e30, 0] }), "gravity"),
        ("joint", json!({ "id": 1, "a": 1, "b": 2, "type": "revolute", "limits": [-1e30, 1e30] }), "limits"),
        ("joint", json!({ "id": 1, "a": 1, "b": 2, "type": "revolute", "axis": [0, 0, 0] }), "axis"),
        ("joint", json!({ "id": 1, "a": 1, "b": 2, "type": "rope", "length": 1e30 }), "length"),
        ("joint", json!({ "id": 1, "a": 1, "b": 2, "type": "spherical", "anchor1": [1e30, 0, 0] }), "anchor"),
        ("joint", json!({ "id": 1, "a": 1, "b": 2, "type": "prismatic", "motor": { "velocity": 1e30 } }), "motor"),
        ("ray", json!({ "origin": [1e30, 0, 0], "direction": [1, 0, 0] }), "origin"),
        ("ray", json!({ "origin": [0, 0, 0], "direction": [0, 0, 0] }), "direction"),
        ("overlap", json!({ "shape": sphere(1.0), "position": [1e30, 0, 0] }), "position"),
    ] {
        let e = b.call(&mut s, op, &a).expect_err(&format!("{op} {a} is refused"));
        assert!(e.contains(why), "{op} {a}: {e}");
    }
    assert_eq!(b.bodies(), 2);
    run(&mut b, &mut s, 60);
    // An axis or a rotation of any finite size is a direction: [1e20, 1e20, 0, 0] is a half turn about (1, 1, 0) (the
    // square of 1e20 overflows: it was a zero quaternion, and a body turned by it went non-finite).
    ok(&mut b, &mut s, "pose", json!({ "body": 1, "position": [0, 2, 0], "rotation": [1e20, 1e20, 0, 0] }));
    let q = ok(&mut b, &mut s, "state", json!({ "body": 1 }))["rotation"].clone();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    assert!((0..4).all(|k| (q[k].as_f64().unwrap() - [h, h, 0.0, 0.0][k]).abs() < 1e-6), "{q}");
    ok(&mut b, &mut s, "joint", json!({ "id": 1, "a": 1, "b": 2, "type": "revolute", "axis": [1e20, 0, 1e20] }));
    run(&mut b, &mut s, 30);
    assert_eq!(ok(&mut b, &mut s, "state", json!({ "body": 1 }))["sleeping"], false);
}

/// A1, the fallback: a body whose pose or velocity goes non-finite all the same (here behind the checks, as only
/// Rapier could) is taken out after the tick with its colliders and joints instead of staying disabled and frozen
/// with `sleeping: false`; "state" says why, "faults" reports it once, and a joint on it is refused instead of
/// panicking the next tick. One past the world's edge goes the same way.
#[test]
fn a_body_gone_non_finite_is_taken_out() {
    let (mut b, mut s) = world();
    ok(&mut b, &mut s, "collider", json!({ "id": 1000, "position": [0, -0.5, 0], "shape": { "type": "box", "halfExtents": [50, 0.5, 50] } }));
    ok(&mut b, &mut s, "body", json!({ "id": 1, "object": 1, "slot": 0 }));
    ok(&mut b, &mut s, "collider", json!({ "id": 1, "body": 1, "shape": sphere(0.3) }));
    ok(&mut b, &mut s, "body", json!({ "id": 2, "kind": "fixed", "position": [0, 4, 0] }));
    ok(&mut b, &mut s, "body", json!({ "id": 3, "position": [3, 1, 0] }));
    ok(&mut b, &mut s, "collider", json!({ "id": 3, "body": 3, "shape": sphere(0.3) }));
    ok(&mut b, &mut s, "joint", json!({ "id": 1, "a": 1, "b": 3, "type": "rope", "length": 4 }));
    run(&mut b, &mut s, 2);
    let w = b.bodies[&1];
    b.core.break_body(w);
    run(&mut b, &mut s, 1);
    assert_eq!(ok(&mut b, &mut s, "state", json!({ "body": 1 })), json!({ "removed": "its pose or velocity went non-finite" }));
    assert_eq!(ok(&mut b, &mut s, "faults", json!({})), json!([{ "body": 1, "object": 1, "reason": "its pose or velocity went non-finite" }]));
    assert_eq!(ok(&mut b, &mut s, "faults", json!({})), json!([]), "reported once");
    assert_eq!(b.bodies(), 2);
    assert!(b.call(&mut s, "joint", &json!({ "id": 2, "a": 1, "b": 2, "type": "spherical" })).unwrap_err().contains("unknown body"));
    // Its collider and joint went with it: their ids are free, its object and slot too.
    ok(&mut b, &mut s, "collider", json!({ "id": 1, "body": 3, "position": [0, 0.5, 0], "shape": sphere(0.1) }));
    ok(&mut b, &mut s, "joint", json!({ "id": 1, "a": 3, "b": 2, "type": "spherical" }));
    ok(&mut b, &mut s, "body", json!({ "id": 1, "object": 1, "slot": 0 }));
    assert!(b.call(&mut s, "state", &json!({ "body": 1 })).unwrap()["position"].is_array());
    run(&mut b, &mut s, 30);
    // Past the world's edge (put there behind the checks: 400 m/s, Rapier's cap, would take 40 minutes).
    let w = b.bodies[&3];
    b.core.put_body(w, [0.0, -2.0 * WORLD_EDGE, 0.0]);
    run(&mut b, &mut s, 1);
    assert_eq!(ok(&mut b, &mut s, "state", json!({ "body": 3 }))["removed"], "it left the world (past 1e6 m from the origin)");
    run(&mut b, &mut s, 30);
}

/// A2: a joint from a body to itself (reachable from the binding: `new HingeJoint(a, a)`), then that body removed,
/// panicked the next tick (impulse_joint_set.rs). It is refused; the review's sequence then ticks on.
#[test]
fn a_joint_from_a_body_to_itself_is_refused() {
    let (mut b, mut s) = world();
    for (k, kind) in ["fixed", "kinematic", "dynamic", "kinematic", "fixed"].iter().enumerate() {
        let id = k as u32 + 1;
        ok(&mut b, &mut s, "body", json!({ "id": id, "kind": kind, "position": [id, 2, 0] }));
        ok(&mut b, &mut s, "collider", json!({ "id": id, "body": id, "shape": sphere(0.3) }));
    }
    ok(&mut b, &mut s, "joint", json!({ "id": 1, "a": 2, "b": 1, "type": "revolute" }));
    ok(&mut b, &mut s, "joint", json!({ "id": 2, "a": 5, "b": 3, "type": "prismatic" }));
    let e = b.call(&mut s, "joint", &json!({ "id": 3, "a": 4, "b": 4, "type": "prismatic" })).unwrap_err();
    assert!(e.contains("two different bodies"), "{e}");
    ok(&mut b, &mut s, "joint", json!({ "id": 4, "a": 1, "b": 4, "type": "rope", "length": 0.24 }));
    for r in [2, 5, 4] {
        ok(&mut b, &mut s, "remove", json!({ "body": r }));
    }
    run(&mut b, &mut s, 2);
}

/// A3: a capsule of half height 0 failed parry's assertion in a tick (point_segment.rs: `sqnab != 0`); under 1 mm it
/// is its sphere now. The review's case (a big sensor capsule of no length on a body that a spinning body hits).
#[test]
fn a_capsule_of_no_length_is_its_sphere() {
    for (radius, far) in [(2000.0, -1909.0), (5000.0, -1909.0), (5.0, 0.0)] {
        let (mut b, mut s) = world();
        ok(&mut b, &mut s, "body", json!({ "id": 7, "position": [-0.19, 0.0002, -1.18] }));
        ok(&mut b, &mut s, "collider", json!({ "id": 1, "body": 7, "events": true, "shape": { "type": "capsule", "halfHeight": 0.82, "radius": 0.587 } }));
        ok(&mut b, &mut s, "body", json!({ "id": 3, "ccd": true }));
        ok(&mut b, &mut s, "collider", json!({ "id": 105, "body": 3, "events": true, "shape": { "type": "box", "halfExtents": [0.6, 1.61, 0.767] } }));
        ok(&mut b, &mut s, "collider", json!({ "id": 3, "body": 3, "position": [0.143, -0.145, far], "shape": sphere(0.04) }));
        ok(&mut b, &mut s, "velocity", json!({ "body": 3, "angvel": [19, -7.34, 15.58], "linvel": [-15.0, 4.95, -1.94] }));
        run(&mut b, &mut s, 4);
        ok(&mut b, &mut s, "collider", json!({ "id": 8, "body": 7, "sensor": true, "shape": { "type": "capsule", "halfHeight": 0, "radius": radius } }));
        run(&mut b, &mut s, 30);
        assert!(ok(&mut b, &mut s, "overlap", json!({ "shape": { "type": "capsule", "halfHeight": 0, "radius": 0.5 }, "position": [0, 0, 0] })).is_array());
    }
}

/// A4: sizes of 1e19 and more panicked parry's EPA (epa3.rs: an index out of bounds) in a tick and in queries, and a
/// hull reaching 1e7–1e9 m panicked in a tick (point_tetrahedron.rs). Sizes and hull points are held to 10 km; a
/// trimesh's vertices to 100 km.
#[test]
fn huge_sizes_are_refused() {
    let (mut b, mut s) = world();
    ok(&mut b, &mut s, "body", json!({ "id": 2, "position": [0.3, 3.5, 0] }));
    ok(&mut b, &mut s, "collider", json!({ "id": 2, "body": 2, "shape": { "type": "box", "halfExtents": [0.5, 0.2, 0.3] } }));
    let huge = json!({ "type": "box", "halfExtents": [1e19, 1e19, 1e19] });
    for (op, a, why) in [
        ("collider", json!({ "id": 1, "shape": huge }), "half extents"),
        ("collider", json!({ "id": 1, "shape": { "type": "box", "halfExtents": [1e5, 0.5, 0.5] } }), "half extents"),
        ("collider", json!({ "id": 1, "shape": { "type": "capsule", "halfHeight": 1e9, "radius": 0.5 } }), "half height"),
        ("collider", json!({ "id": 1, "shape": { "type": "convexHull", "points": [-1, 0, -1, 1, 0, -1, 0, 0, 1, 0, -1, 0, 1e7, 0, 0] } }), "points"),
        ("collider", json!({ "id": 1, "shape": { "type": "trimesh", "vertices": [-1e30, 0, -1e30, 1e30, 0, -1e30, 1e30, 0, 1e30, -1e30, 0, 1e30], "indices": [0, 2, 1, 0, 3, 2] } }), "vertices"),
        ("overlap", json!({ "shape": huge, "position": [0, 0, 0] }), "half extents"),
        ("sweep", json!({ "shape": huge, "position": [0, 9, 0], "direction": [0, -1, 0] }), "half extents"),
        ("sweep", json!({ "shape": { "type": "box", "halfExtents": [1e19, 0.5, 0.5] }, "position": [0, 0.2, 0], "rotation": [0.1, 0.2, 0.3, 0.9], "direction": [0, -1, 0] }), "half extents"),
    ] {
        let e = b.call(&mut s, op, &a).expect_err(&format!("{op} {a} is refused"));
        assert!(e.contains(why), "{op} {a}: {e}");
    }
    // At the bounds it holds: a 10 km static box and a 9 km sliver of a hull, ordinary bodies dropped on both, swept.
    ok(&mut b, &mut s, "collider", json!({ "id": 1, "position": [0, -MAX_SIZE, 0], "shape": { "type": "box", "halfExtents": [MAX_SIZE, MAX_SIZE, MAX_SIZE] } }));
    ok(&mut b, &mut s, "collider", json!({ "id": 3, "position": [0, 0.5, 30], "shape": { "type": "convexHull", "points": [-1, 0, -1, 1, 0, -1, 0, 0, 1, 0, -1, 0, 9e3, 0, 0] } }));
    for k in 0..6u32 {
        let shape = [json!({ "type": "convexHull", "points": [0, 0, 0, 0.5, 0, 0, 0, 0.5, 0, 0, 0, 0.5, 0.4, 0.4, 0.4] }), sphere(0.3), json!({ "type": "capsule", "halfHeight": 0.3, "radius": 0.2 })][k as usize % 3].clone();
        ok(&mut b, &mut s, "body", json!({ "id": 10 + k, "position": [0.1 + k as f64 * 0.7, 1.7, 30.1 - 0.05 * k as f64], "rotation": [0.1 * k as f64, 0.2, 0.3, 0.9] }));
        ok(&mut b, &mut s, "collider", json!({ "id": 10 + k, "body": 10 + k, "shape": shape }));
    }
    run(&mut b, &mut s, 240);
    ok(&mut b, &mut s, "sweep", json!({ "shape": { "type": "box", "halfExtents": [MAX_SIZE, 0.5, 0.5] }, "position": [0, 5, 0], "rotation": [0.1, 0.2, 0.3, 0.9], "direction": [0, -1, 0] }));
    ok(&mut b, &mut s, "sweep", json!({ "shape": { "type": "box", "halfExtents": [0.3, 0.3, 0.3] }, "position": [0.1, -0.2, 0.1], "rotation": [0.1, 0.2, 0.3, 0.9], "direction": [0, -1, 0] }));
}

/// A5: the object and the pose table had the call's numbers, not the body's: `rotation: [0, 0, 0, 2]` is the identity
/// in the body and was [0, 0, 0, 2] in the scene. Placing uses the body's own pose now.
#[test]
fn objects_are_placed_where_their_bodies_are() {
    let (mut b, mut s) = world();
    ok(&mut b, &mut s, "body", json!({ "id": 1, "object": 3, "slot": 0, "rotation": [0, 0, 0, 2], "gravityScale": 0 }));
    assert_eq!(s.at[&3].1, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(b.poses()[3..7], [0.0, 0.0, 0.0, 1.0]);
    ok(&mut b, &mut s, "pose", json!({ "body": 1, "position": [1, 2, 3], "rotation": [0, 0, 3, 0] }));
    assert_eq!(s.at[&3], ([1.0, 2.0, 3.0], [0.0, 0.0, 1.0, 0.0]));
    assert_eq!(b.poses()[..7], [1.0, 2.0, 3.0, 0.0, 0.0, 1.0, 0.0]);
}

/// A6: memory with no bound. Contacts nobody takes are held up to `MAX_CONTACTS` (the oldest go, counted); `moved`
/// between syncs stays within a few times the bodies (an offline settle makes up to 36,000 ticks before its sync); a
/// trimesh's triangles, the world's shape data and a query's shape are bounded; an index past 2^32 is not cut to 0.
#[test]
fn memory_is_bounded() {
    let (mut b, mut s) = world();
    for k in 0..200u32 {
        ok(&mut b, &mut s, "collider", json!({ "id": 1000 + k, "sensor": true, "events": true, "shape": sphere(0.5) }));
    }
    ok(&mut b, &mut s, "body", json!({ "id": 1, "position": [0, 5, 0], "gravityScale": 0, "canSleep": false }));
    for k in 0..20u32 {
        ok(&mut b, &mut s, "collider", json!({ "id": k, "body": 1, "events": true, "shape": sphere(0.5) }));
    }
    // Moved in and out of the sensors every tick: 4,000 contacts begin or end a tick, nobody takes them.
    for k in 0..60 {
        ok(&mut b, &mut s, "pose", json!({ "body": 1, "position": [0, if k % 2 == 0 { 0 } else { 5 }, 0] }));
        run(&mut b, &mut s, 1);
    }
    let dropped = ok(&mut b, &mut s, "hash", json!({}))["contactsDropped"].as_u64().unwrap();
    assert_eq!(ok(&mut b, &mut s, "contacts", Value::Null).as_array().unwrap().len(), MAX_CONTACTS);
    assert!(dropped > 100_000, "{dropped} dropped");
    // `moved`, 1,000 awake bodies through 3,600 ticks without a sync.
    let (mut b, mut s) = world();
    for k in 0..1000u32 {
        ok(&mut b, &mut s, "body", json!({ "id": k, "position": [k, 0, 0], "gravityScale": 0, "canSleep": false }));
    }
    for _ in 0..3600 {
        b.tick();
        assert!(b.moved.len() <= 5 * 1000 + 1024, "{}", b.moved.len());
    }
    assert_eq!(b.sync(&mut s), 1000);
    // Triangles: 2 million on 4 vertices; then the world's total.
    let tris = |n: usize| -> Vec<u32> { (0..n).flat_map(|k| [0u32, 1 + (k % 2) as u32, 3]).collect() };
    let mesh = |n: usize| json!({ "type": "trimesh", "vertices": [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1], "indices": tris(n) });
    assert!(b.call(&mut s, "collider", &json!({ "id": 1, "shape": mesh(2_000_000) })).unwrap_err().contains("at most"));
    let mut k = 0;
    let e = loop {
        match b.call(&mut s, "collider", &json!({ "id": 10 + k, "shape": mesh(MAX_MESH_TRIANGLES) })) {
            Ok(_) => k += 1,
            Err(e) => break e,
        }
    };
    assert!(e.contains("points and triangles") && k == MAX_SHAPE_DATA as u32 / (MAX_MESH_TRIANGLES as u32 + 4), "{k}: {e}");
    ok(&mut b, &mut s, "remove", json!({ "collider": 10 }));
    ok(&mut b, &mut s, "collider", json!({ "id": 10, "shape": mesh(MAX_MESH_TRIANGLES) }));
    assert!(b.call(&mut s, "overlap", &json!({ "shape": mesh(1), "position": [0.2, 0.2, 0.2] })).unwrap_err().contains("not a trimesh"));
    let wrap = json!({ "type": "trimesh", "vertices": [0, 0, 0, 1, 0, 0, 0, 1, 0], "indices": [4294967296u64, 1, 2] });
    assert!(b.call(&mut s, "collider", &json!({ "id": 2, "shape": wrap })).unwrap_err().contains("below 2^32"));
}

/// A7: a convex hull of points in a plane was accepted with no volume (mass 0: a dynamic body on it hung in the air);
/// it is refused, as docs/RUNTIME.md says.
#[test]
fn a_flat_hull_is_refused() {
    let (mut b, mut s) = world();
    ok(&mut b, &mut s, "body", json!({ "id": 1, "position": [0, 2, 0] }));
    let flat = json!({ "type": "convexHull", "points": [0, 0, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0.5, 0, 0.5] });
    assert!(b.call(&mut s, "collider", &json!({ "id": 1, "body": 1, "shape": flat })).unwrap_err().contains("flat"));
    let line = json!({ "type": "convexHull", "points": [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3] });
    assert!(b.call(&mut s, "collider", &json!({ "id": 1, "body": 1, "shape": line })).is_err());
    ok(&mut b, &mut s, "collider", json!({ "id": 1, "body": 1, "shape": { "type": "convexHull", "points": [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1] } }));
}

/// A8: a frame owing exactly `MAX_TICKS` ticks dropped its remainder: 12.5 fps made 562 ticks in 10 s, not 600. Only
/// what a frame owes past `MAX_TICKS` is dropped.
#[test]
fn a_frame_of_five_ticks_keeps_its_remainder() {
    for (fps, expected) in [(12.5, 600), (12.0, 600), (13.0, 600), (60.0, 600), (144.0, 600), (10.0, 500)] {
        let mut b = Bridge::default();
        let frames = (10.0 * fps) as u32;
        let ticks: u32 = (0..frames).map(|_| b.due(1.0 / fps)).sum();
        assert!(ticks.abs_diff(expected) <= 1, "{fps} fps: {ticks} ticks in 10 s");
    }
}

/// A9: a body asleep when its object was removed stayed (only a body that moved found out its object was gone): a new
/// object with the same id was refused a body, and an impulse on the old body moved the new object. The scene tells
/// the bridge what it removed.
#[test]
fn a_sleeping_body_goes_with_its_object() {
    let (mut b, mut s) = world();
    ok(&mut b, &mut s, "collider", json!({ "id": 1000, "position": [0, -0.5, 0], "shape": { "type": "box", "halfExtents": [50, 0.5, 50] } }));
    s.at.insert(7, ([0.0, 0.3, 0.0], [0.0, 0.0, 0.0, 1.0]));
    ok(&mut b, &mut s, "body", json!({ "id": 1, "object": 7, "slot": 0 }));
    ok(&mut b, &mut s, "collider", json!({ "id": 1, "body": 1, "shape": { "type": "box", "halfExtents": [0.3, 0.3, 0.3] } }));
    run(&mut b, &mut s, 300);
    assert_eq!(ok(&mut b, &mut s, "state", json!({ "body": 1 }))["sleeping"], true);
    s.at.remove(&7);
    b.objects_removed(&[7]);
    assert!(b.call(&mut s, "state", &json!({ "body": 1 })).unwrap_err().contains("unknown body"));
    s.at.insert(7, ([30.0, 9.0, 30.0], [0.0, 0.0, 0.0, 1.0]));
    ok(&mut b, &mut s, "body", json!({ "id": 2, "object": 7, "slot": 0 }));
    assert!(b.call(&mut s, "impulse", &json!({ "body": 1, "impulse": [0, 50, 0] })).is_err());
    assert_eq!(s.at[&7].0, [30.0, 9.0, 30.0]);
}

/// B3: the hash takes one NaN for all (the platforms make NaNs of different bits: ffc00000 natively, 7fc00000 in
/// wasm32).
#[test]
fn the_hash_takes_one_nan() {
    use crate::world::hash_bits;
    assert_eq!(hash_bits(f32::from_bits(0xffc0_0000)), hash_bits(f32::from_bits(0x7fc0_0000)));
    assert_eq!(hash_bits(-0.0), (-0.0f32).to_bits(), "only NaNs are made one");
}

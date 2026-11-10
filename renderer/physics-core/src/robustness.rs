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
    assert_eq!(ok(&mut b, &mut s, "faults", json!({})), json!([{ "body": 1, "object": 1, "kind": "nonFinite", "reason": "its pose or velocity went non-finite" }]));
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
    // Reported as having left the world, not as an error: a body that falls off the level is an ordinary thing.
    assert_eq!(ok(&mut b, &mut s, "faults", json!({})), json!([{ "body": 3, "object": null, "kind": "leftWorld", "reason": "it left the world (past 1e6 m from the origin)" }]));
    run(&mut b, &mut s, 30);
}

/// A call that is refused changes nothing: an impulse at a point with a torque, whose point (or impulse) is out of
/// bounds, used to apply the torque and wake the body before it was refused.
#[test]
fn a_refused_call_changes_nothing() {
    let (mut b, mut s) = world();
    ok(&mut b, &mut s, "body", json!({ "id": 1, "position": [0, 2, 0], "gravityScale": 0 }));
    ok(&mut b, &mut s, "collider", json!({ "id": 1, "body": 1, "shape": sphere(0.3) }));
    run(&mut b, &mut s, 200);
    let before = ok(&mut b, &mut s, "state", json!({ "body": 1 }));
    assert_eq!(before["sleeping"], true);
    for a in [
        json!({ "body": 1, "impulse": [0, 1, 0], "point": [0, 3e38, 0], "torque": [0, 5, 0] }),
        json!({ "body": 1, "impulse": [3e38, 0, 0], "point": [0, 2, 0], "torque": [0, 5, 0] }),
        json!({ "body": 1, "impulse": [3e38, 0, 0], "torque": [0, 5, 0] }),
        json!({ "body": 1, "impulse": [0, 1, 0], "torque": [0, 3e38, 0] }),
        json!({ "body": 1, "impulse": [0, 1, 0], "point": [0, 2, 0], "torque": [0, 3e38, 0] }),
    ] {
        assert!(b.call(&mut s, "impulse", &a).is_err(), "{a}");
        assert!(b.call(&mut s, "force", &json!({ "body": 1, "force": [0, 1, 0], "torque": [3e38, 0, 0] })).is_err());
        assert!(b.call(&mut s, "velocity", &json!({ "body": 1, "linvel": [0, 1, 0], "angvel": [3e38, 0, 0] })).is_err());
        run(&mut b, &mut s, 1);
        assert_eq!(ok(&mut b, &mut s, "state", json!({ "body": 1 })), before, "after {a}");
    }
    assert!(b.call(&mut s, "remove", &json!({ "body": 1, "collider": "x" })).is_err());
    assert_eq!(ok(&mut b, &mut s, "state", json!({ "body": 1 })), before, "a remove that is refused removes nothing");
    // Within bounds both act: the body turns and moves.
    ok(&mut b, &mut s, "impulse", json!({ "body": 1, "impulse": [0, 1, 0], "point": [0, 2, 0], "torque": [0, 5, 0] }));
    run(&mut b, &mut s, 1);
    let after = ok(&mut b, &mut s, "state", json!({ "body": 1 }));
    assert!(after["linvel"][1].as_f64().unwrap() > 0.0 && after["angvel"][1].as_f64().unwrap() > 0.0, "{after}");
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

/// A query sees the world as the calls left it, with or without a tick since. (The broad phase is the last step's: it
/// had no collider added since, and had a body moved by "pose" where it was, so a ray met it there and missed it
/// where it is. Two events that arrive with no tick between them are the case: the second one's query must see what
/// the first one added.)
#[test]
fn queries_see_what_the_calls_left_without_a_tick() {
    let (mut b, mut s) = world();
    ok(&mut b, &mut s, "collider", json!({ "id": 1000, "position": [0, -0.5, 0], "shape": { "type": "box", "halfExtents": [50, 0.5, 50] } }));
    let ray = |x: f64| json!({ "origin": [x, 9, 0], "direction": [0, -1, 0], "all": true });
    let ball = |x: f64| json!({ "shape": sphere(0.4), "position": [x, 9, 0], "direction": [0, -1, 0] });
    // What the queries answer: the colliders a ray down at x passes through with their distances, the first one's
    // normal, what a 0.4 m ball overlaps at (x, 2, 0), and what a ball lowered from 9 m meets, where and how far.
    let seen = |b: &mut Bridge, s: &mut Objects, x: f64| -> Value {
        let r = ok(b, s, "ray", ray(x));
        let first = ok(b, s, "ray", json!({ "origin": [x, 9, 0], "direction": [0, -1, 0] }));
        let o = ok(b, s, "overlap", json!({ "shape": sphere(0.4), "position": [x, 2, 0] }));
        let w = ok(b, s, "sweep", ball(x));
        let round = |v: &Value| (v.as_f64().unwrap() * 1e3).round() / 1e3;
        json!({
            "ray": r.as_array().unwrap().iter().map(|h| json!([h["collider"], round(&h["distance"])])).collect::<Vec<_>>(),
            "first": [first["collider"], first["body"], round(&first["normal"][1])],
            "overlap": o.as_array().unwrap().iter().map(|h| h["collider"].clone()).collect::<Vec<_>>(),
            "sweep": [w["collider"], round(&w["distance"]), round(&w["point"][1]), round(&w["normal"][1])],
        })
    };
    // A static collider, then a body with a box and a ball on top of it: seen at once, and the same after a tick.
    let ground = json!({ "ray": [[1000, 9.0]], "first": [1000, null, 1.0], "overlap": [], "sweep": [1000, 8.6, 0.0, 1.0] });
    assert_eq!(seen(&mut b, &mut s, 0.0), ground);
    ok(&mut b, &mut s, "body", json!({ "id": 1, "position": [0, 2, 0], "gravityScale": 0 }));
    ok(&mut b, &mut s, "collider", json!({ "id": 1, "body": 1, "shape": { "type": "box", "halfExtents": [0.5, 0.5, 0.5] } }));
    ok(&mut b, &mut s, "collider", json!({ "id": 2, "body": 1, "position": [0, 0.75, 0], "shape": sphere(0.25) }));
    let there = json!({ "ray": [[2, 6.0], [1, 6.5], [1000, 9.0]], "first": [2, 1, 1.0], "overlap": [1], "sweep": [2, 5.6, 3.0, 1.0] });
    assert_eq!(seen(&mut b, &mut s, 0.0), there);
    assert_eq!(ok(&mut b, &mut s, "ray", json!({ "origin": [0, 9, 0], "direction": [0, -1, 0], "skip": 1 }))["collider"], 1000, "its own ray passes through it");
    run(&mut b, &mut s, 1);
    assert_eq!(seen(&mut b, &mut s, 0.0), there);
    // Moved by a call (turned upside down too: the ball is under the box now): it is where it was put, not where it was.
    ok(&mut b, &mut s, "pose", json!({ "body": 1, "position": [5, 2, 0], "rotation": [1, 0, 0, 0] }));
    let moved = json!({ "ray": [[1, 6.5], [2, 7.5], [1000, 9.0]], "first": [1, 1, 1.0], "overlap": [1], "sweep": [1, 6.1, 2.5, 1.0] });
    assert_eq!(seen(&mut b, &mut s, 0.0), ground);
    assert_eq!(seen(&mut b, &mut s, 5.0), moved);
    run(&mut b, &mut s, 1);
    assert_eq!(seen(&mut b, &mut s, 0.0), ground);
    assert_eq!(seen(&mut b, &mut s, 5.0), moved);
    // Removed: gone at once (as it was before).
    ok(&mut b, &mut s, "remove", json!({ "collider": 2 }));
    ok(&mut b, &mut s, "pose", json!({ "body": 1, "position": [5, 2, 0] }));
    ok(&mut b, &mut s, "remove", json!({ "body": 1 }));
    assert_eq!(seen(&mut b, &mut s, 5.0), ground);
}

/// A small generator of integers (the fuzz below is the same on every machine).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    /// A number: ordinary (within ±scale) at level 0; at higher levels a quarter of them are 0 or of any size up to
    /// 1e±4, 1e±9, 1e±20, 1e±40 (past f32: infinite once cast).
    fn num(&mut self, level: u32, scale: f64) -> f64 {
        if level == 0 || self.chance(0.75) {
            return (self.unit() * 2.0 - 1.0) * scale;
        }
        if self.chance(0.1) {
            return 0.0;
        }
        let e = [4, 9, 20, 40][level as usize - 1];
        let x = 10f64.powi(self.below(2 * e + 1) as i32 - e as i32) * (1.0 + self.unit());
        if self.chance(0.5) {
            x
        } else {
            -x
        }
    }
    fn v3(&mut self, level: u32, scale: f64) -> Value {
        json!([self.num(level, scale), self.num(level, scale), self.num(level, scale)])
    }
    fn quat(&mut self, level: u32) -> Value {
        json!([self.num(level, 1.0), self.num(level, 1.0), self.num(level, 1.0), self.num(level, 1.0)])
    }
    fn shape(&mut self, level: u32) -> Value {
        let size = |r: &mut Rng| r.num(level, 1.0).abs() + if level == 0 { 0.05 } else { 0.0 };
        match self.below(6) {
            0 | 1 => json!({ "type": "box", "halfExtents": [size(self), size(self), size(self)] }),
            2 => json!({ "type": "sphere", "radius": size(self) }),
            3 => json!({ "type": "capsule", "halfHeight": if self.chance(0.2) { 0.0 } else { size(self) }, "radius": size(self) }),
            4 => {
                let flat = self.chance(0.15);
                let points: Vec<f64> = (0..4 + self.below(12)).flat_map(|_| [self.num(level, 1.0), if flat { 0.0 } else { self.num(level, 1.0) }, self.num(level, 1.0)]).collect();
                json!({ "type": "convexHull", "points": points })
            }
            _ => {
                let n = 3 + self.below(8);
                let vertices: Vec<f64> = (0..n * 3).map(|_| self.num(level, 3.0)).collect();
                let indices: Vec<u64> = (0..(1 + self.below(8)) * 3).map(|_| self.below(n + 1)).collect();
                json!({ "type": "trimesh", "vertices": vertices, "indices": indices })
            }
        }
    }
}

/// Random calls with ordinary and with absurd numbers (a small version of the review's fuzz, whose 210,000 runs found
/// the panics above: 611 of 120,000 at its two mildest levels before the fixes, none after): no call and no tick
/// panics, the pose table stays finite, and the world's hash can always be taken.
#[test]
fn random_calls_never_panic() {
    for level in 0..=4u32 {
        for seed in 1..=300u64 {
            let mut r = Rng((seed + level as u64 * 1_000_000).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let (mut b, mut s) = world();
            if r.chance(0.8) {
                ok(&mut b, &mut s, "collider", json!({ "id": 999, "position": [0, -0.5, 0], "shape": { "type": "box", "halfExtents": [50, 0.5, 50] }, "events": true }));
            }
            for _ in 0..100 {
                let id = 1 + r.below(10);
                let (op, a): (&str, Value) = match r.below(26) {
                    0..=4 => {
                        let kind = ["dynamic", "dynamic", "dynamic", "fixed", "kinematic"][r.below(5) as usize];
                        let mut a = json!({ "id": id, "kind": kind, "ccd": r.chance(0.3), "canSleep": r.chance(0.7), "position": r.v3(level, 3.0) });
                        if r.chance(0.4) {
                            a["rotation"] = r.quat(level);
                            a["linvel"] = r.v3(level, 10.0);
                            a["angvel"] = r.v3(level, 10.0);
                        }
                        if r.chance(0.2) {
                            a["linearDamping"] = json!(r.num(level, 2.0));
                            a["gravityScale"] = json!(r.num(level, 2.0));
                        }
                        if r.chance(0.5) {
                            a["object"] = json!(1 + r.below(12));
                            a["slot"] = json!(r.below(14));
                        }
                        ("body", a)
                    }
                    5..=9 => {
                        let mut a = json!({ "id": id + 100 * r.below(3), "shape": r.shape(level), "sensor": r.chance(0.15), "events": r.chance(0.5) });
                        if r.chance(0.85) {
                            a["body"] = json!(1 + r.below(10));
                        }
                        if r.chance(0.3) {
                            a["position"] = r.v3(level, 1.0);
                            a["rotation"] = r.quat(level);
                            a[if r.chance(0.5) { "mass" } else { "density" }] = json!(r.num(level, 100.0));
                            a["friction"] = json!(r.num(level, 2.0));
                        }
                        ("collider", a)
                    }
                    10..=12 => {
                        let ty = ["fixed", "revolute", "prismatic", "spherical", "rope"][r.below(5) as usize];
                        let mut a = json!({ "id": id, "a": 1 + r.below(10), "b": 1 + r.below(10), "type": ty, "length": r.num(level, 2.0).abs(), "anchor1": r.v3(level, 1.0), "anchor2": r.v3(level, 1.0) });
                        if r.chance(0.5) {
                            a["axis"] = r.v3(level, 1.0);
                            a["limits"] = json!([r.num(level, 2.0), r.num(level, 2.0)]);
                            a["motor"] = json!({ "velocity": r.num(level, 5.0), "damping": r.num(level, 1e4), "maxForce": r.num(level, 1e5) });
                        }
                        ("joint", a)
                    }
                    13 => ("remove", [json!({ "body": id }), json!({ "collider": id }), json!({ "joint": id })][r.below(3) as usize].clone()),
                    14 => ("impulse", if r.chance(0.5) { json!({ "body": id, "impulse": r.v3(level, 500.0), "torque": r.v3(level, 50.0) }) } else { json!({ "body": id, "impulse": r.v3(level, 500.0), "point": r.v3(level, 3.0) }) }),
                    15 => ("force", json!({ "body": id, "force": r.v3(level, 5000.0), "torque": r.v3(level, 500.0) })),
                    16 => ("velocity", json!({ "body": id, "linvel": r.v3(level, 20.0), "angvel": r.v3(level, 20.0) })),
                    17 => ("pose", json!({ "body": id, "position": r.v3(level, 3.0), "rotation": r.quat(level) })),
                    18 => ("target", json!({ "body": id, "position": r.v3(level, 3.0), "rotation": r.quat(level) })),
                    19 => {
                        // The scene moves or removes an object; its body follows.
                        let o = 1 + r.below(12) as u32;
                        if r.chance(0.2) {
                            s.at.remove(&o);
                            b.objects_removed(&[o]);
                        } else {
                            s.at.insert(o, ([r.num(level, 3.0) as f32, r.num(level, 3.0) as f32, r.num(level, 3.0) as f32], [0.0, 0.0, 0.0, 1.0]));
                        }
                        ("follow", json!({ "body": id }))
                    }
                    20 => ("ray", json!({ "origin": r.v3(level, 5.0), "direction": r.v3(level, 1.0), "max": r.num(level, 100.0).abs(), "all": r.chance(0.5), "skip": id })),
                    21 => ("overlap", json!({ "shape": r.shape(level), "position": r.v3(level, 3.0), "rotation": r.quat(level) })),
                    22 => ("sweep", json!({ "shape": r.shape(level), "position": r.v3(level, 3.0), "rotation": r.quat(level), "direction": r.v3(level, 1.0), "max": r.num(level, 100.0).abs() })),
                    23 => ("gravity", json!({ "gravity": r.v3(level, 20.0) })),
                    24 => ([("contacts", Value::Null), ("faults", json!({})), ("hash", json!({}))][r.below(3) as usize]).clone(),
                    _ => {
                        for _ in 0..1 + r.below(5) {
                            b.tick();
                        }
                        b.sync(&mut s);
                        ("state", json!({ "body": id }))
                    }
                };
                // (A call may be refused, with its reason; it may not panic.)
                let _ = b.call(&mut s, op, &a);
                assert!(b.poses().iter().all(|x| x.is_finite()), "level {level}, seed {seed}: a pose is not finite after {op} {a}");
            }
        }
    }
}

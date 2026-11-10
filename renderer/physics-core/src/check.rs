//! A fixed scene stepped for a fixed number of ticks, and the hash of where it ends (ROADMAP § 6 step 7's determinism
//! check): a native binary and a wasm32 module (examples/determinism*.rs) both run it.

use crate::world::*;

/// The hash `script(600)` leaves with `enhanced-determinism` (recorded: a change of behaviour, or of Rapier, shows
/// here). Natively and in wasm32 alike.
pub const EXPECTED: u64 = 0x113d_50eb_06a4_7636;

/// A tower of boxes hit by a ball, a hinged door pushed by another, a rope pendulum and a slider: contacts, friction,
/// stacking and four kinds of joints, for `ticks` ticks. Returns the world's hash.
pub fn script(ticks: u32) -> u64 {
    let mut p = Physics::default();
    p.add_collider(None, &ColliderDesc::new(Shape::Box([50.0, 0.5, 50.0]))).unwrap();
    let dynamic = |position: [f32; 3]| BodyDesc { position, ..Default::default() };
    // Ground at y = 0.5 (the box above is centred at 0): a 4 x 6 x 4 tower of half-metre boxes.
    for y in 0..6 {
        for x in 0..4 {
            for z in 0..4 {
                let b = p.add_body(&dynamic([x as f32 * 0.5 - 0.75 + 0.01 * y as f32, 0.75 + y as f32 * 0.5, z as f32 * 0.5 - 0.75])).unwrap();
                p.add_collider(Some(b), &ColliderDesc::new(Shape::Box([0.25; 3]))).unwrap();
            }
        }
    }
    let ball = p.add_body(&BodyDesc { position: [-6.0, 1.5, 0.1], linvel: [14.0, 1.0, 0.0], ccd: true, ..Default::default() }).unwrap();
    p.add_collider(Some(ball), &ColliderDesc { restitution: 0.4, ..ColliderDesc::new(Shape::Sphere(0.4)) }).unwrap();
    // A door on a hinge at a fixed post, pushed by a capsule.
    let post = p.add_body(&BodyDesc { kind: BodyKind::Fixed, position: [5.0, 1.5, 0.0], ..Default::default() }).unwrap();
    let door = p.add_body(&dynamic([5.5, 1.5, 0.0])).unwrap();
    p.add_collider(Some(door), &ColliderDesc::new(Shape::Box([0.5, 1.0, 0.05]))).unwrap();
    p.add_joint(post, door, &JointDesc::Revolute { anchor1: [0.0; 3], anchor2: [-0.5, 0.0, 0.0], axis: [0.0, 1.0, 0.0], limits: Some(Limits { min: -1.5, max: 1.5 }), motor: None }).unwrap();
    let pusher = p.add_body(&BodyDesc { position: [5.7, 1.5, 3.0], linvel: [0.0, 0.0, -6.0], ..Default::default() }).unwrap();
    p.add_collider(Some(pusher), &ColliderDesc::new(Shape::Capsule { half_height: 0.3, radius: 0.2 })).unwrap();
    // A weight on a rope, a hull on a ball joint, a slider with a motor.
    let weight = p.add_body(&dynamic([-5.0, 3.0, 4.0])).unwrap();
    p.add_collider(Some(weight), &ColliderDesc::new(Shape::Sphere(0.3))).unwrap();
    p.add_joint(post, weight, &JointDesc::Rope { anchor1: [-8.0, 3.5, 4.0], anchor2: [0.0; 3], length: 2.5 }).unwrap();
    let hull = p.add_body(&dynamic([8.0, 2.0, -3.0])).unwrap();
    p.add_collider(Some(hull), &ColliderDesc::new(Shape::ConvexHull(vec![[0.0, 0.5, 0.0], [0.4, -0.3, 0.3], [-0.4, -0.3, 0.3], [0.0, -0.3, -0.5], [0.1, 0.1, 0.6]]))).unwrap();
    p.add_joint(post, hull, &JointDesc::Spherical { anchor1: [3.0, 1.5, -3.0], anchor2: [0.0, 0.5, 0.0] }).unwrap();
    let slider = p.add_body(&dynamic([0.0, 3.0, -6.0])).unwrap();
    p.add_collider(Some(slider), &ColliderDesc::new(Shape::Box([0.3; 3]))).unwrap();
    p.add_joint(post, slider, &JointDesc::Prismatic { anchor1: [-5.0, 1.5, -6.0], anchor2: [0.0; 3], axis: [1.0, 0.0, 0.0], limits: Some(Limits { min: -2.0, max: 2.0 }), motor: Some(Motor { velocity: 1.5, damping: 400.0, max_force: 500.0 }) }).unwrap();
    let mut h = 0u64;
    for t in 0..ticks {
        p.step();
        // A kick halfway, and the hash folded in every 30 ticks (so a divergence anywhere on the way shows).
        if t == ticks / 2 {
            p.apply_impulse(ball, [0.0, 80.0, 30.0], [0.0; 3]).unwrap();
        }
        if t % 30 == 29 {
            h = (h ^ p.hash()).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h ^ p.hash()
}

/// `n` boxes dropped in a heap on the ground (a 10 x 10 column of them, slightly offset so they tumble), stepped
/// `ticks` ticks: the solver's cost with many contacts. Returns the world's hash.
pub fn pile(n: u32, ticks: u32) -> u64 {
    let mut p = Physics::default();
    p.add_collider(None, &ColliderDesc::new(Shape::Box([50.0, 0.5, 50.0]))).unwrap();
    for k in 0..n {
        let (x, z, y) = (k % 10, (k / 10) % 10, k / 100);
        let jitter = ((k * 7919) % 13) as f32 * 0.01;
        let b = p.add_body(&BodyDesc { position: [x as f32 * 0.55 - 2.5 + jitter, 0.8 + y as f32 * 0.55, z as f32 * 0.55 - 2.5 - jitter], ..Default::default() }).unwrap();
        p.add_collider(Some(b), &ColliderDesc::new(Shape::Box([0.25; 3]))).unwrap();
    }
    for _ in 0..ticks {
        p.step();
    }
    p.hash()
}

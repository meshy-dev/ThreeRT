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

/// Random worlds for the determinism check, by kind: each is `case(kind, seed)`. The script above has one convex hull
/// and no body of several colliders, and those are where the platforms' math first showed (the principal axes of an
/// inertia tensor that is not diagonal: an eigen decomposition with an acos and two cos).
pub const KINDS: [(&str, u32); 4] = [
    ("convex hulls", 400),
    ("bodies of several colliders", 400),
    ("joints with limits and motors", 200),
    // The rest of what a game's calls reach (docs/ROADMAP.md § 6 step 9 has the upstream list of platform math calls
    // it is checked against): continuous collision detection, a static triangle mesh, a kinematic body, fixed and ball
    // joints, and the answers of rays, overlaps and sweeps (asked with and without a tick since the last change).
    ("ccd on a trimesh with a kinematic body, fixed and ball joints and queries", 200),
];

/// The hash of all the cases (`cases()`) with `enhanced-determinism`, natively and in wasm32 alike.
pub const EXPECTED_CASES: u64 = 0x6fd1_eba4_551b_8d39;

/// A generator of integers only: a float made from one is exact arithmetic, the same bits on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// In [-1, 1), 24 bits.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 8_388_608.0 - 1.0
    }
    fn v3(&mut self, scale: f32) -> [f32; 3] {
        [self.unit() * scale, self.unit() * scale, self.unit() * scale]
    }
    /// In [lo, hi).
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.unit() * 0.5 + 0.5) * (hi - lo)
    }
    fn quat(&mut self) -> [f32; 4] {
        [self.unit(), self.unit(), self.unit(), self.unit() + 1.5]
    }
}

/// One random world of a kind (`KINDS`), stepped 90 ticks on a ground: its hash, folded every 10 ticks.
pub fn case(kind: u32, seed: u32) -> u64 {
    let mut r = Rng((seed as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ ((kind as u64 + 1) << 56) | 1);
    for _ in 0..4 {
        r.next();
    }
    let mut p = Physics::default();
    p.add_collider(None, &ColliderDesc { position: [0.0, -0.5, 0.0], ..ColliderDesc::new(Shape::Box([50.0, 0.5, 50.0])) }).unwrap();
    let body = |p: &mut Physics, r: &mut Rng, at: [f32; 3]| p.add_body(&BodyDesc { position: at, rotation: r.quat(), linvel: r.v3(2.0), angvel: r.v3(10.0), ..Default::default() }).unwrap();
    let hull = |r: &mut Rng| Shape::ConvexHull((0..8).map(|_| r.v3(0.5)).collect());
    // (Kind 3's kinematic body: where it is driven, and how it is turned.)
    let mut pusher: Option<(BodyId, [f32; 3], [f32; 4])> = None;
    match kind {
        // Two tumbling hulls of 8 random points that meet over the ground.
        0 => {
            for k in 0..2 {
                let b = body(&mut p, &mut r, [k as f32 * 0.6, 1.5 + k as f32, 0.0]);
                // (Points that hold no volume are refused: another draw.)
                while p.add_collider(Some(b), &ColliderDesc::new(hull(&mut r))).is_err() {}
            }
        }
        // A body of two boxes and one of a capsule and a sphere, each collider at its own place and turn.
        1 => {
            let a = body(&mut p, &mut r, [0.0, 1.5, 0.0]);
            for _ in 0..2 {
                let shape = Shape::Box([r.range(0.1, 0.6), r.range(0.1, 0.6), r.range(0.1, 0.6)]);
                p.add_collider(Some(a), &ColliderDesc { position: r.v3(0.7), rotation: r.quat(), ..ColliderDesc::new(shape) }).unwrap();
            }
            let b = body(&mut p, &mut r, [0.5, 3.5, 0.2]);
            p.add_collider(Some(b), &ColliderDesc { position: r.v3(0.5), rotation: r.quat(), ..ColliderDesc::new(Shape::Capsule { half_height: r.range(0.1, 0.5), radius: r.range(0.1, 0.3) }) }).unwrap();
            p.add_collider(Some(b), &ColliderDesc { position: r.v3(0.5), density: r.range(200.0, 3000.0), ..ColliderDesc::new(Shape::Sphere(r.range(0.1, 0.4))) }).unwrap();
        }
        // A box hinged to a post (limits, a motor) and a hull on a slider and a rope, all turned at random.
        2 => {
            let post = p.add_body(&BodyDesc { kind: BodyKind::Fixed, position: [0.0, 3.0, 0.0], rotation: r.quat(), ..Default::default() }).unwrap();
            let a = body(&mut p, &mut r, [1.0, 3.0, 0.0]);
            p.add_collider(Some(a), &ColliderDesc::new(Shape::Box([r.range(0.2, 0.6), r.range(0.1, 0.3), r.range(0.2, 0.6)]))).unwrap();
            let (lo, hi) = (r.range(-2.0, -0.2), r.range(0.2, 2.0));
            let motor = Motor { velocity: r.unit() * 5.0, damping: r.range(10.0, 500.0), max_force: r.range(10.0, 500.0) };
            p.add_joint(post, a, &JointDesc::Revolute { anchor1: r.v3(0.3), anchor2: r.v3(0.5), axis: r.v3(1.0), limits: Some(Limits { min: lo, max: hi }), motor: Some(motor) }).unwrap();
            let b = body(&mut p, &mut r, [-1.0, 3.0, 0.5]);
            while p.add_collider(Some(b), &ColliderDesc::new(hull(&mut r))).is_err() {}
            p.add_joint(post, b, &JointDesc::Prismatic { anchor1: r.v3(0.3), anchor2: r.v3(0.3), axis: r.v3(1.0), limits: Some(Limits { min: r.range(-1.5, -0.1), max: r.range(0.1, 1.5) }), motor: None }).unwrap();
            p.add_joint(a, b, &JointDesc::Rope { anchor1: r.v3(0.3), anchor2: r.v3(0.3), length: r.range(1.0, 2.5) }).unwrap();
        }
        // Fast small bodies with CCD onto a static triangle mesh with relief, a kinematic box driven through them,
        // two boxes held by a fixed joint and a capsule on a ball joint.
        _ => {
            let n = 9u32;
            let vertices: Vec<[f32; 3]> = (0..n * n).map(|k| [(k % n) as f32 - 4.0, 0.3 + 0.15 * r.unit(), (k / n) as f32 - 4.0]).collect();
            let indices: Vec<[u32; 3]> = (0..(n - 1) * (n - 1)).flat_map(|k| { let a = k / (n - 1) * n + k % (n - 1); [[a, a + n, a + 1], [a + 1, a + n, a + n + 1]] }).collect();
            p.add_collider(None, &ColliderDesc::new(Shape::Trimesh { vertices, indices })).unwrap();
            for k in 0..3 {
                let b = p.add_body(&BodyDesc { position: [k as f32 - 1.0 + 0.3 * r.unit(), 4.0 + k as f32, 0.3 * r.unit()], linvel: [r.unit() * 5.0, r.range(-60.0, -30.0), r.unit() * 5.0], angvel: r.v3(10.0), ccd: true, ..Default::default() }).unwrap();
                let shape = if k == 1 { Shape::Box([r.range(0.05, 0.15), r.range(0.05, 0.15), r.range(0.05, 0.15)]) } else { Shape::Sphere(r.range(0.05, 0.15)) };
                p.add_collider(Some(b), &ColliderDesc { restitution: r.range(0.0, 0.8), ..ColliderDesc::new(shape) }).unwrap();
            }
            pusher = Some((p.add_body(&BodyDesc { kind: BodyKind::Kinematic, position: [-3.0, 0.8, 0.0], ..Default::default() }).unwrap(), [r.range(1.0, 3.0), 0.0, r.unit()], r.quat()));
            p.add_collider(pusher.map(|x| x.0), &ColliderDesc::new(Shape::Box([0.3, 0.3, 0.6]))).unwrap();
            let a = body(&mut p, &mut r, [0.0, 2.0, 2.0]);
            p.add_collider(Some(a), &ColliderDesc::new(Shape::Box([0.3, 0.2, 0.3]))).unwrap();
            let b = body(&mut p, &mut r, [0.7, 2.0, 2.0]);
            p.add_collider(Some(b), &ColliderDesc::new(Shape::Box([0.2, 0.3, 0.2]))).unwrap();
            p.add_joint(a, b, &JointDesc::Fixed { anchor1: r.v3(0.4), anchor2: r.v3(0.4) }).unwrap();
            let post = p.add_body(&BodyDesc { kind: BodyKind::Fixed, position: [2.0, 3.0, -2.0], ..Default::default() }).unwrap();
            let c = body(&mut p, &mut r, [2.0, 2.4, -2.0]);
            p.add_collider(Some(c), &ColliderDesc::new(Shape::Capsule { half_height: r.range(0.1, 0.4), radius: r.range(0.1, 0.2) })).unwrap();
            p.add_joint(post, c, &JointDesc::Spherical { anchor1: r.v3(0.2), anchor2: [0.0, 0.5, 0.0] }).unwrap();
        }
    }
    let mut h = 0u64;
    let mut fold = |x: u64| h = (h ^ x).wrapping_mul(0x0000_0100_0000_01b3);
    for t in 0..90u32 {
        if let Some((b, v, q)) = pusher {
            let s = (t + 1) as f32 * crate::TICK;
            p.set_kinematic_target(b, [-3.0 + v[0] * s, 0.8, v[2] * s], q).unwrap();
        }
        p.step();
        if t % 10 == 9 {
            fold(p.hash());
            if kind == 3 {
                // Queries, their answers folded in bit for bit: a ray and a ball swept down at a place, what a box
                // overlaps; then the same once more after a body is moved by a call (no tick since: `Physics::fresh`).
                let at = [r.unit() * 3.0, 6.0, r.unit() * 3.0];
                for pass in 0..2 {
                    for hit in [p.cast_ray(at, [r.unit() * 0.3, -1.0, r.unit() * 0.3], 20.0, None).unwrap(), p.sweep(&Shape::Sphere(0.2), at, [0.0, 0.0, 0.0, 1.0], [0.0, -1.0, 0.0], 20.0, None).unwrap()] {
                        match hit {
                            Some(x) => {
                                fold(x.collider as u64);
                                for f in [x.distance].into_iter().chain(x.point).chain(x.normal) {
                                    fold(crate::world::hash_bits(f) as u64);
                                }
                            }
                            None => fold(u64::MAX),
                        }
                    }
                    for c in p.overlap(&Shape::Box([1.0, 0.5, 1.0]), [at[0], 0.6, at[2]], r.quat(), None).unwrap() {
                        fold(c as u64);
                    }
                    if pass == 0 {
                        if let Some((b, _, q)) = pusher {
                            p.set_pose(b, [at[0], 1.0, at[2]], q).unwrap();
                        }
                    }
                }
            }
        }
    }
    h
}

/// Every case's hash folded into one (`EXPECTED_CASES`).
pub fn cases() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (kind, &(_, n)) in KINDS.iter().enumerate() {
        for seed in 0..n {
            h = (h ^ case(kind as u32, seed)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    /// This build steps to the recorded bits (tools/physics-determinism.ts shows the wasm32 build does too): a
    /// dependency or a feature that brings the platform's math functions back shows here.
    #[test]
    fn this_build_steps_to_the_recorded_bits() {
        assert_eq!(format!("{:016x}", super::script(600)), format!("{:016x}", super::EXPECTED));
        assert_eq!(format!("{:016x}", super::cases()), format!("{:016x}", super::EXPECTED_CASES));
    }

    /// Where Rapier 0.36 and parry 0.31 still take a math function from the platform (f32's own methods, whenever
    /// `std` is in the crate graph: third_party/glamx/PATCH.md has how), on the paths a 3D world of ours runs, the
    /// platform this test runs on gives what libm gives, so those calls change nothing:
    ///   - the broad phase's BVH optimizer (parry bvh_optimize.rs `optimization_config`) takes two `log2` of numbers
    ///     that depend on the collider count alone, and rounds what it makes of them to a count of subtrees: the same
    ///     count with either `log2`, for every collider count the world may hold (and twice as many);
    ///   - a convex hull's `support_feature_id_toward` takes the sine and cosine of one constant, a degree.
    /// (The eigen decomposition is patched in glamx, and a hinge's limits reach Rapier centred: world.rs `add_joint`.)
    #[test]
    fn the_platforms_math_changes_nothing_where_rapier_still_calls_it() {
        let subtrees = |leaves: u32, log2: fn(f32) -> f32| -> f32 {
            let root = (leaves as f32).sqrt();
            let (roots, subtree) = (root.ceil(), (root * 4.0).ceil());
            let cost = roots * log2(roots) / (subtree * log2(subtree));
            ((leaves * 5).div_ceil(100) as f32 / subtree - cost).round().max(0.0)
        };
        for leaves in 1..=2 * crate::bridge::MAX_COLLIDERS as u32 {
            assert_eq!(subtrees(leaves, f32::log2), subtrees(leaves, libm::log2f), "{leaves} colliders");
        }
        let degree = (core::f64::consts::PI / 180.0) as f32;
        let ((s, c), (ls, lc)) = (degree.sin_cos(), libm::sincosf(degree));
        assert_eq!((s.to_bits(), c.to_bits()), (ls.to_bits(), lc.to_bits()));
    }

    /// A triangle mesh is built without the flags that make parry compute pseudo normals, which takes the angles of
    /// every triangle with the platform's `acos` (parry vector_ext.rs `angle`; upstream's list, ROADMAP § 6 step 9).
    #[test]
    fn a_trimesh_has_no_pseudo_normals() {
        use rapier3d::prelude::{SharedShape, TriMeshFlags, Vector};
        let s = SharedShape::trimesh(vec![Vector::new(0.0, 0.0, 0.0), Vector::new(1.0, 0.0, 0.0), Vector::new(0.0, 0.0, 1.0)], vec![[0, 2, 1]]).unwrap();
        let mesh = s.as_trimesh().unwrap();
        assert!(mesh.flags().is_empty() && mesh.pseudo_normals().is_none(), "{:?}", mesh.flags());
        assert!(!mesh.flags().intersects(TriMeshFlags::ORIENTED | TriMeshFlags::FIX_INTERNAL_EDGES));
    }
}

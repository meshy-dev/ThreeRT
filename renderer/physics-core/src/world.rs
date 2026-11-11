//! The physics world: plain descriptions in, handles and poses out.

use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::*;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The fixed step, s. The engine steps whole ticks only.
pub const TICK: f32 = 1.0 / 60.0;

/// The bounds every number of a description or a call is held to (docs/RUNTIME.md lists them). Game code is untrusted,
/// and a number Rapier cannot take panics a tick natively and traps it in wasm32, so the world refuses what is outside
/// them (with the reason), whoever calls it. What the simulation itself drives out of bounds is taken out of the world
/// after the tick (`Physics::take_faults`).
pub mod limits {
    /// Positions of bodies and of static colliders, joint anchors, points of impulses, query origins, a trimesh's
    /// vertices, joint limits and a rope's length: |x| ≤ 100 km.
    pub const MAX_COORD: f32 = 1e5;
    /// A body that gets further than this from the origin on any axis is taken out of the world (it left the level).
    pub const WORLD_EDGE: f32 = 1e6;
    /// Half extents, radii, a capsule's radius: from 1 mm to 10 km; a capsule's half height up to 10 km (under 1 mm it
    /// is a sphere). A convex hull's points and a collider's offset on its body: |x| ≤ 10 km.
    pub const MIN_SIZE: f32 = 1e-3;
    pub const MAX_SIZE: f32 = 1e4;
    /// A convex hull holds at least 1 mm³ (points in a plane or on a line have none, so no mass).
    pub const MIN_VOLUME: f32 = 1e-9;
    /// m/s and rad/s, as given (what the simulation reaches is not bounded by these).
    pub const MAX_LINVEL: f32 = 1e4;
    pub const MAX_ANGVEL: f32 = 1e3;
    /// N s (and N m s for an angular one), N (and N m).
    pub const MAX_IMPULSE: f32 = 1e6;
    pub const MAX_FORCE: f32 = 1e8;
    /// A collider's mass (kg) and density (kg/m³): 0 (it adds nothing to its body) or within these.
    pub const MASS: [f32; 2] = [1e-3, 1e7];
    pub const DENSITY: [f32; 2] = [1e-3, 1e5];
    /// |gravity scale|; each component of the world's gravity, m/s².
    pub const MAX_GRAVITY_SCALE: f32 = 100.0;
    pub const MAX_GRAVITY: f32 = 1e3;
    /// Damping (linear and angular, 1/s) and friction, from 0; restitution is from 0 to 1.
    pub const MAX_DAMPING: f32 = 1e3;
    pub const MAX_FRICTION: f32 = 1e3;
    /// A joint motor: |velocity| (rad/s or m/s) and its damping; its force has no upper bound (Rapier's default).
    pub const MAX_MOTOR_VELOCITY: f32 = 1e4;
    pub const MAX_MOTOR_DAMPING: f32 = 1e9;
    /// A query reaches at most this far, m (a longer `max` is cut to it).
    pub const MAX_QUERY_DISTANCE: f32 = 1e6;
    /// A trimesh's triangles, and the shape data the whole world holds (a hull's points, a trimesh's vertices and
    /// triangles, summed over its colliders).
    pub const MAX_MESH_TRIANGLES: usize = 1 << 20;
    pub const MAX_SHAPE_DATA: usize = 1 << 22;
    /// Contact events held until `take_contacts`: past this the oldest go (and are counted).
    pub const MAX_CONTACTS: usize = 8192;
    /// A voxel shape: its cells' size, m; each cell's coordinates (so the shape reaches at most 2^20 cells from its
    /// origin, and `size` x that within `MAX_COORD`); the cells one shape holds.
    pub const VOXEL_SIZE: [f32; 2] = [1e-2, 1e2];
    pub const MAX_VOXEL_COORD: i32 = 1 << 20;
    pub const MAX_VOXELS: usize = 1 << 24;
}
use limits::*;

pub type BodyId = u32;
pub type ColliderId = u32;
pub type JointId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// Moved by forces and contacts.
    Dynamic,
    /// Never moves.
    Fixed,
    /// Moved by the game (`set_kinematic_target`): it pushes dynamic bodies and is pushed by nothing.
    Kinematic,
}

/// A rigid body: position (m), rotation (quaternion xyzw), velocities (m/s, rad/s).
#[derive(Clone, Debug)]
pub struct BodyDesc {
    pub kind: BodyKind,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub linvel: [f32; 3],
    pub angvel: [f32; 3],
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_scale: f32,
    pub can_sleep: bool,
    /// Continuous collision detection (fast, small bodies).
    pub ccd: bool,
}

impl Default for BodyDesc {
    fn default() -> Self {
        BodyDesc { kind: BodyKind::Dynamic, position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], linvel: [0.0; 3], angvel: [0.0; 3], linear_damping: 0.0, angular_damping: 0.0, gravity_scale: 1.0, can_sleep: true, ccd: false }
    }
}

/// A collider shape the engine defines (not one of `Shape`'s): it hands Rapier the shape (the voxel shape of step 8).
pub trait CustomShape {
    fn shared(&self) -> SharedShape;
}

pub enum Shape {
    /// Half extents.
    Box([f32; 3]),
    Sphere(f32),
    /// Along y: half the segment's length, and the radius.
    Capsule { half_height: f32, radius: f32 },
    /// The convex hull of the points.
    ConvexHull(Vec<[f32; 3]>),
    /// Triangles: for fixed bodies and the world's static colliders only. (Rapier gives an open trimesh a mass from its
    /// triangles as if they bounded a volume: a few grams with the centre of mass far away, which a dynamic body
    /// cannot take.)
    Trimesh { vertices: Vec<[f32; 3]>, indices: Vec<[u32; 3]> },
    /// Solid cells of a lattice of cubes `size` metres wide: cell (x, y, z) fills from (x, y, z) x size to (x + 1,
    /// y + 1, z + 1) x size in the collider's space. The engine's shape for voxel objects (ROADMAP § 6 step 8): the
    /// level's, fixed, and the parts that come loose, which move. Its cells can be changed where it is
    /// (`Physics::edit_voxels`). On a moving body its mass is its cells' volume at the collider's density, its centre
    /// of mass and inertia those of the cells.
    Voxels { size: f32, cells: Vec<[i32; 3]> },
    Custom(Box<dyn CustomShape>),
}

/// A collider: its shape, its pose relative to its body (or in the world without one), and its material.
pub struct ColliderDesc {
    pub shape: Shape,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    /// kg/m³ (the body's mass is its colliders').
    pub density: f32,
    /// Its mass in kg, whatever its volume, instead of a density.
    pub mass: Option<f32>,
    pub friction: f32,
    pub restitution: f32,
    /// A sensor reports what overlaps it and collides with nothing.
    pub sensor: bool,
    /// Report its contacts (`Physics::take_contacts`).
    pub events: bool,
}

impl ColliderDesc {
    pub fn new(shape: Shape) -> ColliderDesc {
        ColliderDesc { shape, position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], density: 1000.0, mass: None, friction: 0.5, restitution: 0.0, sensor: false, events: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub min: f32,
    pub max: f32,
}

/// A velocity motor: the joint is driven toward `velocity` (rad/s or m/s) by a force of `damping` x the difference (N
/// s/m, or N m s/rad), at most `max_force`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motor {
    pub velocity: f32,
    pub damping: f32,
    pub max_force: f32,
}

/// A joint between two bodies; anchors and axes in each body's own space.
#[derive(Clone, Debug)]
pub enum JointDesc {
    /// The two bodies keep their relative pose: `anchor1` on the first is held at `anchor2` on the second.
    Fixed { anchor1: [f32; 3], anchor2: [f32; 3] },
    /// A hinge about `axis` through the anchors.
    Revolute { anchor1: [f32; 3], anchor2: [f32; 3], axis: [f32; 3], limits: Option<Limits>, motor: Option<Motor> },
    /// A slider along `axis`.
    Prismatic { anchor1: [f32; 3], anchor2: [f32; 3], axis: [f32; 3], limits: Option<Limits>, motor: Option<Motor> },
    /// A ball joint.
    Spherical { anchor1: [f32; 3], anchor2: [f32; 3] },
    /// The anchors stay within `length` of each other (a rope).
    Rope { anchor1: [f32; 3], anchor2: [f32; 3], length: f32 },
}

/// Two colliders began or stopped touching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contact {
    pub a: ColliderId,
    pub b: ColliderId,
    pub started: bool,
}

/// A body (or a static collider) the world took out after a tick, with the colliders that went with it, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Fault {
    pub body: Option<BodyId>,
    pub colliders: Vec<ColliderId>,
    /// The body left the world (past `WORLD_EDGE`: it fell off the level, an ordinary thing in a game); otherwise its
    /// state or a shape went non-finite (bad data or a blown-up simulation: an error the author should hear of).
    pub left_world: bool,
    pub reason: &'static str,
}

/// What a ray or a swept shape met: the collider and its body, the distance along the direction, the surface normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub collider: ColliderId,
    pub body: Option<BodyId>,
    pub distance: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Default)]
struct Events(Mutex<Vec<CollisionEvent>>);

impl EventHandler for Events {
    fn handle_collision_event(&self, _: &RigidBodySet, _: &ColliderSet, event: CollisionEvent, _: Option<&ContactPair>) {
        self.0.lock().unwrap().push(event);
    }
    fn handle_contact_force_event(&self, _: Real, _: &RigidBodySet, _: &ColliderSet, _: &ContactPair, _: Real) {}
    fn handle_soft_body_tear_event(&self, _: &SoftBodySet, _: &SoftBodyTearEvent) {}
}

fn v(a: [f32; 3]) -> Vector {
    Vector::new(a[0], a[1], a[2])
}

/// The rotation of a quaternion of any finite size (scaled by its largest component first: squaring 1e20 overflows).
/// Err for one that is not finite or is zero.
fn rotation(q: [f32; 4]) -> Result<Rotation, String> {
    let m = q.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if !(m.is_finite() && m > 0.0) {
        return Err("physics: a rotation must be a finite, non-zero quaternion (x, y, z, w)".into());
    }
    Ok(Rotation::from_xyzw(q[0] / m, q[1] / m, q[2] / m, q[3] / m).normalize())
}

fn pose(p: [f32; 3], q: [f32; 4]) -> Result<Pose, String> {
    Ok(Pose::from_parts(v(p), rotation(q)?))
}

/// The unit vector along `a`, of any finite size; Err for one that is not finite or is zero.
fn unit(a: [f32; 3], what: &str) -> Result<Vector, String> {
    let m = a.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if !(m.is_finite() && m > 0.0) {
        return Err(format!("physics: {what} must be a finite, non-zero vector"));
    }
    Ok(Vector::new(a[0] / m, a[1] / m, a[2] / m).normalize())
}

/// Every number of `a` finite and within ±`max`.
fn within(a: &[f32], max: f32, what: &str) -> Result<(), String> {
    if a.iter().all(|x| x.is_finite() && x.abs() <= max) {
        Ok(())
    } else {
        Err(format!("physics: {what} must be finite and within ±{max:e}"))
    }
}

/// `x` finite and within [lo, hi].
fn between(x: f32, lo: f32, hi: f32, what: &str) -> Result<(), String> {
    if x.is_finite() && x >= lo && x <= hi {
        Ok(())
    } else {
        Err(format!("physics: {what} must be from {lo:e} to {hi:e}"))
    }
}

/// A mass or a density: 0, or within `range`.
fn amount(x: f32, range: [f32; 2], what: &str) -> Result<(), String> {
    if x == 0.0 {
        Ok(())
    } else {
        between(x, range[0], range[1], what).map_err(|e| format!("{e} (or 0)"))
    }
}

impl Shape {
    /// The shape's data as the world counts it against `MAX_SHAPE_DATA`: a hull's points, a trimesh's vertices and
    /// triangles (the others hold a few numbers; a custom shape is the engine's, not counted).
    fn data(&self) -> usize {
        match self {
            Shape::ConvexHull(p) => p.len(),
            Shape::Trimesh { vertices, indices } => vertices.len() + indices.len(),
            _ => 0,
        }
    }

    /// The Rapier shape, or why it cannot be built (a size out of bounds, no volume, bad numbers).
    fn shared(&self) -> Result<SharedShape, String> {
        let size = |x: f32, what: &str| between(x, MIN_SIZE, MAX_SIZE, what);
        match self {
            Shape::Box(h) => {
                for &x in h {
                    size(x, "a box's half extents")?;
                }
                Ok(SharedShape::cuboid(h[0], h[1], h[2]))
            }
            Shape::Sphere(r) => size(*r, "a sphere's radius").map(|_| SharedShape::ball(*r)),
            Shape::Capsule { half_height, radius } => {
                size(*radius, "a capsule's radius")?;
                between(*half_height, 0.0, MAX_SIZE, "a capsule's half height")?;
                // (A segment of (nearly) no length fails parry's assertions in a tick: that capsule is its sphere.)
                Ok(if *half_height < MIN_SIZE { SharedShape::ball(*radius) } else { SharedShape::capsule_y(*half_height, *radius) })
            }
            Shape::ConvexHull(points) => {
                if points.len() < 4 {
                    return Err("physics: a convex hull needs 4 points".into());
                }
                for p in points {
                    within(p, MAX_SIZE, "a convex hull's points")?;
                }
                let pts: Vec<Vector> = points.iter().map(|&p| v(p)).collect();
                let s = SharedShape::convex_hull(&pts).ok_or("physics: the convex hull of these points cannot be built")?;
                // (Points in a plane give a flat hull: no volume, so no mass, and a dynamic body on it hangs in the air.)
                let volume = s.mass_properties(1.0).mass();
                if !(volume >= MIN_VOLUME) {
                    return Err(format!("physics: a convex hull must hold a volume (at least {MIN_VOLUME:e} m³): these points are flat"));
                }
                Ok(s)
            }
            Shape::Trimesh { vertices, indices } => {
                if indices.is_empty() || indices.len() > MAX_MESH_TRIANGLES {
                    return Err(format!("physics: a trimesh needs from 1 to {MAX_MESH_TRIANGLES} triangles"));
                }
                for p in vertices {
                    within(p, MAX_COORD, "a trimesh's vertices")?;
                }
                let n = vertices.len();
                if indices.iter().any(|t| t.iter().any(|&i| i as usize >= n)) {
                    return Err("physics: a trimesh's index is past its vertices".into());
                }
                SharedShape::trimesh(vertices.iter().map(|&p| v(p)).collect(), indices.clone()).map_err(|e| format!("physics: the trimesh cannot be built ({e})"))
            }
            Shape::Voxels { size, cells } => {
                between(*size, VOXEL_SIZE[0], VOXEL_SIZE[1], "a voxel shape's cell size")?;
                if cells.is_empty() || cells.len() > MAX_VOXELS {
                    return Err(format!("physics: a voxel shape needs from 1 to {MAX_VOXELS} cells"));
                }
                voxel_cells(*size, cells)?;
                let cells: Vec<IVector> = cells.iter().map(|c| IVector::new(c[0], c[1], c[2])).collect();
                Ok(SharedShape::voxels(Vector::splat(*size), &cells))
            }
            Shape::Custom(c) => Ok(c.shared()),
        }
    }
}

/// A voxel shape's cells are within `MAX_VOXEL_COORD` of its origin, and no further than `MAX_COORD` metres.
fn voxel_cells(size: f32, cells: &[[i32; 3]]) -> Result<(), String> {
    let most = (MAX_VOXEL_COORD as f32).min(MAX_COORD / size) as i32;
    if cells.iter().all(|c| c.iter().all(|&x| x >= -most && x < most)) {
        Ok(())
    } else {
        Err(format!("physics: a voxel shape's cells must be within ±{most} of its origin"))
    }
}

/// A ray against a voxel shape placed at `at`, the cells walked one by one (Amanatides and Woo): the distance to the
/// first solid cell within `max` and the normal of the face it is entered by (a ray that starts in a solid cell: 0,
/// and against the ray). Parry's own (ray_voxels.rs) tests each cell's box with the ray and takes the next cell by
/// the box's bounds, both in floating point: **a ray that lies on a plane of the lattice and runs along it misses
/// every cell** (straight down at x = 1.5 onto cells of 0.1 m, which is where a game asks for the ground). Here a
/// point belongs to the cell its coordinates floor to, the cell is stepped as an integer, and a cell that is solid
/// is a hit.
fn voxel_ray(v: &rapier3d::parry::shape::Voxels, at: &Pose, ray: &Ray, max: f32) -> Option<(f32, Vector)> {
    let size = v.voxel_size().x;
    let (o, d) = (at.inverse_transform_point(ray.origin).to_array(), at.inverse_transform_vector(ray.dir).to_array());
    let [lo, hi] = v.domain();
    let (lo, hi) = (lo.to_array(), hi.to_array());
    // The stretch of the ray inside the box of the shape's cells.
    let (mut t0, mut t1, mut entered) = (0.0f32, max, 3usize);
    for a in 0..3 {
        let (min, max) = (lo[a] as f32 * size, hi[a] as f32 * size);
        if d[a] == 0.0 {
            if o[a] < min || o[a] >= max {
                return None;
            }
            continue;
        }
        let (ta, tb) = ((min - o[a]) / d[a], (max - o[a]) / d[a]);
        let (near, far) = (ta.min(tb), ta.max(tb));
        if near > t0 {
            (t0, entered) = (near, a);
        }
        t1 = t1.min(far);
    }
    if !(t0 <= t1) {
        return None;
    }
    // The cell the ray is in there, held inside the box (it enters on a face, to rounding).
    let mut cell = [0i32; 3];
    for a in 0..3 {
        cell[a] = (((o[a] + d[a] * t0) / size).floor() as i32).clamp(lo[a], hi[a] - 1);
    }
    let normal = |axis: usize| {
        let mut n = [0.0f32; 3];
        if axis < 3 {
            n[axis] = if d[axis] > 0.0 { -1.0 } else { 1.0 };
            at.transform_vector(Vector::new(n[0], n[1], n[2]))
        } else {
            -ray.dir.normalize_or_zero()
        }
    };
    let mut t = t0;
    let mut axis = entered;
    // (At most every cell of the box's three sides: the walk leaves the box.)
    loop {
        if v.voxel_state(IVector::new(cell[0], cell[1], cell[2])).is_some_and(|s| !s.is_empty()) {
            return Some((t, normal(axis)));
        }
        // The next lattice plane on each axis; the nearest is crossed (the lower axis first when they are equal).
        let mut next = (f32::INFINITY, 3usize);
        for a in 0..3 {
            if d[a] != 0.0 {
                let plane = (cell[a] + (d[a] > 0.0) as i32) as f32 * size;
                let ta = ((plane - o[a]) / d[a]).max(t);
                if ta < next.0 {
                    next = (ta, a);
                }
            }
        }
        if next.1 == 3 || next.0 > t1 {
            return None;
        }
        (t, axis) = next;
        cell[axis] += if d[axis] > 0.0 { 1 } else { -1 };
        if cell[axis] < lo[axis] || cell[axis] >= hi[axis] {
            return None;
        }
    }
}

/// A ray against a collider's shape placed at `at` (the collider's pose, or where a fresh one is): the distance and
/// the normal. Parry's answer, but for a voxel shape (`voxel_ray`).
fn shape_ray(c: &Collider, at: &Pose, ray: &Ray, max: f32) -> Option<(f32, Vector)> {
    match c.shape().as_voxels() {
        Some(v) => voxel_ray(v, at, ray, max),
        None => c.shape().cast_ray_and_get_normal(at, ray, max, true).map(|i| (i.time_of_impact, i.normal)),
    }
}

/// A number's bits as the hash takes them: one NaN for all (the platforms make NaNs of different bits, ffc00000
/// natively and 7fc00000 in wasm32).
pub(crate) fn hash_bits(x: f32) -> u32 {
    if x.is_nan() {
        0x7fc0_0000
    } else {
        x.to_bits()
    }
}

/// The physics world. Ids are the world's own, never reused; removed things are simply gone.
pub struct Physics {
    world: PhysicsWorld,
    events: Events,
    bodies: BTreeMap<BodyId, RigidBodyHandle>,
    /// Each collider with its shape's data (`Shape::data`), and their sum.
    colliders: BTreeMap<ColliderId, (ColliderHandle, usize)>,
    shape_data: usize,
    /// The voxel colliders there are: rays are cast at them here (`voxel_ray`), not by the broad phase's query.
    voxel_colliders: std::collections::BTreeSet<ColliderId>,
    joints: BTreeMap<JointId, ImpulseJointHandle>,
    /// The kinematic bodies (`awake`: the islands hold dynamic ones only).
    kinematic: std::collections::BTreeSet<BodyId>,
    next: u32,
    ticks: u64,
    contacts: std::collections::VecDeque<Contact>,
    contacts_dropped: u64,
    /// What was taken out of the world after a tick, and why (`take_faults`).
    faults: Vec<Fault>,
    /// Colliders the broad phase does not have right until the next step: those added since the last one (it does not
    /// have them at all) and those of bodies moved by `set_pose` (it has them where they were). Queries leave them
    /// out of the broad phase's answer and test them one by one where they are, so a query sees the world as the
    /// calls left it, with or without a tick since.
    fresh: std::collections::BTreeSet<ColliderId>,
}

impl Default for Physics {
    fn default() -> Self {
        Physics::new([0.0, -9.81, 0.0])
    }
}

impl Physics {
    pub fn new(gravity: [f32; 3]) -> Physics {
        let mut world = PhysicsWorld::new();
        world.gravity = v(gravity);
        world.integration_parameters.dt = TICK;
        Physics {
            world,
            events: Events::default(),
            bodies: BTreeMap::new(),
            colliders: BTreeMap::new(),
            shape_data: 0,
            voxel_colliders: Default::default(),
            joints: BTreeMap::new(),
            kinematic: Default::default(),
            next: 0,
            ticks: 0,
            contacts: Default::default(),
            contacts_dropped: 0,
            faults: vec![],
            fresh: Default::default(),
        }
    }

    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    pub fn set_gravity(&mut self, gravity: [f32; 3]) -> Result<(), String> {
        within(&gravity, MAX_GRAVITY, "gravity")?;
        self.world.gravity = v(gravity);
        Ok(())
    }

    /// Ticks stepped so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn bodies(&self) -> usize {
        self.bodies.len()
    }

    /// Adds a body; Err (why) if a number is out of its bounds (`limits`).
    pub fn add_body(&mut self, d: &BodyDesc) -> Result<BodyId, String> {
        within(&d.position, MAX_COORD, "a body's position")?;
        within(&d.linvel, MAX_LINVEL, "a body's linear velocity")?;
        within(&d.angvel, MAX_ANGVEL, "a body's angular velocity")?;
        between(d.linear_damping, 0.0, MAX_DAMPING, "linear damping")?;
        between(d.angular_damping, 0.0, MAX_DAMPING, "angular damping")?;
        within(&[d.gravity_scale], MAX_GRAVITY_SCALE, "the gravity scale")?;
        let b = match d.kind {
            BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            BodyKind::Fixed => RigidBodyBuilder::fixed(),
            BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        }
        .pose(pose(d.position, d.rotation)?)
        .linvel(v(d.linvel))
        .angvel(v(d.angvel))
        .linear_damping(d.linear_damping)
        .angular_damping(d.angular_damping)
        .gravity_scale(d.gravity_scale)
        .can_sleep(d.can_sleep)
        .ccd_enabled(d.ccd);
        // (Its id rides in Rapier's user data: from a handle back to the id.)
        let id = self.id();
        let h = self.world.insert_body(b.user_data(id as u128));
        self.bodies.insert(id, h);
        if d.kind == BodyKind::Kinematic {
            self.kinematic.insert(id);
        }
        Ok(id)
    }

    /// Removes a body with its colliders and joints. Returns the colliders that went with it.
    pub fn remove_body(&mut self, id: BodyId) -> Vec<ColliderId> {
        let Some(h) = self.bodies.remove(&id) else { return vec![] };
        self.kinematic.remove(&id);
        // (Its own colliders and joints only: removing a body costs what it holds, not what the world holds.)
        let gone: Vec<ColliderId> = self.world.bodies.get(h).map(|b| b.colliders().iter().filter_map(|&c| self.world.colliders.get(c).map(|c| c.user_data as u32)).collect()).unwrap_or_default();
        let jointed = self.world.impulse_joints.attached_joints(h).next().is_some();
        self.world.remove_body_with_colliders(h, true);
        for c in &gone {
            self.fresh.remove(c);
            self.voxel_colliders.remove(c);
            if let Some((_, data)) = self.colliders.remove(c) {
                self.shape_data -= data;
            }
        }
        if jointed {
            let world = &self.world;
            self.joints.retain(|_, j| world.impulse_joints.get(*j).is_some());
        }
        gone
    }

    /// Adds a collider to `body` (None: a static collider in the world). Err (why) if the body is unknown, the shape
    /// cannot be built, a number is out of its bounds, or the world's shape data would pass `MAX_SHAPE_DATA`.
    pub fn add_collider(&mut self, body: Option<BodyId>, d: &ColliderDesc) -> Result<ColliderId, String> {
        let parent = match body {
            Some(b) => Some(*self.bodies.get(&b).ok_or("physics: unknown body")?),
            None => None,
        };
        // (On a body its offset moves the centre of mass: a size; in the world it is a place.)
        within(&d.position, if parent.is_some() { MAX_SIZE } else { MAX_COORD }, "a collider's position")?;
        amount(d.density, DENSITY, "a collider's density")?;
        if let Some(m) = d.mass {
            amount(m, MASS, "a collider's mass")?;
        }
        between(d.friction, 0.0, MAX_FRICTION, "friction")?;
        between(d.restitution, 0.0, 1.0, "restitution")?;
        if matches!(d.shape, Shape::Trimesh { .. }) && parent.is_some_and(|p| !self.world.bodies[p].is_fixed()) {
            return Err("physics: a trimesh is for fixed bodies and static colliders only (it has no volume to give a moving body a mass)".into());
        }
        let data = d.shape.data();
        if self.shape_data + data > MAX_SHAPE_DATA {
            return Err(format!("physics: the world's colliders would hold more than {MAX_SHAPE_DATA} points and triangles"));
        }
        let shape = d.shape.shared()?;
        if matches!(d.shape, Shape::Voxels { .. }) {
            // A world with a voxel shape looks at every pair's contacts every tick. Rapier 0.36 keeps the contacts of
            // a pair that has hardly moved (`contact_recycling`), and with pairs of many manifolds (a voxel shape's)
            // its solver then came to a manifold the pair no longer had and panicked ("stale ContactRef manifold
            // ordinal"): 4 of 300 random voxel worlds with it, none of 20,000 without (`check::case` 4,
            // `voxel_worlds_never_panic`). A world without voxel shapes is stepped as it was.
            self.world.integration_parameters.contact_recycling = false;
        }
        let mut c = ColliderBuilder::new(shape).position(pose(d.position, d.rotation)?).density(d.density).friction(d.friction).restitution(d.restitution).sensor(d.sensor);
        if let Some(m) = d.mass {
            c = c.mass(m);
        }
        if d.events {
            c = c.active_events(ActiveEvents::COLLISION_EVENTS);
        }
        let id = self.id();
        let h = self.world.insert_collider(c.user_data(id as u128), parent);
        self.colliders.insert(id, (h, data));
        self.shape_data += data;
        self.fresh.insert(id);
        if matches!(d.shape, Shape::Voxels { .. }) {
            self.voxel_colliders.insert(id);
        }
        Ok(id)
    }

    pub fn remove_collider(&mut self, id: ColliderId) {
        if let Some((h, data)) = self.colliders.remove(&id) {
            self.world.remove_collider(h);
            self.shape_data -= data;
            self.fresh.remove(&id);
            self.voxel_colliders.remove(&id);
        }
    }

    /// Joins two bodies; None if one is unknown or the joint's numbers are bad. Anchors are in each body's space, an
    /// axis in the first body's. The bodies are joined as they are turned now: a fixed joint keeps their turn against
    /// each other, and a hinge's or a slider's angle or travel is 0 as they are (its limits count from there).
    pub fn add_joint(&mut self, a: BodyId, b: BodyId, d: &JointDesc) -> Result<JointId, String> {
        let (ha, hb) = (*self.bodies.get(&a).ok_or("physics: unknown body")?, *self.bodies.get(&b).ok_or("physics: unknown body")?);
        // (Rapier's joint graph cannot take a joint from a body to itself: removing that body panics a later tick.)
        if a == b {
            return Err("physics: a joint joins two different bodies".into());
        }
        // (A disabled body's joint panics the next tick; the world never keeps one, this is the guard.)
        if !(self.world.bodies[ha].is_enabled() && self.world.bodies[hb].is_enabled()) {
            return Err("physics: a joint's body is disabled".into());
        }
        let (anchor1, anchor2) = match d {
            JointDesc::Fixed { anchor1, anchor2 } | JointDesc::Revolute { anchor1, anchor2, .. } | JointDesc::Prismatic { anchor1, anchor2, .. } | JointDesc::Spherical { anchor1, anchor2 } | JointDesc::Rope { anchor1, anchor2, .. } => (*anchor1, *anchor2),
        };
        within(&anchor1, MAX_COORD, "a joint's anchor")?;
        within(&anchor2, MAX_COORD, "a joint's anchor")?;
        let limits = |l: &Option<Limits>| -> Result<Option<[f32; 2]>, String> {
            match l {
                None => Ok(None),
                Some(l) => {
                    within(&[l.min, l.max], MAX_COORD, "a joint's limits")?;
                    if l.min > l.max {
                        return Err("physics: a joint's limits must be [min, max] with min <= max".into());
                    }
                    Ok(Some([l.min, l.max]))
                }
            }
        };
        let motor = |m: &Option<Motor>| -> Result<Option<Motor>, String> {
            match m {
                None => Ok(None),
                Some(m) => {
                    within(&[m.velocity], MAX_MOTOR_VELOCITY, "a motor's velocity")?;
                    between(m.damping, 0.0, MAX_MOTOR_DAMPING, "a motor's damping")?;
                    between(m.max_force, 0.0, f32::MAX, "a motor's maximum force")?;
                    Ok(Some(*m))
                }
            }
        };
        let (a1, a2) = (v(anchor1), v(anchor2));
        // The centre of a hinge's limits, rad (0: none, or limits about 0).
        let mut centre = 0.0f32;
        let mut joint: GenericJoint = match d {
            JointDesc::Fixed { .. } => FixedJointBuilder::new().local_anchor1(a1).local_anchor2(a2).build().into(),
            JointDesc::Revolute { axis, limits: l, motor: m, .. } => {
                let mut j = RevoluteJointBuilder::new(unit(*axis, "a joint's axis")?).local_anchor1(a1).local_anchor2(a2);
                // Limits go to Rapier centred, [-h, h], with their centre turned into the first body's joint frame
                // below. Rapier takes the sine and cosine of a quarter of (min + max) with f32's own methods, the
                // platform's libm wherever `std` is in the crate graph (0.024 % of angles then differ in the last bit
                // between macOS and wasm32); of 0 they are exact everywhere, and the centre's are libm's here.
                if let Some([min, max]) = limits(l)? {
                    let half = (max - min) * 0.5;
                    centre = (min + max) * 0.5;
                    j = j.limits([-half, half]);
                }
                if let Some(m) = motor(m)? {
                    j = j.motor_velocity(m.velocity, m.damping).motor_max_force(m.max_force);
                }
                j.build().into()
            }
            JointDesc::Prismatic { axis, limits: l, motor: m, .. } => {
                let mut j = PrismaticJointBuilder::new(unit(*axis, "a joint's axis")?).local_anchor1(a1).local_anchor2(a2);
                if let Some(l) = limits(l)? {
                    j = j.limits(l);
                }
                if let Some(m) = motor(m)? {
                    j = j.motor_velocity(m.velocity, m.damping).motor_max_force(m.max_force);
                }
                j.build().into()
            }
            JointDesc::Spherical { .. } => SphericalJointBuilder::new().local_anchor1(a1).local_anchor2(a2).build().into(),
            JointDesc::Rope { length, .. } => {
                between(*length, 0.0, MAX_COORD, "a rope's length")?;
                RopeJointBuilder::new(*length).local_anchor1(a1).local_anchor2(a2).build().into()
            }
        };
        // The joint's frame in the second body is where the first body's is now (the builders give both bodies the
        // same frame, which is right only for bodies turned alike: others would be wrenched into line).
        if matches!(d, JointDesc::Fixed { .. } | JointDesc::Revolute { .. } | JointDesc::Prismatic { .. }) {
            let (ra, rb) = (self.world.bodies[ha].position().rotation, self.world.bodies[hb].position().rotation);
            joint.local_frame2.rotation = (rb.inverse() * ra * joint.local_frame1.rotation).normalize();
        }
        // A hinge's limits centred: the first body's frame turned by the centre about the hinge (its x axis), so the
        // angle Rapier reads is the hinge's less the centre, -centre as the bodies are now, and [-h, h] is [min, max].
        if centre != 0.0 {
            let (s, c) = libm::sincosf(centre * 0.5);
            joint.local_frame1.rotation = (joint.local_frame1.rotation * Rotation::from_xyzw(s, 0.0, 0.0, c)).normalize();
        }
        let h = self.world.insert_impulse_joint(ha, hb, joint);
        let id = self.id();
        self.joints.insert(id, h);
        Ok(id)
    }

    pub fn remove_joint(&mut self, id: JointId) {
        if let Some(h) = self.joints.remove(&id) {
            self.world.remove_impulse_joint(h);
        }
    }

    pub fn has_collider(&self, id: ColliderId) -> bool {
        self.colliders.contains_key(&id)
    }

    /// Changes a voxel collider's cells: `clear` emptied, then `fill` made solid (cells already so are left). What it
    /// costs is by the cells named, not by the shape's size (a shape other colliders share is copied first, once). A
    /// moving body's mass, centre of mass and inertia follow its cells; every body touching the collider is woken
    /// (what rested on a cell that is gone must fall). Returns the solid cells it has now. **With none left the
    /// collider is gone** (an empty shape meets nothing, and a moving body of no mass cannot be stepped: the caller
    /// removes a body left without colliders). Err: no such collider, not a voxel shape, a cell out of bounds;
    /// nothing is changed then.
    ///
    /// The collider is taken out of Rapier and put back with its cells changed, under the same id. Changing the
    /// shape where it sits (`Collider::shape_mut`) is not safe in Rapier 0.36: its pairs keep contacts found on the
    /// old cells (a pair that has not moved is not looked at again), and the solver's graph keeps entries of
    /// manifolds the pair no longer has: 4 of 300 random voxel worlds (`check::case` 4) panicked in a later tick
    /// ("stale ContactRef manifold ordinal"). Put back, it meets everything anew.
    pub fn edit_voxels(&mut self, id: ColliderId, clear: &[[i32; 3]], fill: &[[i32; 3]]) -> Result<usize, String> {
        let &(h, data) = self.colliders.get(&id).ok_or("physics: unknown collider")?;
        let size = match self.world.colliders[h].shape().as_voxels() {
            Some(v) => v.voxel_size().x,
            None => return Err("physics: not a voxel collider".into()),
        };
        voxel_cells(size, clear)?;
        voxel_cells(size, fill)?;
        // What touches it now, before it goes.
        let touching: Vec<RigidBodyHandle> = self
            .world
            .narrow_phase
            .contact_pairs_with(h)
            .filter_map(|p| self.world.colliders.get(if p.collider1 == h { p.collider2 } else { p.collider1 }).and_then(|c| c.parent()))
            .collect();
        let parent = self.world.colliders[h].parent();
        let mut c = self.world.remove_collider(h).expect("the collider is there");
        let voxels = c.shape_mut().as_voxels_mut().expect("a voxel shape");
        for k in clear {
            voxels.set_voxel(IVector::new(k[0], k[1], k[2]), false);
        }
        for k in fill {
            voxels.set_voxel(IVector::new(k[0], k[1], k[2]), true);
        }
        let left = voxels.voxels().filter(|v| !v.state.is_empty()).count();
        for b in touching.into_iter().chain(parent) {
            if let Some(b) = self.world.bodies.get_mut(b) {
                b.wake_up(true);
            }
        }
        if left == 0 || left > MAX_VOXELS {
            self.colliders.remove(&id);
            self.shape_data -= data;
            self.fresh.remove(&id);
            self.voxel_colliders.remove(&id);
            return if left == 0 { Ok(0) } else { Err(format!("physics: a voxel shape holds at most {MAX_VOXELS} cells (the collider is gone)")) };
        }
        let h = self.world.insert_collider(c, parent);
        if let Some(p) = parent {
            // (Its mass now, not at the next step: a collider put on a body adds its mass to what the body had, and
            // the one taken out is taken off only when the world is stepped.)
            let world = &mut self.world;
            world.bodies[p].recompute_mass_properties_from_colliders(&world.colliders);
        }
        self.colliders.insert(id, (h, data));
        // (The broad phase does not have it until the next step: queries test it where and as it is.)
        self.fresh.insert(id);
        Ok(left)
    }

    /// The memory the world's voxel shapes hold, bytes (parry's own count of each: cells, chunk tables, their tree).
    pub fn voxel_bytes(&self) -> usize {
        self.voxel_colliders.iter().filter_map(|id| self.world.colliders.get(self.colliders.get(id)?.0)?.shape().as_voxels().map(|v| v.total_memory_size())).sum()
    }

    /// A voxel collider's solid cells, counted; None if it is none.
    pub fn voxels(&self, id: ColliderId) -> Option<usize> {
        let &(h, _) = self.colliders.get(&id)?;
        self.world.colliders.get(h)?.shape().as_voxels().map(|v| v.voxels().filter(|v| !v.state.is_empty()).count())
    }

    pub fn has_joint(&self, id: JointId) -> bool {
        self.joints.contains_key(&id)
    }

    /// One tick of `TICK` seconds. Contacts that began or ended are kept for `take_contacts`. A body the tick left in
    /// a state the world cannot keep is then taken out with its colliders and joints (`take_faults` says which and
    /// why): one whose pose or velocity went non-finite (Rapier disables it and leaves it frozen, and a joint on it
    /// would panic a later tick), one whose collider did, and one past `WORLD_EDGE`.
    pub fn step(&mut self) {
        self.world.step_with_events(&(), &self.events);
        self.ticks += 1;
        // (The broad phase has every collider where it is now.)
        self.fresh.clear();
        for e in self.events.0.lock().unwrap().drain(..) {
            // (A removed collider's contact ends without it: its id is no longer known.)
            let id = |h: ColliderHandle| self.world.colliders.get(h).map(|c| c.user_data as u32);
            if let (Some(a), Some(b)) = (id(e.collider1()), id(e.collider2())) {
                if self.contacts.len() == MAX_CONTACTS {
                    self.contacts.pop_front();
                    self.contacts_dropped += 1;
                }
                self.contacts.push_back(Contact { a: a.min(b), b: a.max(b), started: e.started() });
            }
        }
        let q = self.world.quarantine();
        let mut out: Vec<(RigidBodyHandle, bool, &'static str)> = q.bodies().iter().map(|&h| (h, false, "its pose or velocity went non-finite")).collect();
        let mut statics = vec![];
        for &c in q.colliders() {
            match self.world.colliders.get(c).and_then(|c| c.parent()) {
                Some(p) => out.push((p, false, "its collider's shape went non-finite")),
                None => statics.push(c),
            }
        }
        out.extend(self.world.active_bodies().filter(|(_, b)| b.is_dynamic() && !(b.translation().abs().max_element() <= WORLD_EDGE)).map(|(h, _)| (h, true, "it left the world (past 1e6 m from the origin)")));
        for c in statics {
            if let Some(id) = self.world.colliders.get(c).map(|c| c.user_data as u32) {
                self.remove_collider(id);
                self.faults.push(Fault { body: None, colliders: vec![id], left_world: false, reason: "its shape went non-finite" });
            }
        }
        for (h, left_world, reason) in out {
            let Some(id) = self.world.bodies.get(h).map(|b| b.user_data as u32) else { continue };
            if self.bodies.contains_key(&id) {
                let colliders = self.remove_body(id);
                self.faults.push(Fault { body: Some(id), colliders, left_world, reason });
            }
        }
    }

    /// The contacts begun and ended since the last call (colliders with `events`), in the order they happened (the
    /// newest `MAX_CONTACTS` of them).
    pub fn take_contacts(&mut self) -> Vec<Contact> {
        std::mem::take(&mut self.contacts).into()
    }

    /// Contacts dropped so far because nobody took them (`MAX_CONTACTS`).
    pub fn contacts_dropped(&self) -> u64 {
        self.contacts_dropped
    }

    /// What the ticks since the last call took out of the world, and why (`step`).
    pub fn take_faults(&mut self) -> Vec<Fault> {
        std::mem::take(&mut self.faults)
    }

    fn body(&self, id: BodyId) -> Option<&RigidBody> {
        self.world.bodies.get(*self.bodies.get(&id)?)
    }

    fn body_mut(&mut self, id: BodyId) -> Option<&mut RigidBody> {
        let h = *self.bodies.get(&id)?;
        self.world.bodies.get_mut(h)
    }

    /// A body's position and rotation (quaternion xyzw).
    pub fn pose(&self, id: BodyId) -> Option<([f32; 3], [f32; 4])> {
        let p = self.body(id)?.position();
        Some((p.translation.to_array(), p.rotation.to_array()))
    }

    /// A body's linear and angular velocity.
    pub fn velocity(&self, id: BodyId) -> Option<([f32; 3], [f32; 3])> {
        let b = self.body(id)?;
        Some((b.linvel().to_array(), b.angvel().to_array()))
    }

    pub fn sleeping(&self, id: BodyId) -> bool {
        self.body(id).is_some_and(|b| b.is_sleeping())
    }

    pub fn is_kinematic(&self, id: BodyId) -> bool {
        self.kinematic.contains(&id)
    }

    pub fn mass(&self, id: BodyId) -> Option<f32> {
        Some(self.body(id)?.mass())
    }

    /// Bodies that moved in the last step (awake dynamic bodies and kinematic ones), with their poses, in id order.
    pub fn moved(&self) -> Vec<(BodyId, [f32; 3], [f32; 4])> {
        self.bodies
            .iter()
            .filter_map(|(&id, &h)| {
                let b = self.world.bodies.get(h)?;
                (!b.is_fixed() && !b.is_sleeping()).then(|| (id, b.position().translation.to_array(), b.position().rotation.to_array()))
            })
            .collect()
    }

    /// The ids of the bodies that are awake (dynamic ones in an active island, kinematic ones that are not asleep),
    /// added to `out` in no order: what the last step moved, at the cost of the awake bodies only.
    pub fn awake(&self, out: &mut Vec<BodyId>) {
        out.extend(self.world.active_bodies().filter(|(_, b)| b.is_dynamic()).map(|(_, b)| b.user_data as u32));
        for &id in &self.kinematic {
            if self.body(id).is_some_and(|b| !b.is_sleeping()) {
                out.push(id);
            }
        }
    }

    /// Moves a body there at once (a teleport; its velocity kept).
    pub fn set_pose(&mut self, id: BodyId, position: [f32; 3], rotation: [f32; 4]) -> Result<(), String> {
        within(&position, MAX_COORD, "a body's position")?;
        let p = pose(position, rotation)?;
        let Some(&h) = self.bodies.get(&id) else { return Ok(()) };
        let b = &mut self.world.bodies[h];
        b.set_position(p, true);
        // (Until the next step the broad phase has its colliders where they were: `fresh`.)
        let moved: Vec<ColliderId> = b.colliders().iter().filter_map(|&c| self.world.colliders.get(c)).map(|c| c.user_data as u32).collect();
        self.fresh.extend(moved);
        Ok(())
    }

    /// Sets either velocity or both (None keeps that one: a body may have reached more than a call may set).
    pub fn set_velocity(&mut self, id: BodyId, linvel: Option<[f32; 3]>, angvel: Option<[f32; 3]>) -> Result<(), String> {
        if let Some(l) = linvel {
            within(&l, MAX_LINVEL, "a linear velocity")?;
        }
        if let Some(a) = angvel {
            within(&a, MAX_ANGVEL, "an angular velocity")?;
        }
        if let Some(b) = self.body_mut(id) {
            if let Some(l) = linvel {
                b.set_linvel(v(l), true);
            }
            if let Some(a) = angvel {
                b.set_angvel(v(a), true);
            }
        }
        Ok(())
    }

    /// An impulse (N s) at the body's centre of mass, and an angular one (N m s).
    pub fn apply_impulse(&mut self, id: BodyId, impulse: [f32; 3], torque: [f32; 3]) -> Result<(), String> {
        within(&impulse, MAX_IMPULSE, "an impulse")?;
        within(&torque, MAX_IMPULSE, "an angular impulse")?;
        if let Some(b) = self.body_mut(id) {
            b.apply_impulse(v(impulse), true);
            b.apply_torque_impulse(v(torque), true);
        }
        Ok(())
    }

    /// An impulse (N s) at a point of the world (it turns the body as well, unless the point is its centre of mass), and
    /// an angular one (N m s). All of it is checked before any of it acts: a call that is refused changes nothing.
    pub fn apply_impulse_at(&mut self, id: BodyId, impulse: [f32; 3], point: [f32; 3], torque: [f32; 3]) -> Result<(), String> {
        within(&impulse, MAX_IMPULSE, "an impulse")?;
        within(&point, MAX_COORD, "an impulse's point")?;
        within(&torque, MAX_IMPULSE, "an angular impulse")?;
        if let Some(b) = self.body_mut(id) {
            b.apply_impulse_at_point(v(impulse), v(point), true);
            b.apply_torque_impulse(v(torque), true);
        }
        Ok(())
    }

    /// A force (N) at the centre of mass and a torque (N m), added to those the body has: they act in every step until
    /// `clear_forces`.
    pub fn add_force(&mut self, id: BodyId, force: [f32; 3], torque: [f32; 3]) -> Result<(), String> {
        within(&force, MAX_FORCE, "a force")?;
        within(&torque, MAX_FORCE, "a torque")?;
        if let Some(b) = self.body_mut(id) {
            b.add_force(v(force), true);
            b.add_torque(v(torque), true);
        }
        Ok(())
    }

    pub fn clear_forces(&mut self, id: BodyId) {
        if let Some(b) = self.body_mut(id) {
            b.reset_forces(false);
            b.reset_torques(false);
        }
    }

    /// Where a kinematic body is at the end of the next step (it moves there, pushing what is in the way).
    pub fn set_kinematic_target(&mut self, id: BodyId, position: [f32; 3], rotation: [f32; 4]) -> Result<(), String> {
        within(&position, MAX_COORD, "a kinematic body's target")?;
        let p = pose(position, rotation)?;
        if let Some(b) = self.body_mut(id).filter(|b| b.is_kinematic()) {
            b.set_next_kinematic_position(p);
        }
        Ok(())
    }

    fn hit(&self, h: ColliderHandle, distance: f32, point: Vector, normal: Vector) -> Option<Hit> {
        let c = self.world.colliders.get(h)?;
        let collider = c.user_data as u32;
        let body = c.parent().and_then(|p| self.world.bodies.get(p)).map(|b| b.user_data as u32);
        Some(Hit { collider, body, distance, point: point.to_array(), normal: normal.to_array() })
    }

    /// The first collider a ray meets within `max` metres, in the world as the calls left it (`fresh`). `skip`: a body
    /// whose colliders the ray passes through (the one that casts it).
    pub fn cast_ray(&self, origin: [f32; 3], direction: [f32; 3], max: f32, skip: Option<BodyId>) -> Result<Option<Hit>, String> {
        let Some((ray, max)) = Self::ray(origin, direction, max)? else { return Ok(None) };
        let settled = |_: ColliderHandle, c: &Collider| !self.fresh.contains(&(c.user_data as u32)) && c.shape().as_voxels().is_none();
        let mut best = self.world.cast_ray_and_get_normal(&ray, max, true, self.filter(skip, &settled)).and_then(|(h, i)| self.hit(h, i.time_of_impact, ray.point_at(i.time_of_impact), i.normal));
        for (h, c, at) in self.fresh(skip).chain(self.voxels_on(&ray, max, skip)) {
            if let Some((t, n)) = shape_ray(c, &at, &ray, max) {
                best = Self::nearer(best, self.hit(h, t, ray.point_at(t), n));
            }
        }
        Ok(best)
    }

    /// Every collider a ray passes through within `max` metres, nearest first, at most `limit` of them.
    pub fn cast_ray_all(&self, origin: [f32; 3], direction: [f32; 3], max: f32, skip: Option<BodyId>, limit: usize) -> Result<Vec<Hit>, String> {
        let Some((ray, max)) = Self::ray(origin, direction, max)? else { return Ok(vec![]) };
        let settled = |_: ColliderHandle, c: &Collider| !self.fresh.contains(&(c.user_data as u32)) && c.shape().as_voxels().is_none();
        let mut hits: Vec<Hit> = self.world.intersect_ray(ray, max, true, self.filter(skip, &settled)).filter_map(|(h, _, i)| self.hit(h, i.time_of_impact, ray.point_at(i.time_of_impact), i.normal)).collect();
        for (h, c, at) in self.fresh(skip).chain(self.voxels_on(&ray, max, skip)) {
            if let Some((t, n)) = shape_ray(c, &at, &ray, max) {
                hits.extend(self.hit(h, t, ray.point_at(t), n));
            }
        }
        // (Equal distances in collider order: the same answer on every machine.)
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.collider.cmp(&b.collider)));
        hits.truncate(limit);
        Ok(hits)
    }

    /// The settled voxel colliders whose boxes a ray passes within `max` (the broad phase's tree; not `skip`'s), each
    /// where it is: the rays' own test of them (`voxel_ray`). None in a world without voxel shapes.
    fn voxels_on<'a>(&'a self, ray: &'a Ray, max: f32, skip: Option<BodyId>) -> impl Iterator<Item = (ColliderHandle, &'a Collider, Pose)> + 'a {
        let skip = skip.and_then(|b| self.bodies.get(&b)).copied();
        let any = !self.voxel_colliders.is_empty();
        let tree = self.world.query_pipeline().bvh;
        tree.leaves(move |node: &rapier3d::parry::partitioning::BvhNode| any && node.aabb().intersects_local_ray(ray, max)).filter_map(move |leaf| {
            let (c, h) = self.world.colliders.get_unknown_gen(leaf)?;
            if !c.is_enabled() || c.shape().as_voxels().is_none() || self.fresh.contains(&(c.user_data as u32)) || (skip.is_some() && c.parent() == skip) {
                return None;
            }
            Some((h, c, *c.position()))
        })
    }

    /// A query's ray and reach (cut to `MAX_QUERY_DISTANCE`); None for a reach of 0 or less (it meets nothing).
    fn ray(origin: [f32; 3], direction: [f32; 3], max: f32) -> Result<Option<(Ray, f32)>, String> {
        within(&origin, MAX_COORD, "a query's origin")?;
        let d = unit(direction, "a query's direction")?;
        if max.is_nan() {
            return Err("physics: a query's max must be a number".into());
        }
        Ok((max > 0.0).then(|| (Ray::new(v(origin), d), max.min(MAX_QUERY_DISTANCE))))
    }

    /// A query's shape: a box, a sphere, a capsule, a convex hull or the engine's own (a trimesh is built anew for
    /// every call; queries do not take one).
    fn query_shape(shape: &Shape) -> Result<SharedShape, String> {
        if matches!(shape, Shape::Trimesh { .. }) {
            return Err("physics: a query's shape is a box, a sphere, a capsule or a convex hull, not a trimesh".into());
        }
        shape.shared()
    }

    /// A query's filter over the broad phase: without `skip`'s colliders, and without the fresh ones (`settled`), which
    /// the query tests itself.
    fn filter<'a>(&self, skip: Option<BodyId>, settled: &'a impl Fn(ColliderHandle, &Collider) -> bool) -> QueryFilter<'a> {
        let f = match skip.and_then(|b| self.bodies.get(&b)) {
            Some(&h) => QueryFilter::default().exclude_rigid_body(h),
            None => QueryFilter::default(),
        };
        // (Nothing fresh, as after every tick, and no voxel shape: the plain filter.)
        if self.fresh.is_empty() && self.voxel_colliders.is_empty() {
            f
        } else {
            f.predicate(settled)
        }
    }

    /// The fresh colliders a query tests itself (not `skip`'s), each where it is now: on a body, at the body's pose
    /// (its own world pose is the last step's until the next).
    fn fresh(&self, skip: Option<BodyId>) -> impl Iterator<Item = (ColliderHandle, &Collider, Pose)> {
        let skip = skip.and_then(|b| self.bodies.get(&b)).copied();
        self.fresh.iter().filter_map(move |id| {
            let h = self.colliders.get(id)?.0;
            let c = self.world.colliders.get(h)?;
            if !c.is_enabled() || (skip.is_some() && c.parent() == skip) {
                return None;
            }
            let at = match (c.parent().and_then(|p| self.world.bodies.get(p)), c.position_wrt_parent()) {
                (Some(b), Some(local)) => *b.position() * *local,
                _ => *c.position(),
            };
            Some((h, c, at))
        })
    }

    /// The nearer of two hits (equal distances: the lower collider id).
    fn nearer(a: Option<Hit>, b: Option<Hit>) -> Option<Hit> {
        match (a, b) {
            (Some(a), Some(b)) => Some(if b.distance.total_cmp(&a.distance).then(b.collider.cmp(&a.collider)).is_lt() { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    /// The colliders a shape placed at a pose overlaps, in id order.
    pub fn overlap(&self, shape: &Shape, position: [f32; 3], rotation: [f32; 4], skip: Option<BodyId>) -> Result<Vec<ColliderId>, String> {
        let s = Self::query_shape(shape)?;
        within(&position, MAX_COORD, "a query's position")?;
        let p = pose(position, rotation)?;
        let settled = |_: ColliderHandle, c: &Collider| !self.fresh.contains(&(c.user_data as u32));
        let mut out: Vec<ColliderId> = self.world.intersect_shape(p, &*s.0, self.filter(skip, &settled)).map(|(_, c)| c.user_data as u32).collect();
        for (_, c, at) in self.fresh(skip) {
            if rapier3d::parry::query::intersection_test(&p, &*s.0, &at, c.shape()).is_ok_and(|i| i.intersecting) {
                out.push(c.user_data as u32);
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    /// The first collider a shape meets when moved from a pose along `direction` by at most `max` metres.
    pub fn sweep(&self, shape: &Shape, position: [f32; 3], rotation: [f32; 4], direction: [f32; 3], max: f32, skip: Option<BodyId>) -> Result<Option<Hit>, String> {
        let s = Self::query_shape(shape)?;
        let p = pose(position, rotation)?;
        let Some((ray, max)) = Self::ray(position, direction, max)? else { return Ok(None) };
        let options = ShapeCastOptions { max_time_of_impact: max, target_distance: 0.0, stop_at_penetration: true, compute_impact_geometry_on_penetration: true };
        let settled = |_: ColliderHandle, c: &Collider| !self.fresh.contains(&(c.user_data as u32));
        let mut best = self.world.cast_shape(&p, ray.dir, &*s.0, options, self.filter(skip, &settled)).and_then(|(h, hit)| self.hit(h, hit.time_of_impact, hit.witness1, hit.normal1));
        for (h, c, at) in self.fresh(skip) {
            // (As the broad phase's answer: the collider is the first shape, and what is its own is moved to the world.)
            if let Ok(Some(hit)) = rapier3d::parry::query::cast_shapes(&at, Vector::ZERO, c.shape(), &p, ray.dir, &*s.0, options) {
                let hit = hit.transform1_by(&at);
                best = Self::nearer(best, self.hit(h, hit.time_of_impact, hit.witness1, hit.normal1));
            }
        }
        Ok(best)
    }

    /// Tests only: the body's velocity made infinite behind the world's checks (as only Rapier itself could), so that
    /// the next tick quarantines it.
    #[cfg(test)]
    pub(crate) fn break_body(&mut self, id: BodyId) {
        let h = self.bodies[&id];
        self.world.bodies[h].set_linvel(Vector::new(f32::INFINITY, 0.0, 0.0), true);
    }

    /// Tests only: the body moved to a place behind the world's checks.
    #[cfg(test)]
    pub(crate) fn put_body(&mut self, id: BodyId, at: [f32; 3]) {
        let h = self.bodies[&id];
        self.world.bodies[h].set_translation(v(at), true);
    }

    /// A hash of every body's pose and velocity, bit for bit, in id order (FNV-1a): equal worlds hash alike.
    pub fn hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut feed = |x: u32| {
            for b in x.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        for (&id, &handle) in &self.bodies {
            let Some(b) = self.world.bodies.get(handle) else { continue };
            feed(id);
            for x in b.position().translation.to_array().into_iter().chain(b.position().rotation.to_array()).chain(b.linvel().to_array()).chain(b.angvel().to_array()) {
                feed(hash_bits(x));
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: f32 = 9.81;

    /// A world with a ground whose top is the plane y = 0.
    fn ground() -> (Physics, ColliderId) {
        let mut p = Physics::default();
        let g = p.add_collider(None, &ColliderDesc { position: [0.0, -0.5, 0.0], ..ColliderDesc::new(Shape::Box([50.0, 0.5, 50.0])) }).unwrap();
        (p, g)
    }

    fn body(p: &mut Physics, position: [f32; 3], shape: Shape) -> BodyId {
        let b = p.add_body(&BodyDesc { position, ..Default::default() }).unwrap();
        p.add_collider(Some(b), &ColliderDesc::new(shape)).unwrap();
        b
    }

    fn fixed(p: &mut Physics, position: [f32; 3]) -> BodyId {
        p.add_body(&BodyDesc { kind: BodyKind::Fixed, position, ..Default::default() }).unwrap()
    }

    fn run(p: &mut Physics, ticks: u32) {
        for _ in 0..ticks {
            p.step();
        }
    }

    fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
        (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
    }

    /// Free fall follows the integrator's own law: Rapier integrates a tick in 4 substeps (velocity first, then
    /// position), so after n ticks a body has dropped g dt² (n² / 2 + n / 8), within a millimetre of it; that is within
    /// 0.5 % of g t² / 2 after a second.
    #[test]
    fn free_fall() {
        let mut p = Physics::default();
        let b = body(&mut p, [0.0, 100.0, 0.0], Shape::Sphere(0.5));
        run(&mut p, 60);
        let y = p.pose(b).unwrap().0[1];
        let expect = 100.0 - G * TICK * TICK * (60.0 * 60.0 / 2.0 + 60.0 / 8.0);
        assert!((y - expect).abs() < 1e-3, "{y} for {expect}");
        assert!((100.0 - y - G / 2.0).abs() < 0.005 * G / 2.0);
        let v = p.velocity(b).unwrap().0[1];
        assert!((v + G).abs() < 1e-3, "{v}");
        assert_eq!(p.ticks(), 60);
    }

    /// Five 1 m boxes stacked with a millimetre between them come to rest in a column: box k's centre at k + 0.5 m
    /// within 2 mm for each contact under it (Rapier lets a contact sink about a millimetre), not drifted sideways,
    /// upright, asleep.
    #[test]
    fn stacking() {
        let (mut p, _) = ground();
        let boxes: Vec<BodyId> = (0..5).map(|k| body(&mut p, [0.0, 0.5 + k as f32 * 1.001, 0.0], Shape::Box([0.5; 3]))).collect();
        run(&mut p, 600);
        for (k, &b) in boxes.iter().enumerate() {
            let (pos, rot) = p.pose(b).unwrap();
            assert!((pos[1] - (k as f32 + 0.5)).abs() < 2e-3 * (k as f32 + 1.0), "box {k} at {}", pos[1]);
            assert!(pos[0].abs() < 5e-3 && pos[2].abs() < 5e-3, "box {k} drifted to {pos:?}");
            assert!(rot[3].abs() > 0.9999, "box {k} turned: {rot:?}");
            assert!(p.sleeping(b), "box {k} still awake");
        }
        assert!(p.moved().is_empty(), "asleep bodies are not reported as moved");
        // Their masses: 1 m³ of 1000 kg/m³.
        assert!((p.mass(boxes[0]).unwrap() - 1000.0).abs() < 1.0);
    }

    /// A box on a 20° slope (tan 20° = 0.364) stays with friction 0.6 and slides with friction 0.1, at g (sin - mu cos).
    #[test]
    fn friction_on_a_slope() {
        let slide = |mu: f32| {
            let mut p = Physics::default();
            let a = 20f32.to_radians();
            let q = [0.0, 0.0, (a / 2.0).sin(), (a / 2.0).cos()]; // about z: the slope rises along +x
            p.add_collider(None, &ColliderDesc { rotation: q, friction: mu, ..ColliderDesc::new(Shape::Box([20.0, 0.5, 20.0])) }).unwrap();
            // On the slope's surface (0.5 above its middle plane, plus half the box), turned with it.
            let up = [-a.sin(), a.cos(), 0.0];
            let b = p.add_body(&BodyDesc { position: [up[0] * 0.752, up[1] * 0.752, 0.0], rotation: q, can_sleep: false, ..Default::default() }).unwrap();
            p.add_collider(Some(b), &ColliderDesc { friction: mu, ..ColliderDesc::new(Shape::Box([0.25; 3])) }).unwrap();
            let start = p.pose(b).unwrap().0;
            run(&mut p, 60);
            (dist(p.pose(b).unwrap().0, start), a)
        };
        let (held, _) = slide(0.6);
        assert!(held < 0.01, "slid {held} m with friction 0.6");
        let (slid, a) = slide(0.1);
        let expect = 0.5 * G * (a.sin() - 0.1 * a.cos());
        assert!((slid - expect).abs() < 0.1 * expect, "slid {slid} m, expected {expect}");
    }

    /// A weight on a hinge 1 m from it, let go level: the hinge holds it at 1 m (within 5 mm), it stays in the plane
    /// across the axis, and at the bottom it moves at sqrt(2 g L) = 4.43 m/s within 2 %.
    #[test]
    fn revolute_pendulum() {
        let mut p = Physics::default();
        let post = fixed(&mut p, [0.0, 5.0, 0.0]);
        let w = p.add_body(&BodyDesc { position: [1.0, 5.0, 0.0], can_sleep: false, ..Default::default() }).unwrap();
        p.add_collider(Some(w), &ColliderDesc::new(Shape::Sphere(0.05))).unwrap();
        p.add_joint(post, w, &JointDesc::Revolute { anchor1: [0.0; 3], anchor2: [-1.0, 0.0, 0.0], axis: [0.0, 0.0, 1.0], limits: None, motor: None }).unwrap();
        let (mut lowest, mut fastest) = (f32::MAX, 0.0f32);
        for _ in 0..120 {
            p.step();
            let (pos, _) = p.pose(w).unwrap();
            assert!((dist(pos, [0.0, 5.0, 0.0]) - 1.0).abs() < 5e-3, "{pos:?}");
            assert!(pos[2].abs() < 1e-3);
            let v = p.velocity(w).unwrap().0;
            lowest = lowest.min(pos[1]);
            fastest = fastest.max((v[0] * v[0] + v[1] * v[1]).sqrt());
        }
        assert!((lowest - 4.0).abs() < 0.01, "lowest {lowest}");
        let expect = (2.0 * G).sqrt();
        assert!((fastest - expect).abs() < 0.02 * expect, "fastest {fastest}, expected {expect}");
    }

    /// A door on a hinge limited to ±0.5 rad, pushed by its own weight about a level axis: it swings down to the limit
    /// and stops there (0.5 rad within 0.03).
    #[test]
    fn revolute_limits() {
        let mut p = Physics::default();
        let post = fixed(&mut p, [0.0, 5.0, 0.0]);
        let door = p.add_body(&BodyDesc { position: [0.5, 5.0, 0.0], can_sleep: false, ..Default::default() }).unwrap();
        p.add_collider(Some(door), &ColliderDesc::new(Shape::Box([0.5, 0.05, 0.4]))).unwrap();
        p.add_joint(post, door, &JointDesc::Revolute { anchor1: [0.0; 3], anchor2: [-0.5, 0.0, 0.0], axis: [0.0, 0.0, 1.0], limits: Some(Limits { min: -0.5, max: 0.5 }), motor: None }).unwrap();
        let mut most = 0.0f32;
        for _ in 0..300 {
            p.step();
            let pos = p.pose(door).unwrap().0;
            most = most.max((-(pos[1] - 5.0)).atan2(pos[0]));
        }
        assert!(most < 0.53, "swung to {most} rad");
        let pos = p.pose(door).unwrap().0;
        assert!(((-(pos[1] - 5.0)).atan2(pos[0]) - 0.5).abs() < 0.03, "rests at {pos:?}");
    }

    /// Limits that are not about 0 (the world hands Rapier centred ones and turns the centre into the joint's frame):
    /// the same door limited to [-0.9, -0.2] rad swings down to 0.9 rad and stops; limited to [0.2, 0.9] it is lifted
    /// to its lower limit, 0.2 rad up, and its weight holds it there.
    #[test]
    fn revolute_limits_off_centre() {
        for (min, max, rests) in [(-0.9, -0.2, 0.9), (0.2, 0.9, -0.2)] {
            let mut p = Physics::default();
            let post = fixed(&mut p, [0.0, 5.0, 0.0]);
            let door = p.add_body(&BodyDesc { position: [0.5, 5.0, 0.0], can_sleep: false, ..Default::default() }).unwrap();
            p.add_collider(Some(door), &ColliderDesc::new(Shape::Box([0.5, 0.05, 0.4]))).unwrap();
            let j = p.add_joint(post, door, &JointDesc::Revolute { anchor1: [0.0; 3], anchor2: [-0.5, 0.0, 0.0], axis: [0.0, 0.0, 1.0], limits: Some(Limits { min, max }), motor: None }).unwrap();
            // What Rapier has: limits about 0, so the sine and cosine it takes of their centre are exact everywhere.
            let l = p.world.impulse_joints.get(p.joints[&j]).unwrap().data.limits(JointAxis::AngX).unwrap();
            assert_eq!(l.min, -l.max);
            assert!((l.max - 0.35).abs() < 1e-6, "{l:?}");
            run(&mut p, 400);
            let pos = p.pose(door).unwrap().0;
            let down = (-(pos[1] - 5.0)).atan2(pos[0]);
            assert!((down - rests).abs() < 0.03, "limits [{min}, {max}]: rests {down} rad down, expected {rests}");
        }
    }

    /// A slider along x: gravity does not take it off its axis (within 2 mm); its motor brings it to 1.5 m/s; it stops at
    /// the limit, 2 m along.
    #[test]
    fn prismatic_motor_and_limits() {
        let mut p = Physics::default();
        let rail = fixed(&mut p, [0.0, 3.0, 0.0]);
        let s = p.add_body(&BodyDesc { position: [0.0, 3.0, 0.0], can_sleep: false, ..Default::default() }).unwrap();
        p.add_collider(Some(s), &ColliderDesc::new(Shape::Box([0.2; 3]))).unwrap();
        p.add_joint(rail, s, &JointDesc::Prismatic { anchor1: [0.0; 3], anchor2: [0.0; 3], axis: [1.0, 0.0, 0.0], limits: Some(Limits { min: -2.0, max: 2.0 }), motor: Some(Motor { velocity: 1.5, damping: 1e5, max_force: 1e5 }) }).unwrap();
        run(&mut p, 30);
        let (pos, vel) = (p.pose(s).unwrap().0, p.velocity(s).unwrap().0);
        assert!((pos[1] - 3.0).abs() < 2e-3 && pos[2].abs() < 2e-3, "{pos:?}");
        assert!((vel[0] - 1.5).abs() < 0.05, "{vel:?}");
        run(&mut p, 300);
        let pos = p.pose(s).unwrap().0;
        assert!((pos[0] - 2.0).abs() < 0.02 && (pos[1] - 3.0).abs() < 2e-3, "{pos:?}");
    }

    /// A ball joint keeps its anchor (1 m, within 5 mm) while the body swings in any direction; a rope lets its weight
    /// fall freely until it is taut and then holds it at its length (2 m within 2 cm); a fixed joint keeps two bodies'
    /// offset (within 2 mm) as they fall and tumble together.
    #[test]
    fn spherical_rope_and_fixed() {
        let mut p = Physics::default();
        let post = fixed(&mut p, [0.0, 10.0, 0.0]);
        let ball = p.add_body(&BodyDesc { position: [1.0, 10.0, 0.0], linvel: [0.0, 0.0, 2.0], can_sleep: false, ..Default::default() }).unwrap();
        p.add_collider(Some(ball), &ColliderDesc::new(Shape::Sphere(0.1))).unwrap();
        p.add_joint(post, ball, &JointDesc::Spherical { anchor1: [0.0; 3], anchor2: [-1.0, 0.0, 0.0] }).unwrap();
        let weight = p.add_body(&BodyDesc { position: [5.0, 10.0, 0.0], can_sleep: false, ..Default::default() }).unwrap();
        p.add_collider(Some(weight), &ColliderDesc::new(Shape::Sphere(0.1))).unwrap();
        p.add_joint(post, weight, &JointDesc::Rope { anchor1: [5.0, 0.0, 0.0], anchor2: [0.0; 3], length: 2.0 }).unwrap();
        let a = p.add_body(&BodyDesc { position: [-5.0, 10.0, 0.0], angvel: [1.0, 2.0, 0.5], can_sleep: false, ..Default::default() }).unwrap();
        p.add_collider(Some(a), &ColliderDesc::new(Shape::Box([0.2; 3]))).unwrap();
        let b = body(&mut p, [-4.0, 10.0, 0.0], Shape::Box([0.2; 3]));
        p.add_joint(a, b, &JointDesc::Fixed { anchor1: [0.5, 0.0, 0.0], anchor2: [-0.5, 0.0, 0.0] }).unwrap();
        let mut swung = 0.0f32;
        for t in 0..240 {
            p.step();
            let pos = p.pose(ball).unwrap().0;
            assert!((dist(pos, [0.0, 10.0, 0.0]) - 1.0).abs() < 5e-3, "ball joint at tick {t}: {pos:?}");
            swung = swung.max(pos[2].abs());
            let w = p.pose(weight).unwrap().0;
            assert!(dist(w, [5.0, 10.0, 0.0]) < 2.02, "rope at tick {t}: {w:?}");
            assert!((dist(p.pose(a).unwrap().0, p.pose(b).unwrap().0) - 1.0).abs() < 2e-3, "fixed joint at tick {t}");
        }
        assert!(swung > 0.3, "the ball joint lets it swing sideways ({swung})");
        let w = p.pose(weight).unwrap().0;
        assert!((dist(w, [5.0, 10.0, 0.0]) - 2.0).abs() < 0.02 && w[1] < 8.1, "hangs at {w:?}");
        assert!(p.pose(a).unwrap().0[1] < 0.0, "the fixed pair fell");
    }

    /// Joints take the bodies as they are turned when joined. A bar turned a quarter about x, hinged to a post about the
    /// post's y axis, is not wrenched into line: it stays as it is, and set turning it turns about that axis, its end on
    /// the hinge. Two bodies turned 0.7 rad against each other and fixed together stay so.
    #[test]
    fn joints_join_bodies_as_they_are_turned() {
        let quarter = [std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2];
        let mut p = Physics::new([0.0; 3]);
        let post = p.add_body(&BodyDesc { kind: BodyKind::Fixed, ..Default::default() }).unwrap();
        let bar = p.add_body(&BodyDesc { position: [0.5, 0.0, 0.0], rotation: quarter, ..Default::default() }).unwrap();
        p.add_collider(Some(bar), &ColliderDesc::new(Shape::Box([0.5, 0.1, 0.1]))).unwrap();
        p.add_joint(post, bar, &JointDesc::Revolute { anchor1: [0.0; 3], anchor2: [-0.5, 0.0, 0.0], axis: [0.0, 1.0, 0.0], limits: None, motor: None }).unwrap();
        let dot = |a: [f32; 4], b: [f32; 4]| (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs();
        for _ in 0..30 {
            p.step();
        }
        let (at, q) = p.pose(bar).unwrap();
        assert!(dot(q, quarter) > 0.99999 && (at[0] - 0.5).abs() < 1e-3 && at[1].abs() < 1e-3 && at[2].abs() < 1e-3, "left as it was: {at:?} {q:?}");
        // One radian a second about the post's y for a second: the bar's middle goes round the hinge.
        p.set_velocity(bar, Some([0.0, 0.0, -0.5]), Some([0.0, 1.0, 0.0])).unwrap();
        for _ in 0..60 {
            p.step();
        }
        let (at, q) = p.pose(bar).unwrap();
        assert!((at[0] - 0.5 * 1f32.cos()).abs() < 0.01 && at[1].abs() < 1e-3 && (at[2] + 0.5 * 1f32.sin()).abs() < 0.01, "{at:?}");
        // (Its turn: a radian about y after the quarter about x.)
        let (s, c) = (0.5f32.sin(), 0.5f32.cos());
        let k = std::f32::consts::FRAC_1_SQRT_2;
        assert!(dot(q, [c * k, s * k, -s * k, c * k]) > 0.9999, "{q:?}");
        // Fixed: the turn of one against the other is kept.
        let turned = [0.0, 0.0, 0.35f32.sin(), 0.35f32.cos()];
        let a = p.add_body(&BodyDesc { position: [10.0, 0.0, 0.0], ..Default::default() }).unwrap();
        let b = p.add_body(&BodyDesc { position: [11.0, 0.0, 0.0], rotation: turned, ..Default::default() }).unwrap();
        for body in [a, b] {
            p.add_collider(Some(body), &ColliderDesc::new(Shape::Sphere(0.2))).unwrap();
        }
        p.add_joint(a, b, &JointDesc::Fixed { anchor1: [1.0, 0.0, 0.0], anchor2: [0.0; 3] }).unwrap();
        for _ in 0..30 {
            p.step();
        }
        let ((pa, qa), (pb, qb)) = (p.pose(a).unwrap(), p.pose(b).unwrap());
        assert!(dot(qa, [0.0, 0.0, 0.0, 1.0]) > 0.99999 && dot(qb, turned) > 0.99999, "{qa:?} {qb:?}");
        assert!((pa[0] - 10.0).abs() < 1e-3 && (pb[0] - 11.0).abs() < 1e-3 && pb[1].abs() < 1e-3, "{pa:?} {pb:?}");
    }

    /// Contacts of colliders that ask for them: a ball landing reports one contact begun with the ground, and one ended
    /// when it is thrown up again. A sensor reports what enters it and stops nothing.
    #[test]
    fn contact_events() {
        let (mut p, g) = ground();
        let b = p.add_body(&BodyDesc { position: [0.0, 1.0, 0.0], can_sleep: false, ..Default::default() }).unwrap();
        let c = p.add_collider(Some(b), &ColliderDesc { events: true, ..ColliderDesc::new(Shape::Sphere(0.25)) }).unwrap();
        let quiet = body(&mut p, [5.0, 1.0, 0.0], Shape::Sphere(0.25));
        run(&mut p, 120);
        assert_eq!(p.take_contacts(), vec![Contact { a: g, b: c, started: true }], "the ball without `events` reports nothing");
        assert!(p.take_contacts().is_empty());
        p.set_velocity(b, Some([0.0, 5.0, 0.0]), Some([0.0; 3])).unwrap();
        run(&mut p, 10);
        assert_eq!(p.take_contacts(), vec![Contact { a: g, b: c, started: false }]);
        let _ = quiet;
        // A sensor 3 m up: the ball thrown through it enters and leaves, and is not slowed more than gravity does.
        let s = p.add_collider(None, &ColliderDesc { position: [0.0, 3.0, 0.0], sensor: true, events: true, ..ColliderDesc::new(Shape::Box([1.0, 0.2, 1.0])) }).unwrap();
        p.set_pose(b, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]).unwrap();
        p.set_velocity(b, Some([0.0, 12.0, 0.0]), Some([0.0; 3])).unwrap();
        run(&mut p, 40);
        let seen = p.take_contacts();
        assert_eq!(seen, vec![Contact { a: c, b: s, started: true }, Contact { a: c, b: s, started: false }]);
        assert!(p.pose(b).unwrap().0[1] > 3.5, "through the sensor");
    }

    /// Rays, overlaps and sweeps against a box standing on the ground.
    #[test]
    fn scene_queries() {
        let (mut p, g) = ground();
        let b = body(&mut p, [0.0, 0.5, 0.0], Shape::Box([0.5; 3]));
        run(&mut p, 2);
        let c = p.cast_ray([0.2, 10.0, 0.1], [0.0, -1.0, 0.0], 100.0, None).unwrap().unwrap();
        assert_eq!(c.body, Some(b));
        assert!((c.distance - 9.0).abs() < 1e-3 && (c.point[1] - 1.0).abs() < 1e-3, "{c:?}");
        assert!(dist(c.normal, [0.0, 1.0, 0.0]) < 1e-4);
        // Past the box: the ground (a collider without a body). Skipping the box's body: the ground under it.
        let h = p.cast_ray([3.0, 10.0, 0.0], [0.0, -2.0, 0.0], 100.0, None).unwrap().unwrap();
        assert_eq!((h.collider, h.body), (g, None));
        assert!((h.distance - 10.0).abs() < 1e-3);
        assert_eq!(p.cast_ray([0.2, 10.0, 0.1], [0.0, -1.0, 0.0], 100.0, Some(b)).unwrap().unwrap().collider, g);
        // Too short, pointing away, bad numbers: nothing.
        assert!(p.cast_ray([0.2, 10.0, 0.1], [0.0, -1.0, 0.0], 5.0, None).unwrap().is_none());
        assert!(p.cast_ray([0.2, 10.0, 0.1], [0.0, 1.0, 0.0], 100.0, None).unwrap().is_none());
        assert!(p.cast_ray([0.2, f32::NAN, 0.1], [0.0, -1.0, 0.0], 100.0, None).is_err());
        assert!(p.cast_ray([0.2, 10.0, 0.1], [0.0; 3], 100.0, None).is_err());
        // Overlap: a sphere at the box's corner touches the box and the ground; one high up nothing.
        let hits = p.overlap(&Shape::Sphere(0.3), [0.6, 0.1, 0.6], [0.0, 0.0, 0.0, 1.0], None).unwrap();
        assert_eq!(hits, vec![g, c.collider]);
        assert_eq!(p.overlap(&Shape::Sphere(0.3), [0.6, 0.1, 0.6], [0.0, 0.0, 0.0, 1.0], Some(b)).unwrap(), vec![g]);
        assert!(p.overlap(&Shape::Sphere(0.3), [0.0, 5.0, 0.0], [0.0, 0.0, 0.0, 1.0], None).unwrap().is_empty());
        // Sweep: a sphere of 0.25 m lowered from 5 m meets the box's top (y = 1) after 3.75 m.
        let s = p.sweep(&Shape::Sphere(0.25), [0.0, 5.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, -1.0, 0.0], 100.0, None).unwrap().unwrap();
        assert_eq!(s.body, Some(b));
        assert!((s.distance - 3.75).abs() < 1e-3, "{s:?}");
        // Its point and normal are the box's (as a ray's are), in world space: on the top, facing up.
        assert!(dist(s.point, [0.0, 1.0, 0.0]) < 1e-3 && dist(s.normal, [0.0, 1.0, 0.0]) < 1e-4, "{s:?}");
        // Sideways: it meets the box's side x = -0.5 after 4.25 m, at the side's point facing it, -x.
        let s = p.sweep(&Shape::Sphere(0.25), [-5.0, 0.5, 0.2], [0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 100.0, None).unwrap().unwrap();
        assert!((s.distance - 4.25).abs() < 1e-3, "{s:?}");
        assert!(dist(s.point, [-0.5, 0.5, 0.2]) < 1e-3 && dist(s.normal, [-1.0, 0.0, 0.0]) < 1e-4, "{s:?}");
        assert!(p.sweep(&Shape::Sphere(0.25), [0.0, 5.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, -1.0, 0.0], 2.0, None).unwrap().is_none());
    }

    /// A kinematic body goes where it is told and pushes a dynamic one; nothing pushes it.
    #[test]
    fn kinematic_bodies_push() {
        let (mut p, _) = ground();
        let k = p.add_body(&BodyDesc { kind: BodyKind::Kinematic, position: [-2.0, 0.5, 0.0], ..Default::default() }).unwrap();
        p.add_collider(Some(k), &ColliderDesc::new(Shape::Box([0.5; 3]))).unwrap();
        let d = body(&mut p, [0.0, 0.5, 0.0], Shape::Box([0.5; 3]));
        for t in 1..=120 {
            p.set_kinematic_target(k, [-2.0 + t as f32 * 0.02, 0.5, 0.0], [0.0, 0.0, 0.0, 1.0]).unwrap();
            p.step();
        }
        let (kp, dp) = (p.pose(k).unwrap().0, p.pose(d).unwrap().0);
        assert!((kp[0] - 0.4).abs() < 1e-3 && (kp[1] - 0.5).abs() < 1e-6, "{kp:?}");
        assert!(dp[0] > kp[0] + 0.99, "pushed ahead of it: {dp:?}");
        // (A target for a dynamic body is ignored.)
        p.set_kinematic_target(d, [9.0, 9.0, 9.0], [0.0, 0.0, 0.0, 1.0]).unwrap();
        p.step();
        assert!(p.pose(d).unwrap().0[1] < 1.0);
    }

    /// Shapes: a capsule, a convex hull and an engine-defined shape (`CustomShape`) rest on a trimesh floor at the
    /// heights their shapes give; a hull has mass.
    #[test]
    fn shapes() {
        struct Ball(f32);
        impl CustomShape for Ball {
            fn shared(&self) -> SharedShape {
                SharedShape::ball(self.0)
            }
        }
        let mut p = Physics::default();
        // A floor of two triangles at y = 0.
        p.add_collider(None, &ColliderDesc::new(Shape::Trimesh { vertices: vec![[-20.0, 0.0, -20.0], [20.0, 0.0, -20.0], [20.0, 0.0, 20.0], [-20.0, 0.0, 20.0]], indices: vec![[0, 2, 1], [0, 3, 2]] })).unwrap();
        let capsule = body(&mut p, [0.0, 1.0, 0.0], Shape::Capsule { half_height: 0.3, radius: 0.2 });
        let hull = body(&mut p, [3.0, 1.0, 0.0], Shape::ConvexHull(vec![[-0.3, -0.2, -0.3], [0.3, -0.2, -0.3], [0.3, -0.2, 0.3], [-0.3, -0.2, 0.3], [-0.3, 0.2, -0.3], [0.3, 0.2, -0.3], [0.3, 0.2, 0.3], [-0.3, 0.2, 0.3]]));
        let custom = body(&mut p, [-3.0, 1.0, 0.0], Shape::Custom(Box::new(Ball(0.35))));
        run(&mut p, 300);
        assert!((p.pose(capsule).unwrap().0[1] - 0.5).abs() < 5e-3, "{:?}", p.pose(capsule));
        assert!((p.pose(hull).unwrap().0[1] - 0.2).abs() < 5e-3, "{:?}", p.pose(hull));
        assert!((p.pose(custom).unwrap().0[1] - 0.35).abs() < 5e-3, "{:?}", p.pose(custom));
        assert!((p.mass(hull).unwrap() - 0.6 * 0.4 * 0.6 * 1000.0).abs() < 1.0);
    }

    /// Cells `lo..hi` (hi exclusive).
    fn block(lo: [i32; 3], hi: [i32; 3]) -> Vec<[i32; 3]> {
        (lo[0]..hi[0]).flat_map(|x| (lo[1]..hi[1]).flat_map(move |y| (lo[2]..hi[2]).map(move |z| [x, y, z]))).collect()
    }

    /// Voxel shapes (step 8): a block of cells rests on a ground of cells where its cells end, and sleeps; another
    /// rests on it; mass, centre of mass and a ray are the cells'.
    #[test]
    fn voxel_shapes_rest_and_stack() {
        let mut p = Physics::default();
        // A ground of cells of 0.1 m, 4 m a side, its top at y = 0.3.
        let ground = p.add_collider(None, &ColliderDesc::new(Shape::Voxels { size: 0.1, cells: block([-20, 0, -20], [20, 3, 20]) })).unwrap();
        // Blocks of 4 x 3 x 4 cells whose cells start at the body's origin: 0.4 x 0.3 x 0.4 m.
        let lump = |p: &mut Physics, at: [f32; 3]| {
            let b = p.add_body(&BodyDesc { position: at, ..Default::default() }).unwrap();
            let c = p.add_collider(Some(b), &ColliderDesc { density: 2000.0, ..ColliderDesc::new(Shape::Voxels { size: 0.1, cells: block([0, 0, 0], [4, 3, 4]) }) }).unwrap();
            (b, c)
        };
        let (a, ca) = lump(&mut p, [0.0, 0.6, 0.0]);
        let (b, _) = lump(&mut p, [0.05, 1.4, 0.05]);
        assert!((p.mass(a).unwrap() - 48.0 * 0.001 * 2000.0).abs() < 1e-2, "{:?}", p.mass(a));
        assert_eq!((p.voxels(ca), p.voxels(ground)), (Some(48), Some(40 * 40 * 3)));
        run(&mut p, 240);
        let (ya, yb) = (p.pose(a).unwrap().0[1], p.pose(b).unwrap().0[1]);
        assert!((ya - 0.3).abs() < 5e-3 && (yb - 0.6).abs() < 1e-2, "{ya}, {yb}");
        assert!(p.sleeping(a) && p.sleeping(b));
        // It stood where it fell: no drift, no turn.
        let (pa, qa) = p.pose(a).unwrap();
        assert!(pa[0].abs() < 5e-3 && pa[2].abs() < 5e-3 && qa[3] > 0.9999, "{pa:?} {qa:?}");
        // A ray down onto the upper block, and one beside both onto the ground.
        let hit = p.cast_ray([0.2, 5.0, 0.2], [0.0, -1.0, 0.0], 10.0, None).unwrap().unwrap();
        assert!(hit.body == Some(b) && (hit.point[1] - 0.9).abs() < 1e-2 && hit.normal[1] > 0.99, "{hit:?}");
        let hit = p.cast_ray([1.5, 5.0, 1.5], [0.0, -1.0, 0.0], 10.0, None).unwrap().unwrap();
        assert!(hit.collider == ground && (hit.point[1] - 0.3).abs() < 1e-3, "{hit:?}");
        // A ball swept down meets the ground's cells; a box over the blocks overlaps both.
        let hit = p.sweep(&Shape::Sphere(0.2), [-1.0, 3.0, -1.0], [0.0, 0.0, 0.0, 1.0], [0.0, -1.0, 0.0], 10.0, None).unwrap().unwrap();
        assert!(hit.collider == ground && (hit.distance - 2.5).abs() < 1e-2, "{hit:?}");
        assert_eq!(p.overlap(&Shape::Box([0.1, 1.0, 0.1]), [0.2, 0.8, 0.2], [0.0, 0.0, 0.0, 1.0], None).unwrap().len(), 3);
    }

    /// A voxel shape's cells are changed where it is: the ground carved under a sleeping block wakes it and it
    /// falls to the new floor (a query sees the pit before any tick); a block carved weighs what is left, and one
    /// carved to nothing is gone; bad edits change nothing.
    #[test]
    fn voxel_shapes_are_edited() {
        let mut p = Physics::default();
        let ground = p.add_collider(None, &ColliderDesc::new(Shape::Voxels { size: 0.1, cells: block([-20, 0, -20], [20, 6, 20]) })).unwrap();
        let b = p.add_body(&BodyDesc { position: [-0.2, 0.7, -0.2], ..Default::default() }).unwrap();
        let c = p.add_collider(Some(b), &ColliderDesc::new(Shape::Voxels { size: 0.1, cells: block([0, 0, 0], [4, 4, 4]) })).unwrap();
        run(&mut p, 180);
        assert!((p.pose(b).unwrap().0[1] - 0.6).abs() < 5e-3 && p.sleeping(b), "{:?}", p.pose(b));
        // A pit 0.8 m wide and three cells deep under it.
        let left = p.edit_voxels(ground, &block([-4, 3, -4], [4, 6, 4]), &[]).unwrap();
        assert_eq!(left, 40 * 40 * 6 - 8 * 8 * 3);
        assert!(!p.sleeping(b), "woken");
        let hit = p.cast_ray([0.3, 5.0, 0.3], [0.0, -1.0, 0.0], 10.0, Some(b)).unwrap().unwrap();
        assert!(hit.collider == ground && (hit.point[1] - 0.3).abs() < 1e-3, "the pit's floor before any tick: {hit:?}");
        run(&mut p, 180);
        assert!((p.pose(b).unwrap().0[1] - 0.3).abs() < 5e-3 && p.sleeping(b), "{:?}", p.pose(b));
        // The ground filled again under a corner of the pit: cells that are solid stay as they are.
        assert_eq!(p.edit_voxels(ground, &[], &block([-4, 3, -4], [-2, 6, -2])).unwrap(), 40 * 40 * 6 - 8 * 8 * 3 + 2 * 2 * 3);
        assert_eq!(p.edit_voxels(ground, &[], &block([-4, 3, -4], [-2, 6, -2])).unwrap(), 40 * 40 * 6 - 8 * 8 * 3 + 2 * 2 * 3);
        // The block loses its upper half: half the mass, the centre of mass lower; it stays where it is.
        let m = p.mass(b).unwrap();
        assert_eq!(p.edit_voxels(c, &block([0, 2, 0], [4, 4, 4]), &[]).unwrap(), 32);
        assert!((p.mass(b).unwrap() - m / 2.0).abs() < 1e-3 * m, "{:?} of {m}", p.mass(b));
        run(&mut p, 120);
        assert!((p.pose(b).unwrap().0[1] - 0.3).abs() < 5e-3, "{:?}", p.pose(b));
        // Bad edits: not a voxel collider, an unknown one, a cell out of bounds. Nothing changes.
        let ball = body(&mut p, [3.0, 3.0, 3.0], Shape::Sphere(0.2));
        let _ = ball;
        let plain = p.add_collider(None, &ColliderDesc { position: [9.0, 0.0, 9.0], ..ColliderDesc::new(Shape::Box([0.5; 3])) }).unwrap();
        assert!(p.edit_voxels(plain, &[[0, 0, 0]], &[]).is_err() && p.edit_voxels(9999, &[], &[]).is_err());
        assert!(p.edit_voxels(c, &[[0, 0, 0]], &[[1 << 21, 0, 0]]).is_err() && p.edit_voxels(c, &[[i32::MIN, 0, 0]], &[]).is_err());
        assert_eq!(p.voxels(c), Some(32));
        // Carved to nothing: the collider is gone, the body is the caller's to remove.
        assert_eq!(p.edit_voxels(c, &block([0, 0, 0], [4, 2, 4]), &[]).unwrap(), 0);
        assert!(!p.has_collider(c) && p.voxels(c).is_none());
        assert!(p.remove_body(b).is_empty());
        run(&mut p, 30);
        assert!(p.take_faults().is_empty());
    }

    /// A ray at a voxel shape meets what testing every solid cell's box finds, for random rays at a turned shape; and
    /// a ray that lies on a plane of the lattice and runs along it meets the cells (parry's own walk misses them all).
    #[test]
    fn rays_at_voxel_shapes() {
        use rapier3d::parry::query::RayCast;
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / 16_777_216.0
        };
        let cells: Vec<[i32; 3]> = block([-6, -3, -5], [7, 4, 6]).into_iter().filter(|_| rnd() < 0.3).collect();
        let voxels = rapier3d::parry::shape::Voxels::new(Vector::splat(0.25), &cells.iter().map(|c| IVector::new(c[0], c[1], c[2])).collect::<Vec<_>>());
        let at = pose([0.3, -0.2, 0.1], [0.2, -0.4, 0.1, 0.9]).unwrap();
        let (mut hits, mut misses) = (0, 0);
        for _ in 0..4000 {
            let o = Vector::new(rnd() * 8.0 - 4.0, rnd() * 8.0 - 4.0, rnd() * 8.0 - 4.0);
            // (Towards the shape, about the origin, or anywhere.)
            let to = Vector::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5) * if rnd() < 0.7 { 3.0 } else { 30.0 };
            let d = (to - o).normalize();
            let ray = Ray::new(o, d);
            let max = 1.0 + rnd() * 8.0;
            let brute = cells
                .iter()
                .filter_map(|c| {
                    let centre = at.transform_point(Vector::new(c[0] as f32 + 0.5, c[1] as f32 + 0.5, c[2] as f32 + 0.5) * 0.25);
                    Cuboid::new(Vector::splat(0.125)).cast_ray(&Pose::from_parts(centre, at.rotation), &ray, max, true)
                })
                .fold(f32::INFINITY, f32::min);
            match voxel_ray(&voxels, &at, &ray, max) {
                Some((t, n)) => {
                    assert!((t - brute).abs() < 1e-4, "{t} for {brute}: from {o:?} along {d:?}");
                    assert!((n.length() - 1.0).abs() < 1e-4 && (t == 0.0 || n.dot(d) < 1e-4), "{n:?}");
                    hits += 1;
                }
                None => {
                    assert!(brute == f32::INFINITY, "missed a cell at {brute}: from {o:?} along {d:?}");
                    misses += 1;
                }
            }
        }
        assert!(hits > 800 && misses > 800, "{hits} hits, {misses} misses");
        // On the lattice's planes, along them: a slab of cells of 0.1 m from above at every corner of its cells.
        let slab = rapier3d::parry::shape::Voxels::new(Vector::splat(0.1), &block([-20, 0, -20], [20, 3, 20]).iter().map(|c| IVector::new(c[0], c[1], c[2])).collect::<Vec<_>>());
        let still = Pose::identity();
        let mut parry_missed = 0;
        for x in -19..19 {
            for z in -19..19 {
                let ray = Ray::new(Vector::new(x as f32 * 0.1, 5.0, z as f32 * 0.1), Vector::new(0.0, -1.0, 0.0));
                let (t, n) = voxel_ray(&slab, &still, &ray, 10.0).unwrap_or_else(|| panic!("a ray down at ({x}, {z}) x 0.1 m misses the slab"));
                assert!((t - 4.7).abs() < 1e-4 && n.y == 1.0, "{t} {n:?}");
                parry_missed += slab.cast_ray(&still, &ray, 10.0, true).is_none() as u32;
            }
        }
        // (Why the walk is here. If parry mends its own, this count falls to 0 and the walk can go.)
        assert!(parry_missed > 0, "parry's ray meets them all now");
        // Along the top face, in it: nothing above the cells is met; one cell lower the first cell is.
        assert!(voxel_ray(&slab, &still, &Ray::new(Vector::new(-5.0, 0.3, 0.05), Vector::new(1.0, 0.0, 0.0)), 20.0).is_none());
        let (t, n) = voxel_ray(&slab, &still, &Ray::new(Vector::new(-5.0, 0.2, 0.05), Vector::new(1.0, 0.0, 0.0)), 20.0).unwrap();
        assert!((t - 3.0).abs() < 1e-5 && n.x == -1.0);
        // From inside a cell: at once, against the ray.
        let (t, n) = voxel_ray(&slab, &still, &Ray::new(Vector::new(0.05, 0.15, 0.05), Vector::new(0.0, 1.0, 0.0)), 20.0).unwrap();
        assert!(t == 0.0 && n.y == -1.0);
    }

    /// What a voxel shape refuses; and a world steps as it did until it has one (Rapier's contact recycling is on
    /// without voxel shapes, off with: `add_collider`).
    #[test]
    fn voxel_shapes_are_checked() {
        let mut p = Physics::default();
        let bad = |p: &mut Physics, size: f32, cells: Vec<[i32; 3]>| p.add_collider(None, &ColliderDesc::new(Shape::Voxels { size, cells })).is_err();
        assert!(bad(&mut p, 0.1, vec![]) && bad(&mut p, 0.0, vec![[0, 0, 0]]) && bad(&mut p, f32::NAN, vec![[0, 0, 0]]) && bad(&mut p, 1e3, vec![[0, 0, 0]]));
        assert!(bad(&mut p, 0.1, vec![[0, 1 << 20, 0]]) && bad(&mut p, 0.1, vec![[i32::MIN, 0, 0]]) && bad(&mut p, 100.0, vec![[1001, 0, 0]]));
        assert!(p.world.integration_parameters.contact_recycling, "no voxel shape yet");
        // The same cell twice is one cell.
        let c = p.add_collider(None, &ColliderDesc::new(Shape::Voxels { size: 0.1, cells: vec![[0, 0, 0], [0, 0, 0], [1, 0, 0]] })).unwrap();
        assert_eq!(p.voxels(c), Some(2));
        assert!(!p.world.integration_parameters.contact_recycling);
        // A sensor of cells, and cells at the far end of what a shape may reach.
        p.add_collider(None, &ColliderDesc { sensor: true, ..ColliderDesc::new(Shape::Voxels { size: 0.01, cells: vec![[(1 << 20) - 1, 0, -(1 << 20)]] }) }).unwrap();
        run(&mut p, 5);
    }

    /// Bad input adds nothing; removing a body takes its colliders and joints with it.
    #[test]
    fn bad_input_and_removal() {
        let (mut p, g) = ground();
        assert!(p.add_body(&BodyDesc { position: [f32::NAN, 0.0, 0.0], ..Default::default() }).is_err());
        let b = body(&mut p, [0.0, 2.0, 0.0], Shape::Box([0.5; 3]));
        assert!(p.add_collider(Some(999), &ColliderDesc::new(Shape::Sphere(1.0))).is_err());
        assert!(p.add_collider(Some(b), &ColliderDesc::new(Shape::Sphere(-1.0))).is_err());
        assert!(p.add_collider(Some(b), &ColliderDesc::new(Shape::Box([1.0, 0.0, 1.0]))).is_err());
        assert!(p.add_collider(Some(b), &ColliderDesc::new(Shape::ConvexHull(vec![[0.0; 3], [1.0, 0.0, 0.0]]))).is_err());
        assert!(p.add_collider(Some(b), &ColliderDesc::new(Shape::Trimesh { vertices: vec![[0.0; 3]], indices: vec![[0, 1, 2]] })).is_err());
        let c = body(&mut p, [2.0, 2.0, 0.0], Shape::Box([0.5; 3]));
        assert!(p.add_joint(b, 999, &JointDesc::Spherical { anchor1: [0.0; 3], anchor2: [0.0; 3] }).is_err());
        assert!(p.add_joint(b, c, &JointDesc::Revolute { anchor1: [0.0; 3], anchor2: [0.0; 3], axis: [0.0; 3], limits: None, motor: None }).is_err());
        assert!(p.add_joint(b, c, &JointDesc::Rope { anchor1: [0.0; 3], anchor2: [0.0; 3], length: f32::NAN }).is_err());
        let j = p.add_joint(b, c, &JointDesc::Fixed { anchor1: [1.0, 0.0, 0.0], anchor2: [-1.0, 0.0, 0.0] }).unwrap();
        run(&mut p, 2);
        assert_eq!((p.bodies(), p.colliders.len(), p.joints.len()), (2, 3, 1));
        p.remove_body(b);
        assert_eq!((p.bodies(), p.colliders.len(), p.joints.len()), (1, 2, 0));
        assert!(p.pose(b).is_none() && p.pose(c).is_some());
        p.remove_joint(j);
        run(&mut p, 120);
        // The other falls on the ground alone; rays find the ground where the removed box was.
        assert!((p.pose(c).unwrap().0[1] - 0.5).abs() < 5e-3);
        assert_eq!(p.cast_ray([0.0, 10.0, 0.0], [0.0, -1.0, 0.0], 100.0, None).unwrap().unwrap().collider, g);
        p.remove_collider(g);
        run(&mut p, 60);
        assert!(p.pose(c).unwrap().0[1] < 0.0, "with the ground gone it falls on");
    }

    /// The same scene stepped twice ends in the same bits, and (with `enhanced-determinism`) in the recorded ones, which
    /// the wasm32 build gives too (tools/physics-determinism.ts).
    #[test]
    fn steps_repeat() {
        let (a, b) = (crate::check::script(600), crate::check::script(600));
        assert_eq!(a, b);
        assert_ne!(a, crate::check::script(599));
        #[cfg(feature = "enhanced-determinism")]
        assert_eq!(a, crate::check::EXPECTED, "the script's end changed: if meant, record {a:#x}");
    }
}


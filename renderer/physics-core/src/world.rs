//! The physics world: plain descriptions in, handles and poses out.

use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::*;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The fixed step, s. The engine steps whole ticks only.
pub const TICK: f32 = 1.0 / 60.0;

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
    /// Triangles (static geometry: a trimesh has no volume, so no mass).
    Trimesh { vertices: Vec<[f32; 3]>, indices: Vec<[u32; 3]> },
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

fn pose(p: [f32; 3], q: [f32; 4]) -> Pose {
    let r = Rotation::from_xyzw(q[0], q[1], q[2], q[3]);
    Pose::from_parts(v(p), if r.length_squared() > 1e-12 { r.normalize() } else { Rotation::IDENTITY })
}

fn finite(a: &[f32]) -> bool {
    a.iter().all(|x| x.is_finite())
}

impl Shape {
    /// The Rapier shape; None for a shape that cannot be built (no volume, bad numbers).
    fn shared(&self) -> Option<SharedShape> {
        match self {
            Shape::Box(h) => (finite(h) && h.iter().all(|&x| x > 0.0)).then(|| SharedShape::cuboid(h[0], h[1], h[2])),
            Shape::Sphere(r) => (r.is_finite() && *r > 0.0).then(|| SharedShape::ball(*r)),
            Shape::Capsule { half_height, radius } => (half_height.is_finite() && radius.is_finite() && *half_height >= 0.0 && *radius > 0.0).then(|| SharedShape::capsule_y(*half_height, *radius)),
            Shape::ConvexHull(points) => {
                if points.len() < 4 || !points.iter().all(|p| finite(p)) {
                    return None;
                }
                let pts: Vec<Vector> = points.iter().map(|&p| v(p)).collect();
                SharedShape::convex_hull(&pts)
            }
            Shape::Trimesh { vertices, indices } => {
                let n = vertices.len() as u32;
                if indices.is_empty() || !vertices.iter().all(|p| finite(p)) || indices.iter().any(|t| t.iter().any(|&i| i >= n)) {
                    return None;
                }
                SharedShape::trimesh(vertices.iter().map(|&p| v(p)).collect(), indices.clone()).ok()
            }
            Shape::Custom(c) => Some(c.shared()),
        }
    }
}

/// The physics world. Ids are the world's own, never reused; removed things are simply gone.
pub struct Physics {
    world: PhysicsWorld,
    events: Events,
    bodies: BTreeMap<BodyId, RigidBodyHandle>,
    colliders: BTreeMap<ColliderId, ColliderHandle>,
    joints: BTreeMap<JointId, ImpulseJointHandle>,
    /// The kinematic bodies (`awake`: the islands hold dynamic ones only).
    kinematic: std::collections::BTreeSet<BodyId>,
    next: u32,
    ticks: u64,
    contacts: Vec<Contact>,
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
        Physics { world, events: Events::default(), bodies: BTreeMap::new(), colliders: BTreeMap::new(), joints: BTreeMap::new(), kinematic: Default::default(), next: 0, ticks: 0, contacts: vec![] }
    }

    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    pub fn set_gravity(&mut self, gravity: [f32; 3]) {
        if finite(&gravity) {
            self.world.gravity = v(gravity);
        }
    }

    /// Ticks stepped so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn bodies(&self) -> usize {
        self.bodies.len()
    }

    /// Adds a body; None if its numbers are not finite.
    pub fn add_body(&mut self, d: &BodyDesc) -> Option<BodyId> {
        if !(finite(&d.position) && finite(&d.rotation) && finite(&d.linvel) && finite(&d.angvel) && finite(&[d.linear_damping, d.angular_damping, d.gravity_scale])) {
            return None;
        }
        let b = match d.kind {
            BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            BodyKind::Fixed => RigidBodyBuilder::fixed(),
            BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        }
        .pose(pose(d.position, d.rotation))
        .linvel(v(d.linvel))
        .angvel(v(d.angvel))
        .linear_damping(d.linear_damping.max(0.0))
        .angular_damping(d.angular_damping.max(0.0))
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
        Some(id)
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
            self.colliders.remove(c);
        }
        if jointed {
            let world = &self.world;
            self.joints.retain(|_, j| world.impulse_joints.get(*j).is_some());
        }
        gone
    }

    /// Adds a collider to `body` (None: a static collider in the world). None if the body is unknown or the shape
    /// cannot be built.
    pub fn add_collider(&mut self, body: Option<BodyId>, d: &ColliderDesc) -> Option<ColliderId> {
        let parent = match body {
            Some(b) => Some(*self.bodies.get(&b)?),
            None => None,
        };
        if !(finite(&d.position) && finite(&d.rotation) && finite(&[d.density, d.friction, d.restitution, d.mass.unwrap_or(0.0)])) {
            return None;
        }
        let shape = d.shape.shared()?;
        let mut c = ColliderBuilder::new(shape).position(pose(d.position, d.rotation)).density(d.density.max(0.0)).friction(d.friction.max(0.0)).restitution(d.restitution.clamp(0.0, 1.0)).sensor(d.sensor);
        if let Some(m) = d.mass {
            c = c.mass(m.max(0.0));
        }
        if d.events {
            c = c.active_events(ActiveEvents::COLLISION_EVENTS);
        }
        let id = self.id();
        let h = self.world.insert_collider(c.user_data(id as u128), parent);
        self.colliders.insert(id, h);
        Some(id)
    }

    pub fn remove_collider(&mut self, id: ColliderId) {
        if let Some(h) = self.colliders.remove(&id) {
            self.world.remove_collider(h);
        }
    }

    /// Joins two bodies; None if one is unknown or the joint's numbers are bad. Anchors are in each body's space, an
    /// axis in the first body's. The bodies are joined as they are turned now: a fixed joint keeps their turn against
    /// each other, and a hinge's or a slider's angle or travel is 0 as they are (its limits count from there).
    pub fn add_joint(&mut self, a: BodyId, b: BodyId, d: &JointDesc) -> Option<JointId> {
        let (ha, hb) = (*self.bodies.get(&a)?, *self.bodies.get(&b)?);
        let unit = |axis: [f32; 3]| -> Option<Vector> {
            let x = v(axis);
            (finite(&axis) && x.length_squared() > 1e-12).then(|| x.normalize())
        };
        let ok = |p: &[[f32; 3]]| p.iter().all(|a| finite(a));
        let mut joint: GenericJoint = match d {
            JointDesc::Fixed { anchor1, anchor2 } => {
                if !ok(&[*anchor1, *anchor2]) {
                    return None;
                }
                FixedJointBuilder::new().local_anchor1(v(*anchor1)).local_anchor2(v(*anchor2)).build().into()
            }
            JointDesc::Revolute { anchor1, anchor2, axis, limits, motor } => {
                if !ok(&[*anchor1, *anchor2]) {
                    return None;
                }
                let mut j = RevoluteJointBuilder::new(unit(*axis)?).local_anchor1(v(*anchor1)).local_anchor2(v(*anchor2));
                if let Some(l) = limits.filter(|l| l.min.is_finite() && l.max.is_finite() && l.min <= l.max) {
                    j = j.limits([l.min, l.max]);
                }
                if let Some(m) = motor.filter(|m| m.velocity.is_finite() && m.damping.is_finite() && m.max_force.is_finite()) {
                    j = j.motor_velocity(m.velocity, m.damping.max(0.0)).motor_max_force(m.max_force.max(0.0));
                }
                j.build().into()
            }
            JointDesc::Prismatic { anchor1, anchor2, axis, limits, motor } => {
                if !ok(&[*anchor1, *anchor2]) {
                    return None;
                }
                let mut j = PrismaticJointBuilder::new(unit(*axis)?).local_anchor1(v(*anchor1)).local_anchor2(v(*anchor2));
                if let Some(l) = limits.filter(|l| l.min.is_finite() && l.max.is_finite() && l.min <= l.max) {
                    j = j.limits([l.min, l.max]);
                }
                if let Some(m) = motor.filter(|m| m.velocity.is_finite() && m.damping.is_finite() && m.max_force.is_finite()) {
                    j = j.motor_velocity(m.velocity, m.damping.max(0.0)).motor_max_force(m.max_force.max(0.0));
                }
                j.build().into()
            }
            JointDesc::Spherical { anchor1, anchor2 } => {
                if !ok(&[*anchor1, *anchor2]) {
                    return None;
                }
                SphericalJointBuilder::new().local_anchor1(v(*anchor1)).local_anchor2(v(*anchor2)).build().into()
            }
            JointDesc::Rope { anchor1, anchor2, length } => {
                if !ok(&[*anchor1, *anchor2]) || !length.is_finite() || *length < 0.0 {
                    return None;
                }
                RopeJointBuilder::new(*length).local_anchor1(v(*anchor1)).local_anchor2(v(*anchor2)).build().into()
            }
        };
        // The joint's frame in the second body is where the first body's is now (the builders give both bodies the
        // same frame, which is right only for bodies turned alike: others would be wrenched into line).
        if matches!(d, JointDesc::Fixed { .. } | JointDesc::Revolute { .. } | JointDesc::Prismatic { .. }) {
            let (ra, rb) = (self.world.bodies.get(ha)?.position().rotation, self.world.bodies.get(hb)?.position().rotation);
            joint.local_frame2.rotation = (rb.inverse() * ra * joint.local_frame1.rotation).normalize();
        }
        let h = self.world.insert_impulse_joint(ha, hb, joint);
        let id = self.id();
        self.joints.insert(id, h);
        Some(id)
    }

    pub fn remove_joint(&mut self, id: JointId) {
        if let Some(h) = self.joints.remove(&id) {
            self.world.remove_impulse_joint(h);
        }
    }

    pub fn has_collider(&self, id: ColliderId) -> bool {
        self.colliders.contains_key(&id)
    }

    pub fn has_joint(&self, id: JointId) -> bool {
        self.joints.contains_key(&id)
    }

    /// One tick of `TICK` seconds. Contacts that began or ended are kept for `take_contacts`.
    pub fn step(&mut self) {
        self.world.step_with_events(&(), &self.events);
        self.ticks += 1;
        for e in self.events.0.lock().unwrap().drain(..) {
            // (A removed collider's contact ends without it: its id is no longer known.)
            let id = |h: ColliderHandle| self.world.colliders.get(h).map(|c| c.user_data as u32);
            if let (Some(a), Some(b)) = (id(e.collider1()), id(e.collider2())) {
                self.contacts.push(Contact { a: a.min(b), b: a.max(b), started: e.started() });
            }
        }
    }

    /// The contacts begun and ended since the last call (colliders with `events`), in the order they happened.
    pub fn take_contacts(&mut self) -> Vec<Contact> {
        std::mem::take(&mut self.contacts)
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

    pub fn set_pose(&mut self, id: BodyId, position: [f32; 3], rotation: [f32; 4]) {
        if finite(&position) && finite(&rotation) {
            if let Some(b) = self.body_mut(id) {
                b.set_position(pose(position, rotation), true);
            }
        }
    }

    pub fn set_velocity(&mut self, id: BodyId, linvel: [f32; 3], angvel: [f32; 3]) {
        if finite(&linvel) && finite(&angvel) {
            if let Some(b) = self.body_mut(id) {
                b.set_linvel(v(linvel), true);
                b.set_angvel(v(angvel), true);
            }
        }
    }

    /// An impulse (N s) at the body's centre of mass, and an angular one (N m s).
    pub fn apply_impulse(&mut self, id: BodyId, impulse: [f32; 3], torque: [f32; 3]) {
        if finite(&impulse) && finite(&torque) {
            if let Some(b) = self.body_mut(id) {
                b.apply_impulse(v(impulse), true);
                b.apply_torque_impulse(v(torque), true);
            }
        }
    }

    /// An impulse (N s) at a point of the world: it turns the body as well, unless the point is its centre of mass.
    pub fn apply_impulse_at(&mut self, id: BodyId, impulse: [f32; 3], point: [f32; 3]) {
        if finite(&impulse) && finite(&point) {
            if let Some(b) = self.body_mut(id) {
                b.apply_impulse_at_point(v(impulse), v(point), true);
            }
        }
    }

    /// A force (N) at the centre of mass and a torque (N m), added to those the body has: they act in every step until
    /// `clear_forces`.
    pub fn add_force(&mut self, id: BodyId, force: [f32; 3], torque: [f32; 3]) {
        if finite(&force) && finite(&torque) {
            if let Some(b) = self.body_mut(id) {
                b.add_force(v(force), true);
                b.add_torque(v(torque), true);
            }
        }
    }

    pub fn clear_forces(&mut self, id: BodyId) {
        if let Some(b) = self.body_mut(id) {
            b.reset_forces(false);
            b.reset_torques(false);
        }
    }

    /// Where a kinematic body is at the end of the next step (it moves there, pushing what is in the way).
    pub fn set_kinematic_target(&mut self, id: BodyId, position: [f32; 3], rotation: [f32; 4]) {
        if finite(&position) && finite(&rotation) {
            if let Some(b) = self.body_mut(id).filter(|b| b.is_kinematic()) {
                b.set_next_kinematic_position(pose(position, rotation));
            }
        }
    }

    fn hit(&self, h: ColliderHandle, distance: f32, point: Vector, normal: Vector) -> Option<Hit> {
        let c = self.world.colliders.get(h)?;
        let collider = c.user_data as u32;
        let body = c.parent().and_then(|p| self.world.bodies.get(p)).map(|b| b.user_data as u32);
        Some(Hit { collider, body, distance, point: point.to_array(), normal: normal.to_array() })
    }

    /// The first collider a ray meets within `max` metres (the world as of the last step). `skip`: a body whose
    /// colliders the ray passes through (the one that casts it).
    pub fn cast_ray(&self, origin: [f32; 3], direction: [f32; 3], max: f32, skip: Option<BodyId>) -> Option<Hit> {
        let d = v(direction);
        if !(finite(&origin) && finite(&direction) && max.is_finite() && max > 0.0) || d.length_squared() < 1e-12 {
            return None;
        }
        let ray = Ray::new(v(origin), d.normalize());
        let (h, i) = self.world.cast_ray_and_get_normal(&ray, max, true, self.filter(skip))?;
        self.hit(h, i.time_of_impact, ray.point_at(i.time_of_impact), i.normal)
    }

    /// Every collider a ray passes through within `max` metres, nearest first, at most `limit` of them.
    pub fn cast_ray_all(&self, origin: [f32; 3], direction: [f32; 3], max: f32, skip: Option<BodyId>, limit: usize) -> Vec<Hit> {
        let d = v(direction);
        if !(finite(&origin) && finite(&direction) && max.is_finite() && max > 0.0) || d.length_squared() < 1e-12 {
            return vec![];
        }
        let ray = Ray::new(v(origin), d.normalize());
        let mut hits: Vec<Hit> = self.world.intersect_ray(ray, max, true, self.filter(skip)).filter_map(|(h, _, i)| self.hit(h, i.time_of_impact, ray.point_at(i.time_of_impact), i.normal)).collect();
        // (Equal distances in collider order: the same answer on every machine.)
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.collider.cmp(&b.collider)));
        hits.truncate(limit);
        hits
    }

    fn filter(&self, skip: Option<BodyId>) -> QueryFilter<'_> {
        match skip.and_then(|b| self.bodies.get(&b)) {
            Some(&h) => QueryFilter::default().exclude_rigid_body(h),
            None => QueryFilter::default(),
        }
    }

    /// The colliders a shape placed at a pose overlaps, in id order.
    pub fn overlap(&self, shape: &Shape, position: [f32; 3], rotation: [f32; 4], skip: Option<BodyId>) -> Vec<ColliderId> {
        let Some(s) = shape.shared() else { return vec![] };
        if !(finite(&position) && finite(&rotation)) {
            return vec![];
        }
        let mut out: Vec<ColliderId> = self.world.intersect_shape(pose(position, rotation), &*s.0, self.filter(skip)).map(|(_, c)| c.user_data as u32).collect();
        out.sort_unstable();
        out
    }

    /// The first collider a shape meets when moved from a pose along `direction` by at most `max` metres.
    pub fn sweep(&self, shape: &Shape, position: [f32; 3], rotation: [f32; 4], direction: [f32; 3], max: f32, skip: Option<BodyId>) -> Option<Hit> {
        let s = shape.shared()?;
        let d = v(direction);
        if !(finite(&position) && finite(&rotation) && finite(&direction) && max.is_finite() && max > 0.0) || d.length_squared() < 1e-12 {
            return None;
        }
        let d = d.normalize();
        let options = ShapeCastOptions { max_time_of_impact: max, target_distance: 0.0, stop_at_penetration: true, compute_impact_geometry_on_penetration: true };
        let (h, hit) = self.world.cast_shape(&pose(position, rotation), d, &*s.0, options, self.filter(skip))?;
        self.hit(h, hit.time_of_impact, hit.witness1, hit.normal1)
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
                feed(x.to_bits());
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
        p.set_velocity(bar, [0.0, 0.0, -0.5], [0.0, 1.0, 0.0]);
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
        p.set_velocity(b, [0.0, 5.0, 0.0], [0.0; 3]);
        run(&mut p, 10);
        assert_eq!(p.take_contacts(), vec![Contact { a: g, b: c, started: false }]);
        let _ = quiet;
        // A sensor 3 m up: the ball thrown through it enters and leaves, and is not slowed more than gravity does.
        let s = p.add_collider(None, &ColliderDesc { position: [0.0, 3.0, 0.0], sensor: true, events: true, ..ColliderDesc::new(Shape::Box([1.0, 0.2, 1.0])) }).unwrap();
        p.set_pose(b, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]);
        p.set_velocity(b, [0.0, 12.0, 0.0], [0.0; 3]);
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
        let c = p.cast_ray([0.2, 10.0, 0.1], [0.0, -1.0, 0.0], 100.0, None).unwrap();
        assert_eq!(c.body, Some(b));
        assert!((c.distance - 9.0).abs() < 1e-3 && (c.point[1] - 1.0).abs() < 1e-3, "{c:?}");
        assert!(dist(c.normal, [0.0, 1.0, 0.0]) < 1e-4);
        // Past the box: the ground (a collider without a body). Skipping the box's body: the ground under it.
        let h = p.cast_ray([3.0, 10.0, 0.0], [0.0, -2.0, 0.0], 100.0, None).unwrap();
        assert_eq!((h.collider, h.body), (g, None));
        assert!((h.distance - 10.0).abs() < 1e-3);
        assert_eq!(p.cast_ray([0.2, 10.0, 0.1], [0.0, -1.0, 0.0], 100.0, Some(b)).unwrap().collider, g);
        // Too short, pointing away, bad numbers: nothing.
        assert!(p.cast_ray([0.2, 10.0, 0.1], [0.0, -1.0, 0.0], 5.0, None).is_none());
        assert!(p.cast_ray([0.2, 10.0, 0.1], [0.0, 1.0, 0.0], 100.0, None).is_none());
        assert!(p.cast_ray([0.2, f32::NAN, 0.1], [0.0, -1.0, 0.0], 100.0, None).is_none());
        assert!(p.cast_ray([0.2, 10.0, 0.1], [0.0; 3], 100.0, None).is_none());
        // Overlap: a sphere at the box's corner touches the box and the ground; one high up nothing.
        let hits = p.overlap(&Shape::Sphere(0.3), [0.6, 0.1, 0.6], [0.0, 0.0, 0.0, 1.0], None);
        assert_eq!(hits, vec![g, c.collider]);
        assert_eq!(p.overlap(&Shape::Sphere(0.3), [0.6, 0.1, 0.6], [0.0, 0.0, 0.0, 1.0], Some(b)), vec![g]);
        assert!(p.overlap(&Shape::Sphere(0.3), [0.0, 5.0, 0.0], [0.0, 0.0, 0.0, 1.0], None).is_empty());
        // Sweep: a sphere of 0.25 m lowered from 5 m meets the box's top (y = 1) after 3.75 m.
        let s = p.sweep(&Shape::Sphere(0.25), [0.0, 5.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, -1.0, 0.0], 100.0, None).unwrap();
        assert_eq!(s.body, Some(b));
        assert!((s.distance - 3.75).abs() < 1e-3, "{s:?}");
        assert!(p.sweep(&Shape::Sphere(0.25), [0.0, 5.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, -1.0, 0.0], 2.0, None).is_none());
    }

    /// A kinematic body goes where it is told and pushes a dynamic one; nothing pushes it.
    #[test]
    fn kinematic_bodies_push() {
        let (mut p, _) = ground();
        let k = p.add_body(&BodyDesc { kind: BodyKind::Kinematic, position: [-2.0, 0.5, 0.0], ..Default::default() }).unwrap();
        p.add_collider(Some(k), &ColliderDesc::new(Shape::Box([0.5; 3]))).unwrap();
        let d = body(&mut p, [0.0, 0.5, 0.0], Shape::Box([0.5; 3]));
        for t in 1..=120 {
            p.set_kinematic_target(k, [-2.0 + t as f32 * 0.02, 0.5, 0.0], [0.0, 0.0, 0.0, 1.0]);
            p.step();
        }
        let (kp, dp) = (p.pose(k).unwrap().0, p.pose(d).unwrap().0);
        assert!((kp[0] - 0.4).abs() < 1e-3 && (kp[1] - 0.5).abs() < 1e-6, "{kp:?}");
        assert!(dp[0] > kp[0] + 0.99, "pushed ahead of it: {dp:?}");
        // (A target for a dynamic body is ignored.)
        p.set_kinematic_target(d, [9.0, 9.0, 9.0], [0.0, 0.0, 0.0, 1.0]);
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

    /// Bad input adds nothing; removing a body takes its colliders and joints with it.
    #[test]
    fn bad_input_and_removal() {
        let (mut p, g) = ground();
        assert!(p.add_body(&BodyDesc { position: [f32::NAN, 0.0, 0.0], ..Default::default() }).is_none());
        let b = body(&mut p, [0.0, 2.0, 0.0], Shape::Box([0.5; 3]));
        assert!(p.add_collider(Some(999), &ColliderDesc::new(Shape::Sphere(1.0))).is_none());
        assert!(p.add_collider(Some(b), &ColliderDesc::new(Shape::Sphere(-1.0))).is_none());
        assert!(p.add_collider(Some(b), &ColliderDesc::new(Shape::Box([1.0, 0.0, 1.0]))).is_none());
        assert!(p.add_collider(Some(b), &ColliderDesc::new(Shape::ConvexHull(vec![[0.0; 3], [1.0, 0.0, 0.0]]))).is_none());
        assert!(p.add_collider(Some(b), &ColliderDesc::new(Shape::Trimesh { vertices: vec![[0.0; 3]], indices: vec![[0, 1, 2]] })).is_none());
        let c = body(&mut p, [2.0, 2.0, 0.0], Shape::Box([0.5; 3]));
        assert!(p.add_joint(b, 999, &JointDesc::Spherical { anchor1: [0.0; 3], anchor2: [0.0; 3] }).is_none());
        assert!(p.add_joint(b, c, &JointDesc::Revolute { anchor1: [0.0; 3], anchor2: [0.0; 3], axis: [0.0; 3], limits: None, motor: None }).is_none());
        assert!(p.add_joint(b, c, &JointDesc::Rope { anchor1: [0.0; 3], anchor2: [0.0; 3], length: f32::NAN }).is_none());
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
        assert_eq!(p.cast_ray([0.0, 10.0, 0.0], [0.0, -1.0, 0.0], 100.0, None).unwrap().collider, g);
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

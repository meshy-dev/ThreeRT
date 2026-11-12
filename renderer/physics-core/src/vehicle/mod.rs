//! Vehicles (ROADMAP § 6 step 10 c2; the design: `out/web-probe/step10/c2-design.md`). A vehicle is a chassis (any
//! dynamic body, a voxel body included) and its wheels, which are not bodies. Each tick, before the world's step, every
//! vehicle (in id order) probes the ground under each wheel and pushes its chassis with the suspension's and the tyres'
//! impulses: Rapier's ray-cast vehicle controller (Bullet's model), copied and changed in `controller.rs`. Behind the
//! suspension's travel each wheel has a stop: a collider of the tyre's shape on the chassis where the wheel is when
//! fully compressed, which Rapier's contact solver stops with every other contact (a hard stop as the PhysX, Chaos and
//! Jolt vehicles have in their solvers). Its contacts are given the tyre's friction and the surface of a wheel rolling
//! at the hub's speed by a contact hook (`StopHooks`), so the stop rolls and does not drag.
//!
//! The state the hook reads is made anew each tick before the step, from the vehicles alone; the hook only reads it
//! (it may be called from the solver's threads).

pub mod controller;
#[doc(hidden)]
pub mod scenarios;
pub mod sweep;

use super::{between, unit, v, within, BodyId, ColliderDesc, ColliderId, Physics, Shape};
use controller::{DynamicRayCastVehicleController, WheelTuning};
use rapier3d::math::{Pose, Real, Vector};
use rapier3d::prelude::*;
use std::collections::HashMap;

pub type VehicleId = u32;

pub const MAX_VEHICLES: usize = 64;
pub const MAX_WHEELS: usize = 8;
pub const WHEEL_RADIUS: [f32; 2] = [0.05, 5.0];
pub const MAX_TRAVEL: f32 = 2.0;
pub const MAX_STEERING: f32 = 1.5;

/// A wheel, in the chassis body's frame. Stiffness and damping are Bullet's: per kg of the chassis (a wheel's spring
/// pushes with `stiffness` x compression x the chassis's mass), so a wheel's spring rate is `stiffness` x mass N/m.
#[derive(Clone, Debug, PartialEq)]
pub struct WheelDesc {
    /// The hard point: where the suspension is fixed to the chassis (the wheel's centre at length 0).
    pub anchor: [f32; 3],
    /// Down the suspension, and the axle (the wheel spins about it; the forward is `direction × axle`'s opposite:
    /// with direction -y and axle -x, +z).
    pub direction: [f32; 3],
    pub axle: [f32; 3],
    /// The spring's length unloaded (m): the wheel's centre at `anchor + direction × rest` with no load.
    pub rest: f32,
    /// How far the suspension compresses from `rest` (m): fully compressed at `rest − travel`.
    pub travel: f32,
    pub radius: f32,
    /// The tyre's width (m: the stop's).
    pub width: f32,
    pub stiffness: f32,
    /// Damping on compression and on extension (per kg of the chassis, N s/m per kg).
    pub compression: f32,
    pub relaxation: f32,
    /// The tyre's grip: the friction coefficient of its forward impulse (Bullet's `friction_slip`), and the sideways
    /// impulse's share.
    pub friction_slip: f32,
    pub side_stiffness: f32,
    /// The suspension's largest force (N).
    pub max_force: f32,
    /// The bump stop: over the last `bump_length` of the travel the stiffness rises from 0 to `bump_stiffness` (per kg,
    /// as `stiffness`) with the square of the compression into it (`controller.rs`).
    pub bump_length: f32,
    pub bump_stiffness: f32,
    /// How far the stop collider is inside the tyre (m): the probe meets the ground first.
    pub stop_inset: f32,
}

impl Default for WheelDesc {
    fn default() -> Self {
        WheelDesc {
            anchor: [0.0; 3],
            direction: [0.0, -1.0, 0.0],
            axle: [-1.0, 0.0, 0.0],
            rest: 0.3,
            travel: 0.2,
            radius: 0.33,
            width: 0.2,
            stiffness: 20.0,
            compression: 1.2,
            relaxation: 1.9,
            friction_slip: 1.0,
            side_stiffness: 1.0,
            max_force: 1e6,
            bump_length: 0.0,
            bump_stiffness: 0.0,
            stop_inset: 0.025,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VehicleDesc {
    pub chassis: BodyId,
    pub wheels: Vec<WheelDesc>,
    /// The chassis's forward and up axes (0 x, 1 y, 2 z).
    pub forward_axis: usize,
    pub up_axis: usize,
    /// The chassis's centre of mass (body frame), replacing its shapes': a car's mass is not spread as its body's
    /// volume is (another vehicle SDK's centre of mass override). Its mass and inertia stay. None: the body's own.
    pub center_of_mass: Option<[f32; 3]>,
}

/// A wheel's controls, held until changed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelControl {
    /// The drive's force at the contact (N; negative backwards), the brake (the largest rolling impulse, N s), the
    /// steering angle about the suspension (rad).
    pub engine: f32,
    pub brake: f32,
    pub steering: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelState {
    /// The wheel's centre (world); its turn for drawing it (world, xyzw: the chassis's, the steering about the
    /// suspension's up, the spin about the axle; a wheel modelled in the chassis's axes takes it as it is); its spin
    /// about the axle (rad) and its steering; on the ground or not; the suspension's length.
    pub center: [f32; 3],
    pub orientation: [f32; 4],
    pub spin: f32,
    pub steering: f32,
    pub contact: bool,
    pub suspension: f32,
    /// Where it touches and the ground's normal (world; meaningful with `contact`).
    pub point: [f32; 3],
    pub normal: [f32; 3],
    /// The wheel was lost (its anchor's material carved away, or cut off: `Physics::remove_wheel`); `center` is where
    /// it was then, the rest is empty.
    pub lost: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VehicleState {
    /// Along the chassis's forward axis (m/s).
    pub speed: f32,
    pub wheels: Vec<WheelState>,
}

pub(super) struct Vehicle {
    chassis: BodyId,
    ctl: DynamicRayCastVehicleController,
    /// Per wheel: its stop collider, where the stop sits on the chassis and the suspension's up (chassis frame: the
    /// stop turns about it with the steering), and the steering the stop is turned to.
    stops: Vec<ColliderId>,
    stop_frames: Vec<(Vector, Vector)>,
    stop_steering: Vec<f32>,
    /// Per wheel as described (scene code's index): its place among the controller's wheels and stops (None: lost),
    /// where it was when it was lost, and its suspension's down and its axle (chassis frame).
    slots: Vec<Option<usize>>,
    lost_at: Vec<Option<[f32; 3]>>,
    frames: Vec<(Vector, Vector)>,
    /// The centre of mass override, and what it is made of: the chassis's own additional mass properties (those of
    /// `set_mass_properties`), and its colliders' own, which the override holds (the colliders are made massless while
    /// it does). Both put back when the vehicle goes.
    center_of_mass: Option<Vector>,
    own_additional: MassProperties,
    held: Vec<(ColliderId, MassProperties)>,
}

/// A stop collider's state for the hook: the wheel's axle and forward (world), and the speed of its surface along the
/// forward (the hub's).
struct StopState {
    axle: Vector,
    forward: Vector,
    surface: Real,
    /// The tyre's friction (the wheel's `friction_slip`).
    friction: Real,
}

/// The contact hook on the stops (`ActiveHooks::MODIFY_SOLVER_CONTACTS`): the tyre's friction, and a surface moving as
/// a wheel rolling at its hub's speed. Built before each step; read only.
#[derive(Default)]
pub(super) struct StopHooks {
    stops: HashMap<ColliderHandle, StopState>,
}

impl PhysicsHooks for StopHooks {
    fn modify_solver_contacts(&self, ctx: &mut ContactModificationContext) {
        let (stop, side) = match (self.stops.get(&ctx.collider1), self.stops.get(&ctx.collider2)) {
            (Some(s), _) => (s, 1.0),
            (None, Some(s)) => (s, -1.0),
            _ => return,
        };
        let Some(m) = ctx.rigid_mut() else { return };
        *m.friction = stop.friction;
        let n = *m.normal;
        let t = n.cross(stop.axle);
        let len = t.length();
        if !(len > 1e-6) {
            return;
        }
        let t = if t.dot(stop.forward) < 0.0 { -t / len } else { t / len };
        // (Rapier's tangent velocity is the second collider's surface against the first's: the stop's surface moves
        // backward under a wheel rolling forward, so its sign is the opposite of the hub's motion. The test
        // `the_stop_rolls_with_the_hub` holds the sign.)
        for c in m.solver_contacts.iter_mut() {
            c.tangent_velocity = t * (-stop.surface * side);
        }
    }
}

/// The tyre's outline for the stop: 24 points round each rim (cos and sin of multiples of 15 degrees, exact to f32).
fn tyre_points(radius: f32, width: f32, up: Vector, forward: Vector, axle: Vector) -> Vec<[f32; 3]> {
    const C: [f32; 7] = [1.0, 0.965_925_8, 0.866_025_4, 0.707_106_77, 0.5, 0.258_819_04, 0.0];
    let mut out = Vec::with_capacity(48);
    for rim in [-0.5f32, 0.5] {
        for k in 0..24usize {
            // (cos and sin of k x 15 degrees from the table by quadrant.)
            let (q, i) = (k / 6, k % 6);
            let (c, s) = match q {
                0 => (C[i], C[6 - i]),
                1 => (-C[6 - i], C[i]),
                2 => (-C[i], -C[6 - i]),
                _ => (C[6 - i], -C[i]),
            };
            let p = axle * (rim * width) + (up * c + forward * s) * radius;
            out.push(p.to_array());
        }
    }
    out
}

impl Physics {
    /// Adds a vehicle on a dynamic body; Err (why) if the body is unknown or not dynamic, or a number is out of its
    /// bounds.
    pub fn add_vehicle(&mut self, d: &VehicleDesc) -> Result<VehicleId, String> {
        if self.vehicles.len() >= MAX_VEHICLES {
            return Err(format!("physics: at most {MAX_VEHICLES} vehicles"));
        }
        let hb = *self.bodies.get(&d.chassis).ok_or("physics: unknown body")?;
        if !self.world.bodies[hb].is_dynamic() {
            return Err("physics: a vehicle's chassis must be a dynamic body".into());
        }
        if d.wheels.is_empty() || d.wheels.len() > MAX_WHEELS {
            return Err(format!("physics: a vehicle has 1 to {MAX_WHEELS} wheels"));
        }
        if d.forward_axis > 2 || d.up_axis > 2 || d.forward_axis == d.up_axis {
            return Err("physics: a vehicle's forward and up axes are two of 0, 1, 2".into());
        }
        let mut wheels = vec![];
        for w in &d.wheels {
            within(&w.anchor, super::MAX_SIZE, "a wheel's anchor")?;
            let dir = unit(w.direction, "a wheel's direction")?;
            let axle = unit(w.axle, "a wheel's axle")?;
            if dir.cross(axle).length() < 0.5 {
                return Err("physics: a wheel's axle must be across its direction".into());
            }
            between(w.radius, WHEEL_RADIUS[0], WHEEL_RADIUS[1], "a wheel's radius")?;
            between(w.travel, 0.0, MAX_TRAVEL, "a wheel's travel")?;
            between(w.rest, w.travel, MAX_TRAVEL + w.travel, "a wheel's rest length (at least its travel)")?;
            between(w.width, 0.01, 2.0 * w.radius, "a wheel's width (at least 1 cm)")?;
            for (x, what) in [(w.stiffness, "stiffness"), (w.compression, "compression damping"), (w.relaxation, "relaxation damping"), (w.bump_stiffness, "bump stiffness")] {
                between(x, 0.0, super::MAX_FORCE, what)?;
            }
            between(w.max_force, 0.0, super::MAX_FORCE, "a wheel's largest force")?;
            between(w.friction_slip, 0.0, super::MAX_FRICTION, "a wheel's friction")?;
            between(w.side_stiffness, 0.0, super::MAX_FRICTION, "a wheel's side stiffness")?;
            between(w.bump_length, 0.0, w.travel, "a wheel's bump stop length (at most its travel)")?;
            between(w.stop_inset, 0.0, w.radius * 0.5, "a wheel's stop inset")?;
            wheels.push((w, dir, axle));
        }
        let com = match d.center_of_mass {
            Some(c) => {
                within(&c, super::MAX_SIZE, "a vehicle's centre of mass")?;
                Some(v(c))
            }
            None => None,
        };
        let mut ctl = DynamicRayCastVehicleController::new(hb);
        ctl.index_forward_axis = d.forward_axis;
        ctl.index_up_axis = d.up_axis;
        let g = self.world.gravity;
        ctl.up = if g.length() > 0.0 { -g.normalize() } else { Vector::Y };
        let (mut stops, mut stop_frames, mut frames) = (vec![], vec![], vec![]);
        for (w, dir, axle) in wheels {
            frames.push((dir, axle));
            let t = WheelTuning {
                suspension_stiffness: w.stiffness,
                suspension_compression: w.compression,
                suspension_damping: w.relaxation,
                max_suspension_travel: w.travel,
                side_friction_stiffness: w.side_stiffness,
                friction_slip: w.friction_slip,
                max_suspension_force: w.max_force,
                bump_length: w.bump_length,
                bump_stiffness: w.bump_stiffness,
            };
            ctl.add_wheel(v(w.anchor), dir, axle, w.rest, w.radius, &t);
            // The stop: the tyre where the wheel is fully compressed, `stop_inset` inside it; no mass, no friction of
            // its own (the hook gives its contacts the tyre's).
            let forward = -dir.cross(axle).normalize();
            let at = v(w.anchor) + dir * (w.rest - w.travel);
            let mut stop = ColliderDesc::new(Shape::ConvexHull(tyre_points(w.radius - w.stop_inset, w.width, -dir, forward, axle.normalize())));
            stop.position = at.to_array();
            stop.density = 0.0;
            stop.friction = 0.0;
            // (A stop that cannot be made takes the ones made before it with it: a refused vehicle leaves nothing.)
            let id = match self.add_collider(Some(d.chassis), &stop) {
                Ok(id) => id,
                Err(e) => {
                    for s in stops {
                        self.remove_collider(s);
                    }
                    return Err(e);
                }
            };
            let h = self.colliders[&id].0;
            self.world.colliders[h].set_active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
            stops.push(id);
            stop_frames.push((at, -dir));
        }
        let own_additional = self.own_additional(hb);
        let n = stops.len();
        let id = self.id();
        self.vehicles.insert(id, Vehicle { chassis: d.chassis, ctl, stops, stop_frames, stop_steering: vec![0.0; n], slots: (0..n).map(Some).collect(), lost_at: vec![None; n], frames, center_of_mass: com, own_additional, held: vec![] });
        self.apply_center_of_mass(id);
        Ok(id)
    }

    /// A body's additional mass properties: its whole less its colliders'.
    fn own_additional(&self, hb: RigidBodyHandle) -> MassProperties {
        let b = &self.world.bodies[hb];
        let total = b.mass_properties().local_mprops;
        let mut shapes = MassProperties::default();
        for &c in b.colliders() {
            if let Some((co, at)) = self.world.colliders.get(c).and_then(|co| Some((co, co.position_wrt_parent()?))) {
                shapes += co.mass_properties().transform_by(at);
            }
        }
        // (None, or nearly none: exactly none, not the rounding of a subtraction.)
        if total.mass() - shapes.mass() <= 1e-6 * total.mass() {
            MassProperties::default()
        } else {
            total - shapes
        }
    }

    /// A vehicle's centre of mass override, made anew from what the chassis holds: its own additional mass properties,
    /// the colliders the override already holds, and any collider with a mass of its own, which it takes over (the
    /// collider made massless, its mass properties kept): the body's mass and inertia (about the centre of mass) as
    /// all of them sum, the centre of mass where the vehicle puts it. Rapier sums a body's colliders' and its
    /// additional mass properties, and a centre of mass cannot move by adding a massless part, so the override is all of
    /// the body's mass in its additional properties. Run again when the chassis's mass changes (`set_mass_properties`,
    /// a collider added or removed).
    pub(super) fn apply_center_of_mass(&mut self, id: VehicleId) {
        let Some(veh) = self.vehicles.get_mut(&id) else { return };
        let (Some(com), Some(&hb)) = (veh.center_of_mass, self.bodies.get(&veh.chassis)) else { return };
        // (A held collider that is gone is forgotten: its mass went with it.)
        veh.held.retain(|(c, _)| self.colliders.contains_key(c));
        let world = &mut self.world;
        let mut total = veh.own_additional;
        for &c in world.bodies[hb].colliders() {
            let Some(co) = world.colliders.get_mut(c) else { continue };
            let Some(&at) = co.position_wrt_parent() else { continue };
            let cid = co.user_data as ColliderId;
            if let Some((_, own)) = veh.held.iter().find(|(h, _)| *h == cid) {
                total += own.transform_by(&at);
            } else {
                let own = co.mass_properties();
                if own.mass() > 0.0 {
                    total += own.transform_by(&at);
                    veh.held.push((cid, own));
                    co.set_mass_properties(MassProperties::default());
                }
            }
        }
        if !(total.mass() > 0.0) {
            return;
        }
        let target = MassProperties::with_inertia_matrix(com, total.mass(), total.reconstruct_inertia_matrix());
        let b = &mut world.bodies[hb];
        b.set_additional_mass_properties(target, true);
        b.recompute_mass_properties_from_colliders(&world.colliders);
    }

    /// Removes a vehicle with its stops, and puts the chassis's own mass properties back (its colliders' and its
    /// additional ones).
    pub fn remove_vehicle(&mut self, id: VehicleId) {
        let Some(veh) = self.vehicles.remove(&id) else { return };
        for s in veh.stops {
            self.remove_collider(s);
        }
        let Some(&hb) = self.bodies.get(&veh.chassis) else { return };
        if veh.center_of_mass.is_some() {
            for (c, own) in veh.held {
                if let Some(&(h, _)) = self.colliders.get(&c) {
                    self.world.colliders[h].set_mass_properties(own);
                }
            }
            let world = &mut self.world;
            let b = &mut world.bodies[hb];
            b.set_additional_mass_properties(veh.own_additional, true);
            b.recompute_mass_properties_from_colliders(&world.colliders);
        }
    }

    /// The vehicles on a body that override its centre of mass.
    fn overriding(&self, body: BodyId) -> Vec<VehicleId> {
        self.vehicles.iter().filter(|(_, v)| v.chassis == body && v.center_of_mass.is_some()).map(|(&i, _)| i).collect()
    }

    /// After `set_mass_properties` on a chassis (`props`, its new additional mass properties): its vehicles' centre of
    /// mass again, on top of them.
    pub(super) fn vehicle_mass_set(&mut self, body: BodyId, props: MassProperties) {
        for id in self.overriding(body) {
            self.vehicles.get_mut(&id).unwrap().own_additional = props;
            self.apply_center_of_mass(id);
        }
    }

    /// After a collider was added to or removed from a body: its vehicles' centre of mass again.
    pub(super) fn vehicle_colliders_changed(&mut self, body: BodyId) {
        for id in self.overriding(body) {
            self.apply_center_of_mass(id);
        }
    }

    pub fn has_vehicle(&self, id: VehicleId) -> bool {
        self.vehicles.contains_key(&id)
    }

    /// Sets a vehicle's wheels' controls (by wheel; fewer than its wheels leave the rest as they are). Err for an
    /// unknown vehicle or a number out of its bounds.
    pub fn set_vehicle_controls(&mut self, id: VehicleId, controls: &[WheelControl]) -> Result<(), String> {
        let veh = self.vehicles.get_mut(&id).ok_or("physics: unknown vehicle")?;
        for c in controls {
            within(&[c.engine], super::MAX_FORCE, "a wheel's engine force")?;
            between(c.brake, 0.0, super::MAX_IMPULSE, "a wheel's brake")?;
            within(&[c.steering], MAX_STEERING, "a wheel's steering")?;
        }
        // (By the wheels as described: a lost wheel's controls go nowhere.)
        for (slot, c) in veh.slots.iter().zip(controls) {
            if let Some(w) = slot.and_then(|i| veh.ctl.wheels_mut().get_mut(i)) {
                w.engine_force = c.engine;
                w.brake = c.brake;
                w.steering = c.steering;
            }
        }
        if controls.iter().any(|c| c.engine != 0.0) {
            if let Some(&hb) = self.bodies.get(&veh.chassis) {
                self.world.bodies[hb].wake_up(true);
            }
        }
        Ok(())
    }

    /// A vehicle's speed and wheels (as described, a lost one included), as of the last tick.
    pub fn vehicle_state(&self, id: VehicleId) -> Option<VehicleState> {
        let veh = self.vehicles.get(&id)?;
        let b = self.body(veh.chassis)?;
        let rot = b.position().rotation;
        let forward = rot * Vector::ith(veh.ctl.index_forward_axis, 1.0);
        let wheels = veh
            .slots
            .iter()
            .enumerate()
            .map(|(k, slot)| match slot.map(|i| &veh.ctl.wheels()[i]) {
                Some(w) => {
                    let r = w.raycast_info();
                    let (dir, axle) = veh.frames[k];
                    // (Bullet's wheel turn: the steering about the suspension's up, then the spin about the axle.)
                    let q = rot * rapier3d::math::rotation_from_angle(-dir * w.steering) * rapier3d::math::rotation_from_angle(axle * -w.rotation);
                    WheelState { center: w.center().to_array(), orientation: q.to_array(), spin: w.rotation, steering: w.steering, contact: r.is_in_contact, suspension: r.suspension_length, point: r.contact_point_ws.to_array(), normal: r.contact_normal_ws.to_array(), lost: false }
                }
                None => WheelState { center: veh.lost_at[k].unwrap_or([0.0; 3]), orientation: [0.0, 0.0, 0.0, 1.0], spin: 0.0, steering: 0.0, contact: false, suspension: 0.0, point: [0.0; 3], normal: [0.0; 3], lost: true },
            })
            .collect();
        Some(VehicleState { speed: b.linvel().dot(forward), wheels })
    }

    /// A vehicle's wheels that are not lost: each one's index as described and its anchor (the hard point, in the
    /// chassis's frame). The engine finds by them what material each wheel belongs to (docs/VOXEL.md).
    pub fn vehicle_wheels(&self, id: VehicleId) -> Vec<(usize, [f32; 3])> {
        let Some(veh) = self.vehicles.get(&id) else { return vec![] };
        veh.slots.iter().enumerate().filter_map(|(k, s)| s.map(|i| (k, veh.ctl.wheels()[i].chassis_connection_point_cs.to_array()))).collect()
    }

    /// The vehicles on a body, in id order.
    pub fn vehicles_on(&self, body: BodyId) -> Vec<VehicleId> {
        self.vehicles.iter().filter(|(_, v)| v.chassis == body).map(|(&i, _)| i).collect()
    }

    /// A wheel lost (its anchor's material carved away or cut off): out of the vehicle, its stop with it; the vehicle
    /// goes on with the rest. Returns where it was (world), None if it was lost already or there is no such wheel.
    pub fn remove_wheel(&mut self, id: VehicleId, wheel: usize) -> Option<[f32; 3]> {
        let veh = self.vehicles.get_mut(&id)?;
        let i = (*veh.slots.get(wheel)?)?;
        let at = veh.ctl.wheels()[i].center().to_array();
        veh.ctl.remove_wheel(i);
        let stop = veh.stops.remove(i);
        veh.stop_frames.remove(i);
        veh.stop_steering.remove(i);
        veh.slots[wheel] = None;
        veh.lost_at[wheel] = Some(at);
        for s in veh.slots.iter_mut().flatten() {
            if *s > i {
                *s -= 1;
            }
        }
        self.remove_collider(stop);
        Some(at)
    }

    /// The chassis a vehicle is on, and its wheels' stop colliders.
    pub fn vehicle_parts(&self, id: VehicleId) -> Option<(BodyId, Vec<ColliderId>)> {
        let veh = self.vehicles.get(&id)?;
        Some((veh.chassis, veh.stops.clone()))
    }

    /// Every vehicle (in id order) probes and pushes its chassis; the stops' state for this step's hook. A vehicle
    /// whose chassis is gone is removed (`take_vehicles_gone`). The probes see the world as the calls left it, as the
    /// queries do (`fresh`). A chassis asleep is not pushed, but its wheels are probed: ground changed under it (carved,
    /// filled, moved) wakes it, and it is updated at once.
    pub(super) fn update_vehicles(&mut self) -> StopHooks {
        let mut hooks = StopHooks::default();
        if self.vehicles.is_empty() {
            return hooks;
        }
        let dt = self.world.integration_parameters.dt;
        let gone: Vec<VehicleId> = self.vehicles.iter().filter(|(_, v)| !self.bodies.contains_key(&v.chassis)).map(|(&i, _)| i).collect();
        for id in gone {
            self.remove_vehicle(id);
            self.vehicles_gone.push(id);
        }
        // The stops turned with the steering (as the last controls left it).
        let mut turned = vec![];
        for veh in self.vehicles.values_mut() {
            for (i, w) in veh.ctl.wheels().iter().enumerate() {
                if w.steering != veh.stop_steering[i] {
                    veh.stop_steering[i] = w.steering;
                    let (at, up) = veh.stop_frames[i];
                    let h = self.colliders[&veh.stops[i]].0;
                    self.world.colliders[h].set_position_wrt_parent(Pose::from_parts(at, rapier3d::math::rotation_from_angle(up * w.steering)));
                    turned.push(veh.stops[i]);
                }
            }
        }
        self.fresh.extend(turned);
        // The fresh colliders, where they are now (`fresh` in world.rs).
        let fresh: Vec<(ColliderHandle, Pose, Option<RigidBodyHandle>)> = self
            .fresh
            .iter()
            .filter_map(|id| {
                let h = self.colliders.get(id)?.0;
                let c = self.world.colliders.get(h)?;
                if !c.is_enabled() {
                    return None;
                }
                let at = match (c.parent().and_then(|p| self.world.bodies.get(p)), c.position_wrt_parent()) {
                    (Some(b), Some(local)) => *b.position() * *local,
                    _ => *c.position(),
                };
                Some((h, at, c.parent()))
            })
            .collect();
        let fresh_ids = &self.fresh;
        let settled = |_: ColliderHandle, c: &Collider| !fresh_ids.contains(&(c.user_data as u32));
        let world = &mut self.world;
        for veh in self.vehicles.values_mut() {
            let hb = self.bodies[&veh.chassis];
            let mine: Vec<(ColliderHandle, Pose)> = fresh.iter().filter(|f| f.2 != Some(hb)).map(|f| (f.0, f.1)).collect();
            let filter = QueryFilter::default().exclude_rigid_body(hb).predicate(&settled);
            if world.bodies[hb].is_sleeping() {
                let q = world.broad_phase.as_query_pipeline_mut(world.narrow_phase.query_dispatcher(), &mut world.bodies, &mut world.colliders, filter);
                if !veh.ctl.ground_changed(q, &mine) {
                    continue;
                }
                world.bodies[hb].wake_up(true);
            }
            let q = world.broad_phase.as_query_pipeline_mut(world.narrow_phase.query_dispatcher(), &mut world.bodies, &mut world.colliders, filter);
            veh.ctl.update_vehicle(dt, q, &mine);
            // The stops roll at the hub's speed.
            let b = &world.bodies[hb];
            for (w, s) in veh.ctl.wheels().iter().zip(&veh.stops) {
                let h = self.colliders[s].0;
                let at = world.colliders[h].position().translation;
                let axle = w.axle();
                let forward = w.suspension().cross(axle).normalize_or_zero() * -1.0;
                let hub = b.velocity_at_point(at).dot(forward);
                hooks.stops.insert(h, StopState { axle, forward, surface: hub, friction: w.friction_slip });
            }
        }
        hooks
    }

    /// The vehicles removed since the last call because their chassis went (removed, or taken out by the world).
    pub fn take_vehicles_gone(&mut self) -> Vec<VehicleId> {
        std::mem::take(&mut self.vehicles_gone)
    }
}

#[cfg(test)]
mod tests {
    use super::scenarios::{CELL, SEDAN};
    use super::*;
    use crate::BodyDesc;

    fn flat(p: &mut Physics) -> ColliderId {
        let cells: Vec<[i32; 3]> = (-40..40).flat_map(|x| (-60..60).flat_map(move |z| (-3..0).map(move |y| [x, y, z]))).collect();
        p.add_collider(None, &ColliderDesc { friction: 1.0, ..ColliderDesc::new(Shape::Voxels { size: CELL, cells }) }).unwrap()
    }

    fn wheels() -> Vec<WheelDesc> {
        [(-0.7, 1.4), (0.7, 1.4), (-0.7, -1.4), (0.7, -1.4)].iter().map(|&(x, z)| WheelDesc { anchor: [x, 0.0, z], rest: 0.4, travel: 0.25, radius: 0.35, stiffness: 30.0, ..Default::default() }).collect()
    }

    /// A plain box chassis, its mass its collider's density: the vehicle's centre of mass takes over (mass and inertia
    /// kept); a collider added later adds its mass with the centre of mass staying where the vehicle put it; one taken
    /// off takes its mass with it; the vehicle removed, the body's own mass properties are back.
    #[test]
    fn a_box_chassis_takes_the_vehicle_centre_of_mass() {
        let mut p = Physics::default();
        let b = p.add_body(&BodyDesc { position: [0.0, 1.0, 0.0], ..Default::default() }).unwrap();
        p.add_collider(Some(b), &ColliderDesc { density: 300.0, ..ColliderDesc::new(Shape::Box([0.8, 0.3, 2.0])) }).unwrap();
        let sorted = |mut i: [f32; 3]| {
            i.sort_by(f32::total_cmp);
            i
        };
        let (mass, com, inertia) = p.mass_properties(b).unwrap();
        let inertia = sorted(inertia);
        let v = p.add_vehicle(&VehicleDesc { chassis: b, wheels: wheels(), forward_axis: 2, up_axis: 1, center_of_mass: Some([0.0, -0.2, 0.3]) }).unwrap();
        let (m, c, i) = p.mass_properties(b).unwrap();
        assert!((m - mass).abs() < 1e-3 * mass && (c[1] + 0.2).abs() < 1e-5 && (c[2] - 0.3).abs() < 1e-5, "{m} {c:?}");
        assert!(sorted(i).iter().zip(inertia).all(|(a, b)| (a - b).abs() < 1e-3 * b), "{i:?} against {inertia:?}");
        let extra = p.add_collider(Some(b), &ColliderDesc { mass: Some(100.0), position: [0.0, 0.5, -1.0], ..ColliderDesc::new(Shape::Box([0.2, 0.2, 0.2])) }).unwrap();
        let (m2, c2, _) = p.mass_properties(b).unwrap();
        assert!((m2 - mass - 100.0).abs() < 1e-2 && (c2[2] - 0.3).abs() < 1e-5, "{m2} {c2:?}");
        p.remove_collider(extra);
        assert!((p.mass_properties(b).unwrap().0 - mass).abs() < 1e-2);
        p.remove_vehicle(v);
        let (m3, c3, i3) = p.mass_properties(b).unwrap();
        assert!((m3 - mass).abs() < 1e-3 * mass && c3.iter().zip(com).all(|(a, b)| (a - b).abs() < 1e-5) && sorted(i3).iter().zip(inertia).all(|(a, b)| (a - b).abs() < 1e-3 * b), "{m3} {c3:?} {i3:?}");
    }

    /// The stops turn with the steering, about the suspension's axis, as the wheels do.
    #[test]
    fn the_stops_turn_with_the_steering() {
        let mut p = Physics::default();
        flat(&mut p);
        let (_, v) = SEDAN.add(&mut p, [0.0, 0.0, 0.0], false);
        let c: Vec<WheelControl> = (0..4).map(|i| WheelControl { steering: if i < 2 { 0.5 } else { 0.0 }, ..Default::default() }).collect();
        p.set_vehicle_controls(v, &c).unwrap();
        p.step();
        let (_, stops) = p.vehicle_parts(v).unwrap();
        let turn = |s: ColliderId| {
            let h = p.colliders[&s].0;
            let r = p.world.colliders[h].position_wrt_parent().unwrap().rotation;
            r.to_scaled_axis()
        };
        assert!((turn(stops[0]) - Vector::new(0.0, 0.5, 0.0)).length() < 1e-5, "{:?}", turn(stops[0]));
        assert!(turn(stops[2]).length() < 1e-6);
    }

    /// A car at rest, its ground carved away under it: the probes see the carve at once (no tick late), and the car
    /// falls; awake, and asleep (a car at rest goes to sleep: the ground changed under it wakes it).
    #[test]
    fn a_car_falls_when_its_ground_is_carved() {
        for sleep in [false, true] {
            let mut p = Physics::default();
            let ground = flat(&mut p);
            let (b, v) = SEDAN.add(&mut p, [0.0, 0.0, 0.0], sleep);
            for _ in 0..300 {
                p.step();
            }
            assert_eq!(p.sleeping(b), sleep, "asleep after 5 s: {}", p.sleeping(b));
            let clear: Vec<[i32; 3]> = (-15..15).flat_map(|x| (-30..30).flat_map(move |z| (-3..0).map(move |y| [x, y, z]))).collect();
            p.edit_voxels(ground, &clear, &[]).unwrap();
            p.step();
            assert!(p.vehicle_state(v).unwrap().wheels.iter().all(|w| !w.contact), "sleep {sleep}: a wheel still on the carved ground");
            for _ in 0..30 {
                p.step();
            }
            assert!(p.pose(b).unwrap().0[1] < -0.5, "sleep {sleep}: the car did not fall: {:?}", p.pose(b));
        }
    }

    /// Ground added under a wheel that hangs over a hole: the probe sees the new collider in the very tick it is added
    /// (the broad phase does not have it until that tick's step).
    #[test]
    fn a_wheel_finds_ground_added_this_tick() {
        let mut p = Physics::default();
        let cells: Vec<[i32; 3]> = (-40..40).flat_map(|x| (-60..60).flat_map(move |z| (-3..0).map(move |y| [x, y, z]))).filter(|c| !(c[0] >= -12 && c[0] < -4 && c[2] >= 10 && c[2] < 18)).collect();
        p.add_collider(None, &ColliderDesc { friction: 1.0, ..ColliderDesc::new(Shape::Voxels { size: CELL, cells }) }).unwrap();
        let (_, v) = SEDAN.add(&mut p, [0.0, 0.0, 0.0], false);
        for _ in 0..120 {
            p.step();
        }
        assert!(!p.vehicle_state(v).unwrap().wheels[0].contact, "the front left wheel hangs over the hole");
        let fill = ColliderDesc { position: [-0.8, -0.15, 1.4], ..ColliderDesc::new(Shape::Box([0.4, 0.15, 0.4])) };
        p.add_collider(None, &fill).unwrap();
        p.step();
        assert!(p.vehicle_state(v).unwrap().wheels[0].contact, "the wheel did not find the ground added this tick");
    }

    /// A car asleep, pushed by a blast: it wakes and moves.
    #[test]
    fn a_blast_wakes_a_car_at_rest() {
        let mut p = Physics::default();
        flat(&mut p);
        let (b, _) = SEDAN.add(&mut p, [0.0, 0.0, 0.0], true);
        for _ in 0..300 {
            p.step();
        }
        assert!(p.sleeping(b));
        let before = p.pose(b).unwrap().0;
        assert!(p.blast([3.0, 0.5, 0.0], 3.0, 3000.0, 0.0, &[]).unwrap() > 0);
        for _ in 0..30 {
            p.step();
        }
        let after = p.pose(b).unwrap().0;
        assert!(!p.sleeping(b) || (after[0] - before[0]).abs() > 0.05, "{before:?} -> {after:?}");
        assert!((after[0] - before[0]).abs() > 0.05, "the blast did not move it: {before:?} -> {after:?}");
    }

    /// A chassis removed takes its vehicle with it, reported once.
    #[test]
    fn a_vehicle_goes_with_its_chassis() {
        let mut p = Physics::default();
        flat(&mut p);
        let (b, v) = SEDAN.add(&mut p, [0.0, 0.0, 0.0], false);
        p.step();
        p.remove_body(b);
        p.step();
        assert!(!p.has_vehicle(v));
        assert_eq!(p.take_vehicles_gone(), vec![v]);
        assert!(p.take_vehicles_gone().is_empty());
        assert!(p.set_vehicle_controls(v, &[]).is_err() && p.vehicle_state(v).is_none());
    }

    /// Numbers out of bounds are refused before anything is made: NaN, infinities, 0 and huge radii, travel, nine
    /// wheels, an axle along the suspension, a fixed chassis, wild controls.
    #[test]
    fn a_vehicle_out_of_bounds_is_refused() {
        let mut p = Physics::default();
        let b = p.add_body(&BodyDesc::default()).unwrap();
        p.add_collider(Some(b), &ColliderDesc::new(Shape::Box([1.0, 0.3, 2.0]))).unwrap();
        let fixed = p.add_body(&BodyDesc { kind: crate::BodyKind::Fixed, ..Default::default() }).unwrap();
        let ok = VehicleDesc { chassis: b, wheels: wheels(), forward_axis: 2, up_axis: 1, center_of_mass: None };
        let colliders = p.colliders_of(b).len();
        let bad: Vec<VehicleDesc> = vec![
            VehicleDesc { wheels: vec![WheelDesc { radius: f32::NAN, ..Default::default() }], ..ok.clone() },
            VehicleDesc { wheels: vec![WheelDesc { radius: 0.0, ..Default::default() }], ..ok.clone() },
            VehicleDesc { wheels: vec![WheelDesc { radius: 1e9, ..Default::default() }], ..ok.clone() },
            VehicleDesc { wheels: vec![WheelDesc { travel: f32::INFINITY, ..Default::default() }], ..ok.clone() },
            VehicleDesc { wheels: vec![WheelDesc { axle: [0.0, -1.0, 0.0], ..Default::default() }], ..ok.clone() },
            VehicleDesc { wheels: vec![WheelDesc { stiffness: -1.0, ..Default::default() }], ..ok.clone() },
            VehicleDesc { wheels: vec![WheelDesc::default(), WheelDesc { width: 0.0, ..Default::default() }], ..ok.clone() },
            VehicleDesc { wheels: vec![WheelDesc::default(); 9], ..ok.clone() },
            VehicleDesc { wheels: vec![], ..ok.clone() },
            VehicleDesc { forward_axis: 1, ..ok.clone() },
            VehicleDesc { center_of_mass: Some([f32::NAN, 0.0, 0.0]), ..ok.clone() },
            VehicleDesc { chassis: fixed, ..ok.clone() },
            VehicleDesc { chassis: 999, ..ok.clone() },
        ];
        for d in &bad {
            assert!(p.add_vehicle(d).is_err(), "{d:?}");
        }
        assert_eq!(p.colliders_of(b).len(), colliders, "a refused vehicle left a stop behind");
        let v = p.add_vehicle(&ok).unwrap();
        for c in [WheelControl { engine: f32::NAN, ..Default::default() }, WheelControl { brake: -1.0, ..Default::default() }, WheelControl { steering: 2.0, ..Default::default() }, WheelControl { engine: f32::INFINITY, ..Default::default() }] {
            assert!(p.set_vehicle_controls(v, &[c]).is_err(), "{c:?}");
        }
    }
}

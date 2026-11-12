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
use rapier3d::math::{Real, Vector};
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
    /// The wheel's centre (world), its spin about the axle (rad), on the ground or not, the suspension's length.
    pub center: [f32; 3],
    pub rotation: f32,
    pub contact: bool,
    pub suspension: f32,
    /// Where it touches and the ground's normal (world; meaningful with `contact`).
    pub point: [f32; 3],
    pub normal: [f32; 3],
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
    /// Per wheel: its stop collider.
    stops: Vec<ColliderId>,
    center_of_mass: Option<Vector>,
    /// The chassis's own mass properties before the override (put back on removal).
    own_mass: Option<MassProperties>,
}

/// A stop collider's state for the hook: the wheel's axle and forward (world), and the speed of its surface along the
/// forward (the hub's).
struct StopState {
    axle: Vector,
    forward: Vector,
    surface: Real,
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
        *m.friction = 1.0;
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
            between(w.width, 0.0, 2.0 * w.radius, "a wheel's width")?;
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
        let own_mass = match com {
            Some(c) => Some(self.override_center_of_mass(hb, c)?),
            None => None,
        };
        let mut ctl = DynamicRayCastVehicleController::new(hb);
        ctl.index_forward_axis = d.forward_axis;
        ctl.index_up_axis = d.up_axis;
        let g = self.world.gravity;
        ctl.up = if g.length() > 0.0 { -g.normalize() } else { Vector::Y };
        let mut stops = vec![];
        for (w, dir, axle) in wheels {
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
            let id = self.add_collider(Some(d.chassis), &stop)?;
            let h = self.colliders[&id].0;
            self.world.colliders[h].set_active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
            stops.push(id);
        }
        let id = self.id();
        self.vehicles.insert(id, Vehicle { chassis: d.chassis, ctl, stops, center_of_mass: com, own_mass });
        Ok(id)
    }

    /// The chassis's mass properties with the centre of mass at `com` (body frame), its mass and inertia (about the
    /// centre of mass) kept: Rapier sums the colliders' and the body's additional mass properties, so the additional
    /// ones are set to what is needed on top of the colliders'. That needs mass in the additional ones (a voxel body's,
    /// `set_mass_properties`): a centre of mass cannot move by adding no mass. Returns the body's own properties
    /// before.
    fn override_center_of_mass(&mut self, hb: RigidBodyHandle, com: Vector) -> Result<MassProperties, String> {
        let world = &mut self.world;
        let b = &world.bodies[hb];
        let total = b.mass_properties().local_mprops;
        let mut shapes = MassProperties::default();
        for &c in b.colliders() {
            if let Some(co) = world.colliders.get(c) {
                if let Some(at) = co.position_wrt_parent() {
                    shapes += co.mass_properties().transform_by(at);
                }
            }
        }
        if !(total.mass() - shapes.mass() > 1e-3 * total.mass().max(1e-6)) {
            return Err("physics: a vehicle's centre of mass needs the chassis's mass given by `set_mass_properties` (its colliders without density)".into());
        }
        let target = MassProperties::with_inertia_matrix(com, total.mass(), total.reconstruct_inertia_matrix());
        let b = &mut world.bodies[hb];
        b.set_additional_mass_properties(target - shapes, true);
        b.recompute_mass_properties_from_colliders(&world.colliders);
        Ok(total - shapes)
    }

    /// Removes a vehicle with its stops, and puts the chassis's own centre of mass back.
    pub fn remove_vehicle(&mut self, id: VehicleId) {
        let Some(veh) = self.vehicles.remove(&id) else { return };
        for s in veh.stops {
            self.remove_collider(s);
        }
        if let (Some(own), Some(&hb)) = (veh.own_mass, self.bodies.get(&veh.chassis)) {
            let world = &mut self.world;
            let b = &mut world.bodies[hb];
            b.set_additional_mass_properties(own, true);
            b.recompute_mass_properties_from_colliders(&world.colliders);
        }
    }

    /// After `set_mass_properties` on a chassis: its vehicles' centre of mass again on top of the new properties.
    pub(super) fn vehicle_mass_changed(&mut self, body: BodyId) {
        let Some(&hb) = self.bodies.get(&body) else { return };
        let ids: Vec<VehicleId> = self.vehicles.iter().filter(|(_, v)| v.chassis == body && v.center_of_mass.is_some()).map(|(&i, _)| i).collect();
        for id in ids {
            let com = self.vehicles[&id].center_of_mass.unwrap();
            if let Ok(own) = self.override_center_of_mass(hb, com) {
                self.vehicles.get_mut(&id).unwrap().own_mass = Some(own);
            }
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
        for (w, c) in veh.ctl.wheels_mut().iter_mut().zip(controls) {
            w.engine_force = c.engine;
            w.brake = c.brake;
            w.steering = c.steering;
        }
        if controls.iter().any(|c| c.engine != 0.0) {
            if let Some(&hb) = self.bodies.get(&veh.chassis) {
                self.world.bodies[hb].wake_up(true);
            }
        }
        Ok(())
    }

    /// A vehicle's speed and wheels, as of the last tick.
    pub fn vehicle_state(&self, id: VehicleId) -> Option<VehicleState> {
        let veh = self.vehicles.get(&id)?;
        let b = self.body(veh.chassis)?;
        let forward = b.position().rotation * Vector::ith(veh.ctl.index_forward_axis, 1.0);
        Some(VehicleState {
            speed: b.linvel().dot(forward),
            wheels: veh
                .ctl
                .wheels()
                .iter()
                .map(|w| {
                    let r = w.raycast_info();
                    WheelState { center: w.center().to_array(), rotation: w.rotation, contact: r.is_in_contact, suspension: r.suspension_length, point: r.contact_point_ws.to_array(), normal: r.contact_normal_ws.to_array() }
                })
                .collect(),
        })
    }

    /// The chassis a vehicle is on, and its wheels' stop colliders.
    pub fn vehicle_parts(&self, id: VehicleId) -> Option<(BodyId, Vec<ColliderId>)> {
        let veh = self.vehicles.get(&id)?;
        Some((veh.chassis, veh.stops.clone()))
    }

    /// Every vehicle (in id order) probes and pushes its chassis; the stops' state for this step's hook. A vehicle
    /// whose chassis is gone is removed (`take_vehicles_gone`).
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
        let world = &mut self.world;
        for veh in self.vehicles.values_mut() {
            let hb = self.bodies[&veh.chassis];
            if world.bodies[hb].is_sleeping() {
                continue;
            }
            let filter = QueryFilter::default().exclude_rigid_body(hb);
            let q = world.broad_phase.as_query_pipeline_mut(world.narrow_phase.query_dispatcher(), &mut world.bodies, &mut world.colliders, filter);
            veh.ctl.update_vehicle(dt, q);
            // The stops roll at the hub's speed.
            let b = &world.bodies[hb];
            for (w, s) in veh.ctl.wheels().iter().zip(&veh.stops) {
                let h = self.colliders[s].0;
                let at = world.colliders[h].position().translation;
                let axle = w.axle();
                let forward = w.suspension().cross(axle).normalize_or_zero() * -1.0;
                let hub = b.velocity_at_point(at).dot(forward);
                hooks.stops.insert(h, StopState { axle, forward, surface: hub });
            }
        }
        hooks
    }

    /// The vehicles removed since the last call because their chassis went (removed, or taken out by the world).
    pub fn take_vehicles_gone(&mut self) -> Vec<VehicleId> {
        std::mem::take(&mut self.vehicles_gone)
    }
}

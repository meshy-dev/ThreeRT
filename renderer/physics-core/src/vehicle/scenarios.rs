//! The vehicle's scenarios on destroyed voxel ground (step 10 c's wheel comparison, `out/web-probe/step10/
//! c-wheel-compare.md`, run on this module): debris, narrow gaps, craters, steps, thin posts and rails, wheels starting
//! in the ground, a drop, a car parked on lattice planes. The car is a mid-size sedan (`sedan`). Stability, the tyre's
//! true contact with the ground, speed, the per-step cost and the world's hash. Not part of the engine's API: the
//! scenarios' tests and `examples/vehicles.rs` use it.

use super::{VehicleDesc, VehicleId, WheelControl, WheelDesc};
use crate::{BodyDesc, ColliderDesc, Physics, Shape};
use rapier3d::parry::query::contact;
use rapier3d::parry::shape::{Cuboid, Cylinder};
use rapier3d::prelude::*;

pub const CELL: f32 = 0.1;

/// The car: a 2018 Toyota Camry LE (the eighth generation, XV70), the figures the maker publishes and what they
/// imply. Where a figure is ours, it says so.
pub mod sedan {
    /// Curb weight 3,296 lb.
    pub const MASS: f32 = 1495.0;
    /// Wheelbase 111.2 in; track 63 in, front and rear.
    pub const WHEELBASE: f32 = 2.825;
    pub const TRACK: f32 = 1.60;
    /// Tyres 215/55 R17: 17 in + 2 x 55 % of 215 mm across, 215 mm wide.
    pub const RADIUS: f32 = 0.334;
    pub const WIDTH: f32 = 0.215;
    /// Body 4.885 x 1.840 m (the voxel body: 4.8 x 1.8 m, 0.1 m cells), roof at 1.445 m, ground clearance 0.15 m.
    /// The centre of mass: 0.55 m above the ground (ours: the static stability factor T / 2h of mid-size sedans rated
    /// five stars for rollover is about 1.4-1.5, so h = 1.60 / (2 x 1.45)), 59 % of the weight on the front axle (ours:
    /// a front-engined, front-driven sedan's usual share), so 0.41 x 2.825 = 1.158 m behind the front axle.
    pub const COM_HEIGHT: f32 = 0.55;
    pub const FRONT_SHARE: f32 = 0.59;
    /// The approach and departure angles (ours: a sedan's usual 15 degrees; rad).
    pub const ANGLE: f32 = 0.261_799_4;
    /// Roll, pitch and yaw inertia about the centre of mass (kg m²; ours: of the order measured for mid-size sedans).
    pub const INERTIA: [f32; 3] = [550.0, 2500.0, 2700.0];
    /// The suspension (ours, from passenger-car practice): ride frequency 1.5 Hz front and rear, so each axle's spring
    /// carries its share with the same static sag g / (2 pi 1.5)² = 0.110 m; damping ratios 0.25 in compression and
    /// 0.4 in extension; from the unloaded length 0.30 m, 0.22 m of compression travel (11 cm of jounce above the ride
    /// height); the bump stop over the last 4 cm.
    pub const RIDE_HZ: f32 = 1.5;
    pub const REST: f32 = 0.30;
    pub const TRAVEL: f32 = 0.22;
    pub const ZETA: [f32; 2] = [0.25, 0.4];
    /// The bump stop's curve (ours): stiffness rising with the square of the compression into it, k(x) = K (x / L)²,
    /// to K = 1.6e6 N/m at the wheel at full compression, L = 4 cm. A passenger car's microcellular polyurethane bump
    /// stop is of that order: about 50 N/mm at first, over 2,000 N/mm in its last 5 mm; at the wheel (a motion ratio of
    /// about 0.9, squared) about 1,600 N/mm. Full compression then holds K L / 3 = 21 kN, 5.8 x the static load of a
    /// front wheel, and the stop takes K L² / 12 = 213 J.
    pub const BUMP_LENGTH: f32 = 0.04;
    pub const BUMP_RATE: f32 = 1.6e6;
    /// Dry asphalt's friction.
    pub const FRICTION: f32 = 1.0;
}

/// The chassis's frame: x left to right is -x to +x (the axle along -x), y up, z forward; its origin on the ground at
/// the ride height, halfway between the axles.
pub fn sedan_wheels() -> Vec<WheelDesc> {
    use sedan::*;
    let g = 9.81f32;
    let omega = 2.0 * core::f32::consts::PI * RIDE_HZ;
    let sag = g / (omega * omega);
    let hard = RADIUS + REST - sag;
    let half = WHEELBASE / 2.0;
    let mut out = vec![];
    for (z, share) in [(half, FRONT_SHARE / 2.0), (-half, (1.0 - FRONT_SHARE) / 2.0)] {
        for x in [-TRACK / 2.0, TRACK / 2.0] {
            // Bullet's stiffness is per kg of the whole chassis: a wheel carrying `share` of the weight sags by `sag`
            // at share x g / sag.
            let stiffness = share * g / sag;
            // Damping per kg: c = 2 zeta sqrt(k m) for the wheel's spring rate k and its share of the mass m.
            let crit = 2.0 * (stiffness * share).sqrt();
            out.push(WheelDesc {
                anchor: [x, hard, z],
                direction: [0.0, -1.0, 0.0],
                axle: [-1.0, 0.0, 0.0],
                rest: REST,
                travel: TRAVEL,
                radius: RADIUS,
                width: WIDTH,
                stiffness,
                compression: ZETA[0] * crit,
                relaxation: ZETA[1] * crit,
                friction_slip: FRICTION,
                side_stiffness: 1.0,
                max_force: 1e6,
                bump_length: BUMP_LENGTH,
                bump_stiffness: BUMP_RATE / MASS,
                stop_inset: 0.025,
            });
        }
    }
    out
}

/// The chassis's cells (collider at the origin of the chassis's frame plus (0, 0.05, 0): the body's bottom at 0.15
/// m): the lower body 1.8 x 0.7 x 4.8 m with a wheel well round each wheel (the outer 0.3 m, 1 m long, its whole
/// height), its overhangs' underside rising at 15 degrees, and a cabin 1.6 x 0.6 x 2.2 m on it.
fn sedan_cells() -> Vec<[i32; 3]> {
    let half = sedan::WHEELBASE / 2.0;
    let mut cells = vec![];
    for z in -24..24 {
        for x in -9..9 {
            for y in 1..14 {
                let (cx, cz) = ((x as f32 + 0.5) * CELL, (z as f32 + 0.5) * CELL);
                let cabin = y >= 8;
                if cabin && !((-8..8).contains(&x) && (-12..10).contains(&z)) {
                    continue;
                }
                let well = cx.abs() > 0.6 && ((cz - half).abs() < 0.5 || (cz + half).abs() < 0.5);
                if !cabin && well {
                    continue;
                }
                // The overhangs' underside rises at the approach and departure angle (15 degrees: the line from the
                // ground tangent to the tyre, `ANGLE`), so the bumpers' bottoms are 0.25 m up, not 0.15.
                let bottom = y as f32 * CELL + 0.05;
                let outer = (cz.abs() + CELL / 2.0) - (half + sedan::RADIUS * (sedan::ANGLE / 2.0).tan());
                if outer > 0.0 && bottom < outer * sedan::ANGLE.tan() {
                    continue;
                }
                cells.push([x, y, z]);
            }
        }
    }
    cells
}

/// A sedan at `at` (its frame's origin), its centre of mass where the sedan's is.
pub fn add_sedan(p: &mut Physics, at: [f32; 3]) -> (u32, VehicleId) {
    let b = p.add_body(&BodyDesc { position: at, can_sleep: false, ..Default::default() }).unwrap();
    let cells = sedan_cells();
    let n = cells.len() as f32;
    let centroid = cells.iter().fold([0.0f32; 3], |a, c| [a[0] + (c[0] as f32 + 0.5) * CELL / n, a[1] + ((c[1] as f32 + 0.5) * CELL + 0.05) / n, a[2] + (c[2] as f32 + 0.5) * CELL / n]);
    let mut d = ColliderDesc::new(Shape::Voxels { size: CELL, cells });
    d.position = [0.0, 0.05, 0.0];
    d.density = 0.0;
    d.friction = 0.8;
    p.add_collider(Some(b), &d).unwrap();
    // The body's mass as a voxel body's is given (`set_mass_properties`): its cells' centroid; the vehicle's centre
    // of mass then replaces it.
    let [roll, pitch, yaw] = sedan::INERTIA;
    p.set_mass_properties(b, sedan::MASS, centroid, [pitch, yaw, roll, 0.0, 0.0, 0.0]).unwrap();
    let com_z = sedan::WHEELBASE / 2.0 - (1.0 - sedan::FRONT_SHARE) * sedan::WHEELBASE;
    let v = p.add_vehicle(&VehicleDesc { chassis: b, wheels: sedan_wheels(), forward_axis: 2, up_axis: 1, center_of_mass: Some([0.0, sedan::COM_HEIGHT, com_z]) }).unwrap();
    (b, v)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sc {
    /// Debris on the road: fixed chunks of voxels (10-30 cm), or the same chunks as voxel bodies (asleep when met).
    DebrisFixed,
    DebrisBodies,
    /// The fixed debris no higher than 0.2 m, and the full debris at 4 m/s.
    DebrisLow,
    DebrisSlow,
    /// The fixed debris one cell high (0.1 m).
    DebrisTravel,
    /// Trenches across the road 0.1, 0.2, 0.3, 0.5 m wide, square to it, or at 30 degrees.
    GapsStraight,
    GapsOblique,
    /// A 1 m and a 4 m crater (an explosion's: a hemisphere), the right wheels over its edge.
    Rim1,
    Rim4,
    /// Out of a 1 m crater (the rear left wheel in it) and out of the bottom of a 4 m one (the sedan, 4.8 m long, rests
    /// on its bumpers against the walls of that one, its wheels off the ground; in a 6 m one too) and of an 8 m one (on its
    /// wheels, the bumpers 5 cm clear of the walls): at a
    /// full 1 g of drive the steep wall is not climbed.
    Out1,
    Out4,
    Out8,
    /// A step up of 0.1, 0.2, 0.3 m.
    Step(u8),
    /// Posts of 0.1 x 0.1 x 0.3 m and rails 0.1 m wide, 0.2 high, beside each wheel's centre line (0.05 m off it):
    /// inside the tyre's width.
    Posts,
    /// The probes starting in the ground (the chassis 0.22 m down: each wheel's ball 3 cm into the ground, the body's
    /// bottom 7 cm); dropped from 2.5 m.
    SunkStart,
    Drop,
    /// Parked for 10 s with the wheels' hard points on lattice planes (x = +-0.8; the rear axle at z = -1.4).
    Parked,
}

impl Sc {
    pub fn all() -> Vec<Sc> {
        vec![Sc::DebrisFixed, Sc::DebrisBodies, Sc::DebrisLow, Sc::DebrisSlow, Sc::DebrisTravel, Sc::GapsStraight, Sc::GapsOblique, Sc::Rim1, Sc::Rim4, Sc::Out1, Sc::Out4, Sc::Out8, Sc::Step(1), Sc::Step(2), Sc::Step(3), Sc::Posts, Sc::SunkStart, Sc::Drop, Sc::Parked]
    }
    pub fn name(self) -> String {
        match self {
            Sc::Step(h) => format!("step {:.1} m", h as f32 * 0.1),
            other => format!("{other:?}"),
        }
    }
    /// Target speed (m/s), seconds, drive (the drive's largest force over the weight).
    pub fn plan(self) -> (f32, f32, f32) {
        match self {
            Sc::DebrisFixed | Sc::DebrisBodies | Sc::DebrisLow | Sc::DebrisTravel => (8.0, 9.0, 0.4),
            Sc::DebrisSlow => (4.0, 16.0, 0.4),
            Sc::GapsStraight | Sc::GapsOblique => (8.0, 10.0, 0.4),
            Sc::Rim1 | Sc::Rim4 | Sc::Step(_) => (8.0, 5.0, 0.4),
            Sc::Out1 => (5.0, 6.0, 0.6),
            Sc::Out4 | Sc::Out8 => (6.0, 10.0, 1.0),
            Sc::Posts => (8.0, 10.0, 0.4),
            Sc::SunkStart => (5.0, 6.0, 0.4),
            Sc::Drop => (0.0, 5.0, 0.4),
            Sc::Parked => (0.0, 10.0, 0.4),
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The debris chunks: corner cell and size in cells (1-3 each way), not touching, on the road from 15 to 55 m, across
/// `half` m of it.
fn chunks(half: f32, tallest: i32) -> Vec<([i32; 3], [i32; 3])> {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let mut taken = std::collections::BTreeSet::new();
    let mut out = vec![];
    let n = (half * 75.0) as usize;
    let span = (half * 2.0 / CELL) as u64;
    for _ in 0..n * 3 {
        if out.len() >= n {
            break;
        }
        let size = [1 + r.below(3) as i32, 1 + (r.below(3) as i32).min(tallest - 1), 1 + r.below(3) as i32];
        let at = [r.below(span) as i32 - (half / CELL) as i32, 0, 150 + r.below(400) as i32];
        let cells: Vec<[i32; 2]> = (0..size[0]).flat_map(|x| (0..size[2]).map(move |z| [at[0] + x, at[2] + z])).collect();
        if cells.iter().any(|c| (-1..=1).any(|dx| (-1..=1).any(|dz| taken.contains(&[c[0] + dx, c[1] + dz])))) {
            continue;
        }
        taken.extend(cells);
        out.push((at, size));
    }
    out
}

/// The ground: one fixed voxel collider (cell i spans i x 0.1 .. (i + 1) x 0.1), top at y = 0, x from -half to half,
/// z from -10 to 80 m, 0.3 m thick.
fn ground(p: &mut Physics, sc: Sc, half: f32) {
    let (x0, x1) = (-(half / CELL) as i32, (half / CELL) as i32);
    let (z0, z1) = (-100, 800);
    let w = sedan::TRACK / 2.0;
    let crater = match sc {
        Sc::Rim1 => Some(([w + 0.7, 20.0], 1.0)),
        Sc::Rim4 => Some(([w + 3.8, 20.0], 4.0)),
        Sc::Out1 => Some(([-w, -sedan::WHEELBASE / 2.0], 1.0)),
        Sc::Out4 => Some(([0.0, 0.0], 4.0)),
        Sc::Out8 => Some(([0.0, 0.0], 8.0)),
        _ => None,
    };
    let (s30, c30) = (0.5f32, 0.866_025_4f32);
    let gaps = [(150, 1), (300, 2), (450, 3), (600, 5)];
    let mut cells = Vec::new();
    for z in z0..z1 {
        for x in x0..x1 {
            let (cx, cz) = ((x as f32 + 0.5) * CELL, (z as f32 + 0.5) * CELL);
            let in_gap = match sc {
                Sc::GapsStraight => gaps.iter().any(|&(g, w)| z >= g && z < g + w),
                Sc::GapsOblique => gaps.iter().any(|&(g, w)| ((cz - g as f32 * CELL) * c30 - cx * s30).abs() < w as f32 * CELL * 0.5),
                _ => false,
            };
            if in_gap {
                continue;
            }
            let mut bottom = -3;
            if let Some((c, r)) = crater {
                if (cx - c[0]).abs() < r + 0.5 && (cz - c[1]).abs() < r + 0.5 {
                    bottom = -((r + 0.5) / CELL) as i32;
                }
            }
            let top = match sc {
                Sc::Step(h) if z >= 200 => h as i32,
                _ => 0,
            };
            for y in bottom..top {
                if let Some((c, r)) = crater {
                    let cy = (y as f32 + 0.5) * CELL;
                    if (cx - c[0]).powi(2) + cy * cy + (cz - c[1]).powi(2) < r * r {
                        continue;
                    }
                }
                cells.push([x, y, z]);
            }
            // Posts every 4 m from 12 to 44 m, beside the left and the right wheels in turn, and rails from 50 to 70
            // m beside both (the car at x = 0.05: its wheels' centre lines at x = -0.75 and 0.85).
            if sc == Sc::Posts {
                let post = (120..=440).step_by(40).enumerate().any(|(k, pz)| z == pz && x == if k % 2 == 0 { -7 } else { 9 });
                let rail = (500..700).contains(&z) && (x == -7 || x == 9);
                let h = if post { 3 } else if rail { 2 } else { 0 };
                for y in 0..h {
                    cells.push([x, y, z]);
                }
            }
        }
    }
    if matches!(sc, Sc::DebrisFixed | Sc::DebrisLow | Sc::DebrisSlow | Sc::DebrisTravel) {
        for (at, size) in chunks(if half > 10.0 { half } else { half.min(1.6) }, match sc { Sc::DebrisLow => 2, Sc::DebrisTravel => 1, _ => 3 }) {
            for x in 0..size[0] {
                for y in 0..size[1] {
                    for z in 0..size[2] {
                        cells.push([at[0] + x, y, at[2] + z]);
                    }
                }
            }
        }
    }
    let mut d = ColliderDesc::new(Shape::Voxels { size: CELL, cells });
    d.friction = 1.0;
    p.add_collider(None, &d).unwrap();
    if sc == Sc::DebrisBodies {
        for (at, size) in chunks(half.min(1.6), 3) {
            let b = p.add_body(&BodyDesc { position: [at[0] as f32 * CELL, 0.0, at[2] as f32 * CELL], ..Default::default() }).unwrap();
            let cells: Vec<[i32; 3]> = (0..size[0]).flat_map(|x| (0..size[1]).flat_map(move |y| (0..size[2]).map(move |z| [x, y, z]))).collect();
            let mut d = ColliderDesc::new(Shape::Voxels { size: CELL, cells });
            d.density = 2400.0;
            d.friction = 1.0;
            p.add_collider(Some(b), &d).unwrap();
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub hash: u64,
    /// Roll and pitch rate RMS (rad/s) after the first second.
    pub roll_pitch_rms: f32,
    /// Mean forward speed while driving (after 2 s) over the target, and at the end (m/s).
    pub speed_held: f32,
    pub final_speed: f32,
    pub flipped: bool,
    /// The chassis's largest vertical speed after the first second (m/s), and the fastest any body went.
    pub max_vy: f32,
    pub max_speed: f32,
    /// Wheel-ticks the tyre (a cylinder at the wheel's centre) is more than 3 cm into the ground, and the deepest.
    pub sunk: u32,
    pub sink_max: f32,
    /// Wheel-ticks the vehicle has no ground while the tyre at the rest length would touch it (ground missed).
    pub missed: u32,
    pub no_contact: u32,
    pub wheel_ticks: u32,
    /// Where the chassis ended (z, y), and how far it moved while parked after the first second (m).
    pub end: [f32; 2],
    pub parked_drift: f32,
    /// Per step: the vehicles' update (probes and controller) and the world's step (ms).
    pub vehicles_ms: f64,
    pub world_ms: f64,
    pub nan: bool,
    /// Wheel-ticks with the suspension in the bump stop's range, fully compressed, and with its stop collider touching
    /// something; the deepest into the bump stop's range (m).
    pub bump_ticks: u32,
    pub bottomed_ticks: u32,
    pub stop_ticks: u32,
    pub bump_max: f32,
    /// At the end: the deepest tyre (m) and the wheels in contact.
    pub end_depth: f32,
    pub end_contacts: u32,
    /// The trace (with `trace`): per tick, time, position, up, velocity, wheels in contact, suspension lengths.
    pub trace: Vec<String>,
}

/// The deepest the tyre (a cylinder of the wheel's radius and half width about the axle) at `center` goes into any
/// voxel collider not on `skip` (m; 0 if it touches nothing), against each solid cell's box (parry has no contact of
/// a cylinder with a voxel shape whole).
fn tyre_depth(p: &Physics, center: Vector, axle: Vector, skip: RigidBodyHandle) -> f32 {
    let w = &p.world;
    let cyl = Cylinder::new(sedan::WIDTH / 2.0, sedan::RADIUS);
    let pose = Pose::from_parts(center, Rotation::from_rotation_arc(Vector::Y, axle));
    let aabb = rapier3d::parry::shape::Shape::compute_aabb(&cyl, &pose);
    let mut deepest = 0.0f32;
    let q = w.query_pipeline_with_filter(QueryFilter::default().exclude_rigid_body(skip));
    for (_, c) in q.intersect_aabb_conservative(aabb) {
        let Some(v) = c.shape().as_voxels() else { continue };
        let cell = Cuboid::new(v.voxel_size() / 2.0);
        for vd in v.voxels_intersecting_local_aabb(&aabb.transform_by(&c.position().inverse())) {
            if vd.state.is_empty() {
                continue;
            }
            let at = *c.position() * Pose::from_translation(v.voxel_center(vd.grid_coords));
            if let Ok(Some(k)) = contact(&pose, &cyl, &at, &cell, 0.0) {
                deepest = deepest.max(-k.dist);
            }
        }
    }
    deepest
}

/// Runs a scenario with `n` sedans side by side (3 m apart); measures the first. `clock`: a clock for the cost
/// (seconds; none in wasm32).
pub fn run(sc: Sc, n: usize, measure: bool, trace: bool, clock: Option<&dyn Fn() -> f64>) -> Report {
    let mut p = Physics::new([0.0, -9.81, 0.0]);
    // (The road wide enough that a car turned aside by posts or loose debris stays on it: none steers.)
    let half = (3.0 * n as f32 + 2.0).max(if matches!(sc, Sc::Posts | Sc::DebrisBodies) { 12.0 } else { 4.0 });
    ground(&mut p, sc, half);
    let (target, seconds, drive_g) = sc.plan();
    let (x0, y0, z0) = match sc {
        Sc::SunkStart => (0.0, -0.22, 0.0),
        Sc::Drop => (0.0, 2.5, 0.0),
        // (The sphere's surface under the wheels: -sqrt(16 - 0.8² - 1.4125²).)
        Sc::Out4 => (0.0, -3.63, 0.0),
        // (-sqrt(64 - 0.8² - 1.4125²).)
        Sc::Out8 => (0.0, -7.81, 0.0),
        Sc::Posts => (0.05, 0.0, 0.0),
        Sc::Parked => (0.0, 0.0, 0.0125),
        _ => (0.0, 0.0, 0.0),
    };
    let cars: Vec<(u32, VehicleId)> = (0..n).map(|i| add_sedan(&mut p, [x0 + (i as f32 - (n as f32 - 1.0) / 2.0) * 3.0, y0, z0])).collect();
    p.step();
    let ticks = (seconds * 60.0) as u32;
    let mut rep = Report::default();
    let (mut rp, mut rp_n, mut v_sum, mut v_n) = (0.0f64, 0u32, 0.0f64, 0u32);
    let mut parked_at: Option<Vector> = None;
    let (mut veh_s, mut world_s) = (0.0f64, 0.0f64);
    for t in 0..ticks {
        let time = t as f32 / 60.0;
        for &(b, v) in &cars {
            let speed = p.vehicle_state(v).unwrap().speed;
            let fmax = sedan::MASS * drive_g * 9.81 / 2.0;
            let parked = target == 0.0;
            // (Front-wheel drive: the front wheels are 0 and 1.)
            let force = if parked { 0.0 } else { ((target - speed) * sedan::MASS).clamp(-fmax, fmax) };
            let brake = if parked { 500.0 } else { 0.0 };
            let c: Vec<WheelControl> = (0..4).map(|i| WheelControl { engine: if i < 2 { force } else { 0.0 }, brake, steering: 0.0 }).collect();
            p.set_vehicle_controls(v, &c).unwrap();
            let _ = b;
        }
        let t0 = clock.map(|c| c());
        let hooks = p.update_vehicles();
        let t1 = clock.map(|c| c());
        p.step_world(&hooks);
        if let (Some(c), Some(t0), Some(t1)) = (clock, t0, t1) {
            veh_s += t1 - t0;
            world_s += c() - t1;
        }
        if !measure {
            continue;
        }
        let (b0, v0) = cars[0];
        let hb = p.bodies[&b0];
        let (pos, rot, lv, av) = {
            let b = &p.world.bodies[hb];
            (b.translation(), b.position().rotation, b.linvel(), b.angvel())
        };
        if !(pos.is_finite() && lv.is_finite()) {
            rep.nan = true;
            break;
        }
        let up = rot * Vector::Y;
        let st = p.vehicle_state(v0).unwrap();
        if trace {
            rep.trace.push(format!(
                "t {time:.3} pos {:.3} {:.3} {:.3} up {:.3} {:.3} {:.3} v {:.2} {:.2} {:.2} w {:.2} {:.2} {:.2} contact {} susp {}",
                pos.x, pos.y, pos.z, up.x, up.y, up.z, lv.x, lv.y, lv.z, av.x, av.y, av.z,
                st.wheels.iter().map(|w| if w.contact { '1' } else { '0' }).collect::<String>(),
                st.wheels.iter().map(|w| format!("{:.3}", w.suspension)).collect::<Vec<_>>().join(",")
            ));
            let veh = &p.vehicles[&v0];
            rep.trace.push(format!(
                "    normals {} load {} fwd {} side {}",
                st.wheels.iter().map(|w| format!("({:.2} {:.2} {:.2})", w.normal[0], w.normal[1], w.normal[2])).collect::<Vec<_>>().join(" "),
                veh.ctl.wheels().iter().map(|w| format!("{:.0}", w.wheel_suspension_force)).collect::<Vec<_>>().join(","),
                veh.ctl.wheels().iter().map(|w| format!("{:.1}", w.forward_impulse)).collect::<Vec<_>>().join(","),
                veh.ctl.wheels().iter().map(|w| format!("{:.1}", w.side_impulse)).collect::<Vec<_>>().join(",")
            ));
        }
        rep.flipped |= up.y < 0.3;
        let fwd = rot * Vector::Z;
        let local = rot.inverse() * av;
        if time > 1.0 {
            rp += (local.x * local.x + local.z * local.z) as f64;
            rp_n += 1;
            if !(sc == Sc::Drop && time < 2.0) {
                rep.max_vy = rep.max_vy.max(lv.y.abs());
            }
        }
        if time > 2.0 && target > 0.0 {
            v_sum += lv.dot(fwd) as f64;
            v_n += 1;
        }
        for (_, b) in p.world.bodies.iter() {
            rep.max_speed = rep.max_speed.max(b.linvel().length());
        }
        rep.final_speed = lv.dot(fwd);
        rep.end = [pos.z, pos.y];
        if sc == Sc::Parked && time >= 1.0 {
            let a = *parked_at.get_or_insert(pos);
            rep.parked_drift = rep.parked_drift.max((pos - a).length());
        }
        if time > 0.5 {
            let axle = rot * Vector::new(-1.0, 0.0, 0.0);
            let down = rot * Vector::new(0.0, -1.0, 0.0);
            let veh = &p.vehicles[&v0];
            for (w, s) in veh.ctl.wheels().iter().zip(&veh.stops) {
                rep.wheel_ticks += 1;
                if w.raycast_info().is_in_contact {
                    let full = sedan::REST - sedan::TRAVEL;
                    let into = full + sedan::BUMP_LENGTH - w.raycast_info().suspension_length;
                    rep.bump_ticks += (into > 0.0) as u32;
                    rep.bump_max = rep.bump_max.max(into);
                    rep.bottomed_ticks += (w.raycast_info().suspension_length <= full) as u32;
                }
                let h = p.colliders[s].0;
                rep.stop_ticks += p.world.narrow_phase.contact_pairs_with(h).any(|c| c.has_any_active_contact()) as u32;
                let depth = tyre_depth(&p, w.center(), axle, hb);
                if depth > 0.03 {
                    rep.sunk += 1;
                }
                rep.sink_max = rep.sink_max.max(depth);
                if !w.raycast_info().is_in_contact {
                    rep.no_contact += 1;
                    let reach = w.raycast_info().hard_point_ws + down * sedan::REST;
                    if tyre_depth(&p, reach, axle, hb) > 0.0 {
                        rep.missed += 1;
                    }
                }
            }
        }
    }
    if measure && !rep.nan {
        let (b0, v0) = cars[0];
        let hb = p.bodies[&b0];
        let axle = p.world.bodies[hb].position().rotation * Vector::new(-1.0, 0.0, 0.0);
        for w in p.vehicles[&v0].ctl.wheels() {
            rep.end_depth = rep.end_depth.max(tyre_depth(&p, w.center(), axle, hb));
            rep.end_contacts += w.raycast_info().is_in_contact as u32;
        }
    }
    rep.roll_pitch_rms = if rp_n > 0 { (rp / rp_n as f64).sqrt() as f32 } else { 0.0 };
    rep.speed_held = if v_n > 0 && target > 0.0 { (v_sum / v_n as f64) as f32 / target } else { 0.0 };
    rep.vehicles_ms = veh_s * 1e3 / ticks as f64;
    rep.world_ms = world_s * 1e3 / ticks as f64;
    rep.hash = p.hash();
    rep
}

/// Whether a run passed (the rules of `out/web-probe/step10/c-wheel-compare.md`, the tall obstacles' as revised).
pub fn passed(sc: Sc, r: &Report) -> bool {
    let sound = !r.flipped && !r.nan && r.max_vy <= 3.0;
    match sc {
        Sc::Drop => sound && r.end_depth <= 0.03 && r.end_contacts == 4,
        Sc::SunkStart => sound && r.end_depth <= 0.03 && r.end_contacts == 4 && r.speed_held >= 0.8,
        Sc::Parked => sound && r.no_contact == 0 && r.parked_drift < 0.01,
        Sc::Out1 => sound && r.end[0] > 4.0 && r.sink_max <= 0.05,
        // (Its wall too steep for the tyres to hold: the car stays down there, on its wheels.)
        Sc::Out4 | Sc::Out8 => sound,
        _ => sound && r.sink_max <= 0.05 && (r.missed as f32) <= 0.02 * r.wheel_ticks.max(1) as f32 && r.speed_held >= 0.8,
    }
}

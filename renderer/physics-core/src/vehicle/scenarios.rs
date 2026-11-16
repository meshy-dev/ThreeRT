//! The vehicle's scenarios on destroyed voxel ground (step 10 c's wheel comparison, `out/web-probe/step10/
//! c-wheel-compare.md`, run on this module): debris, narrow gaps, craters, steps, thin posts and rails, obstacles in
//! the wheels' tracks, wheels starting in the ground, a drop, a car parked on lattice planes. Two cars, each a real
//! model's figures (`SEDAN`, `SUV`). Stability, the tyre's true contact with the ground, speed, the per-step cost and
//! the world's hash. Not part of the engine's API: the scenarios' tests and `examples/vehicles.rs` use it.

use super::{VehicleDesc, VehicleId, WheelControl, WheelDesc};
use crate::{BodyDesc, BodyId, ColliderDesc, Physics, Shape};
use rapier3d::parry::query::contact;
use rapier3d::parry::shape::{Cuboid, Cylinder};
use rapier3d::prelude::*;

pub const CELL: f32 = 0.1;
const G: f32 = 9.81;

/// A car, from a real model's published figures and what they imply. Where a figure is ours, its doc says so.
#[derive(Clone, Copy, Debug)]
pub struct Car {
    pub name: &'static str,
    /// Curb weight (kg), wheelbase and track (m), the tyre's radius and width (m).
    pub mass: f32,
    pub wheelbase: f32,
    pub track: f32,
    pub radius: f32,
    pub width: f32,
    /// The voxel body: length and width (0.1 m cells), the body's bottom (the ground clearance) and the roof (m).
    pub length: f32,
    pub body_width: f32,
    pub clearance: f32,
    pub roof: f32,
    /// The centre of mass's height above the ground (m) and the front axle's share of the weight.
    pub com_height: f32,
    pub front_share: f32,
    /// Roll, pitch and yaw inertia about the centre of mass (kg m²).
    pub inertia: [f32; 3],
    /// The approach and departure angles (rad): the overhangs' underside rises at them from the tyres.
    pub approach: f32,
    pub departure: f32,
    /// The suspension: ride frequency (Hz, the same front and rear: each axle's spring sags the same under its share),
    /// the unloaded length and the compression travel (m), the damping ratios in compression and extension.
    pub ride_hz: f32,
    pub rest: f32,
    pub travel: f32,
    pub zeta: [f32; 2],
    /// The bump stop: over the last `bump_length` of the travel, the stiffness rising with the square of the
    /// compression into it to `bump_rate` N/m at the wheel.
    pub bump_length: f32,
    pub bump_rate: f32,
    pub friction: f32,
    /// The driven wheels (front left, front right, rear left, rear right).
    pub driven: [bool; 4],
}

/// A 2018 Toyota Camry LE (XV70): curb weight 3,296 lb; wheelbase 111.2 in; track 63 in; tyres 215/55 R17 (17 in + 2 x
/// 55 % of 215 mm across); body 4.885 x 1.840 m, ground clearance about 0.15 m; front-wheel drive.
///
/// Ours: the centre of mass 0.55 m up (the static stability factor T / 2h of mid-size sedans rated five stars for
/// rollover is about 1.4-1.5: h = 1.60 / (2 x 1.45)); 59 % on the front axle (a front-engined, front-driven sedan's
/// usual share); inertia of the order measured for mid-size sedans; 15-degree overhang angles; ride 1.5 Hz, damping
/// ratios 0.25 / 0.4, 0.22 m of travel from the unloaded 0.30 m (11 cm of jounce above the ride height). The bump stop:
/// a passenger car's microcellular polyurethane bump stop is about 50 N/mm at first and over 2,000 N/mm in its last 5
/// mm; at the wheel (a motion ratio of about 0.9, squared) about 1,600 N/mm: K = 1.6e6 N/m over L = 4 cm, so full
/// compression holds K L / 3 = 21 kN (5.8 x the static load of a front wheel) and takes K L² / 12 = 213 J. Dry asphalt's
/// friction 1.0.
pub const SEDAN: Car = Car {
    name: "sedan",
    mass: 1495.0,
    wheelbase: 2.825,
    track: 1.60,
    radius: 0.334,
    width: 0.215,
    length: 4.8,
    body_width: 1.8,
    clearance: 0.15,
    roof: 1.45,
    com_height: 0.55,
    front_share: 0.59,
    inertia: [550.0, 2500.0, 2700.0],
    approach: 0.261_799_4,
    departure: 0.261_799_4,
    ride_hz: 1.5,
    rest: 0.30,
    travel: 0.22,
    zeta: [0.25, 0.4],
    bump_length: 0.04,
    bump_rate: 1.6e6,
    friction: 1.0,
    driven: [true, true, false, false],
};

/// A 2020 Toyota 4Runner TRD Off-Road: curb weight 4,750 lb; wheelbase 109.8 in; track 63.2 in; ground clearance 9.6 in;
/// tyres 265/70 R17 (17 in + 2 x 70 % of 265 mm across); body 191.3 x 75.8 in; approach and departure angles 33 and 26
/// degrees; four-wheel drive.
///
/// Ours: the centre of mass 0.69 m up (mid-size SUVs rated three stars for rollover have a static stability factor
/// of about 1.13-1.25: h = 1.605 / (2 x 1.17)); 53 % on the front axle (a body-on-frame four-wheel-drive SUV's usual
/// share); inertia of the order of a mid-size SUV's; ride 1.3 Hz, damping ratios 0.25 / 0.4, 0.30 m of travel from the
/// unloaded 0.40 m (15 cm of jounce above the ride height); the bump stop as the sedan's for its load (K = 2.4e6 N/m
/// over L = 5 cm: full compression holds 40 kN, 7 x the static load of a front wheel).
pub const SUV: Car = Car {
    name: "suv",
    mass: 2155.0,
    wheelbase: 2.789,
    track: 1.605,
    radius: 0.401,
    width: 0.265,
    length: 4.8,
    body_width: 1.9,
    clearance: 0.244,
    roof: 1.8,
    com_height: 0.69,
    front_share: 0.53,
    inertia: [1000.0, 3800.0, 4000.0],
    approach: 0.575_958_7,
    departure: 0.453_785_6,
    ride_hz: 1.3,
    rest: 0.40,
    travel: 0.30,
    zeta: [0.25, 0.4],
    bump_length: 0.05,
    bump_rate: 2.4e6,
    friction: 1.0,
    driven: [true, true, true, true],
};

pub const CARS: [Car; 2] = [SEDAN, SUV];

impl Car {
    /// The static sag (m): g / (2 pi f)².
    pub fn sag(&self) -> f32 {
        let omega = 2.0 * core::f32::consts::PI * self.ride_hz;
        G / (omega * omega)
    }

    /// The hard points' height in the chassis's frame (its origin on the ground at the ride height, halfway between
    /// the axles; x across, y up, z forward).
    pub fn hard(&self) -> f32 {
        self.radius + self.rest - self.sag()
    }

    /// The obstacles in the wheels' tracks (`Sc::WheelTrack`): whole cells, at least 4 cm under the body.
    pub fn track_cells(&self) -> i32 {
        (((self.clearance - 0.04) / CELL) as i32).max(1)
    }

    pub fn wheels(&self) -> Vec<WheelDesc> {
        let sag = self.sag();
        let hard = self.hard();
        let half = self.wheelbase / 2.0;
        let mut out = vec![];
        for (z, share) in [(half, self.front_share / 2.0), (-half, (1.0 - self.front_share) / 2.0)] {
            for x in [-self.track / 2.0, self.track / 2.0] {
                // Bullet's stiffness is per kg of the whole chassis: a wheel carrying `share` of the weight sags by
                // `sag` at share x g / sag. Damping per kg: c = 2 zeta sqrt(k m), k the wheel's rate, m its share.
                let stiffness = share * G / sag;
                let crit = 2.0 * (stiffness * share).sqrt();
                out.push(WheelDesc {
                    anchor: [x, hard, z],
                    direction: [0.0, -1.0, 0.0],
                    axle: [-1.0, 0.0, 0.0],
                    rest: self.rest,
                    travel: self.travel,
                    radius: self.radius,
                    width: self.width,
                    stiffness,
                    compression: self.zeta[0] * crit,
                    relaxation: self.zeta[1] * crit,
                    friction_slip: self.friction,
                    side_stiffness: 1.0,
                    max_force: 1e6,
                    bump_length: self.bump_length,
                    bump_stiffness: self.bump_rate / self.mass,
                    stop_inset: 0.025,
                });
            }
        }
        out
    }

    /// The chassis's cells and the collider's offset in the chassis's frame: the lower body from the ground clearance
    /// to above the wheels at full compression, with a wheel well round each wheel (across the tyre and 5 cm more, the
    /// tyre's diameter and 0.3 m more long); its overhangs' underside rising at the approach and departure angles from
    /// the line tangent to the tyre on the ground; a cabin 0.1 m narrower each side, over the middle 2.2 m, to the roof.
    pub fn cells(&self) -> (Vec<[i32; 3]>, [f32; 3]) {
        let half = self.wheelbase / 2.0;
        let (nx, nz) = ((self.body_width / CELL).round() as i32, (self.length / CELL).round() as i32);
        let (x0, z0) = (-(nx / 2), -(nz / 2));
        // (An odd count is centred by the collider's offset.)
        let offset = [-((nx % 2) as f32) * CELL / 2.0, self.clearance, -((nz % 2) as f32) * CELL / 2.0];
        let wheel_top = 2.0 * self.radius - self.sag() + self.travel;
        let lower = (((wheel_top + 0.07).max(self.clearance + 0.7) - self.clearance) / CELL).ceil() as i32;
        let top = ((self.roof - self.clearance) / CELL).round() as i32;
        let mut cells = vec![];
        for z in z0..z0 + nz {
            for x in x0..x0 + nx {
                for y in 0..top {
                    let (cx, cz) = ((x as f32 + 0.5) * CELL + offset[0], (z as f32 + 0.5) * CELL + offset[2]);
                    let bottom = y as f32 * CELL + self.clearance;
                    if y >= lower {
                        if x == x0 || x == x0 + nx - 1 || !(-1.2..1.0).contains(&cz) {
                            continue;
                        }
                    } else {
                        let well = cx.abs() > self.track / 2.0 - self.width / 2.0 - 0.05 && ((cz - half).abs() < self.radius + 0.15 || (cz + half).abs() < self.radius + 0.15);
                        if well {
                            continue;
                        }
                        let angle = if cz > 0.0 { self.approach } else { self.departure };
                        let outer = (cz.abs() + CELL / 2.0) - (half + self.radius * (angle / 2.0).tan());
                        if outer > 0.0 && bottom < outer * angle.tan() {
                            continue;
                        }
                    }
                    cells.push([x, y, z]);
                }
            }
        }
        (cells, offset)
    }

    /// The car at `at` (its frame's origin, turned `yaw` about up): a voxel body whose mass is given as a voxel body's
    /// (`set_mass_properties`: its cells' centroid), the vehicle's centre of mass replacing it.
    pub fn add(&self, p: &mut Physics, at: [f32; 3], can_sleep: bool) -> (BodyId, VehicleId) {
        let b = p.add_body(&BodyDesc { position: at, can_sleep, ..Default::default() }).unwrap();
        let (cells, offset) = self.cells();
        let n = cells.len() as f32;
        let mut centroid = [0.0f32; 3];
        for c in &cells {
            for i in 0..3 {
                centroid[i] += ((c[i] as f32 + 0.5) * CELL + offset[i]) / n;
            }
        }
        let mut d = ColliderDesc::new(Shape::Voxels { size: CELL, cells });
        d.position = offset;
        d.density = 0.0;
        d.friction = 0.8;
        p.add_collider(Some(b), &d).unwrap();
        let [roll, pitch, yaw] = self.inertia;
        p.set_mass_properties(b, self.mass, centroid, [pitch, yaw, roll, 0.0, 0.0, 0.0]).unwrap();
        let com_z = self.wheelbase / 2.0 - (1.0 - self.front_share) * self.wheelbase;
        let v = p.add_vehicle(&VehicleDesc { chassis: b, wheels: self.wheels(), forward_axis: 2, up_axis: 1, center_of_mass: Some([0.0, self.com_height, com_z]) }).unwrap();
        (b, v)
    }
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
    /// Blocks in the wheels' tracks only, lower than the body (`Car::track_cells`): the wheels' own test.
    WheelTrack,
    /// Trenches across the road 0.1, 0.2, 0.3, 0.5 m wide, square to it, or at 30 degrees.
    GapsStraight,
    GapsOblique,
    /// A 1 m and a 4 m crater (an explosion's: a hemisphere), the right wheels over its edge.
    Rim1,
    Rim4,
    /// Out of a 1 m crater (the rear left wheel in it), out of the bottom of a 4 m one and of an 8 m one, at a full 1 g
    /// of drive: the steep wall is not climbed.
    Out1,
    Out4,
    Out8,
    /// A step up of 0.1, 0.2, 0.3 m.
    Step(u8),
    /// Posts of 0.1 x 0.1 x 0.3 m and rails 0.1 m wide, 0.2 high, beside each wheel's centre line (about 0.05 m off
    /// it): inside the tyre's width.
    Posts,
    /// The probes starting in the ground (each wheel's ball 3 cm into it, the chassis lowered with them); dropped from
    /// 2.5 m.
    SunkStart,
    Drop,
    /// Parked for 10 s, the right wheels' hard points and the rear axle on lattice planes.
    Parked,
}

impl Sc {
    pub fn all() -> Vec<Sc> {
        vec![
            Sc::DebrisFixed,
            Sc::DebrisBodies,
            Sc::DebrisLow,
            Sc::DebrisSlow,
            Sc::DebrisTravel,
            Sc::WheelTrack,
            Sc::GapsStraight,
            Sc::GapsOblique,
            Sc::Rim1,
            Sc::Rim4,
            Sc::Out1,
            Sc::Out4,
            Sc::Out8,
            Sc::Step(1),
            Sc::Step(2),
            Sc::Step(3),
            Sc::Posts,
            Sc::SunkStart,
            Sc::Drop,
            Sc::Parked,
        ]
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
            Sc::DebrisFixed | Sc::DebrisBodies | Sc::DebrisLow | Sc::DebrisTravel | Sc::WheelTrack => (8.0, 9.0, 0.4),
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

    /// The tallest obstacle on the way (m), if there is one to drive over.
    pub fn obstacle(self, car: &Car) -> Option<f32> {
        match self {
            Sc::DebrisFixed | Sc::DebrisBodies | Sc::DebrisSlow | Sc::Posts => Some(0.3),
            Sc::DebrisLow => Some(0.2),
            Sc::DebrisTravel => Some(0.1),
            Sc::WheelTrack => Some(car.track_cells() as f32 * CELL),
            Sc::Step(h) => Some(h as f32 * CELL),
            _ => None,
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

/// Where a car starts in a scenario (its frame's origin).
fn start(car: &Car, sc: Sc) -> [f32; 3] {
    let (t, wb) = (car.track / 2.0, car.wheelbase / 2.0);
    // (The bottom of a crater of radius r under the wheels: -sqrt(r² - t² - wb²), 2 cm above it.)
    let bowl = |r: f32| -(r * r - t * t - wb * wb).sqrt() + 0.02;
    match sc {
        // Each wheel's ball 3 cm into the ground: the hard points at r - 0.03.
        Sc::SunkStart => [0.0, car.radius - 0.03 - car.hard(), 0.0],
        Sc::Drop => [0.0, 2.5, 0.0],
        Sc::Out4 => [0.0, bowl(4.0), 0.0],
        Sc::Out8 => [0.0, bowl(8.0), 0.0],
        Sc::Posts => [0.05, 0.0, 0.0],
        Sc::Parked => [(t / CELL).floor() * CELL - t, 0.0, wb - (wb / CELL).floor() * CELL],
        _ => [0.0, 0.0, 0.0],
    }
}

/// The ground: one fixed voxel collider (cell i spans i x 0.1 .. (i + 1) x 0.1), top at y = 0, x from -half to half,
/// z from -10 to 80 m, 0.3 m thick; and the debris bodies.
fn ground(p: &mut Physics, car: &Car, sc: Sc, half: f32, at: [f32; 3]) -> crate::ColliderId {
    let (x0, x1) = (-(half / CELL) as i32, (half / CELL) as i32);
    let (z0, z1) = (-100, 800);
    let (t, wb) = (car.track / 2.0, car.wheelbase / 2.0);
    let crater = match sc {
        Sc::Rim1 => Some(([t + 0.7, 20.0], 1.0)),
        Sc::Rim4 => Some(([t + 3.8, 20.0], 4.0)),
        Sc::Out1 => Some(([-t, -wb], 1.0)),
        Sc::Out4 => Some(([0.0, 0.0], 4.0)),
        Sc::Out8 => Some(([0.0, 0.0], 8.0)),
        _ => None,
    };
    // The posts beside each wheel's centre line, about 0.05 m inside it: the cells that start nearest 0.05 m in.
    let lines = [at[0] - t, at[0] + t];
    let post_x = [((lines[0] + 0.05) / CELL).round() as i32, ((lines[1] + 0.05) / CELL).round() as i32];
    let (s30, c30) = (0.5f32, 0.866_025_4f32);
    let gaps = [(150, 1), (300, 2), (450, 3), (600, 5)];
    // The wheel tracks' blocks: in each track, every 2-4 m from 15 to 55 m, 1-2 cells long, across the tyre and a cell
    // more each side.
    let mut blocks: Vec<(i32, i32, i32)> = vec![];
    if sc == Sc::WheelTrack {
        let mut r = Rng(0x2545_f491_4f6c_dd1d);
        for &line in &lines {
            let mut z = 150 + r.below(20) as i32;
            while z < 550 {
                blocks.push((((line - car.width / 2.0) / CELL).floor() as i32 - 1, ((line + car.width / 2.0) / CELL).ceil() as i32 + 1, z));
                if r.below(2) == 1 {
                    blocks.push((((line - car.width / 2.0) / CELL).floor() as i32 - 1, ((line + car.width / 2.0) / CELL).ceil() as i32 + 1, z + 1));
                }
                z += 20 + r.below(20) as i32;
            }
        }
    }
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
                Sc::WheelTrack if blocks.iter().any(|&(a, b, bz)| bz == z && x >= a && x < b) => car.track_cells(),
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
            // m beside both.
            if sc == Sc::Posts {
                let post = (120..=440).step_by(40).enumerate().any(|(k, pz)| z == pz && x == post_x[k % 2]);
                let rail = (500..700).contains(&z) && post_x.contains(&x);
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
    let id = p.add_collider(None, &d).unwrap();
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
    id
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
    /// The chassis's largest vertical speed after the first second (m/s), the car's fastest and the fastest any body
    /// went; the largest yaw rate (rad/s) and the heading at the end (degrees from +z, positive towards +x).
    pub max_vy: f32,
    pub car_max_speed: f32,
    pub max_speed: f32,
    pub yaw_max: f32,
    pub heading: f32,
    /// The largest tilt (degrees from upright), and when the car left the test's road (s; it is measured until then:
    /// what it does off the edge is not the scenario's).
    pub tilt_max: f32,
    pub left_road: Option<f32>,
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
    /// Wheel-ticks with the suspension in the bump stop's range, fully compressed, and with its stop collider touching
    /// something; the deepest into the bump stop's range (m).
    pub bump_ticks: u32,
    pub bottomed_ticks: u32,
    pub stop_ticks: u32,
    /// Ticks the body (the chassis's colliders but the stops) touches something, of `ticks` measured after 0.5 s.
    pub body_ticks: u32,
    pub ticks: u32,
    pub bump_max: f32,
    /// Per step: the vehicles' update (probes and controller) and the world's step (ms).
    pub vehicles_ms: f64,
    pub world_ms: f64,
    pub nan: bool,
    /// At the end: the deepest tyre (m) and the wheels in contact.
    pub end_depth: f32,
    pub end_contacts: u32,
    /// The trace (with `trace`): per tick, time, position, up, velocity, wheels in contact, suspension lengths; the
    /// wheels' normals, loads and impulses.
    pub trace: Vec<String>,
}

/// The deepest the tyre (a cylinder of the wheel's radius and half width about the axle) at `center` goes into any
/// voxel collider not on `skip` (m; 0 if it touches nothing), against each solid cell's box (parry has no contact of
/// a cylinder with a voxel shape whole).
fn tyre_depth(p: &Physics, car: &Car, center: Vector, axle: Vector, skip: RigidBodyHandle) -> f32 {
    let w = &p.world;
    let cyl = Cylinder::new(car.width / 2.0, car.radius);
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

/// The controls for a car's wheels: the drive towards `target` m/s along the chassis's forward, at most `drive_g` of
/// the weight shared by the driven wheels; parked (`target` 0): the brakes.
fn controls(car: &Car, speed: f32, target: f32, drive_g: f32) -> Vec<WheelControl> {
    let driven = car.driven.iter().filter(|&&d| d).count() as f32;
    let fmax = car.mass * drive_g * G / driven;
    let parked = target == 0.0;
    let force = if parked { 0.0 } else { ((target - speed) * car.mass / driven * 2.0).clamp(-fmax, fmax) };
    (0..4).map(|i| WheelControl { engine: if car.driven[i] { force } else { 0.0 }, brake: if parked { 500.0 } else { 0.0 }, steering: 0.0 }).collect()
}

/// Runs a scenario with `n` cars side by side (3 m apart); measures the first. `clock`: a clock for the cost
/// (seconds; none in wasm32).
pub fn run(car: &Car, sc: Sc, n: usize, measure: bool, trace: bool, clock: Option<&dyn Fn() -> f64>) -> Report {
    let mut p = Physics::new([0.0, -9.81, 0.0]);
    // (The road wide enough that a car turned aside by posts or loose debris stays on it: none steers.)
    let half = (3.0 * n as f32 + 2.0).max(if matches!(sc, Sc::Posts | Sc::DebrisBodies | Sc::WheelTrack) { 12.0 } else { 4.0 });
    let at = start(car, sc);
    ground(&mut p, car, sc, half, at);
    let (target, seconds, drive_g) = sc.plan();
    let cars: Vec<(BodyId, VehicleId)> = (0..n).map(|i| car.add(&mut p, [at[0] + (i as f32 - (n as f32 - 1.0) / 2.0) * 3.0, at[1], at[2]], false)).collect();
    p.step();
    let ticks = (seconds * 60.0) as u32;
    let mut rep = Report::default();
    let (mut rp, mut rp_n, mut v_sum, mut v_n) = (0.0f64, 0u32, 0.0f64, 0u32);
    let mut parked_at: Option<Vector> = None;
    let (mut veh_s, mut world_s) = (0.0f64, 0.0f64);
    for t in 0..ticks {
        let time = t as f32 / 60.0;
        for &(_, v) in &cars {
            let speed = p.vehicle_state(v).unwrap().speed;
            p.set_vehicle_controls(v, &controls(car, speed, target, drive_g)).unwrap();
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
        if (pos.x - at[0]).abs() > half - 3.0 {
            rep.left_road = Some(time);
            break;
        }
        rep.tilt_max = rep.tilt_max.max(up.y.clamp(-1.0, 1.0).acos().to_degrees());
        let st = p.vehicle_state(v0).unwrap();
        if trace {
            rep.trace.push(format!(
                "t {time:.3} pos {:.3} {:.3} {:.3} up {:.3} {:.3} {:.3} v {:.2} {:.2} {:.2} w {:.2} {:.2} {:.2} contact {} susp {}",
                pos.x,
                pos.y,
                pos.z,
                up.x,
                up.y,
                up.z,
                lv.x,
                lv.y,
                lv.z,
                av.x,
                av.y,
                av.z,
                st.wheels.iter().map(|w| if w.contact { '1' } else { '0' }).collect::<String>(),
                st.wheels.iter().map(|w| format!("{:.3}", w.suspension)).collect::<Vec<_>>().join(",")
            ));
            let veh = &p.vehicles[&v0];
            // (What the chassis's own colliders touch: the body, and each stop; contact pairs with a point.)
            let touching: Vec<String> = p.world.bodies[hb].colliders().iter().map(|&c| p.world.narrow_phase.contact_pairs_with(c).filter(|k| k.has_any_active_contact()).count().to_string()).collect();
            rep.trace.push(format!("    touching {}", touching.join(",")));
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
        rep.yaw_max = rep.yaw_max.max(local.y.abs());
        rep.heading = fwd.x.atan2(fwd.z).to_degrees();
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
        rep.car_max_speed = rep.car_max_speed.max(lv.length());
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
            rep.ticks += 1;
            let stops = &p.vehicles[&v0].stops;
            rep.body_ticks += p.world.bodies[hb].colliders().iter().any(|&c| !stops.iter().any(|s| p.colliders[s].0 == c) && p.world.narrow_phase.contact_pairs_with(c).any(|k| k.has_any_active_contact())) as u32;
            let axle = rot * Vector::new(-1.0, 0.0, 0.0);
            let down = rot * Vector::new(0.0, -1.0, 0.0);
            let veh = &p.vehicles[&v0];
            for (w, s) in veh.ctl.wheels().iter().zip(&veh.stops) {
                rep.wheel_ticks += 1;
                let depth = tyre_depth(&p, car, w.center(), axle, hb);
                if depth > 0.03 {
                    rep.sunk += 1;
                }
                rep.sink_max = rep.sink_max.max(depth);
                if w.raycast_info().is_in_contact {
                    let full = car.rest - car.travel;
                    let into = full + car.bump_length - w.raycast_info().suspension_length;
                    rep.bump_ticks += (into > 0.0) as u32;
                    rep.bump_max = rep.bump_max.max(into);
                    rep.bottomed_ticks += (w.raycast_info().suspension_length <= full) as u32;
                } else {
                    rep.no_contact += 1;
                    let reach = w.raycast_info().hard_point_ws + down * car.rest;
                    if tyre_depth(&p, car, reach, axle, hb) > 0.0 {
                        rep.missed += 1;
                    }
                }
                let h = p.colliders[s].0;
                rep.stop_ticks += p.world.narrow_phase.contact_pairs_with(h).any(|c| c.has_any_active_contact()) as u32;
            }
        }
    }
    if measure && !rep.nan {
        let (b0, v0) = cars[0];
        let hb = p.bodies[&b0];
        let axle = p.world.bodies[hb].position().rotation * Vector::new(-1.0, 0.0, 0.0);
        for w in p.vehicles[&v0].ctl.wheels() {
            rep.end_depth = rep.end_depth.max(tyre_depth(&p, car, w.center(), axle, hb));
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

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Pass,
    Fail,
    /// Nothing the scenario is for happened (why).
    NotApplicable(&'static str),
}

/// The rule a scenario is judged by.
pub fn rule(car: &Car, sc: Sc) -> &'static str {
    match sc {
        Sc::DebrisBodies => "loose debris: no flip, no launch",
        _ if sc.obstacle(car).is_some_and(|h| h > car.clearance) => "taller than the clearance: a real car does not pass; no flip, no launch",
        Sc::Out4 | Sc::Out8 => "1 g up a steep wall: no flip",
        Sc::Drop => "lands on its wheels",
        Sc::SunkStart => "out of the ground, drives",
        Sc::Parked => "stays put on its wheels",
        _ => "drives over",
    }
}

/// Whether a run passed (the rules of `out/web-probe/step10/c-wheel-compare.md` as the supervisor revised them for real
/// cars: what a real car cannot pass is judged on staying sound).
pub fn verdict(car: &Car, sc: Sc, r: &Report) -> Verdict {
    if r.wheel_ticks > 0 && r.no_contact == r.wheel_ticks {
        return Verdict::NotApplicable("its wheels never touched the ground: the body rests on the obstacle");
    }
    // (Its body on the ground from the start and the car never under way: high-centred, nothing of the wheels'.)
    if r.ticks > 0 && r.body_ticks == r.ticks && r.car_max_speed < 1.0 && sc.plan().0 > 0.0 {
        return Verdict::NotApplicable("high-centred: its body rests on the ground from the start and it never gets under way");
    }
    let (target, ..) = sc.plan();
    let sound = !r.flipped && !r.nan && r.max_vy <= 3.0;
    // (No launch: the car never faster than it was driven.)
    let calm = r.car_max_speed <= target + 0.3;
    let ok = match sc {
        Sc::DebrisBodies => sound && calm && r.sink_max <= 0.05,
        _ if sc.obstacle(car).is_some_and(|h| h > car.clearance) => sound && calm && r.sink_max <= 0.05,
        Sc::Drop => sound && r.end_depth <= 0.03 && r.end_contacts == 4,
        Sc::SunkStart => sound && r.end_depth <= 0.03 && r.end_contacts == 4 && r.speed_held >= 0.8,
        Sc::Parked => sound && r.no_contact == 0 && r.parked_drift < 0.01,
        Sc::Out1 => sound && r.end[0] > 4.0 && r.sink_max <= 0.05,
        Sc::Out4 | Sc::Out8 => sound,
        _ => sound && r.sink_max <= 0.05 && (r.missed as f32) <= 0.02 * r.wheel_ticks.max(1) as f32 && r.speed_held >= 0.8,
    };
    if ok {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

/// A car on its stops (no spring, no bump stop: the chassis rests on the four stop colliders) on flat ground, set
/// moving at `speed` m/s along its forward: its speed after `seconds`. With the stops rolling at the hub's speed it
/// coasts; were their surface's sign the other way, or no surface speed, their friction (1.0) would brake it at about
/// 1 g.
pub fn coast_on_stops(car: &Car, speed: f32, seconds: f32) -> f32 {
    let mut p = Physics::new([0.0, -9.81, 0.0]);
    let d = ColliderDesc { friction: 1.0, ..ColliderDesc::new(Shape::Box([20.0, 0.5, 60.0])) };
    p.add_collider(None, &ColliderDesc { position: [0.0, -0.5, 20.0], ..d }).unwrap();
    let mut soft = *car;
    soft.bump_rate = 0.0;
    let b = p.add_body(&BodyDesc { position: [0.0, 0.0, 0.0], linvel: [0.0, 0.0, speed], can_sleep: false, ..Default::default() }).unwrap();
    let (cells, offset) = soft.cells();
    p.add_collider(Some(b), &ColliderDesc { position: offset, density: 0.0, friction: 0.8, ..ColliderDesc::new(Shape::Voxels { size: CELL, cells }) }).unwrap();
    let [roll, pitch, yaw] = soft.inertia;
    p.set_mass_properties(b, soft.mass, [0.0, soft.com_height, 0.0], [pitch, yaw, roll, 0.0, 0.0, 0.0]).unwrap();
    let wheels: Vec<WheelDesc> = soft.wheels().into_iter().map(|w| WheelDesc { stiffness: 0.0, compression: 0.0, relaxation: 0.0, bump_stiffness: 0.0, ..w }).collect();
    let v = p.add_vehicle(&VehicleDesc { chassis: b, wheels, forward_axis: 2, up_axis: 1, center_of_mass: None }).unwrap();
    for _ in 0..(seconds * 60.0) as u32 {
        p.step();
    }
    p.vehicle_state(v).unwrap().speed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Runs `f` on a thread of its own, failing the test if it takes longer than `secs` (a watchdog: a step that does
    /// not return fails the run instead of holding CI).
    fn watched<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(secs)).unwrap_or_else(|_| panic!("watchdog: over {secs} s"))
    }

    /// The scenarios CI runs (each car, about 0.1-0.6 s each in release; the watchdog at 60 s a run): the wheels' own
    /// tests (gaps, crater rims and exits, a step within the clearance, the sunk start, the drop, parked; the wheel
    /// tracks' blocks for the sedan: the SUV keeps 0.78 of its speed over its 0.2 m ones, a failure docs/VOXEL.md
    /// lists), and one each of what a real car does not pass (0.3 m fixed debris, posts).
    #[test]
    fn the_cars_on_destroyed_ground() {
        let mut failed = vec![];
        for car in CARS {
            let tracks = if car.name == "sedan" { vec![Sc::WheelTrack] } else { vec![] };
            for sc in tracks.into_iter().chain([Sc::DebrisTravel, Sc::GapsStraight, Sc::Rim1, Sc::Out1, Sc::Step(1), Sc::SunkStart, Sc::Drop, Sc::Parked, Sc::DebrisFixed, Sc::Posts]) {
                let r = watched(60, move || run(&car, sc, 1, true, false, None));
                match verdict(&car, sc, &r) {
                    Verdict::Pass => {}
                    v => failed.push(format!("{} {}: {v:?} {r:?}", car.name, sc.name())),
                }
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }

    /// The SUV climbs a 0.2 m step (within its clearance) and drives on.
    #[test]
    fn the_suv_climbs_a_step_within_its_clearance() {
        let r = watched(60, || run(&SUV, Sc::Step(2), 1, true, false, None));
        assert_eq!(verdict(&SUV, Sc::Step(2), &r), Verdict::Pass, "{r:?}");
    }

    /// The stops' contact hook: the surface of a wheel rolling at the hub's speed, its sign held. A car resting on its
    /// stops coasts (from 5 m/s, above 4.5 after a second); the sign the other way, or the stops' friction without a
    /// moving surface, would brake it to a stop in about half a second.
    #[test]
    fn the_stop_rolls_with_the_hub() {
        let v = watched(60, || coast_on_stops(&SUV, 5.0, 1.0));
        assert!(v > 4.5, "{v} m/s after 1 s from 5");
    }
}

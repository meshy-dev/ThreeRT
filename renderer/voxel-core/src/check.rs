//! A fixed script of edits on a small level, and the hash of what it leaves (ROADMAP § 6 step 6's determinism check):
//! the unit tests, a native binary and a wasm32 module (examples/determinism*.rs) all run it and must agree.

use crate::fixed::{to_fp, Shape, FP};
use crate::grid::VoxelGrid;
use crate::level::{gravity_for, Level, HARD, INDESTRUCTIBLE, MEDIUM, RADII};
use crate::place::Placement;
use std::rc::Rc;

/// The hash the script leaves (recorded: a change of behaviour shows here).
pub const EXPECTED: u64 = 0x733d_b9ab_69f4_95e9;

fn block(size: [i32; 3], material: impl Fn(i32, i32, i32) -> u8) -> Rc<VoxelGrid> {
    let mut g = VoxelGrid::default();
    for x in 0..size[0] {
        for y in 0..size[1] {
            for z in 0..size[2] {
                g.set(x, y, z, material(x, y, z));
            }
        }
    }
    Rc::new(g)
}

/// Builds the level, runs the edits and the ticks between them, returns the level's hash mixed with what each edit
/// reported.
pub fn script() -> u64 {
    let mut l = Level::new(gravity_for(0.1, 60.0));
    l.hardness[3] = MEDIUM;
    l.hardness[4] = HARD;
    l.hardness[5] = INDESTRUCTIBLE;
    // Ground (anchor) of two tiles, a hill on one; a house of walls, floor slabs and a roof (four objects, the walls
    // turned by quarters); a tree whose grid two trees share; a wall of every hardness.
    l.add_object(block([64, 8, 64], |x, y, z| if y < 4 + (x + z) % 5 { 1 } else { 0 }), Placement::new([0, 0, 0], 0, [0; 3]), true);
    l.add_object(block([64, 8, 64], |_, y, _| if y < 5 { 5 } else { 0 }), Placement::new([64, 0, 0], 0, [0; 3]), true);
    for turn in 0..4 {
        l.add_object(block([30, 40, 2], |x, y, _| if (x % 10 > 5) && (y % 12 > 6) { 0 } else { 2 + (y / 14) as u8 }), Placement::new([84, 5, 24], turn, [-15, 0, 14]), false);
    }
    for floor in 0..3 {
        l.add_object(block([26, 2, 26], |_, _, _| 3), Placement::new([84, 15 + 12 * floor, 24], 0, [-13, 0, -13]), false);
    }
    l.add_object(block([34, 3, 34], |_, _, _| 4), Placement::new([84, 45, 24], 0, [-17, 0, -17]), false);
    let tree = block([9, 24, 9], |x, y, z| if y < 14 { ((x == 4) && (z == 4)) as u8 * 2 } else { 6 });
    l.add_object(tree.clone(), Placement::new([20, 5, 20], 0, [-4, 0, -4]), false);
    l.add_object(tree, Placement::new([40, 6, 30], 3, [-4, 0, -4]), false);
    l.add_object(block([40, 12, 3], |x, _, _| 1 + (x / 8) as u8), Placement::new([10, 8, 50], 2, [-20, 0, 0]), false);
    l.settle();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    let mut report = |l: &Level, e: &crate::level::Edit| {
        mix(e.voxels as u64);
        mix(e.pieces as u64);
        mix(e.piece_voxels as u64);
        mix(e.debris.len() as u64);
        mix(e.created.iter().chain(&e.removed).chain(e.changed.keys()).fold(0u64, |a, &b| a.wrapping_mul(31).wrapping_add(b as u64)));
        mix(l.hash());
    };
    let sphere = |c: [f64; 3], r: f64| Shape::Sphere { c: c.map(to_fp), r: to_fp(r) };
    let ticks = |l: &mut Level, n: usize| {
        let mut landed = 0u64;
        for _ in 0..n {
            landed = landed.wrapping_mul(1_000_003).wrapping_add(l.tick().landed.len() as u64 + 1);
        }
        landed
    };
    // Explosions: a tree's trunk, the house's corner (several objects), the ground, the wall of every hardness.
    let e = l.carve(sphere([20.31, 9.07, 20.52], 2.6), RADII);
    report(&l, &e);
    let t = ticks(&mut l, 7);
    let e = l.carve(sphere([98.2, 12.4, 37.9], 9.3), RADII);
    report(&l, &e);
    let e = l.carve(sphere([40.5, 6.1, 30.5], 4.2), RADII);
    report(&l, &e);
    let t2 = ticks(&mut l, 90);
    let e = l.carve(sphere([12.0, 14.0, 51.5], 11.0), RADII);
    report(&l, &e);
    // Cuts: the house's ground storey sawn through (a box), a line through the roof; paint; added voxels, on the
    // ground and in the air.
    let e = l.carve(Shape::Box { lo: [64 * FP, 9 * FP, 0], hi: [110 * FP, 11 * FP, 48 * FP] }, [256; 4]);
    report(&l, &e);
    let t3 = ticks(&mut l, 40);
    let e = l.carve(Shape::Capsule { a: [to_fp(66.3), to_fp(44.2), to_fp(8.1)], b: [to_fp(101.7), to_fp(47.9), to_fp(39.6)], r: to_fp(1.7) }, [256; 4]);
    report(&l, &e);
    let e = l.paint(sphere([84.0, 30.0, 24.0], 14.5), 7);
    report(&l, &e);
    let e = l.add(Shape::Box { lo: [30 * FP, 9 * FP, 40 * FP], hi: [36 * FP, 12 * FP, 44 * FP] }, 8);
    report(&l, &e);
    let e = l.add(sphere([50.0, 40.0, 50.0], 3.3), 9);
    report(&l, &e);
    let t4 = ticks(&mut l, 400);
    for v in [t, t2, t3, t4, l.falling() as u64, l.hash()] {
        mix(v);
    }
    h
}

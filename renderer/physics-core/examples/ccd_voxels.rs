//! What a fast body's continuous collision detection costs over voxel ground (ROADMAP § 6 step 10 c2; docs/VOXEL.md
//! "Where the engine works round its dependencies"): a box (1 x 0.5 x 2 m), a ball (r 0.3 m) and a convex hull (14
//! points, about 0.6 m) moving along the ground at 5, 15 and 30 m/s, 5 cm above its highest cells (no contact, no
//! gravity), over flat voxel ground and a voxel slope rising 0.1 m a metre (0.1 m cells), and over the same surfaces as
//! a triangle mesh (each free face of a solid cell, two triangles). Per body per tick: the world's step with the body
//! less the step without it (ms), over 240 ticks, the best of 5 runs.
//!   cargo run --release --example ccd_voxels [-- --spin | --lever]
//! `--spin`: the bodies also turn, 3 rad/s about x. `--lever`: as a car's part off its centre of mass (a stop): the shape
//! 1.4 m ahead of and 1 m below the body's centre of mass (a heavy ball there, too high to meet anything), 0.15 m above
//! the stairs' tips, the body pitching back and forth (0.5 sin(2 pi t) rad/s: about ±4.5 degrees at 1 Hz).
use physics_core::{BodyDesc, ColliderDesc, Physics, Shape};

const CELL: f32 = 0.1;

fn ground(slope: bool, mesh: bool) -> Shape {
    let top = |z: i32| if slope { z.max(0) / 10 } else { 0 };
    let cells: Vec<[i32; 3]> = (-100..1200i32).flat_map(|z| (-30..30).flat_map(move |x| (top(z) - 3..top(z)).map(move |y| [x, y, z]))).collect();
    if !mesh {
        return Shape::Voxels { size: CELL, cells };
    }
    // The same surface: each face of a solid cell with no solid neighbour, two triangles.
    let solid: std::collections::HashSet<[i32; 3]> = cells.iter().copied().collect();
    let (mut vertices, mut indices) = (vec![], vec![]);
    for c in &cells {
        for (n, quad) in [
            ([0, 1, 0], [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]]),
            ([0, -1, 0], [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]]),
            ([1, 0, 0], [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]]),
            ([-1, 0, 0], [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]]),
            ([0, 0, 1], [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]]),
            ([0, 0, -1], [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]]),
        ] {
            if solid.contains(&[c[0] + n[0], c[1] + n[1], c[2] + n[2]]) {
                continue;
            }
            let k = vertices.len() as u32;
            for q in quad {
                vertices.push([(c[0] + q[0]) as f32 * CELL, (c[1] + q[1]) as f32 * CELL, (c[2] + q[2]) as f32 * CELL]);
            }
            indices.push([k, k + 1, k + 2]);
            indices.push([k, k + 2, k + 3]);
        }
    }
    Shape::Trimesh { vertices, indices }
}

fn shape(kind: &str) -> (Shape, f32) {
    match kind {
        "box" => (Shape::Box([0.5, 0.25, 1.0]), 0.25),
        "ball" => (Shape::Sphere(0.3), 0.3),
        _ => {
            let points: Vec<[f32; 3]> = (0..14).map(|k| {
                let a = k as f32 * 0.4488;
                [0.3 * libm::cosf(a) * if k % 2 == 0 { 1.0 } else { 0.7 }, 0.3 * libm::sinf(a * 1.7), 0.3 * libm::sinf(a)]
            }).collect();
            (Shape::ConvexHull(points), 0.3)
        }
    }
}

/// The world's step (ms a tick) with or without one body over 240 ticks.
fn run(slope: bool, mesh: bool, body: Option<(&str, f32)>, spin: bool, lever: bool) -> f64 {
    let mut p = Physics::new([0.0, 0.0, 0.0]);
    let mut d = ColliderDesc::new(ground(slope, mesh));
    d.friction = 1.0;
    p.add_collider(None, &d).unwrap();
    let mut pitched = None;
    if let Some((kind, v)) = body {
        let (s, half) = shape(kind);
        // Along the slope, its lowest point 5 cm above the stairs' tips (the mean slope line plus 0.1 m).
        let (dir, lift) = if slope { ([0.0, 0.0995, 0.995], 0.1) } else { ([0.0, 0.0, 1.0], 0.0) };
        let (y, angvel) = if lever { (half + 0.15 + lift + 1.0, [0.0; 3]) } else { (half + 0.05 + lift, if spin { [3.0, 0.0, 0.0] } else { [0.0; 3] }) };
        let b = p.add_body(&BodyDesc { position: [0.0, y, 0.0], linvel: [dir[0] * v, dir[1] * v, dir[2] * v], angvel, gravity_scale: 0.0, can_sleep: false, ..Default::default() }).unwrap();
        if lever {
            pitched = Some(b);
            // The shape 1.4 m ahead and 1 m below the centre of mass (a ball of 1,000 kg there, too high to meet anything).
            p.add_collider(Some(b), &ColliderDesc { mass: Some(1000.0), ..ColliderDesc::new(Shape::Sphere(0.2)) }).unwrap();
            p.add_collider(Some(b), &ColliderDesc { position: [0.0, -1.0, 1.4], mass: Some(1.0), ..ColliderDesc::new(s) }).unwrap();
        } else {
            p.add_collider(Some(b), &ColliderDesc::new(s)).unwrap();
        }
    }
    p.step();
    let t = std::time::Instant::now();
    for k in 0..240 {
        if let Some(b) = pitched {
            let w = 0.5 * libm::sinf(core::f32::consts::TAU * k as f32 / 60.0);
            p.set_velocity(b, None, Some([w, 0.0, 0.0])).unwrap();
        }
        p.step();
    }
    t.elapsed().as_secs_f64() * 1e3 / 240.0
}

fn main() {
    // `--case slope,voxels,box,30`: that one only, run for a while (to profile it).
    if let Some(i) = std::env::args().position(|a| a == "--case") {
        let a: Vec<String> = std::env::args().nth(i + 1).unwrap().split(',').map(String::from).collect();
        let lever = std::env::args().any(|a| a == "--lever");
        for _ in 0..20 {
            println!("{:.4}", run(a[0] == "slope", a[1] == "trimesh", Some((&a[2], a[3].parse().unwrap())), false, lever));
        }
        return;
    }
    let spin = std::env::args().any(|a| a == "--spin");
    let lever = std::env::args().any(|a| a == "--lever");
    println!("ground\ttarget\tshape\tm/s\tms a body a tick");
    for slope in [false, true] {
        for mesh in [false, true] {
            let base = (0..5).map(|_| run(slope, mesh, None, spin, lever)).fold(f64::MAX, f64::min);
            for kind in ["box", "ball", "hull"] {
                for v in [5.0, 15.0, 30.0] {
                    let with = (0..5).map(|_| run(slope, mesh, Some((kind, v)), spin, lever)).fold(f64::MAX, f64::min);
                    println!("{}\t{}\t{kind}\t{v}\t{:.4}", if slope { "slope" } else { "flat" }, if mesh { "trimesh" } else { "voxels" }, (with - base).max(0.0));
                }
            }
        }
    }
}

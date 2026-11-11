//! Which collision shape voxel objects get (ROADMAP § 6 step 8; docs/VOXEL.md "Voxel bodies"): a measurement, no test.
//! The candidates, on Rapier as the world steps it (`enhanced-determinism`, one thread, the tick of 1 / 60 s):
//!   voxels    parry's `Voxels` (8³ chunks of a byte a cell, each cell knowing which neighbours it has)
//!   boxes     a compound of boxes, the cells merged greedily
//!   trimesh   the cells' outer faces as triangles (fixed colliders only: a trimesh has no volume)
//!   hull      the convex hull of the cells' corners (moving bodies only)
//! Measured: the memory and time to build a fixed ground of cells, an edit of it (a sphere of cells cleared), and
//! pieces of debris dropped onto it in a heap: the tick's time while they fall and when they rest, how many come to
//! rest and sleep, whether any went through the ground.
//!
//!   cargo run --release --example voxel_shapes [pieces = 100] [ground cells a side = 256]
use rapier3d::prelude::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Counting;
static HELD: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        HELD.fetch_add(l.size(), Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        HELD.fetch_sub(l.size(), Ordering::Relaxed);
        System.dealloc(p, l)
    }
}
#[global_allocator]
static A: Counting = Counting;

const SIZE: f32 = 0.1;

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
}

type Cells = Vec<[i32; 3]>;

/// A ground `n` cells a side and 6 deep with low hills, and a few walls on it.
fn ground(n: i32) -> Cells {
    let mut c = vec![];
    for x in 0..n {
        for z in 0..n {
            let h = 6 + (((x as f32 * 0.07).sin() + (z as f32 * 0.05).cos()) * 1.5) as i32;
            for y in 0..h {
                c.push([x, y, z]);
            }
            if (x % 64 == 20 && z % 8 < 6) || (z % 64 == 40 && x % 8 < 6) {
                for y in h..h + 20 {
                    c.push([x, y, z]);
                }
            }
        }
    }
    c
}

/// A piece of debris: a lump of `n` cells a side with cells missing (connected through its core).
fn piece(rng: &mut Rng, n: i32) -> Cells {
    let mut c = vec![];
    for x in 0..n {
        for y in 0..n {
            for z in 0..n {
                let core = (x - n / 2).abs() <= 1 || (y - n / 2).abs() <= 1 || (z - n / 2).abs() <= 1;
                if core || rng.f() < 0.55 {
                    c.push([x, y, z]);
                }
            }
        }
    }
    c
}

/// The cells as boxes: each cell not yet in one grown along x, then z, then y. (lo, hi) in cells, hi exclusive.
fn boxes(cells: &Cells) -> Vec<([i32; 3], [i32; 3])> {
    let lo = (0..3).map(|a| cells.iter().map(|c| c[a]).min().unwrap()).collect::<Vec<_>>();
    let hi = (0..3).map(|a| cells.iter().map(|c| c[a]).max().unwrap() + 1).collect::<Vec<_>>();
    let d = [(hi[0] - lo[0]) as usize, (hi[1] - lo[1]) as usize, (hi[2] - lo[2]) as usize];
    let at = |x: usize, y: usize, z: usize| (y * d[2] + z) * d[0] + x;
    let mut solid = vec![false; d[0] * d[1] * d[2]];
    for c in cells {
        solid[at((c[0] - lo[0]) as usize, (c[1] - lo[1]) as usize, (c[2] - lo[2]) as usize)] = true;
    }
    let mut out = vec![];
    for y in 0..d[1] {
        for z in 0..d[2] {
            for x in 0..d[0] {
                if !solid[at(x, y, z)] {
                    continue;
                }
                let mut x1 = x + 1;
                while x1 < d[0] && solid[at(x1, y, z)] {
                    x1 += 1;
                }
                let mut z1 = z + 1;
                while z1 < d[2] && (x..x1).all(|i| solid[at(i, y, z1)]) {
                    z1 += 1;
                }
                let mut y1 = y + 1;
                while y1 < d[1] && (z..z1).all(|k| (x..x1).all(|i| solid[at(i, y1, k)])) {
                    y1 += 1;
                }
                for j in y..y1 {
                    for k in z..z1 {
                        for i in x..x1 {
                            solid[at(i, j, k)] = false;
                        }
                    }
                }
                out.push(([x as i32 + lo[0], y as i32 + lo[1], z as i32 + lo[2]], [x1 as i32 + lo[0], y1 as i32 + lo[1], z1 as i32 + lo[2]]));
            }
        }
    }
    out
}

fn iv(c: &Cells) -> Vec<IVector> {
    c.iter().map(|c| IVector::new(c[0], c[1], c[2])).collect()
}

fn shape(kind: &str, cells: &Cells) -> SharedShape {
    match kind {
        "voxels" => SharedShape::voxels(Vector::splat(SIZE), &iv(cells)),
        "boxes" => SharedShape::compound(
            boxes(cells)
                .into_iter()
                .map(|(lo, hi)| {
                    let (a, b) = (Vector::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) * SIZE, Vector::new(hi[0] as f32, hi[1] as f32, hi[2] as f32) * SIZE);
                    (Pose::from_translation((a + b) * 0.5), SharedShape::cuboid((b.x - a.x) * 0.5, (b.y - a.y) * 0.5, (b.z - a.z) * 0.5))
                })
                .collect(),
        ),
        "trimesh" => {
            let v = rapier3d::parry::shape::Voxels::new(Vector::splat(SIZE), &iv(cells));
            let (vertices, indices) = v.to_trimesh();
            SharedShape::trimesh(vertices, indices).unwrap()
        }
        "hull" => {
            let pts: Vec<Vector> = cells.iter().flat_map(|c| (0..8).map(move |k| Vector::new((c[0] + (k & 1)) as f32, (c[1] + ((k >> 1) & 1)) as f32, (c[2] + ((k >> 2) & 1)) as f32) * SIZE)).collect();
            SharedShape::convex_hull(&pts).unwrap()
        }
        _ => unreachable!(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pieces: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(100);
    let n: i32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(256);
    let g = ground(n);
    println!("ground: {n} x {n} cells of {SIZE} m, {} solid; {} pieces of 6³ cells (about {} solid each)", g.len(), pieces, piece(&mut Rng(1), 6).len());
    // The fixed ground: memory, build, an edit.
    println!("\nfixed ground        build ms   heap MiB   parts      edit ms (a 1 m sphere cleared: {} cells)", {
        let c = [n / 2, 6, n / 2];
        g.iter().filter(|p| (0..3).map(|a| (p[a] - c[a]).pow(2)).sum::<i32>() <= 100).count()
    });
    for kind in ["voxels", "boxes", "trimesh"] {
        let before = HELD.load(Ordering::Relaxed);
        let t = Instant::now();
        let mut s = shape(kind, &g);
        let build = t.elapsed().as_secs_f64() * 1e3;
        let heap = HELD.load(Ordering::Relaxed) - before;
        let c = [n / 2, 6, n / 2];
        let gone: Cells = g.iter().copied().filter(|p| (0..3).map(|a| (p[a] - c[a]).pow(2)).sum::<i32>() <= 100).collect();
        let t = Instant::now();
        let parts = match kind {
            "voxels" => {
                let v = s.make_mut().as_voxels_mut().unwrap();
                for k in &gone {
                    v.set_voxel(IVector::new(k[0], k[1], k[2]), false);
                }
                g.len() - gone.len()
            }
            // (No edit in place: the shape is made again from the cells that are left.)
            _ => {
                let left: Cells = g.iter().copied().filter(|p| (0..3).map(|a| (p[a] - c[a]).pow(2)).sum::<i32>() > 100).collect();
                s = shape(kind, &left);
                match kind {
                    "boxes" => s.as_compound().unwrap().shapes().len(),
                    _ => s.as_trimesh().unwrap().indices().len(),
                }
            }
        };
        let edit = t.elapsed().as_secs_f64() * 1e3;
        println!("  {kind:<16} {build:>9.1} {:>10.1} {parts:>8} {edit:>12.2}{}", heap as f64 / 1048576.0, if kind == "voxels" { " (in place)" } else { " (built again)" });
        drop(s);
    }
    // Debris onto the ground.
    println!("\nmoving / fixed      tick ms: falling (mean, worst)   resting (mean)   asleep at 15 s   through the ground   fastest at the end, m/s");
    for (moving, fixed) in [("voxels", "voxels"), ("boxes", "voxels"), ("hull", "voxels"), ("voxels", "trimesh"), ("boxes", "trimesh"), ("boxes", "boxes"), ("voxels", "boxes")] {
        let mut w = PhysicsWorld::new();
        w.integration_parameters.dt = 1.0 / 60.0;
        w.insert_collider(ColliderBuilder::new(shape(fixed, &g)), None);
        let mut rng = Rng(7);
        let mid = n as f32 * SIZE * 0.5;
        let mut bodies = vec![];
        for k in 0..pieces {
            let cells = piece(&mut rng, 6);
            let at = Vector::new(mid + (rng.f() - 0.5) * 6.0, 2.5 + 0.9 * (k / 16) as f32 + rng.f() * 0.3, mid + (rng.f() - 0.5) * 6.0);
            let b = w.insert_body(RigidBodyBuilder::dynamic().translation(at).rotation(Vector::new(rng.f(), rng.f(), rng.f())));
            w.insert_collider(ColliderBuilder::new(shape(moving, &cells)).density(2000.0).friction(0.6), Some(b));
            bodies.push(b);
        }
        let (mut fall, mut worst, mut rest) = (0.0f64, 0.0f64, 0.0f64);
        for tick in 0..900 {
            let t = Instant::now();
            w.step();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            if tick < 240 {
                fall += ms / 240.0;
            } else if tick >= 780 {
                rest += ms / 120.0;
            }
            worst = worst.max(ms);
        }
        let asleep = bodies.iter().filter(|&&b| w.bodies[b].is_sleeping()).count();
        let through = bodies.iter().filter(|&&b| w.bodies[b].translation().y < 0.2).count();
        let fastest = bodies.iter().map(|&b| w.bodies[b].linvel().length()).fold(0.0f32, f32::max);
        println!("  {:<17} {fall:>14.2} {worst:>8.2} {rest:>18.3} {asleep:>12} of {pieces} {through:>16} {fastest:>22.3}", format!("{moving} / {fixed}"));
    }
}

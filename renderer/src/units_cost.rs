//! What a ray costs under software traversal, counted on the CPU (no GPU): the kernel's walks as units.rs transcribes
//! them, over a bundle's scene as `prepare` lays it out, with units and flattened, for the same rays (a frame's kinds:
//! the camera's, shadow rays to a far and to a near light, one bounce and its shadow rays). Counted per ray: the loop
//! iterations (what the loop guard counts), the nodes read of each tree, the triangles tested, the unit records read.
//! The rays are made once from the flattened scene and kept (`/tmp/units-cost`), with its answers: every layout must
//! give them again.
//!
//!   UNITS_COST=out/bundles/kabukicho.js cargo test --release --lib units_cost -- --ignored --nocapture
//!
//! `UNITS_COST_FLAT=1`: the flattened scene's counts too (it is built anyway when the rays are not there yet).
//! `UNITS_COST_VIEW=WxH` (192x108). `UNITS_COST_ASSETS` (`.`).
use crate::units::cpu::{Kernel, Reads, INF, NO_INST, TRACE_CLOSEST, TRACE_SHADOW};
use crate::{bvh, default_presets_dir, prepare_as, scene, units};
use glam::Vec3A;
use std::time::Instant;

/// A scene as the kernel is given it.
struct Laid {
    nodes: Vec<bvh::GpuNode>,
    tri_verts: Vec<[f32; 4]>,
    instances: Vec<scene::GpuInstance>,
    top: u32,
    units: u32,
    /// For the report.
    what: String,
    /// The still scene's triangles (the first so many), and each unit's: its record, its first triangle, how many.
    still: u32,
    unit_tris: Vec<(u32, u32, u32)>,
}

/// A ray and the flattened scene's answer to it (`t`: the hit's distance, INF for none).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Ray {
    o: [f32; 3],
    tmax: f32,
    d: [f32; 3],
    mode: u32,
    t: f32,
    /// 0: the camera's, 1: a shadow ray, 2: a bounce.
    kind: u32,
}

fn lay(world: &crate::world::World, assets: &std::path::Path, with_units: bool) -> (Laid, scene::Scene) {
    let mut sc = scene::build(world, assets, &default_presets_dir(), None).unwrap();
    let (flat, placed) = (sc.tris.len(), sc.instances.iter().map(|i| sc.meshes[i.mesh as usize].count as usize).sum::<usize>());
    let t = Instant::now();
    let p = prepare_as(false, with_units, &mut sc, &mut vec![]);
    let s = t.elapsed().as_secs_f64();
    let (top, count) = (p.params.motion[2].to_bits(), p.params.motion[3].to_bits());
    let stored = p.tri_verts.len() / 3;
    let what = format!("{} triangles stored ({:.1} MiB of vertices + attributes), {} nodes ({:.1} MiB), {} units; flattened {} triangles; prepare {s:.2} s", stored, (stored * 144) as f64 / 1048576.0, p.nodes.len(), (p.nodes.len() * 32) as f64 / 1048576.0, sc.units.as_ref().map_or(0, |u| u.count), flat + placed);
    let instances = if sc.gpu_instances.is_empty() { vec![scene::GpuInstance::default()] } else { sc.gpu_instances.clone() };
    let unit_tris = sc.units.as_ref().map_or(vec![], |u| sc.instances.iter().enumerate().filter(|(r, i)| !i.hidden && u.slots.get(*r).is_some_and(|s| s.is_some())).map(|(r, i)| (r as u32, sc.gpu_instances[r].info[0], sc.meshes[i.mesh as usize].count)).collect());
    (Laid { nodes: p.nodes, tri_verts: p.tri_verts, instances, top, units: if sc.units.is_some() { count } else { 0 }, what, still: sc.tris.len() as u32, unit_tris }, sc)
}

impl Laid {
    /// The closest hit by every triangle tested one by one, the still scene's in the world and each unit's in the
    /// unit's space, as the walk tests them: what the walk must find, whatever its trees are.
    fn brute(&self, o: Vec3A, d: Vec3A, tmax: f32) -> f32 {
        let k = Kernel { budget: u64::MAX, ..self.kernel() };
        let mut best = tmax;
        for ti in 0..self.still {
            best = best.min(k.intersect_tri(o, d, ti, best).unwrap());
        }
        for &(rec, first, count) in &self.unit_tris {
            let r = &self.instances[rec as usize];
            let (lo, ld) = (Kernel::proc_point(r, o), Kernel::proc_dir(r, d));
            for ti in first..first + count {
                best = best.min(k.intersect_tri(lo, ld, ti, best).unwrap());
            }
        }
        best
    }

    fn kernel(&self) -> Kernel<'_> {
        Kernel { nodes: &self.nodes, tri_verts: &self.tri_verts, instances: &self.instances, top: self.top, units: self.units, steps: Default::default(), budget: 1 << 26, deepest: Default::default(), reads: Default::default() }
    }
}

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
}

/// A frame's rays through the flattened scene `l`: the camera's over a `w` x `h` view; from each hit a shadow ray to
/// a far light (one direction for all: a sun), one to a point a few metres off (a lamp), and a bounce (cosine
/// distributed); from the bounce's hit the two shadow rays again.
fn rays(l: &Laid, cam: &scene::CameraData, w: usize, h: usize) -> Vec<Ray> {
    let (pos, fwd, right, up) = (Vec3A::from(cam.position), Vec3A::from(cam.forward), Vec3A::from(cam.right), Vec3A::from(cam.up));
    let sun = Vec3A::new(0.35, 0.8, 0.5).normalize();
    let mut rng = Rng(77);
    let mut out = vec![];
    let ask = |o: Vec3A, d: Vec3A, tmax: f32, mode: u32, kind: u32, out: &mut Vec<Ray>| {
        let hit = l.kernel().trace(o, d, tmax, mode).unwrap();
        let t = if hit.tri == 0xffff_ffff { INF } else { hit.t };
        out.push(Ray { o: o.into(), tmax, d: d.into(), mode, t, kind });
        (t < INF).then(|| {
            // The hit's point, a little off the triangle on the ray's side.
            let v = |k: u32| { let v = l.tri_verts[(hit.tri * 3 + k) as usize]; Vec3A::new(v[0], v[1], v[2]) };
            let n = v(1).cross(v(2)).normalize_or_zero();
            let n = if n.dot(d) > 0.0 { -n } else { n };
            (o + d * hit.t + n * 1e-3, n)
        })
    };
    let lit = |p: Vec3A, n: Vec3A, rng: &mut Rng, out: &mut Vec<Ray>| {
        if n.dot(sun) > 0.0 {
            ask(p, sun, INF, TRACE_SHADOW, 1, out);
        }
        let q = p + n * (0.5 + 3.0 * rng.f()) + Vec3A::new(rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5) * 8.0;
        let (d, far) = ((q - p).normalize_or_zero(), (q - p).length());
        if n.dot(d) > 0.0 {
            ask(p, d, far, TRACE_SHADOW, 1, out);
        }
    };
    for y in 0..h {
        for x in 0..w {
            let (u, v) = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / h as f32 * 2.0);
            let d = (fwd + right * (u * cam.tan_half_fov * w as f32 / h as f32) + up * (v * cam.tan_half_fov)).normalize();
            let Some((p, n)) = ask(pos, d, INF, TRACE_CLOSEST, 0, &mut out) else { continue };
            lit(p, n, &mut rng, &mut out);
            // A cosine-distributed direction about n.
            let (r1, r2) = (rng.f(), rng.f());
            let (t, b) = n.any_orthonormal_pair();
            let (s, phi) = (r1.sqrt(), std::f32::consts::TAU * r2);
            let d = (t * (s * phi.cos()) + b * (s * phi.sin()) + n * (1.0 - r1).max(0.0).sqrt()).normalize();
            let Some((p, n)) = ask(p, d, INF, TRACE_CLOSEST, 2, &mut out) else { continue };
            lit(p, n, &mut rng, &mut out);
        }
    }
    out
}

#[derive(Default, Clone, Copy)]
struct Sum {
    rays: u64,
    steps: u64,
    most: u64,
    r: [u64; 7],
    wrong: u64,
    deepest: usize,
}

/// The rays through `l`, counted; by kind (the camera's, shadow, bounce) and all together.
fn count(l: &Laid, rays: &[Ray]) -> [Sum; 4] {
    let mut sums = [Sum::default(); 4];
    for ray in rays {
        let k = l.kernel();
        let hit = k.trace(Vec3A::from(ray.o), Vec3A::from(ray.d), ray.tmax, ray.mode).unwrap_or_else(|e| panic!("the kernel {e}"));
        let t = if hit.tri == 0xffff_ffff { INF } else { hit.t };
        // A closest hit at the flattened scene's distance (to the rounding of a triangle tested in its own space); a
        // shadow ray blocked or not as there.
        let same = if ray.mode == TRACE_CLOSEST { (t == INF) == (ray.t == INF) && (t == INF || (t - ray.t).abs() <= 2e-4 * (1.0 + ray.t.min(1e4))) } else { (t == INF) == (ray.t == INF) };
        let Reads { still_nodes, top_nodes, mesh_nodes, still_verts, mesh_verts, records, still_steps, .. } = k.reads.get();
        for which in [ray.kind as usize, 3] {
            let s = &mut sums[which];
            s.rays += 1;
            s.steps += k.steps.get();
            s.most = s.most.max(k.steps.get());
            for (a, b) in s.r.iter_mut().zip([still_nodes, top_nodes, mesh_nodes, still_verts / 3, mesh_verts / 3, records, still_steps]) {
                *a += b;
            }
            s.wrong += !same as u64;
            s.deepest = s.deepest.max(k.deepest.get());
        }
        if !same && sums[3].wrong <= 8 {
            println!("  differs: a ray of kind {} from {:?} along {:?}, mode {}: {} here (triangle {}, record {}), {} in the flattened scene; every triangle here tested one by one: {}", ray.kind, ray.o, ray.d, ray.mode, t, hit.tri, if hit.inst == NO_INST { -1 } else { hit.inst as i64 }, ray.t, l.brute(Vec3A::from(ray.o), Vec3A::from(ray.d), ray.tmax));
        }
    }
    sums
}

fn report(name: &str, l: &Laid, rays: &[Ray], pixels: usize, base: Option<f64>) -> f64 {
    let t = Instant::now();
    let sums = count(l, rays);
    println!("{name}: {}", l.what);
    println!("  {:<8} {:>8} {:>9} {:>8} | {:>9} {:>9} {:>9} | {:>9} {:>9} {:>8} | {:>9} {:>6}", "rays", "count", "steps", "most", "still nd", "top nd", "mesh nd", "still tri", "mesh tri", "records", "bytes", "wrong");
    let mut all = 0.0;
    for (kind, s) in ["camera", "shadow", "bounce", "all"].iter().zip(&sums) {
        let per = |v: u64| v as f64 / s.rays.max(1) as f64;
        // What a ray reads: 32 bytes a node, 48 a triangle, 112 a record's rows.
        let bytes = 32.0 * per(s.r[0] + s.r[1] + s.r[2]) + 48.0 * per(s.r[3] + s.r[4]) + 112.0 * per(s.r[5]);
        println!("  {kind:<8} {:>8} {:>9.2} {:>8} | {:>9.2} {:>9.2} {:>9.2} | {:>9.2} {:>9.2} {:>8.3} | {:>9.0} {:>6}", s.rays, per(s.steps), s.most, per(s.r[0]), per(s.r[1]), per(s.r[2]), per(s.r[3]), per(s.r[4]), per(s.r[5]), bytes, s.wrong);
        all = bytes * s.rays as f64 / pixels as f64;
    }
    let s = &sums[3];
    println!("  a pixel: {:.0} steps ({:.0} of them the still walk's), {:.0} bytes read{}; the stack's deepest {}; counted in {:.1} s", s.steps as f64 / pixels as f64, s.r[6] as f64 / pixels as f64, all, base.map_or(String::new(), |b| format!(" ({:+.1} % on the flattened scene's)", (all / b - 1.0) * 100.0)), s.deepest, t.elapsed().as_secs_f64());
    all
}

/// A top tree's cost to a ray by the surface-area measure: the chance a ray through the root's box meets each
/// node's, summed (interior nodes: two boxes tested; leaves: a unit entered).
fn top_cost(nodes: &[bvh::GpuNode]) -> (f64, f64, usize) {
    let area = |n: &bvh::GpuNode| { let e = Vec3A::from(n.bmax) - Vec3A::from(n.bmin); 2.0 * (e.x * e.y + e.y * e.z + e.z * e.x) as f64 };
    let root = area(&nodes[0]).max(1e-30);
    let (mut inner, mut leaves, mut depth) = (0.0, 0.0, 0);
    let mut stack = vec![(0usize, 0usize)];
    while let Some((i, d)) = stack.pop() {
        depth = depth.max(d);
        let n = &nodes[i];
        if n.count > 0 {
            leaves += area(n) / root;
        } else {
            inner += area(n) / root;
            stack.push((n.left_or_first as usize, d + 1));
            stack.push((n.left_or_first as usize + 1, d + 1));
        }
    }
    (inner, leaves, depth)
}

#[test]
#[ignore = "a measurement: UNITS_COST=<bundle.js>"]
fn units_cost() {
    let Ok(bundle) = std::env::var("UNITS_COST") else { return println!("UNITS_COST=<bundle.js>") };
    let assets = std::path::PathBuf::from(std::env::var("UNITS_COST_ASSETS").unwrap_or(".".into()));
    let view = std::env::var("UNITS_COST_VIEW").unwrap_or("192x108".into());
    let (w, h) = view.split_once('x').map(|(a, b)| (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap())).unwrap();
    let host = crate::script::ScriptHost::new(&std::fs::read_to_string(&bundle).unwrap()).unwrap();
    host.setup("r", None).unwrap();
    // (A voxel level the scene code asked for: loaded before the first scene build, as the session does.)
    if host.voxels.borrow_mut().realize(&mut host.world.borrow_mut(), &assets).unwrap().is_some() {
        let _ = host.world.borrow_mut().take_changes();
    }
    let world = host.world.borrow();
    let stem = std::path::Path::new(&bundle).file_stem().unwrap().to_string_lossy().to_string();
    let kept = std::path::PathBuf::from(format!("/tmp/units-cost/{stem}-{view}.rays"));
    let mut base = std::fs::read_to_string(kept.with_extension("flat")).ok().and_then(|s| s.trim().parse::<f64>().ok());
    if !kept.exists() || std::env::var("UNITS_COST_FLAT").is_ok() {
        let (flat, sc) = lay(&world, &assets, false);
        if !kept.exists() {
            std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
            std::fs::write(&kept, bytemuck::cast_slice::<Ray, u8>(&rays(&flat, &sc.camera, w, h))).unwrap();
        }
        let rays: Vec<Ray> = bytemuck::pod_collect_to_vec(&std::fs::read(&kept).unwrap());
        let b = report(&format!("{stem}, flattened"), &flat, &rays, w * h, None);
        std::fs::write(kept.with_extension("flat"), format!("{b}")).unwrap();
        base = Some(b);
    }
    let rays: Vec<Ray> = bytemuck::pod_collect_to_vec(&std::fs::read(&kept).unwrap());
    let (with, mut sc) = lay(&world, &assets, true);
    let Some(u) = sc.units.as_mut() else { return println!("{stem}: no units") };
    report(&format!("{stem}, units"), &with, &rays, w * h, base);
    // The top tree: its build's time, its depth, its cost by the surface-area measure.
    let (mut ms, mut refit) = (vec![], vec![]);
    let mut top = vec![];
    for _ in 0..20 {
        u.kept.nodes.clear();
        let t = Instant::now();
        top = u.top().unwrap();
        ms.push(t.elapsed().as_secs_f64() * 1e3);
        // (Again with nothing moved: the kept tree, its boxes brought up to date and checked.)
        let t = Instant::now();
        assert_eq!(bytemuck::cast_slice::<_, u8>(&u.top().unwrap()), bytemuck::cast_slice::<_, u8>(&top));
        refit.push(t.elapsed().as_secs_f64() * 1e3);
    }
    ms.sort_by(f64::total_cmp);
    refit.sort_by(f64::total_cmp);
    if !top.is_empty() {
        let (inner, leaves, depth) = top_cost(&top);
        println!("  the top tree: {} units, depth {depth}, built in {:.3} ms, kept (refit) in {:.3} ms (medians of 20); by surface area a ray through its root's box tests {:.2} pairs of boxes and enters {:.2} units", u.count, ms[10], refit[10], inner, leaves);
    }
    // The units' transforms, and how many units a mesh has.
    let (mut identity, mut moved, mut scaled, mut general) = (0, 0, 0, 0);
    for i in sc.instances.iter().filter(|i| !i.hidden) {
        let m = glam::Mat3A::from_mat4(i.world);
        let diagonal = m.x_axis.y == 0.0 && m.x_axis.z == 0.0 && m.y_axis.x == 0.0 && m.y_axis.z == 0.0 && m.z_axis.x == 0.0 && m.z_axis.y == 0.0;
        if m == glam::Mat3A::IDENTITY {
            *(if i.world.w_axis.truncate() == glam::Vec3::ZERO { &mut identity } else { &mut moved }) += 1;
        } else if diagonal {
            scaled += 1;
        } else {
            general += 1;
        }
    }
    let mut uses = vec![0usize; sc.meshes.len()];
    for i in sc.instances.iter().filter(|i| !i.hidden) {
        uses[i.mesh as usize] += 1;
    }
    let once: Vec<usize> = (0..uses.len()).filter(|&k| uses[k] == 1).collect();
    let tris = |ks: &mut dyn Iterator<Item = usize>| ks.map(|k| sc.meshes[k].count as usize * uses[k]).sum::<usize>();
    println!("  transforms: {identity} the identity, {moved} a translation alone, {scaled} a scale along the axes, {general} others");
    println!("  meshes: {} with one unit ({} triangles), {} with more ({} units, {} triangles placed, {} stored)", once.len(), tris(&mut once.iter().copied()), uses.iter().filter(|&&n| n > 1).count(), uses.iter().filter(|&&n| n > 1).sum::<usize>(), tris(&mut (0..uses.len()).filter(|&k| uses[k] > 1)), (0..uses.len()).filter(|&k| uses[k] > 1).map(|k| sc.meshes[k].count as usize).sum::<usize>());
    let _ = units::MAX_UNITS;
}

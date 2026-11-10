//! Walkable ground of a scene on an XZ grid (games/bistro-crowd: where its walkers and vehicles may go).
//!   cargo run --release --example walkable -- <bundle.js> <assets> <x0> <z0> <x1> <z1> <cell m>  > grid.json
//! Per cell, vertical rays (centre and four quarter points) find the ground (the lowest surface within 1.5 m of the
//! street level y0, the first one under 3 m) and whether anything solid stands between 0.15 and 2.2 m above it.
//! Output: {x0, z0, cell, w, h, y: [ground height per cell, null = none], free: "0101.." (1 = clear above the ground)}.
use glam::Vec3A;
use threert_render::{scene, script::ScriptHost};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (bundle, assets) = (&a[1], std::path::Path::new(&a[2]));
    let f = |i: usize| a[i].parse::<f32>().unwrap();
    let (x0, z0, x1, z1, cell) = (f(3), f(4), f(5), f(6), f(7));
    let host = ScriptHost::new(&std::fs::read_to_string(bundle)?)?;
    host.setup("r", None)?;
    let world = host.world.borrow();
    let sc = scene::build(&world, assets, &threert_render::default_presets_dir(), None)?;
    // World-space triangles: the flattened ones and every instance's pool, transformed.
    let mut tris: Vec<[Vec3A; 3]> = sc.tris.clone();
    for inst in &sc.instances {
        let m = sc.meshes[inst.mesh as usize];
        for t in &sc.pool_tris[m.first as usize..(m.first + m.count) as usize] {
            tris.push(t.map(|p| inst.world.transform_point3a(p)));
        }
    }
    let (w, h) = (((x1 - x0) / cell).ceil() as usize, ((z1 - z0) / cell).ceil() as usize);
    // Triangles binned by the grid cells their XZ bounds touch.
    let mut bins: Vec<Vec<u32>> = vec![vec![]; w * h];
    for (i, t) in tris.iter().enumerate() {
        let (lo, hi) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
        if hi.y < -2.0 || lo.y > 6.0 { continue; }
        let (cx0, cz0) = (((lo.x - x0) / cell).floor().max(0.0) as usize, ((lo.z - z0) / cell).floor().max(0.0) as usize);
        let (cx1, cz1) = (((hi.x - x0) / cell).floor() as isize, ((hi.z - z0) / cell).floor() as isize);
        if cx1 < 0 || cz1 < 0 { continue; }
        for cz in cz0..=(cz1 as usize).min(h - 1) {
            for cx in cx0..=(cx1 as usize).min(w - 1) {
                bins[cz * w + cx].push(i as u32);
            }
        }
    }
    // Heights where a vertical line at (x, z) meets the cell's triangles.
    let column = |c: usize, x: f32, z: f32| -> Vec<f32> {
        let mut ys = vec![];
        for &i in &bins[c] {
            let [p0, p1, p2] = tris[i as usize];
            let (e1, e2) = (p1 - p0, p2 - p0);
            let d = e1.x * e2.z - e1.z * e2.x;
            if d.abs() < 1e-12 { continue; }
            let (px, pz) = (x - p0.x, z - p0.z);
            let u = (px * e2.z - pz * e2.x) / d;
            let v = (e1.x * pz - e1.z * px) / d;
            if u < 0.0 || v < 0.0 || u + v > 1.0 { continue; }
            ys.push(p0.y + e1.y * u + e2.y * v);
        }
        ys
    };
    let y0 = 0.33;
    let mut yv: Vec<Option<f32>> = vec![None; w * h];
    let mut free = String::with_capacity(w * h);
    for cz in 0..h {
        for cx in 0..w {
            let c = cz * w + cx;
            let mut ok = true;
            let mut ground = None;
            for (ox, oz) in [(0.5, 0.5), (0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let (x, z) = (x0 + (cx as f32 + ox) * cell, z0 + (cz as f32 + oz) * cell);
                let ys = column(c, x, z);
                let g = ys.iter().copied().filter(|y| (y - y0).abs() < 1.5).fold(f32::NAN, f32::min);
                if g.is_nan() { ok = false; continue; }
                if ys.iter().any(|&y| y > g + 0.15 && y < g + 2.2) { ok = false; }
                if (ox, oz) == (0.5, 0.5) { ground = Some(g); }
            }
            yv[c] = ground;
            free.push(if ok && ground.is_some() { '1' } else { '0' });
        }
    }
    println!("{}", serde_json::json!({ "x0": x0, "z0": z0, "cell": cell, "w": w, "h": h, "y": yv, "free": free }));
    Ok(())
}

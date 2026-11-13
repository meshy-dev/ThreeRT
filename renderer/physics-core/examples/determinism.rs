//! Prints the hash the determinism script leaves after 600 ticks (physics_core::check), natively.
//! examples/determinism_wasm.rs gives the hash in wasm32 (tools/physics-determinism.ts compares them).
//! `--vehicles`: the hash of the vehicles' script (physics_core::check::vehicles) instead.
//! `--cases`: the hash of every random case of physics_core::check instead (hulls, bodies of several colliders, joints).
//! `--bench`: times it instead (the best and the median of 30 runs of the script, and of `check::pile(1000)`: a
//! thousand boxes dropped in a heap, 300 ticks), ms per tick.
fn main() {
    if std::env::args().any(|a| a == "--bench") {
        let time = |f: &dyn Fn() -> u64, ticks: f64| {
            let mut ms: Vec<f64> = (0..30)
                .map(|_| {
                    let t = std::time::Instant::now();
                    std::hint::black_box(f());
                    t.elapsed().as_secs_f64() * 1e3 / ticks
                })
                .collect();
            ms.sort_by(|a, b| a.total_cmp(b));
            (ms[0], ms[15])
        };
        let (s, p) = (time(&|| physics_core::check::script(600), 600.0), time(&|| physics_core::check::pile(1000, 300), 300.0));
        println!("script: best {:.4} median {:.4} ms/tick; pile of 1000: best {:.3} median {:.3} ms/tick", s.0, s.1, p.0, p.1);
        return;
    }
    // `--cases`: every random case's hash, a line each ("kind seed hash").
    if std::env::args().any(|a| a == "--cases") {
        for (kind, &(_, n)) in physics_core::check::KINDS.iter().enumerate() {
            for seed in 0..n {
                println!("{kind} {seed} {:016x}", physics_core::check::case(kind as u32, seed));
            }
        }
        return;
    }
    // `--vehicles`: the vehicles' script's hash (physics_core::check::vehicles, 600 ticks).
    if std::env::args().any(|a| a == "--vehicles") {
        println!("{:016x}", physics_core::check::vehicles(600));
        return;
    }
    println!("{:016x}", physics_core::check::script(600));
}

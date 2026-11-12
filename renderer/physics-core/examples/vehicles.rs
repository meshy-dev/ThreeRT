//! The vehicle's scenarios (`physics_core::vehicle::scenarios`): `--table` (every scenario, one car), `--cost` (the
//! per-step cost with 1 and 20 cars), `--trace <scenario>` (one run's ticks), `--hashes` (every run's hash).
use physics_core::vehicle::scenarios::{passed, run, Sc};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let start = std::time::Instant::now();
    let clock = move || start.elapsed().as_secs_f64();
    let find = |name: &str| Sc::all().into_iter().find(|s| s.name() == name).unwrap_or_else(|| panic!("no scenario {name}"));
    if args.iter().any(|a| a == "--table") {
        println!("scenario\tpass\tflip\tmax_vy\tmax_speed\tsink_max\tsunk\tmissed\tno_contact\twheel_ticks\tspeed_held\tfinal_speed\tend_z\tend_y\tparked_drift\tend_depth\tend_contacts\tbump_ticks\tbump_max\tbottomed\tstop_ticks\trp_rms\tveh_ms\tworld_ms\thash");
        for sc in Sc::all() {
            let r = run(sc, 1, true, false, Some(&clock));
            println!(
                "{}\t{}\t{}\t{:.2}\t{:.2}\t{:.3}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:.1}\t{:.2}\t{:.4}\t{:.3}\t{}\t{}\t{:.3}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:016x}",
                sc.name(), if passed(sc, &r) { "PASS" } else { "FAIL" }, r.flipped, r.max_vy, r.max_speed, r.sink_max, r.sunk, r.missed, r.no_contact, r.wheel_ticks, r.speed_held, r.final_speed, r.end[0], r.end[1], r.parked_drift, r.end_depth, r.end_contacts, r.bump_ticks, r.bump_max, r.bottomed_ticks, r.stop_ticks, r.roll_pitch_rms, r.vehicles_ms, r.world_ms, r.hash
            );
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--cost") {
        let names: Vec<String> = args[i + 1..].iter().take_while(|a| !a.starts_with("--")).cloned().collect();
        let list: Vec<Sc> = if names.is_empty() { vec![Sc::DebrisFixed, Sc::DebrisTravel, Sc::GapsStraight, Sc::Parked] } else { names.iter().map(|n| find(n)).collect() };
        println!("scenario\tcars\tvehicles_ms\tworld_ms");
        for sc in list {
            for n in [1, 20] {
                let r = run(sc, n, false, false, Some(&clock));
                println!("{}\t{n}\t{:.4}\t{:.3}", sc.name(), r.vehicles_ms, r.world_ms);
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--trace") {
        let r = run(find(&args[i + 1]), 1, true, true, None);
        for l in &r.trace {
            println!("{l}");
        }
    }
    if args.iter().any(|a| a == "--hashes") {
        for sc in Sc::all() {
            println!("{}\t{:016x}", sc.name(), run(sc, 1, false, false, None).hash);
        }
    }
}

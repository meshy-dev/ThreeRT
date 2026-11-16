//! The vehicle's scenarios (`physics_core::vehicle::scenarios`), for each car (`--car sedan|suv`, both by default):
//! `--table` (every scenario, one car), `--cost [scenario...]` (the per-step cost with 1 and 20 cars), `--trace
//! <scenario>` (one run's ticks, the first car), `--hashes` (every run's hash).
use physics_core::vehicle::scenarios::{rule, run, verdict, Car, Sc, Verdict, CARS};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let start = std::time::Instant::now();
    let clock = move || start.elapsed().as_secs_f64();
    let find = |name: &str| Sc::all().into_iter().find(|s| s.name() == name).unwrap_or_else(|| panic!("no scenario {name}"));
    let cars: Vec<Car> = match args.iter().position(|a| a == "--car") {
        Some(i) => CARS.into_iter().filter(|c| c.name == args[i + 1]).collect(),
        None => CARS.to_vec(),
    };
    if args.iter().any(|a| a == "--table") {
        println!("car\tscenario\tverdict\trule\tflip\tmax_vy\tcar_max_speed\tmax_speed\tyaw_max\theading\ttilt_max\tleft_road\tsink_max\tsunk\tmissed\tno_contact\twheel_ticks\tspeed_held\tfinal_speed\tend_z\tend_y\tparked_drift\tend_depth\tend_contacts\tbump_ticks\tbump_max\tbottomed\tstop_ticks\tbody_ticks\trp_rms\tveh_ms\tworld_ms\thash");
        for car in &cars {
            for sc in Sc::all() {
                let r = run(car, sc, 1, true, false, Some(&clock));
                let v = match verdict(car, sc, &r) {
                    Verdict::Pass => "PASS".to_string(),
                    Verdict::Fail => "FAIL".to_string(),
                    Verdict::NotApplicable(why) => format!("N/A: {why}"),
                };
                println!(
                    "{}\t{}\t{v}\t{}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.0}\t{:.0}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:.1}\t{:.2}\t{:.4}\t{:.3}\t{}\t{}\t{:.3}\t{}\t{}\t{}/{}\t{:.3}\t{:.3}\t{:.3}\t{:016x}",
                    car.name, sc.name(), rule(car, sc), r.flipped, r.max_vy, r.car_max_speed, r.max_speed, r.yaw_max, r.heading, r.tilt_max, r.left_road.map_or("-".to_string(), |t| format!("{t:.1}")), r.sink_max, r.sunk, r.missed, r.no_contact, r.wheel_ticks, r.speed_held, r.final_speed, r.end[0], r.end[1], r.parked_drift, r.end_depth, r.end_contacts, r.bump_ticks, r.bump_max, r.bottomed_ticks, r.stop_ticks, r.body_ticks, r.ticks, r.roll_pitch_rms, r.vehicles_ms, r.world_ms, r.hash
                );
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--cost") {
        let names: Vec<String> = args[i + 1..].iter().take_while(|a| !a.starts_with("--")).cloned().collect();
        let list: Vec<Sc> = if names.is_empty() { vec![Sc::DebrisFixed, Sc::DebrisTravel, Sc::DebrisBodies, Sc::GapsStraight, Sc::Rim4, Sc::Parked] } else { names.iter().map(|n| find(n)).collect() };
        println!("car\tscenario\tcars\tvehicles_ms\tworld_ms");
        for car in &cars {
            for &sc in &list {
                for n in [1, 20] {
                    let r = run(car, sc, n, false, false, Some(&clock));
                    println!("{}\t{}\t{n}\t{:.4}\t{:.3}", car.name, sc.name(), r.vehicles_ms, r.world_ms);
                }
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--trace") {
        let r = run(&cars[0], find(&args[i + 1]), 1, true, true, None);
        for l in &r.trace {
            println!("{l}");
        }
    }
    if args.iter().any(|a| a == "--hashes") {
        for car in &cars {
            for sc in Sc::all() {
                println!("{} {}\t{:016x}", car.name, sc.name(), run(car, sc, 1, false, false, None).hash);
            }
        }
    }
}

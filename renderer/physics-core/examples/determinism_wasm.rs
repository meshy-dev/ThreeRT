//! The determinism script's hash (physics_core::check) as a wasm32 module's export: the same code as the native
//! examples/determinism.rs, built for `wasm32-unknown-unknown` and run by Node (tools/physics-determinism.ts).
#[no_mangle]
pub extern "C" fn physics_determinism(ticks: u32) -> u64 {
    physics_core::check::script(ticks)
}

/// One random case's hash (physics_core::check::case).
#[no_mangle]
pub extern "C" fn physics_determinism_case(kind: u32, seed: u32) -> u64 {
    physics_core::check::case(kind, seed)
}

/// The vehicles' script's hash (physics_core::check::vehicles).
#[no_mangle]
pub extern "C" fn physics_vehicles(ticks: u32) -> u64 {
    physics_core::check::vehicles(ticks)
}

/// One vehicle scenario's hash (physics_core::vehicle::scenarios: car `car` of `CARS`, the scenario by its index in
/// `Sc::all`; 0 past the last).
#[no_mangle]
pub extern "C" fn physics_vehicle_scenario(car: u32, index: u32) -> u64 {
    use physics_core::vehicle::scenarios::{run, Sc, CARS};
    match (CARS.get(car as usize), Sc::all().get(index as usize)) {
        (Some(c), Some(&sc)) => run(c, sc, 1, false, false, None).hash,
        _ => 0,
    }
}

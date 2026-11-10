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

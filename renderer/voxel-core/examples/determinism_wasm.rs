//! The determinism script's hash (voxel_core::check) as a wasm32 module's export: the same code as the native
//! examples/determinism.rs, built for `wasm32-unknown-unknown` (no imports) and run by Node in CI
//! (tools/voxel-determinism.ts).
#[no_mangle]
pub extern "C" fn voxel_determinism() -> u64 {
    voxel_core::check::script()
}

//! The physics' recorded determinism hashes (physics_core::check) computed in the renderer's dependency graph, as a
//! wasm32 module's exports: tools/physics-determinism.ts builds it for `wasm32-unknown-unknown` and compares them with
//! the recorded ones: the engine's library in a browser steps to the bits the native build does.
//! What this does not show is the glamx patch (third_party/glamx/PATCH.md): in wasm32 an f32's own `acos` is Rust's
//! port of libm, the same function the patch calls by name, so these hashes come out right with or without it
//! (measured). The patch matters natively, where the method is the platform's: the native test
//! `physics::tests::the_renderers_physics_steps_to_the_recorded_bits` fails without it, and every native crate that
//! depends on physics-core needs that test.
#[no_mangle]
pub extern "C" fn physics_script() -> u64 {
    physics_core::check::script(600)
}

#[no_mangle]
pub extern "C" fn physics_cases() -> u64 {
    physics_core::check::cases()
}

/// The voxel bodies' script (renderer/src/voxel.rs `VoxelWorld::check_script`: the level's colliders, a part that
/// topples as a body, carved as it lies): the adapter between the voxels and the physics, in wasm32.
#[no_mangle]
pub extern "C" fn voxel_bodies_script() -> u64 {
    threert_render::voxel::VoxelWorld::check_script()
}

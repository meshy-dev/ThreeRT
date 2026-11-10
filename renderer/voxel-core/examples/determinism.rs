//! Prints the hash the determinism script leaves (voxel_core::check), natively; examples/determinism_wasm.rs gives it
//! in the browser's wasm32. CI compares the two (tools/voxel-determinism.ts).
fn main() {
    println!("{:016x}", voxel_core::check::script());
}

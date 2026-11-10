# glamx 0.3.1, patched for ThreeRT

This is glamx 0.3.1 from crates.io (MIT OR Apache-2.0, the licenses beside this file) with one change, in
`src/eigen3.rs` (`SymmetricEigen3::eigenvalues`), and a lint allowed. `renderer/Cargo.toml` and `renderer/physics-core/Cargo.toml` use it
through `[patch.crates-io]`.

## Why

Rapier's physics must end in the same bits natively and in wasm32 (ROADMAP § 6 step 7: players run the game in the
browser, and step 9 replays commands). The principal axes of a body's inertia come from this eigen decomposition
whenever the inertia tensor is not diagonal (a convex hull, a body of several colliders), and it calls `r.acos()` and
`phi.cos()` with method syntax. On an `f32` that is the type's own method (the platform's libm) in every crate graph
that has `std` in it: glamx depends on num-traits, and any crate that turns on `num-traits/std` makes those methods
visible here, whatever glamx's own features are. Rapier's `enhanced-determinism` and building without `std` do not
reach it. Natively (macOS) and in wasm32 the eigenvalues then differ in the last bit: 48 of 1,000 random worlds ended
in other bits (`physics_core::check::cases`).

## What

- `acos` and the two `cos` are called through `simba::scalar::ComplexField` by name: libm's under simba's
  `libm_force` (which `enhanced-determinism` turns on), on every platform and in every crate graph.
- The five `powi(2)` are written as products (`powi` resolves to a multiplication with `std` and to `powf(x, 2.0)`
  without).

- `src/lib.rs` allows unused imports (as a path dependency the crate's warnings show).

Nothing else differs from the published crate (`.github`, `.gitignore` and cargo's packaging files are left out).
To update: copy the new version here, apply the same change, run `node tools/physics-determinism.ts` and
`cargo test --release` in `renderer/physics-core` and `renderer`.

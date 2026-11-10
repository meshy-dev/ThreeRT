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

## Upstream

The same finding and the same kind of change are open upstream, not merged or released when this copy was made
(2026-11-10):

- dimforge/glamx issue #9, "SymmetricEigen3::eigenvalues calls platform acos/cos in std builds, breaking
  enhanced-determinism": https://github.com/dimforge/glamx/issues/9
- dimforge/glamx pull request #10, "Route eigenvalue trigonometry through ComplexField under std":
  https://github.com/dimforge/glamx/pull/10
- dimforge/rapier issue #1054 lists about twenty more platform math calls in Rapier and parry 0.36.1 / 0.31.1; which
  of them our worlds reach, and what is done about each, is in docs/ROADMAP.md § 6 step 9.

**Delete this copy and both `[patch.crates-io]` tables once a glamx release has the fix and Rapier uses it**; the
recorded hashes (`physics_core::check`) and `tools/physics-determinism.ts` say whether the release behaves the same.

## Every crate that is built must name the patch

`[patch.crates-io]` only takes effect in the manifest of the crate (or workspace root) that cargo is building: a
dependency's own `[patch]` is ignored. Today that is `renderer/Cargo.toml` and `renderer/physics-core/Cargo.toml`.
**Any new crate that depends on physics-core (the web build's wasm crate, a tool, a bench) must repeat it in its own
manifest**:

```toml
[patch.crates-io]
glamx = { path = "<path to>/renderer/third_party/glamx" }
```

Left out, nothing fails to build: the crate silently gets crates.io's glamx and, with `std` anywhere in its graph,
the platform's math functions again. What shows it is a test in that crate that steps physics-core's recorded script
and cases and compares the hashes, as the renderer's does (`renderer/src/physics.rs`,
`the_renderers_physics_steps_to_the_recorded_bits`):

```rust
use physics_core::check;
assert_eq!(check::script(600), check::EXPECTED);
assert_eq!(check::cases(), check::EXPECTED_CASES);
```

Every such crate needs that test (for a wasm crate: run in wasm32, as `tools/physics-determinism.ts` runs
physics-core's). Without the patch the renderer's fails: `f66d349c1eae947a` against the recorded case hash.

Nothing else differs from the published crate (`.github`, `.gitignore` and cargo's packaging files are left out).
To update: copy the new version here, apply the same change, run `node tools/physics-determinism.ts` and
`cargo test --release` in `renderer/physics-core` and `renderer`.

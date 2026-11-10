//! The voxel engine's objects (docs/VOXEL.md, "Voxel data"; ROADMAP § 6 step 2c-4): a sparse grid of 8³ bricks with a
//! byte per voxel, its layout as procedural geometry data (the words `VOXEL_BRICKS` reads, packages/voxel/src/grid.ts),
//! greedy meshing (the same faces as packages/voxel `greedyMesh`), and the level: objects in one world lattice, edits in
//! fixed point (carve, paint, add), connectivity to the anchors, loose parts falling (`level`).
//!
//! Data in, data out: no GPU, no file system, no threads, so it builds for `wasm32-unknown-unknown` (players run the
//! game in the browser). The renderer's adapter (renderer/src/voxel.rs) reads the files and hands the results to the
//! world.

pub mod check;
mod fixed;
mod grid;
mod lattice;
mod layout;
mod level;
mod mesh;
mod place;

pub use fixed::{to_fp, Shape, FP, MAX_CELLS, MAX_RADIUS};
pub use grid::{VoxelGrid, BRICK};
pub use lattice::Key;
pub use level::{gravity_for, Edit, Fall, Level, Object, Tick, FALL_FP, HARD, INDESTRUCTIBLE, MEDIUM, MIN_PIECE, RADII, SOFT};
pub use place::Placement;
pub use layout::{decode, layout, Granularity, Layout, LayoutError, HEADER_WORDS};
pub use mesh::{greedy_mesh, merge_by_class, MeshPart, VoxelMesh};

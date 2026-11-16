//! ThreeRT's physics (ROADMAP § 6 step 7): rigid bodies, colliders, joints, contact events and scene queries, on Rapier.
//! The world is the engine's: it is stepped at a fixed tick (`TICK`), and game scripts reach it only through the
//! calls of `bridge::Bridge` (which also places the scene objects that bodies drive), never Rapier itself. Engine code
//! may extend it with its own collider shapes (`CustomShape`: the voxel shape of step 8).
//!
//! Data in, data out: no GPU, no file system, no threads, so it builds for `wasm32-unknown-unknown` (players run the
//! game in the browser).

pub mod bridge;
pub mod check;
mod world;

pub use world::{
    limits, BodyDesc, BodyId, BodyKind, ColliderDesc, ColliderId, Contact, CustomShape, Fault, Hit, JointDesc, JointId, JointKind, Limits, Motor, Physics, Shape, SharedVoxels, TICK,
};
pub use world::vehicle;
/// Rapier itself, for engine code that implements `CustomShape` (not for game scripts).
pub use rapier3d;

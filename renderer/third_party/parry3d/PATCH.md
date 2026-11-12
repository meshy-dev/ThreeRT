# parry3d 0.31.1, patched for ThreeRT

This is parry3d 0.31.1 from crates.io (Apache-2.0: `LICENSE`, taken from the upstream repository, since the crate
does not ship it) with one change, in `src/query/ray/ray_voxels.rs` (`impl RayCast for VoxelsChunkRef`), and a lint
allowed. Only what the crate needs to build is here: its `src`, its `README.md`, and its `Cargo.toml` without the
examples, tests, benches and dev-dependencies (they are not in this copy). `renderer/Cargo.toml`,
`renderer/physics-core/Cargo.toml` and `renderer/web/Cargo.toml` use it through `[patch.crates-io]`.

## Why

A ray cast at a voxel shape misses its cells when it lies on a plane of the lattice and runs along it: straight
down at x = 1.5 m onto cells of 0.1 m, which is where a game asks for the ground (a vehicle's wheels, a character's
feet, a click). Through Rapier's query pipeline, 4,000 rays straight down at a slab missed 320 with x on a plane,
872 with x and z on planes and 600 with z on a plane (0 mid-cell; measured for ThreeRT's vehicle comparison,
2026-11-12).

The walk over a chunk's cells reached the right solid cell, then asked that cell's box for the hit again
(`aabb.cast_local_ray_and_get_normal`). A ray in one of the box's face planes grazes the box: depending on the
rounding of the box's bounds (`voxel_aabb`: centre plus or minus half a cell) it answered none. The walk then went
on to the next cell, left the chunk and reported a miss.

ThreeRT had worked round it with a walk of its own over every voxel collider (`voxel_ray` in physics-core). That
walk is gone with this patch: every ray now goes through Rapier's query.

## What

In `VoxelsChunkRef::cast_local_ray_and_get_normal` (the chunk level, `impl RayCast for Voxels`, is unchanged):

- The ray is clipped to the chunk's box, whose bounds are the lattice planes `key * voxel_size`. The walk keeps the
  axis it entered by.
- A solid cell is a hit where the ray entered it, on the face it crossed:
  - the normal points against the ray along that axis;
  - the feature is `Face(axis)` or `Face(axis + 3)`, as the box reports;
  - on a tie between axes, the lower axis is taken (the box reported `-dir` on an exact edge).
- A ray that starts in a solid cell (the one its origin floors into) is, cast `solid`, a hit at 0 with no normal, as
  every shape answers from inside; the cell's box is not asked (it may round off a plane the origin lies on and
  graze: found by Bugbot on the first draft). Only the hollow cast still asks that box where the ray leaves it.
- The next cell is the one across the nearest lattice plane ahead, computed from the integer key. The walk never
  steps back behind where the ray already is.
- A ray running along the planes of an axis belongs to the cells its coordinate floors into, the same rule as
  `voxel_at_point`. On a plane between two chunks it belongs to the upper one. Both chunks decide this from one
  floor, so exactly one of them takes the ray.

The chunk level needed no change. The `Bvh` tests the chunks' boxes inclusively, so rays in a chunk's face plane
reach both chunks, and the walk above decides which one takes them. The test rays include rays on the chunks' faces
(every 8 cells) and on the shape's own faces.

Also, `src/lib.rs` allows unused imports: as a path dependency, the crate's warnings show.

Not touched: contacts (`VoxelsWithBalls`, the dispatcher), shape casts and point queries. They do not call this
function.

## Checks

- physics-core `rays_at_voxel_shapes`, through the world's query. It compares against testing every solid cell's
  box, or against the floor rule where the boxes' own rounding is the question:
  - random rays at a turned shape, including rays from inside its cells;
  - 4,000 rays straight down each with x on a plane, x and z on planes, z on a plane, and both mid-cell: 0 misses
    (unpatched, the first group already fails);
  - rays along planes, edges and vertices, and on the chunks' and the shape's faces;
  - a shape far out at negative coordinates;
  - an edited shape.
- Against the walk it replaces, over 48,000 rays (four scenes):
  - every answer is the same: distance to six decimals, normal exactly;
  - the only difference is the 2,090 rays that start inside a cell: distance 0 in both, normal zero here against
    `-dir` before.
- Time (one release build on an M-series Mac): 1.1 to 1.7 times the old walk's. A turned shape takes 206-214 ns a
  ray against 184-192; a slab, 281-295 ns against 164-187.
- The recorded hashes (`physics_core::check`, the voxel bodies' script) and `tools/physics-determinism.ts` are
  unchanged.

## Upstream

The report is drafted in `out/upstream/parry-voxel-ray/DRAFT.md` (minimal repro, cause, this patch). It has not
been sent.

**Delete this copy and the three `[patch.crates-io]` lines once a parry release answers rays on lattice planes and
Rapier uses it.** `rays_at_voxel_shapes` (0 misses on the planes, brute force everywhere else) and the recorded
hashes say whether the release behaves the same.

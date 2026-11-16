# parry3d 0.31.1, patched for ThreeRT

This is parry3d 0.31.1 from crates.io (Apache-2.0: `LICENSE`, taken from the upstream repository, since the crate
does not ship it) with two changes, each below, and a lint allowed:

- rays: `src/query/ray/ray_voxels.rs` (`impl RayCast for VoxelsChunkRef`);
- sweeps: `src/query/nonlinear_shape_cast/nonlinear_shape_cast_voxels_shape.rs` (`cast_shapes_nonlinear_voxels_shape`,
  the cast Rapier's continuous collision detection makes for a fast body against a voxel collider).

 Only what the crate needs to build is here: its `src`, its `README.md`, and its `Cargo.toml` without the
examples, tests, benches and dev-dependencies (they are not in this copy). `renderer/Cargo.toml`,
`renderer/physics-core/Cargo.toml` and `renderer/web/Cargo.toml` use it through `[patch.crates-io]`.

# Rays

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

**Delete this change once a parry release answers rays on lattice planes and Rapier uses it**; the copy and the
three `[patch.crates-io]` lines go when neither change is left. `rays_at_voxel_shapes` (0 misses on the planes, brute
force everywhere else) and the recorded hashes say whether the release behaves the same.

# Sweeps

## Why

Rapier 0.36 sweeps every fast dynamic body against the fixed colliders in its way (`ccd_solver.rs`
`update_ccd_active_flags`: fast is moving more than half the body's thinnest collider in a tick; `ccd_enabled` only
adds sweeps against moving bodies). Against a voxel collider that sweep is parry's
`cast_shapes_nonlinear_voxels_shape` (Rapier's `sweeps.rs` sends voxels to the dispatcher's nonlinear cast: they are
not among its composite shapes). It walked the cells of the shape's box and cast each cell's box exactly (parry's
support-map nonlinear cast, a few to some tens of microseconds each). A car's stops (its tyres' hulls) make the
car fast above about 3 m/s, and then its chassis box is swept too: on a voxel slope its box, tilted, holds some 1,400
cells a cast, and the car's step cost 1.7-2.0 ms where flat ground costs 0.02 (2026-11-13, ROADMAP § 6 step 10 c2).

The walk also missed cells (found by the brute-force test below; all three upstream):

- its window kept the size of the shape's box at the start in cells and slid both ends together, so a box that
  straddles one more row than it did at the start dropped a row it still overlapped (four of the recorded voxel
  cases: a ball's step that never reached a cell the ball meets);
- it moved the window by the shape's velocity in the world's axes over a grid in the voxel shape's own: a turned
  voxel shape (a tumbling lump) was walked in the wrong direction;
- a shape turning about a centre off it (a car's part) moves further than its box moved with the linear velocity.

## What

In `cast_shapes_nonlinear_voxels_shape` (the comment in the code has each step's reasoning):

1. **The walk.** The shape's box at the start, in the voxel shape's frame, grown by the turn's reach (the turn over
   the times by the farthest point's distance from the motion's centre of rotation) and a margin (2 % of a cell), moved
   by the shape's velocity in that frame: row by row along the axis it moves most along, the cells of the row it
   overlaps while it is in the row. Every cell it overlaps at some time, once.
2. **A cull.** The shape's box oriented with it at the start and grown by how far any point can move: a cell outside
   it (by a face axis of either box) is never touched.
3. **A first bound.** The shape is taken moving without turning: at its orientation halfway, its box's centre on
   the straight line between where the motion has it at the start and at the end. The motion keeps every point
   within `r θ²/8 (1 + θ/2) + d θ/2` of that (`r` the centre's distance from the centre of rotation, `θ` the turn,
   `d` the box's half diagonal); each cell's box is grown by that and the margin, and the time it first meets the
   shape's box so moving (a slab test) bounds its contact. A cell never met is never touched.
4. **Best first.** A queue of cells by bound: a cell's first bound, then a conservative-advancement time of impact
   (`sweep_time_of_impact`, what Rapier sweeps composite shapes with) of its grown box against the shape's proxy so
   moving, which is exact without a turn, then the exact cast, the same call as before. The queue stops at the first
   bound past the earliest hit. Of the hits, the earliest is kept; of equal ones, the first cell walked.

The answer is the one casting every solid cell gives, to the bit, except where parry's cast of a cell reports a hit
before that cell's contact (it can, by millimetres, for a shape turning fast) and before the cell's bound: the cell
is then not cast and the answer is later, nearer the contact. Only `+ - * /`, `sqrt`, `abs`, `min`, `max`, `floor`
and `ceil`; the bound's proxies and time of impact are parry's own (`query::sweep_toi`, already on Rapier's path for
meshes).

Not touched: the shape-to-voxels variant (it calls this one), contacts, rays (above), the shape cast without rotation
(`cast_shapes` at voxels).

## Checks

- physics-core `voxel_sweeps_against_every_cell`: 3,000 random sweeps (20,000 run once while writing it) against
  casting every solid cell's box on its own: scattered cells and a staircase slab, cells of 5, 10 and 25 cm; balls,
  boxes, capsules, hulls and cylinders (no proxy: cast without the second bound); up to 30 m/s, turning up to 30 rad/s
  and some at 250, about centres up to 1 m off the shape; starts overlapping cells (about three in ten); both
  `stop_at_penetration`; the voxel shape moving and turned. All 20,000 the same to the bit (time, normal, witness).
  Unpatched, trial 445 already fails (a ball turning about a centre 1.3 m off it: the third miss above).
- physics-core `a_turning_centre_keeps_to_its_chord`: the bound of step 3 over random motions up to a whole turn.
- The recorded hashes: the script is unchanged; the cases' hash is re-recorded for the four voxel cases above
  (`physics_core::check::EXPECTED_CASES`), each checked to be the brute force's answer. `tools/physics-determinism.ts`:
  native and wasm32 the same.
- Time, one release build on an M-series Mac shared with another session (`examples/ccd_voxels.rs`, and a car on a
  voxel slope): in the PR of 2026-11-13 and `docs/VOXEL.md` "Where the engine works round its dependencies".

## Upstream

The report is drafted in `out/upstream/parry-voxel-sweep/DRAFT.md` (the three misses with repros, the cost, this
patch). It has not been sent.

**Delete this change once a parry release finds every cell a shape sweeps over, at a cost near this one, and Rapier
uses it.** `voxel_sweeps_against_every_cell` and the recorded hashes say whether the release behaves the same, and
`examples/ccd_voxels.rs` what it costs.

# Voxel engine on ThreeRT

Direction of 2026-11-09 (the user's): a voxel game engine built around ThreeRT, for voxel games in general,
measured on a destructible benchmark level. Where the engine needs something every game needs and ThreeRT lacks,
ThreeRT gets it first (through its own gate), then the engine continues. The steps and their acceptance are in
ROADMAP § 6; this file holds the rule, the benchmark level, the research behind the design and the design.

## The rule: what goes into ThreeRT, what stays in the engine

A missing capability goes into ThreeRT when both hold:

1. **General:** at least two different kinds of game need it, e.g. a voxel game and a Bistro-like open world. Not
   voxels only.
2. **A generic contract:** it takes the shape of material functions or per-instance data (WGSL bodies or data the game
   supplies, a feature bit that compiles it out where no scene uses it), not a voxel-specific field in the kernel. This
   is the existing rule (`docs/WORKFLOW.md` scope; ROADMAP § 5 item 6) applied to a new layer.

Everything else stays in the engine layer. The rule guards against two failures: ThreeRT growing into a voxel-only
engine, and the engine forking its own renderer around ThreeRT's gaps.

ThreeRT items are pulled by the engine, not done up front: the engine's next step meets a gap, the gap becomes one
ThreeRT PR at its tier (R1 for new contracts: existing scenes byte-identical, perf smoke on Bistro, cost compiled in
but off), and the engine continues on main. Bistro stays ThreeRT's benchmark; the benchmark level (below) is the
engine's.

| Layer | Where | What |
|---|---|---|
| ThreeRT | `renderer/`, `packages/scene`, `packages/gameplay` | Generic contracts the engine plugs into: procedural geometry, instancing, incremental structure, physics and scene queries, dynamic emission, world data outside the game state |
| Voxel engine | `renderer/voxel-core` (a Rust crate with no GPU, file system or threads: it builds for wasm32, as players run the game in the browser), `renderer/src/voxel.rs` (its adapter: reads the files, hands its meshes and data to the world through ThreeRT's contracts), `packages/voxel` (the game API) | Voxel storage, the voxel intersection body, edits (carve, paint, explosions), connectivity splits, the voxel collision shape, `.vox` import and voxelization |
| Games | `games/` | The benchmark level first, other voxel games after |

What the research below sorts into each layer:

- **ThreeRT** (missing today, needed by every kind of game):
  - Procedural geometry: AABBs plus a game-supplied intersection body, like material functions. SDFs, terrain and
    voxels all plug in here.
  - Instancing of any object, with a TLAS that grows. Only glTF Models instance today, and the TLAS is sized at build
    (`renderer/src/gpu.rs:871`).
  - Incremental structure: adding or removing objects and replacing geometry without the full rebuild
    (`World::set_geometry` sets `structural`, `renderer/src/world.rs:242`; `LiveCore::rebuild` reloads the whole scene,
    `renderer/src/live.rs:1548`; ~0.7 s on Kabukichō, ROADMAP § 4).
  - Physics and scene queries: rigid bodies, contacts, joints, ray / overlap / sweep queries, native (Rust). Done
    (ROADMAP § 6 step 7): `renderer/physics-core` on Rapier, the three.js-style surface in `packages/scene`
    (`mesh.physics`, joints as objects, `Raycaster.intersectPhysics`, `Shapecaster`).
  - Dynamic emission: any geometry can start or stop emitting without a rebuild. Today a material may only become
    emissive if it already had emitter triangles (`renderer/src/tables.rs:144`).
  - World data outside the replicated JSON game state: binary blocks on the node, changed by deterministic commands
    that the room logs and replays. The frame script receives a frozen clone of the state on every commit
    (`packages/gameplay/src/frame.ts`), so voxel data cannot live there.
- **Engine:** voxel storage (sparse 8³ bricks, palettes), the voxel intersection body, `.vox` import, glTF
  voxelization, edit operations, connectivity splits, the voxel collision shape, the game API (`packages/voxel`).

## The benchmark level

A destructible voxel world of our own, at the scale and feature set of the large destructible voxel sandboxes shipped
today, built from CC0 `.vox` content and procedural generation. Its lines:

- **Scale:** about 250 × 25 × 250 m at 10 cm, ~0.5 billion voxels in thousands of objects.
- **Frame:** along a scripted camera path at the 0.5 floor, frame start p95 ≤ 16.7 ms.
- **Destruction:** a large explosion (tens of thousands of voxels carved, the pieces split into bodies) with no frame
  over 50 ms (bench-hitches); about a hundred debris bodies at once within the frame budget.
- **Quality:** ΔE ≤ 3 and SSIM ≥ 0.9 against our offline render, overall and by region (ThreeRT's own target); no
  visible ghosting in motion (within the bench-ghosting limits). Emissive voxels light their surroundings, no seam
  leaks light, every voxel shows its own colour (below).
- **Memory:** ≤ 2 GB per level, the BLASes, triangle data and procedural data together (`tri_blas_mb`, `tri_data_mb`,
  `proc_blas_mb`, `proc_data_mb`); the benchmark level is accepted against it. Today: as meshes 516 MB (BLASes 256.4,
  data 259.6), as region boxes 267 MB. If destruction's pieces or more content bring it near, compressed attributes,
  BLAS compaction and LOD come forward.
- **Features:** fire, smoke, water, vehicles, joints, ropes; a game API over voxel edits, bodies, joints, lights,
  scene queries and effects.

What the look must deliver, each a measured figure (the user's targets, 2026-11-10, where given; the rest set as each is
first measured; ROADMAP § 6):

- **Lighting:** the live frame against our offline render of the same view: ΔE ≤ 3 and SSIM ≥ 0.9, overall and per
  region, on the camera path's keys.
- **Emissive voxels light their surroundings:** with the lamps light-sampled (step 3), the ground and walls within a
  few metres of a lamp at night against the offline render, by region; and the same region with the lamp's voxels
  turned off reads darker (the emission reaches them, not only the camera).
- **No light leaks:** a closed room of the level (a building's storey with its door and windows shut) reads black
  inside under the sun, in the live frame and offline; a mesh's T-junctions (greedy meshing) must not let light
  through: the room's mean against 0, and the count of pixels above the noise floor.
- **Per-voxel colour:** every voxel shows its own palette entry, at any box granularity or as a mesh: a single-voxel
  checker against its reference (layer A `voxel/*` today), and the level's renders across representations (2c-3:
  ΔE 0.21, SSIM 0.997 mesh against boxes).
- **Frame time under destruction:** the frame of an explosion ≤ 50 ms; while destruction goes on (several explosions
  a second), frame start p95 ≤ 16.7 ms (step 2c-4 measures what the representations allow).

The level today (ROADMAP § 6 step 2c-1), `games/voxel-level`:

- **Plan and voxels apart.** `plan.ts` decides from a seed where everything is (no voxels); `tools/voxel-level/
  generate.ts` builds each distinct geometry's voxels into an asset file (`assets/voxel/level/s<seed>v<version>-<granularity>/`,
  in the machine's asset store, not the repo); `level.game.ts` places the objects from the same plan and names the files,
  so the voxels never pass through the scene runtime. `scripts/setup.ts` generates the region-box level when it is
  missing.
- **Contents** (seed 1): terrain on bedrock in 1600 tiles of 64 × 64 voxels (hills, a grid of roads with sidewalks
  and road lines), 172 buildings of 2–7 storeys (each four objects: shell with recessed windows and a door, floors with
  a stairwell, interior walls, a flat or pitched roof), 1884 props (900 trees, 439 parked cars, 172 crates, 172
  barrels, 144 lamp posts, 57 benches; 14 geometries, one per kind and variant, shared). Four ground strips run from
  the level's edge to the horizon (outside it only: its valleys lie below the edge height).
- **Numbers** (seed 1): 4172 objects, 2230 distinct geometries (one file each: the plan names geometries alike the same,
  `buildingGeometry` and `tileGeometry`; until 2026-11-09, 2302 files of which 72 were duplicates), 514.8 M voxels in
  1.32 M bricks; **440.6 k bricks exposed** (a voxel with an empty face neighbour; terrain judged against the whole
  level's ground), **888.7 k full** (bedrock and deep ground: they count toward the total, hardly toward tracing);
  261.5 MB of voxel data (262 MB of region-box files). Boxes: 1.32 M per brick, 46.9 k per 32³ region, 4172 per object,
  all from the same data (`--granularity`, every object). Generated in ~4 s per granularity on the M5 Pro; loaded in
  ~0.7 s.
- **Version 2** (ROADMAP § 6 step 6; `LEVEL_VERSION` in plan.ts, its own asset directories, so checkouts of version
  1 keep their files): everything hangs together by voxel faces. Before, a pitched roof's steps touched each other and
  the walls by edges only (59–83 loose strips a roof), and tree crowns held single loose voxels: 4,552 groups of
  247,430 voxels joined to nothing. Roof steps are three voxels deep and props are one connected piece now. 515.3 M
  voxels in 1.32 M bricks (442.2 k exposed), 262.4 MB; 13,231,484 triangles. The figures of the sections below up to
  "Measured: connectivity" are version 1's.
- **Camera path** (`cameraPath`): 60 s from eye height down a street, along an avenue, over the rooftops, out to the
  whole level; its heights follow the ground.
- **Lighting:** sun and sky; at night the lamps, their voxels emitters on the native mesh route (step 4, "Measured:
  lighting on voxels").

### Measured: the box granularities (2026-11-09)

`tools/bench-voxel.ts` (ROADMAP § 6 step 2c-2), M5 Pro, live at the 0.5 floor (1280 × 720 window) along the 60 s
camera path; the trace pass from the frame timer's timestamps (no THREERT_PROFILE). Medians of 4 alternated rounds
(two runs of 2, `--merge`), with an A/A of region:

| Boxes | Trace p50 / p95 | Frame GPU p50 / p95 | Frame start p95 | BLAS | Data | Boxes | Geometries / instances | Load |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Region 32³ | 30.1 / 38.5 ms | 33.7 / 42.8 ms | 42.1 ms | 3.6 MB | 263.1 MB | 46.9 k | 2230 / 4172 | 1.05 s |
| Region (A/A) | 30.2 / 39.7 | 33.9 / 43.7 | 43.3 | | | | | |
| Brick 8³ | 29.0 / 42.6 | 32.4 / 47.8 | 47.4 | 99.1 MB | 290.1 MB | 1.32 M | 2230 / 4172 | 1.13 s |
| Object | 20.1 / 24.3 | 24.4 / 28.6 | 28.5 | 1.1 MB | 262.4 MB | 4172 | 2230 / 4172 | 1.04 s |
| Region, all props distinct | 33.6 / 40.7 | 36.8 / 44.5 | 45.1 | 5.4 MB | 314.5 MB | 47.5 k | 4100 / 4172 | 1.45 s |

- BLAS: the procedural BLASes at Metal's own build sizes (`proc_blas_mb`). Data: the procedural region of the
  `env_cdf` binding (`proc_data_mb`: boxes and voxel data of each distinct geometry). Geometries: distinct contents,
  what the GPU holds. Load: the offline render's load phase; first frame live ~1.2–1.4 s.
- Per-round trace p50: region 29.8–31.1, its A/A 29.1–40.5, brick 26.9–37.9, object 17.2–32.4, distinct 31.5–33.9.
  Other processes took 8–34 % of the GPU (mean per run; the desktop compositing the window counts), and in one run
  a browser was active: that run's brick, object and distinct rounds have 58–435 frames over 50 ms. Kept, as every
  round; the medians stand on the other three.
- Object boxes trace a third faster than region boxes. Likely why (not yet measured apart): the hardware calls the
  geometry function once per candidate box, and the function's DDA skips empty bricks cheaply. Brick boxes save no trace time and cost 28× the BLAS memory.
- Sharing flatters the props: with every prop its own geometry (each with its own sizes and its index in materials of
  its ground voxels, so no two are alike; the same boxes as a shared prop of that shape) the trace is 3.5 ms slower,
  BLASes 5.4 MB instead of 3.6, data 51 MB more.
- No box granularity is within 16.7 ms frame start (object 28.5 ms).

### Measured: greedy-meshed triangles (2026-11-09)

ROADMAP § 6 step 2c-3: the same objects as triangles (`--granularity mesh`, `level-mesh.game.ts`; `greedyMesh`, below),
one GLB per geometry placed as a model (one BLAS per file, an instance per placement). Measured as above, all five in
one run: 4 alternated rounds, an A/A of region:

| Representation | Trace p50 / p95 | Frame start p95 | BLAS | Data | Triangles (placed / distinct) | Load |
| --- | --- | --- | --- | --- | --- | --- |
| Region boxes | 29.2 / 38.2 ms | 41.6 ms | 3.6 MB | 263.1 MB | – | 0.68 s |
| Region (A/A) | 29.4 / 38.3 | 41.6 | | | | |
| Brick boxes | 24.9 / 30.4 | 33.6 | 99.1 MB | 290.1 MB | – | 0.79 s |
| Object boxes | 17.3 / 20.5 | 23.9 | 1.1 MB | 262.4 MB | – | 0.69 s |
| Region, all props distinct | 31.5 / 39.2 | 42.5 | 5.4 MB | 314.5 MB | – | 0.89 s |
| **Greedy mesh** | **2.19 / 3.49** | **16.67** | 257.7 MB | 260.9 MB | 13.25 M / 1.90 M | 0.36 s |

- The mesh traces 13× faster than region boxes and 8× faster than object boxes, and holds 60 fps along the whole path
  in every round (frame start p95 16.67 ms; trace p50 per round 2.16–2.90). Its cost is memory: BLASes 258 MB (the
  triangle BLASes at Metal's sizes, 192 MB, and their vertex buffer, 65 MB) against 3.6 MB of boxes; its data (the
  triangles' vertices, attributes and BVH nodes, `tri_data_mb`) about the voxels' 261 MB. And an edit changes the
  mesh: step 6 meshes again and rebuilds that object's BLAS, where the voxel data changes only its bricks.
- The images agree: overview at 32 spp, mesh against region boxes, luminance 0.996, mean ΔE 0.21, SSIM 0.9974. The
  mesh's lamps do not glow (emissive triangles would be light-sampled and the voxels are not yet: step 3), the rest is
  the same surface.
- 1.90 M triangles cover 47.8 M exposed voxel faces (as placed): 3.6 faces per triangle on average. The level's outer
  walls are included (after Bugbot: +1412 triangles; the run above was measured without them, behind the horizon).
  Generated in 23 s; 143 MB of GLB. The table's run had 72 duplicate files as separate meshes (models are shared by
  file); named alike since: 1.89 M distinct triangles, BLASes 256.4 MB (−1.5), the same image.
- A/A: trace p50 0.18 ms, frame start p95 0.06 ms. Brick boxes read 24.9 ms here against 29.0 in 2c-2's runs: compare
  within a run. The frame's GPU p95 is not a usable figure on this machine (region against its A/A: 93.6 and 149.5 ms,
  with frame start p95 at 41.6 in both): read the trace and frame start.

### Measured: destruction (2026-11-10)

ROADMAP § 6 step 2c-4: the level loaded by the native voxel objects (renderer/voxel-core, renderer/src/voxel.rs),
explosions applied in place (live.rs `edit_geometry`), pieces split by 4³ cell (8 voxels or more), at rest.
`tools/bench-destruction.ts`: M5 Pro, 0.5 floor, camera still at the path's start, rounds alternated with an A/A of
each route, `THREERT_EDIT_RESERVE=4`. An explosion's frame: the worse of the frame that fires it and the one that
applies its edit (interval, what a player sees). Three routes:

- **Triangles, pieces batched** (the choice): each object one own-BLAS mesh per material; an explosion's pieces one
  mesh per material, rebuilt every frame as if they moved (the same triangle count: rewritten in place, its BLAS
  rebuilt), merged back into the objects they came from 1.5 s after (the stand-in for "settled"; bodies are steps 7
  and 8). 4 rounds (8 runs a figure).
- Triangles, each piece its own object: 2 rounds (4 runs), after the fix below.
- Object boxes (procedural geometry, `VOXEL_BRICKS`): 4 rounds (8 runs).

| One explosion | Voxels / touched / pieces | Batched: frame p50 (max) | Each piece an object | Boxes |
| --- | --- | --- | --- | --- |
| 0.5 m | 268 / 1 / 9 | **27.8** (29.6) ms | 23.6 (25.0) | 17.0 (17.1) |
| 1 m | 1,459 / 3 / 47 | **26.1** (28.3) | 31.8 (32.7) | 17.1 (17.3) |
| 2 m | 17,758 / 12 / 359 | **29.1** (31.7) | 38.1 (44.7) | 17.6 (17.7) |
| 4 m | 25,169 / 8 / 979 | **38.1** (43.5) | 58.7 (64.9) | 23.9 (24.8) |

| Going on: frame start p95 (frames over 50 ms in 8 runs); trace | Batched | Each piece an object | Boxes |
| --- | --- | --- | --- |
| Sustained: 7 a second (1 m), 10 s | **16.67** ms (0); 2.0 ms | 16.67 (0); 2.1 | 20.1 (0); 15.1 |
| Peak: 20 within a second (1 m) | **16.67** (0); 1.9 | 16.67 (0); 1.8 | 18.9 (0); 14.2 |
| Stress: one every frame, 10 s | 28.9; falls back from the 176th (triangle room; since step 5: never, the room grows) | – | 40.9; from the 85th |

- **The batched route meets both lines**: the 4 m explosion's frame ≤ 50 ms, frame start p95 ≤ 16.7 ms while
  destruction goes on (every run 16.67; no frame over 50 ms in any run). Its batches cost every frame: native 0.3–1.7 ms
  (5.8–17 k triangles, up to 20 batches alive at the peak), the in-place rewrite 2.6–3.8 ms of CPU, BLAS / TLAS builds
  4.1–5.2 ms on the GPU (`THREERT_PROFILE_BLAS`); frames between explosions stay at 16.6–16.7 ms. A batch merging back
  (its objects, and the terrain tiles beside the restored voxels, meshed again) costs a 19–21 ms frame (max 25).
- Its 4 m explosion: the renderer's work 18 ms (carve 1.6, the touched objects meshed again 10.7, the 979 pieces meshed
  and batched 6.0), the in-place edit 4.7 ms of CPU for 32 meshes (building triangles 0.8, uploading 1.1, creating and
  recording BLASes 0.6, records, object entries and TLAS slots the rest). An edit's writes go merged, one per
  contiguous range of each buffer; its new BLASes and the TLAS are recorded into one encoder, one submit, and edit
  meshes' BLASes are built fast (`PREFER_FAST_BUILD`; against fast trace, 2 rounds: the same frames and trace within
  noise, 4 m 33.5 / 38.0 ms, trace 1.98 / 2.06 ms).
- **What one piece costs as its own object** (4 m, 979 pieces, 1152 meshes): meshing 4.4 µs and placing it 0.5 µs
  natively (no JavaScript: `voxelExplode` creates the pieces in the renderer; the frame log's `js` is that call), then
  the in-place edit ~23 µs a mesh of CPU (building triangles 1.1 ms in all, uploading 9.3, creating and recording BLASes
  4.0, records, object entries and TLAS slots the rest) and ~13 µs a mesh of BLAS builds on the GPU (14.7 ms max). A
  fixed cost per object, which batching pays once per material.
- Found on the way: `World::set_geometry` looked through every object for a flattened mesh using the geometry, also for
  a new id no object can use: ~40 µs a geometry at 10 k objects, 44 ms of a 4 m explosion's 99 (the first measurement:
  25 / 32 / 60 / 99 ms for 0.5–4 m, each piece an object). It now skips the scan for a new id.
- Boxes: explosions stay cheap (≤ 25 ms), but their trace (14–15 ms at this view; 2c-3: 17.3 ms along the path) keeps
  frame start over 16.7 ms with or without them.
- Loaded natively: 4172 objects from 2230 files decoded in 0.93 s, placed (and, as triangles, meshed) in 1.7 s, first
  frame 3.7 s; as triangles 13,251,212, the TS generator's count to the triangle. Meshing (voxel-core, every file of the
  level): a 64 × 64 terrain tile 0.57–0.59 ms median (1.0 ms max), a building part 0.37–0.44 ms (2.1–2.3 ms max),
  building each column's bits from bricks and writing the vertices included (published binary meshers: 50–200 µs per
  62³ chunk from ready bit masks).
- Memory (computed): triangles 516 MB plus 340 MB of edit room at this reserve; within the level's 2 GB.
- A/A (batched): frame start p95 equal in every pair (16.67); explosion frames p50 within 0.4–3.6 ms (singles) and
  0.0–0.5 ms going on.
- Debris (ROADMAP § 6 step 2e): the voxels too few to make a piece (fewer than 8 in their 4³ cell) thrown as
  instances of one unit cube (`InstancedMesh`; `level-destruction-debris.game.ts`): 30–617 an explosion of 0.5–4 m,
  up to 1,677 in flight going on, explosion frames within 1 ms of batching alone (max 44.6 ms), frame start p95
  16.67 ms, the instance update 0.05–0.11 ms a frame.

**The choice: triangles only, pieces batched.** Considered and dropped: the hybrid (the static world as triangles,
what was just broken and the pieces as boxes until settled). It keeps two representations of every object to maintain;
the browser's `web/` runs no WGSL, so it cannot draw the boxes; and it does not touch the cost per piece, which
batching removes.

### Measured: connectivity and edits (2026-11-10)

ROADMAP § 6 step 6. The level's voxels are a `voxel_core::Level`: every object in one world lattice, edits in fixed
point, and after each edit a connectivity search: what no chain of face-sharing voxels joins to an anchor (the
terrain) is cut loose and falls. The removed voxels are debris, not pieces. Level version 2, `bench-destruction
--routes batch` with an A/A, M5 Pro, 0.5 floor, other processes at 12–17 % of the GPU.

| One explosion | Voxels / objects touched | Frame (A, A/A) | before (step 5) | of it: edit and split / remesh |
| --- | --- | --- | --- | --- |
| 0.5 m | 268 / 1 | 19.6, 20.1 ms | 27.8 | 0.07–0.10 / 1.2 ms |
| 1 m | 1,459 / 3 | 22.8, 22.5 | 26.1 | 0.25–0.27 / 3.8 |
| 2 m | 17,520 / 3 | 22.6, 21.9 | 29.1 | 0.38–0.39 / 2.7–2.9 |
| 4 m (a wall) | 25,169 / 8 | 16.2, 28.3 | 38.1 | 3.2–3.8 / 6.8–7.6 |
| 4 m (open ground: `crater`) | 140,785 / 8 | 29.5, 31.9 | 48.3–53.4 | 2.1–2.2 / 7.4–9.3 |

- Going on: sustained (70 explosions) and peak (20 in a second) hold frame start p95 16.67 ms with no frame over 50 ms
  and no fallback, as before.
- **Explosions that split** (`topple`: 0.6 m at the trunks of eight trees, one every half second): 108 voxels each,
  the crown cut loose as one object; the explosion's frame 19.6–20.9 ms (max 25.9), the frame it lands in 16.3–17.5 ms.
- **The connectivity itself:** 277,681 world bricks of destructible voxels, their components built in 0.62–0.68 s at
  load. An edit's share (changing the voxels and the search) is the "edit and split" column: 2 ms for 140,785 voxels.
- **The limit** (`fell`: a saw cut 1 m thick over 30 x 30 m): 46,469 voxels cut, 20 loose parts of 199,426 voxels,
  buildings among them. The cut's frame 88 ms; while they fall, 26–28 ms a frame, as 280,572 triangles are written
  again every tick. No fallback, no rebuild. A falling object as a moved instance is step 8's.
- **Determinism:** two lockstep runs of the level through `singles`, `fell` and `crater` leave the same hash after
  each (`tools/voxel-determinism-level.ts`); voxel-core's own script gives the same hash natively and in wasm32
  (`tools/voxel-determinism.ts`, in CI).
- **Whole objects are meshed and uploaded again**, not their changed chunks: for the crater's 8 tiles 7.4–9.3 ms of
  meshing, 1.0–1.2 ms of upload, 4.0–4.9 ms of BLAS builds on the GPU. Not changed in this step (ROADMAP § 6 step 6).
- Memory: the lattice and its cache of the anchors' cells are CPU memory, 98 MB for the level (the load log's
  `lattice_mb`). The GPU's is as before: triangles 259.4 MB, BLASes 323.0 MB, and 340 MB of edit room at reserve 4,
  within the level's 2 GB.

### Measured: lighting on voxels (2026-11-10)

ROADMAP § 6 step 4: the level at night (`games/voxel-level/level-night.game.ts`): the sun 18° below the horizon, a
faint sky glow (0.2 nits), the lamps' voxels (20,000 nits, 2304 triangles of 192 lamps) as emitters, sampled by the
light tree; manual exposure EV100 4. `tools/bench-voxel-light.ts`, M5 Pro: ThreeRT live, 64 frames after a history
reset, camera held still, against our offline render of the same world (no clamps, no denoiser, 16384 spp; 4096 still
differs from 16384 by ΔE 0.83 / SSIM 0.966). "Blown up": the lamp ahead of shot 0 exploded in setup (1 m), its pieces
0.3 s into their flight.

| Mean ΔE / SSIM | 0.5 floor: all | worst of 3 × 3 regions | dynamic resolution: all | worst region |
| --- | --- | --- | --- | --- |
| Shot 0 (down the first road, past a lamp) | 2.52 / 0.893 | 5.01 / 0.785 | 2.26 / 0.906 | 4.44 / 0.811 |
| Shot 1 (the corner of the fourth road) | 2.68 / 0.887 | 4.18 / 0.803 | 2.02 / 0.919 | 3.15 / 0.866 |
| Shot 2 (along the fourth road) | 2.34 / 0.907 | 3.83 / 0.840 | 1.92 / 0.928 | 2.87 / 0.890 |
| Shot 0, lamp blown up | 2.58 / 0.894 | 4.25 / 0.822 | 2.03 / 0.915 | 3.21 / 0.849 |

- **Against the targets** (ΔE ≤ 3, SSIM ≥ 0.9, overall and by region): overall met at the dynamic resolution the
  game runs at (and ΔE at the floor); by region not, anywhere.
- **Where the error is** (shot 0): the trees' canopies (voxels with holes, each shaded on its own) and the edges of
  voxel steps. The live view blurs both: at the floor it traces half the resolution (full resolution: SSIM +0.022,
  ΔE −0.6; the worst region 5.0 -> 3.5), its temporal and denoising passes average over the history, and its radiance
  cache's voxels are coarser than 10 cm ones. Live also reads 4–5 % darker than offline (luminance 0.947 at the floor,
  0.959 at dynamic resolution); the firefly clamp is not why: without it 1 % brighter and noisier (ΔE 2.10 -> 2.63).
  256 frames instead of 64 change little (SSIM +0.003–0.006).
- **Run to run**: two live runs of a shot differ by ΔE 0.5–1.1 (the radiance cache's training, ROADMAP Known issues),
  and read the same against the offline render to ±0.01.
- **Lamps light their surroundings**: the ground and the first 3 m around the lamp's foot read 12.45 nits lit and
  0.006 with the lamps off (offline); with the lamp blown up, its glowing pieces in flight light them at 12.81.
- **Emitters added in place** (the lamp's object changed live, its pole cut, against the same cut in setup; 4 shots
  over that region): in the light tree 3.62 ΔE, variance 7.4e-4; by power share (step 3's way) ΔE +0.92, variance
  × 1.46; in their own subtree (`ltree::build_slotted`, `set_dynamic`) ΔE −0.08, variance × 0.93.
- **Destruction**: the lamp blown up live, its pieces flying and settling back after 3 s, 4 runs: the explosion's frame
  20.3–25.0 ms, frame start p95 16.67 ms in every run, none over 50 ms, no fallback or rebuild.
- **Ghosting** (`bench-ghosting --scene voxel-night --quick`, against the bench's limits): excess over the settled
  frame within them (turn 0.74 / 4.25 ΔE, strafe 2.24 / 6.0, stop 1.56 / 5.4); trail length over at three shots
  (turn-mid 51 / 34 px, strafe-mid 94 / 53, strafe-return +0.5 s 80 / 26): the wall beside a lamp and the canopies
  are still off by ΔE > 10 half a second after the camera stops.

### Measured: vertex colours (2026-11-10)

ROADMAP § 6 step 2g: the native mesh route's palette as one mesh per entry (each object one own-BLAS mesh per palette
entry it uses) against one per class of entries alike in all but their colour (roughness, metalness, emission), each
vertex its voxel's colour (`voxelLevel({ colors: "vertex" })`, `voxel_core::merge_by_class`; F_VCOLOR).
`tools/bench-voxel.ts --colors`: the level by day (`games/voxel-level/level-native*.game.ts`), live at the 0.5 floor
along the camera path, 2 rounds with an A/A; memory from an offline frame.

| | By entry | By entry (A/A) | By class, vertex colours |
| --- | --- | --- | --- |
| Triangles | 13,251,220 | | 13,251,220 |
| Meshes placed (TLAS instances) | 10,459 | | 9,916 |
| BLASes / triangle data | 274.6 / 259.6 MB | | 274.4 / 259.6 MB |
| Vertex colours | – | | 27 MB (per distinct triangle, the edit room's too) |
| Trace p50 / p95 | 2.44 / 4.62 ms | 2.59 / 4.52 | 2.65 / 4.63 |

- The same image (offline, 64 spp: ΔE 0.00, PSNR 89 dB): the colours are the palette's exactly (sRGB 8-bit).
- The same triangles: a greedy rectangle never spans two entries, merged or not.
- Few meshes saved: the palette's 36 entries make 12 classes, but the entries one object uses mostly differ in
  roughness (a terrain tile's grass, dirt, asphalt, road lines), so they stay apart. The trace differs by less than
  the A/A's spread (other processes held 16–26 % of the GPU during these runs).
- **The choice: the level keeps one mesh per entry** (no colours to store, a kernel without the switch). Vertex
  colours are for palettes of many colours in one material: a `.vox` model's up to 255 colours of one material are one
  mesh instead of up to 255, and the browser's three.js draws one mesh per class.

## Two published designs for destructible voxel worlds

- **Dense grids and a world occupancy volume** (the raster-era design):
  - Voxels and storage:
    - 10 cm voxels;
    - every object a dense grid, 1 byte per voxel indexing a 256-entry palette (colour, roughness, emission,
      physical material);
    - objects up to 256³.
  - Rendering:
    - each object drawn as its bounding box, the fragment shader marching its 3D texture (MIPs skip empty space) and
      writing depth;
    - lighting traced through a world-wide 1-bit occupancy volume (2 × 2 × 2 voxels per byte, a few hundred MB for a
      level) that the CPU keeps up to date with small uploads;
    - one shadow ray per light, two AO rays, reflections coloured from the screen, spatial plus temporal denoising.
  - Its limits:
    - no GI;
    - reflections of off-screen geometry fall back to the sky;
    - emissive voxels light nothing;
    - visible grain;
    - the world is capped by 3D-texture limits (a few hundred metres).
  - Physics and destruction:
    - CPU rigid bodies with contacts on the integer grid;
    - connected components split into new bodies;
    - no structural stress.
  - Multiplayer replicates destruction as deterministic fixed-point commands, never voxel data, and bodies by state.
- **Sparse bricks and hardware ray tracing** (the current direction):
  - per-object sparse 8³ chunks;
  - hardware ray tracing with intersection shaders and no triangles (gvox_engine: one AABB per 8³ brick);
  - no occupancy volume, so the world is unbounded, with GI and true reflections;
  - a learned denoiser;
  - a substepped (TGS) parallel physics solver.

ThreeRT already takes the second direction for lighting; the engine follows it for storage and edits.

## What ThreeRT has and lacks for it (read from the code, 2026-11-09)

- **Everything solid is triangles.**
  - `World` geometry is an indexed triangle mesh (`renderer/src/world.rs:16`).
  - `scene::build` flattens to world-space triangles.
  - `trace()` returns `Hit { t, tri, u, v, inst }` (`renderer/shaders/pathtrace.wgsl:346`).
  - About 45 lookups key on `Hit.tri`: `surface()` (`pathtrace.wgsl:803`), alpha tests, emitter MIS, motion vectors.
  - A voxel hit needs its own kind and a `surface()` path of its own that yields the same `Surface`.
- **Reusable after the first hit, as is:**
  - The BSDF and materials, analytic lights, sky and fog.
  - The world-space radiance cache, keyed by position and normal axis (`cache_key_at`, `pathtrace.wgsl:2319`). Its
    finest cell is 6.25 cm, the voxels' own scale.
  - The live passes: temporal, à-trous, specular filters, reflection resolve, DOF, motion blur, exposure, bloom, post;
    OIDN.
  - Volume functions and the media march (smoke, fireballs; at most 32 bounded media, no density grid), translucent
    particles, sparks.
  - The QuickJS runtime, transactions and frame scripts.
- **Hardware RT:** wgpu 30 (`renderer/Cargo.toml`), triangle BLASes only today. wgpu 30.0 added AABB BLAS geometry,
  and naga's Metal output switched to `intersection_query` so AABB candidates can be committed
  (`rayQueryGenerateIntersection`). Measured below.
- **Missing:** voxel storage and traversal, physics of any kind, partial geometry edits, `.vox` import.

## Measured: AABB ray queries on the M5 Pro (2026-11-09)

A scratch probe (wgpu 30.0.1, outside the repo).
- **World:** 1024 × 128 × 1024 voxels, 1 bit each, built from a heightfield, 220 hollow buildings with floors and
  windows, and 200 k scattered voxels: 17.9 M solid, 177 k non-empty 8³ bricks.
- **Rays:** 1920 × 1080 from outside the world, median of 7 runs of 20 dispatches, under the GPU lock.
- **Compute:** a two-level DDA (bricks, then voxels).
- **Hardware:** one AABB per non-empty brick in one BLAS; the candidate loop runs the same in-brick DDA and commits
  with `rayQueryGenerateIntersection`.

| | compute DDA | hardware, AABB per 8³ brick |
|---|---|---|
| primary rays | 7.5 ms | 42.6 ms |
| primary + 1 incoherent secondary ray | 15.4 ms | 68.3 ms |
| agreement | | primary: 1 pixel in 2.07 M differs; secondary: ~0.01 % (the probe's own offset at the origin); run to run identical |

| BLAS + TLAS build (fast build, submit to wait) | 512 AABBs | 4 k | 16 k | 177 k (fast trace) |
|---|---|---|---|---|
| ms | 0.30 | 0.45 | 1.13 | 2.9 |

- **AABB ray queries work on Metal through wgpu 30**, deterministically. The candidate-loop losses seen with cutout
  triangles (Known issues, `cutout-trace`) did not show here.
- **One AABB per brick is 4–6 × slower than the compute DDA.** Each candidate leaves the traversal hardware to run the
  DDA, and a ray meets many bricks.
- **BLAS builds are cheap:** rebuilding one object's BLAS after an edit fits in a frame.
- **What this does not show:** the probe has no MIPs or occupancy masks and measured one camera. It rules out this
  granularity, not hardware RT. The first engine experiment compares granularities (below).

## Design

Where it runs (the user, 2026-11-10):

- The GPU render farm (`/play`) only makes images for agents authoring in GPU-less sandboxes; nothing is streamed.
- Players play in the browser, the client running the game (most likely #27's `web/`); native apps come later.
- How (the user, 2026-11-10; docs/RUNTIME.md "The runtime's shape"): the Rust engine compiled to wasm, with the
  gameplay TypeScript in the browser's own JS engine calling its exports (layout C). Natively the script engine stays
  QuickJS, and nothing depends on a JIT. Scripts are three.js-style; Rust owns the scene, the physics and the voxels;
  commands, poses, events and queries are the four channels between them. In a real browser so far: the physics
  hello world (Chrome, 2026-11-11; docs/RUNTIME.md "The engine in a browser"). The voxel level has not run in one.
- So voxel editing, meshing and physics (Rapier) build for wasm (`wasm32-unknown-unknown`: no wgpu, no file system,
  threads optional), and the browser has no ray queries (WebGPU): it renders with the engine's own kernel under
  software traversal. There a chunk's mesh and a piece of debris are units (a tree of their own under a top tree;
  docs/RUNTIME.md "Software traversal with units", ROADMAP § 6 step 7b), so that an edit rewrites the chunks it
  touched and not the scene. Measured natively under software traversal (2026-11-11): the level's triangle data
  310.9 MiB (flattened: 2,098), an explosion 2.5–4.5 ms in place (6.9–7.9 s), and a still frame's trace 7.85 ms
  where the flattened level's is 4.58 (the chunks' boxes overlap; the top tree is not tuned yet).
- The triangle route's output renders in `web/` as it is: standard meshes or glTF with palette materials, each voxel's
  colour as vertex colours, nothing that needs WGSL. Procedural boxes (a geometry function in WGSL) are native only.

- **Data:**
  - 10 cm voxels.
  - Each object is a set of sparse 8³ bricks: a 64-byte occupancy mask per brick plus 1 byte of material per voxel
    (palette per object), with a brick grid or 64-tree above it.
  - Static terrain uses the same format.
  - Objects have rigid transforms; dynamic objects own their bricks.
- **Intersection:**
  - The TLAS holds objects; the hardware does transforms and coarse culling.
  - Inside an object, a brick DDA with empty-space skipping.
  - Which split is fastest is the first experiment, on the benchmark level at the 0.5 floor. Candidates:
    1. one AABB per object;
    2. one AABB per 32³ region;
    3. one AABB per 8³ brick;
    4. a pure compute DDA over a world brick grid (deferred, 2026-11-10: the triangles trace 8× faster than the best
       box split with ray-tracing hardware, but the browser has no ray queries; the browser's way waits for the user's
       decision on a browser feasibility experiment; ROADMAP § 6 step 2c-4);
    5. greedy-meshed triangles through the existing triangle path.
- **Lighting:** ThreeRT's own.
  - Path-traced direct light through the light tree.
  - The radiance cache for indirect light.
  - Reflections traced, not taken from the screen.
  - Emissive voxels as emitters.

  Against the dense-grid design this adds GI and reflections of off-screen geometry; it is judged against our
  offline render.
- **Edits** (done in step 6; "Measured: connectivity and edits"):
  - Explosions (by material hardness), cuts, paint and added voxels are engine operations, deterministic and in fixed
    point, so multiplayer replicates commands, not voxel data.
  - Connectivity: per world brick its components and their cells on the six faces, then a search over components;
    what reaches no anchor becomes a new object (a body from step 8 on; until then it falls and rests).
  - An edited object is meshed and uploaded again whole; by chunk is open.
- **Physics:**
  - Native, on the node, in ThreeRT's physics module (`renderer/physics-core`, ROADMAP § 6 step 7). The frame script's
    50 ms QuickJS budget cannot hold it: the engine owns the world and steps it at 1 / 60 s; a body moves its object's
    instance without the script; the script sets `physics` on objects, reads their poses when it wants them, and asks
    rays and shapes of the world.
  - physics-core and its bridge to scene objects build for wasm32. The same bits natively and in the browser on what
    is checked (a script, 1,000 random worlds, in physics-core's build and the renderer's), with Rapier's
    `enhanced-determinism` (1–5 % slower natively), a patched glamx and hinge limits handed over centred: Rapier's
    crates otherwise take some math functions from the platform (ROADMAP § 6 step 7 has what is and is not shown).
  - Game scripts cannot crash it: every number a call carries is bounded, and a body whose state goes non-finite is
    taken out of the world and reported (RUNTIME.md, Physics).
  - The voxel collision shape is the engine's: physics-core's `CustomShape` is where it plugs in (step 8).
  - The prototype is Rapier / Parry's `Voxels` shape (dynamic voxel bodies, `setVoxel` edits, since 2025). Whether to
    keep it or write our own (TGS substepping, grid contacts) is decided by its measured cost on the benchmark.
- **Assets:**
  - MagicaVoxel `.vox` via `dot_vox`.
  - glTF voxelized in a compute pass (triangle–box overlap).
  - Characters as rigid voxel parts per bone at first.
- **Gameplay and the cloud:**
  - Handlers write compact commands into `tx.state` ("explosion at p, radius r"), never voxel data.
  - Nodes apply them in order to their world data; the room log replays them for late joiners.
  - Bodies are synced by state. Where physics is authoritative (node or room) is decided at
    step V5.

## Voxel data (`packages/voxel`)

A voxel object is a `VoxelGrid`: sparse 8³ bricks, a byte per voxel (0 empty, 1..255 a palette entry, the index of the
`ProceduralMesh`'s material). `layout(granularity)` turns it into procedural geometry (docs/MATERIAL_FUNCTIONS.md,
"Geometry functions"):

- **Data** (32-bit words from the geometry's data start):

  | words | field |
  |---|---|
  | 0..3 | brick grid size bx, by, bz; voxel size (m) |
  | 4..6 | object-space corner of voxel (0, 0, 0) (the grid starts at the lowest brick, rounded down to a 32³ region) |
  | 7 | first word of the brick grid: bx · by · bz words, 0 = empty, k = brick record k − 1 |
  | 8 | first word of the brick records: per brick 16 words of occupancy (bit x + 8y + 64z), then a material word |
  | 9 | brick count |

  A brick's material word with bit 31 set is the material of every voxel in it (a uniform brick); otherwise it is the
  offset of 128 words holding a byte per voxel.
- **Boxes**: one per non-empty brick, one per non-empty 32³ region, or one for the object (`Granularity`). One layout
  serves all three: the geometry function walks only the bricks inside the box the ray entered.
- **`VOXEL_BRICKS`**: a DDA over the bricks of that box, then over the 8³ voxels of each non-empty brick, returning the
  entry face's normal, the voxel's palette entry as material and as `attr.x`. The voxel walk stops on its cells leaving
  the brick, not on the summed step times, which drift from the brick's exit time (that drift read neighbouring bits:
  phantom bricks). An axis the ray does not move along never bounds its hit test (entered at −∞, left at +∞): with
  the reciprocal's stand-in, axis-aligned rays missed every voxel (Bugbot; `voxel/shadow-axis`).
- **Files**: `parseVox` reads MagicaVoxel models and the palette (other chunks skipped; every length checked: a file may
  come from an untrusted game); `voxGrid` turns its z-up axes to y-up; `paletteMaterials` makes the palette materials.
  `tools/voxel-build.ts` writes a `.vox` file's models as procedural geometry assets for `fromAsset`, so a level's
  voxels never pass through the scene runtime.
- **Triangles**: `greedyMesh(grid, origin, solidOutside)` gives the object's exposed faces (a set voxel next to an empty
  one; `solidOutside` says what fills the space around it, e.g. a neighbouring terrain tile's ground), merged greedily
  per face plane into rectangles of one material, two triangles each, by material. It visits faces, not the volume:
  bricks buried in full bricks are skipped whole. Unit tests: the rectangles, cut back into voxel faces, are exactly
  the exposed faces with their materials, each once, facing out.
- **Native** (`renderer/voxel-core`, ROADMAP § 6 steps 2c-4 and 6): the same bricks and layout in Rust (`decode`,
  `layout`), binary greedy meshing (`greedy_mesh`: per axis each column's voxels as bits, faces a shift away; the same
  rectangles as `greedyMesh`, rows scanned in the same order), and the level (`Level`): objects placed in one world
  lattice (`Placement`), edits as shapes in fixed point (`Shape`; `carve` by hardness, `paint`, `add`), connectivity to
  the anchors (`Lattice`), loose parts falling tick by tick, a hash of it all. Integers throughout: the same on every
  machine. No GPU, file system or threads: CI builds it for `wasm32-unknown-unknown` and compares its edit script's
  hash with the native one. Scene code reaches it through `voxelLevel`, `voxelExplode`, `voxelCarve`, `voxelPaint`,
  `voxelAdd`, `voxelFrame`, `voxelHash` (packages/voxel `native.ts`; renderer/src/voxel.rs), so a level's voxels are
  loaded, edited and meshed in the renderer, never in the scene runtime.
- **Checks**: layer A `voxel/*` (both traversals) and `tools/unbiased-check.ts --scenes voxel` (tools/voxel-scene.ts).

## Open questions (the user's)

- Hardware floor:
  - M3 and later have ray-tracing units.
  - M1 and M2 run ray queries without them; a compute DDA likely wins there.
  - Experiment V1 measures both on the M5; an M1/M2 machine is needed to measure the floor.
- Physics: Rapier for good, or our own after the prototype.
- Priority of this line against § 0 (dynamic content on Bistro).

## Sources

- wgpu changelog (v29 Metal acceleration structures, v30 AABB BLAS, naga `intersection_query`):
  https://github.com/gfx-rs/wgpu/blob/trunk/CHANGELOG.md
- Gabe Rundlett, one AABB per 8³ brick: https://twitter.com/RundlettGabe/status/1781349885603668035
- Sparse 64-trees: https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing
- Aokana (chunked SVDAGs, 2025): https://arxiv.org/html/2505.02017v1
- Ray Tracing Gems II ch. 37, "Ray-Tracing Small Voxel Scenes" (brick granularity against intersection-shader cost)
- Rapier changelog (voxel shapes): https://github.com/dimforge/rapier/blob/master/CHANGELOG.md
- MagicaVoxel `.vox` format: https://github.com/ephtracy/voxel-model/blob/master/MagicaVoxel-file-format-vox.txt

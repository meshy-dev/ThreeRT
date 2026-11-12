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

### Voxel bodies (ROADMAP § 6 step 8; 2026-11-11)

The level collides and what comes loose is a rigid body. Code: `renderer/physics-core` (`Shape::Voxels`,
`Physics::edit_voxels`), `renderer/voxel-core` (`Edit::cleared`, `Level::take`), `renderer/src/voxel.rs` (the
level's colliders, the bodies). First what the CPU costs on its own (this Mac, release builds), then the level on
the GPU ("Measured on the GPU" below).

**The shape** is parry's `Voxels` (8³ chunks, a byte a cell, each cell knowing its neighbours), for the fixed
level and for the moving parts. Measured against the others on Rapier (`physics-core/examples/voxel_shapes.rs`: a
ground of 427 k cells of 0.1 m; pieces of 6³ cells with holes, about 200 solid):

| the fixed ground | build | memory | an edit of 2,080 cells |
| --- | --- | --- | --- |
| voxels | 26 ms | 1.3 MiB | 0.10 ms, in place |
| a compound of boxes (cells merged greedily) | 2.9 ms | 0.3 MiB | 3.7 ms: the object built again |
| a triangle mesh of the faces | 107 ms | 53.7 MiB | 105 ms: built again |

| 100 pieces dropped in a heap: moving / fixed | a tick while they fall: mean, worst | asleep after 15 s |
| --- | --- | --- |
| voxels / voxels | 1.38, 4.7 ms | 100 |
| boxes / voxels | 1.66, 5.9 ms | 98 |
| hull / voxels | 0.44, 2.4 ms | 29 |
| voxels / triangle mesh | – | 0: all fell through the ground |
| boxes / boxes | 0.46, 1.5 ms | 94 |

Voxels against voxels: 400 pieces 7.1 ms a tick while they fall (at worst 19), 1,000 pieces 22.6 ms (52); asleep,
0. The same bits natively and in wasm32.

**On several threads.** Natively the world is stepped by Rapier's parallel solver with `enhanced-determinism`
(physics-core's `parallel`; the renderer's native build has it on, in a pool of the physics' own: 4 threads, or the
machine's performance cores when fewer; `THREERT_PHYSICS_THREADS` sets another count, 1 is sequential). wasm32 is
sequential. The same heaps, a tick while they fall, mean and worst, ms:

| pieces | sequential | 2 threads | 4 | 8 | 15 |
| --- | --- | --- | --- | --- | --- |
| 10 | 0.10, 0.99 | 0.08, 0.42 | 0.08, 0.41 | 0.09, 0.48 | 0.08, 0.59 |
| 100 | 1.24, 3.84 | 0.74, 2.27 | 0.49, 1.71 | 0.47, 2.08 | 0.51, 2.04 |
| 400 | 6.69, 17.75 | 3.72, 9.26 | 2.25, 6.12 | 1.68, 5.32 | 1.52, 6.58 |
| 1,000 | 20.48, 50.81 | – | 6.27, 15.44 | 4.29, 12.76 | 3.66, 13.67 |

Each row ends in the same bits at every thread count and sequentially (a hash of every piece's pose and velocity
after 900 ticks). **That the result does not depend on the thread count is what we checked ourselves, and this is
how far:**

- On this machine (M5 Pro, 15 cores), at 1, 2, 4, 8 and 15 threads: the determinism check's script and its 1,500
  random worlds (hulls, bodies of several colliders, joints with limits and motors, CCD on a triangle mesh with
  kinematic bodies and queries, voxel lumps on voxel ground with edits: each a few to a few dozen bodies, 90
  ticks); heaps of 10, 100, 400 and 1,000 voxel pieces on voxel ground, 900 ticks; 20,000 random voxel worlds
  without a panic.
- In CI (Linux, x86-64), on every change to the physics: the script and the 1,500 worlds at 2 and 4 threads
  against the sequential native build, which is held to wasm32 and to the recorded hashes
  (`tools/physics-determinism.ts`: native parallel = native sequential = wasm32 = recorded).
- The renderer's own build, stepped as it steps (on the physics' own threads): the script, the 1,500 worlds and
  the voxel adapter's script at 1, 2, 4 and 8 threads; the benchmark level run twice through explosions, the 30 m
  cut and a crater gives the level and physics hashes the sequential build gave; the same bodies moving in every
  frame of four destruction scenarios, sequential against 4 threads.
- **Not checked**: other machines and other CPU families at thread counts beyond these (Rapier promises it; it also
  promised the cross-platform bits that needed a patch to glamx); worlds with joints and thousands of bodies at once
  (the worlds with joints here are small; the large ones have no joints); long runs (minutes) of large heaps.

(Rapier's contact recycling stays on: with it off a heap of 400 pieces does not come to rest, 79 asleep after
15 s.)

**The parallel step can hang, and what the renderer does about it.** Rapier 0.36.1 hands the broad phase's tree
optimisation to `rayon::spawn` and waits for it with a blocking `recv`. Stepped on a pool's worker (which is how
the renderer steps, and how Rapier's own `configure_thread_pool` does) the job sits on the stepping worker's own
deque, and if every other worker happens to be falling asleep at that instant none takes it: the step never
returns. It needs steps microseconds apart (ticks back to back: a catch-up, an offline settle, a fuzz); a world of
six bodies stepped back to back hangs after 30,000 to 500,000 steps, the review's fuzz in one run of fifty. The
renderer's wait for a step therefore injects an empty job into the pool every 0.5 ms until the step is back
(`renderer/src/physics.rs` `in_pool`): a worker wakes for it and takes the stranded job on its way. Neither crate
is patched. `renderer/examples/physics_fuzz.rs` is the check (CPU only, with a watchdog): 500 fuzz runs in a row, 20 of the
many-bodies variant and 26 million back-to-back steps without a hang; with the kick off the back-to-back world
hangs in three runs of three.

**The level's colliders.** Every object gets a fixed collider at load; an edit takes out the cells the object lost
(`Edit::cleared`: by the cells, not by the object's size). A collider holds the object's cells within 4 of a cell
that is not solid, a shell; an edit brings in the cells it bared, the neighbouring terrain tiles' too. On the
benchmark level (4,172 objects; 515 M solid cells, 480 M of them the terrain's 1,600 tiles down to the bedrock):

| colliders of | cells | build | memory |
| --- | --- | --- | --- |
| every solid cell | 515.3 M | 26.0 s | 1,279 MiB |
| the shell | 61.6 M | 6.9 s (finding the cells 2.3) | 455 MiB |
| the shell, objects of one file sharing a shape until one is edited | 61.6 M | **5.4 s** (finding the cells 2.1) | **303 MiB** |

A collider holds its object's own cells and stands by the object's placement, so the 2,572 objects that are no
terrain, which come from 630 files, have 630 shapes between them (`physics_core::SharedVoxels`); an edit gives the
edited object cells of its own. What is left is the terrain: 1,600 tiles, whose shells depend on their neighbours,
a shape each. Natively this is a fixed cost of a level of this size (ROADMAP § 6 step 8, "Open").

**An edit on the level, on the CPU** (`voxel.rs` `voxel_level_physics`; at the foot of the largest building part):

| edit | voxels | all | carve | colliders | meshes | parts and bodies |
| --- | --- | --- | --- | --- | --- | --- |
| explosion, 0.5 m | 296 | 5.1 ms | 0.13 | 0.89 | 4.00 | – |
| 1 m | 2,857 | 6.3 ms | 0.27 | 1.33 | 4.72 | – |
| 2 m | 19,919 | 9.0 ms | 0.64 | 2.54 | 5.76 | – |
| 4 m | 144,344 | 19.8 ms | 2.96 | 8.50 | 8.27 | – |
| the cut of 30 x 30 m, 1 m thick | 577,519 | 124 ms | 25.2 | 43.1 | 42.4 | 12.5: six bodies of 112,889 voxels, the largest 95,694 |

After the cut the physics steps in 0.17 ms a frame (p95 0.44; a frame's physics at worst 2.7 ms) while the parts
fall and land, all six sleep within five seconds, and then it is 0. In step 6 the same cut cost 26–28 ms a frame for as long as the building fell (its
vertices written again every frame); now a part moves by its transform.

**A body**: its grid in its own space, a mass from its voxels (a density per palette entry: by hardness 900 / 1,800 /
2,400 / 7,800 kg/m³ unless the entry names one), drawn by driven objects. A carve reaches it through its pose; what
is cut off is a body of its own. At most 300 at once (`maxBodies`; the number is measured, below). At the cap a
new part takes the place of the body that matters least: asleep before awake, then the fewest voxels, then the
oldest, and never one the same edit carved or made (a body being cut keeps its larger part and its id; its chips do
not push out the bodies the same cut is going through). It does so only if that body has no more voxels than the
part; else the part itself is debris. A settled body stays a body.

**Measured on the GPU** (this Mac, M5 Pro; hardware traversal unless said; the benchmark level at 1280 x 720 at
0.5, in lockstep at 1 / 60 s; `games/voxel-level/level-destruction.game.ts`'s scenarios; a frame's GPU time is
5.2–5.7 ms throughout, its trace 1.8 ms).

| | the edit's frame, CPU | frames after it, CPU p50 / p95 | physics step | bodies |
| --- | --- | --- | --- | --- |
| a still level | – | 5.3 ms | 0.01 ms | 0 |
| explosions of 1, 2, 4 m (`singles`) | 10.5, 10.4, **18.4 ms** | 5.2 ms | – | 0 |
| a 4 m crater in open ground (140,785 voxels) | **21.9 ms** | 5.2 ms | – | 0 |
| the cut of 30 x 30 m (`fell`: 46,469 voxels, 20 parts) | 116.7 ms | **5.3–5.5 / 8.1 ms, none over 16.7** | p50 0.28, at worst 0.52 ms | 20 (the largest 32,680 voxels); 17 asleep after 9.5 s |
| eight trees cut at the trunk (`topple`) | 6.9–14.9 ms | 5.5–7.2 / 8.7 ms | 2.25 ms while five crowns move | 5 (10,616–18,894 voxels); four asleep 5–6 s after the last cut, the fifth after 10 s (the CPU's run of the same cuts) |
| 7 explosions a second for 10 s (`sustained`) | p50 9.2, p95 13.4 ms | – | at worst 1.35 ms | 2 |
| an explosion a frame for 60 s (`endurance`) | – | 9–11 / 18 ms in the first 5 s, 5.2–5.5 after | p50 at most 4.4 ms | 9 from the fifth second to the end |

The cut's 116.7 ms: the level's carve 21.6, its colliders 15.3, its meshes 5.3, the parts' meshes and bodies 27.8,
the scene's commit of the new meshes 37.7, the physics' first step 5.6. In step 6 the same fall cost 26–28 ms a
frame for as long as it lasted. The level's load is 10.7 s to the first frame with the colliders and 5.5 s without
(`physics: false`).

**How many bodies a frame holds, and the cap.** `shatter-dense`: the cut of `fell`, then 120 cuts 0.2 m thick half a
metre apart, twenty a second, through what fell: slabs, then columns of buildings, tens to thousands of voxels
each, in one heap, more than 500 of them. **The default configuration** (the parallel solver on 4 threads) with
the default cap of 300: the count reaches 300 and stays there to the end of the run. Frames without an edit, by the
bodies that moved in them (physics step and the frame's CPU time, p50 / p95; two runs, the same bodies in every
frame, the second's figures in brackets):

| bodies moving | frames | physics step | frame CPU | frames over 16.7 ms |
| --- | --- | --- | --- | --- |
| 0–49 | 661 | 0.36 / 0.77 ms | 5.7 / 7.8 ms | 0 |
| 50–99 | 66 | 1.97 / 3.20 | 7.4 / 9.6 | 0 |
| 100–149 | 78 | 3.30 / 4.64 | 8.1 / 10.2 | 1 (a step of 0.8 ms) |
| 150–199 | 63 | 1.15 / 4.75 | 6.9 / 9.8 | 0 |
| 200–249 | 123 | 1.85 / 5.12 | 8.0 / 9.9 | 0 |
| 250–299 (every body of the cap in the air or landing) | 103 | 5.92 / 7.71 (6.18 / 8.23) | 10.7 / 12.8 (10.9 / 13.2) | 0 (0) |

**No frame of 1,094 is over 16.7 ms on account of the physics** in either run (each has one stray, 17.9 and 17.2
ms, with a step under 1 ms). That is why the cap is 300. With a cap of 400 the count holds too, and with every body
in the air a frame is 13.1–13.2 ms at the median and 14.9–16.0 at p95, but in two runs of four a few frames go over
(2 and 3 of 1,094, 17.0–19.3 ms, with 320–400 moving); with 512, 16.3 ms at the median and 5 of 22 such frames over.

The same scenario with the cap at 512, sequential (`THREERT_PHYSICS_THREADS=1`) against the default's 4 threads:

| bodies moving | frames | step, sequential | frame CPU, sequential | over 16.7 ms | step, 4 threads | frame CPU, 4 threads | over 16.7 ms |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 50–99 | 51 | 5.02 / 7.89 ms | 9.2 / 12.1 ms | 1 | 2.10 / 2.96 ms | 7.5 / 9.0 ms | 1 |
| 100–149 | 41 | 10.15 / 14.03 | 14.2 / 17.7 | 9 | 3.45 / 4.55 | 8.4 / 9.3 | 0 |
| 200–249 | 8 | 17.46 / 18.57 | 21.7 / 22.2 | 8 | 5.65 / 5.75 | 9.5 / 11.5 | 0 |
| 300–349 | 6 | 21.01 / 21.08 | 24.9 / 25.3 | 6 | 6.07 / 6.51 | 10.1 / 10.3 | 0 |
| 450–499 | 46 | 35.38 / 39.61 | 39.4 / 43.6 | 46 | 10.41 / 11.99 | 14.7 / 17.1 | 5 |
| 500–549 | 22 | 41.24 / 42.94 | 45.2 / 47.3 | 22 | 12.37 / 13.01 | 16.4 / 18.0 | 6 |

**The number of bodies moving at once at which 60 fps is lost: on one thread about a hundred** (from 150 every
frame is over); **on 4 threads about 450–500.** What takes the frame is Rapier's step (placing the parts is at most
0.17 ms, the scene's commit about 2 ms, the GPU's frame does not change). The bodies and the bodies that moved are
the same in every frame, sequential and parallel (and at 8 threads, which bring the step no gain on the frame).
With few bodies the pool costs nothing that shows: `fell` (20 bodies) steps in 0.18–0.35 ms a frame either way,
hello world in 0.03 ms. **With the parallel solver off the cap to set is 128** (`maxBodies: 128`): sequentially,
with all 128 moving, a frame is 12.1 ms at the median and 7 frames of 1,095 are over 16.7 ms (the worst 22.0): about
at the frame. A cut through the heap is itself a frame of 19–21 ms (p50; p95 32–36 ms) on 4 threads and 26–35 ms
(p95 44–59 ms) on one: it carves, splits and meshes again every body it meets, and the frame's step is in it.

**Under software traversal** (`THREERT_HWRT=0`: bodies are units; not optimised): no rebuild, the parts added and
moved in place (records 10,459 to 10,503 for the 20 bodies of `fell`), the loop guard's count 0; the same bodies
asleep at the same frames as under hardware traversal. Explosions of 1, 2, 4 m: 8.4, 8.6, 18.6 ms; the cut's frame
194 ms (the parts' trees are built on the CPU: the commit is 117 ms of it); after it a frame's CPU p50 8.1, p95
9.1 ms, none over 16.7 ms; a frame's GPU 7.5–8.1 ms (trace 4.2–4.6).

### Voxel bodies as objects (ROADMAP § 6 step 10 a; design: `out/web-probe/step10/a-design.md`)

A loose part is, to scene code, an object: `voxelBodies()` returns `VoxelBody` objects (`@threert/voxel`, an
`Object3D`), the same object for the same body as long as it is; `voxelBody(id)` and a physics hit (`hit.object`) give
it too (the level itself stays `engineColliders`).

- **What it has**: the physics methods every body has, same names and units as a `Mesh` with `physics`
  (`applyImpulse`, `applyForce`, `setLinearVelocity`, `getLinearVelocity`, `sleeping`, ...), sent to the engine at
  once (the body exists: no sync to wait for); `position` / `quaternion` / `rotation` read from the engine (writes do
  not move it); `physics` read-only (`{ type: "dynamic", mass }`); `paint`, `addVoxels`, `carve` of that body alone.
- **Contacts**: a "contactstart" / "contactend" listener turns its collider's reports on; contacts arrive as for any
  body, on both objects with `other` (the level is `engineColliders`: a contact with a collider on no body was
  dropped before, for scene objects too), and in the scene's "contacts" batch. World handlers (`on`) are for the
  game's named events; a listener added in setup acts on the scene directly, as for ordinary bodies.
- **Joints**: `FixedJoint`, `HingeJoint`, ... between voxel objects, or a voxel object and a scene object with
  `physics`: the binding takes a voxel object's engine body (anchors in its own frame, unscaled). Removing a body
  removes its joints in the engine.
- **When it stops being one**: cut, the part that keeps the body keeps the object; retired for the cap, carved away
  or fallen out of the world, it is `gone` (`goneReason`: "cap", "carved", "fell") with a "gone" event at the frame's
  events, and its calls do nothing. A level edit looks at the bodies again at once; a call on a body the engine took
  out before the objects noticed finds it gone then and does nothing either (it does not throw).
- **Contacts, kept straight** (the bridge): a pair's start is reported once, even when an edit takes the collider out
  and puts it back; a collider that goes (a body carved away, retired, removed) ends its pairs.
- **Refused**: scene code's own colliders on a voxel body (the engine edits and splits it by its cells, and weighs it
  by them); join a body of your own with a `FixedJoint`.
- **Paint, added voxels, mass**: paint from the world reaches bodies (their surface cells, as the level's paint);
  `addVoxels` fills the body's empty cells in the shape, at most 65,536 at once and 64 times that looked at, and only
  if the body stays within 2^21 cells (`MAX_BODY_CELLS`) spanning at most 1,024 cells a side and 2^25 in all
  (`MAX_BODY_SIDE`, `MAX_BODY_BOX`: its mesh is made over that box, at most 47 MB a redraw; one add 120 m out took
  2.4 GB before); the collider is edited first, and if anything is refused nothing changes. A part spanning more is
  debris, not a body; a body is never kept without its collider. A material outside the palette is refused. A body's
  mass, centre of mass and inertia are the sum of its cells, each a cube of its palette entry's density (the collider
  carries none). Step 8's `voxelImpulse` is kept, deprecated.

### Fire and smoke (ROADMAP § 6 step 10 b; design: `out/web-probe/step10/b-design.md`)

An explosion's fireball and smoke, on the existing volume and particle paths (the user's scope, 2026-11-12: no fire
spreading along burnable voxels, no smoke stopped by voxels). `Explosions` (`@threert/voxel`) is a group made once in
setup: a pool of fireballs (`Fireball`) and a pool of point lights. It adds no engine call. **The smoke is each
fireball's own**: it cools to dark smoke over its life. `Explosions` takes no smoke grid (it throws if given one): the
grid's box is a volume, a fireball in it would overlap it, and when this was built volumes could not overlap (the
media were taken whole, in the order a ray enters them: below, "Why volumes must not overlap"; the engine draws
overlapping media together since 2026-11-13, and `Explosions` has not been changed for it yet). A game may keep its own `SimulatedSmoke`
where nothing blows up.

```ts
const boom = new Explosions();
scene.add(boom);
// frame(f):
boom.explode(x, y, z, 4, f.time);        // voxelExplode, then fire([x, y, z], 4, f.time)
boom.step(f.time);                       // once a frame: lights follow their fireballs, burnt-out ones parked
```

Positions are the world's: the group stays at the origin, unrotated, unscaled (a fire throws otherwise). `dispose()`
takes the pool out of the scene for good.

- **A fire** (`fire(at, r, time)`): the next fireball of the pool, 2.2 r across (5 cm to 40 m), its centre 0.3 r above
  the explosion's point, from `time` (now or later); a light at its centre while it burns, from the first step after
  it starts (at age 0 it gives none, and takes no light from an older fire). Time going back (a restart) ends every
  fire. Two fires close together make two fireballs that overlap: the engine cannot draw that rightly yet (the last
  temporary way round below; nothing in `Explosions` keeps them apart). The fireball's default peak is 30,000 nits (a
  flame's; `Fireball`'s own 40 is a stylised night's and does not show by day). Every object of the pool is made in
  setup and `movable`: firing moves them in place (a fixed object's first move rebuilds the live scene once: 281 ms on
  the benchmark level, and every pool member would do it once).
- **Why a light**: live, GI, reflection and shadow rays do not see media (MATERIAL_FUNCTIONS.md, live media), so a
  fireball's emission lights nothing around it; offline it does. The light stands in: its intensity and colour over
  the fireball's life come from `FIRE_LIGHT`, measured against the offline renderer (`tools/bench-voxel-fire.ts --only
  calibrate`: fireballs of 1.1, 2.2, 4.4 and 8.8 m at 12 ages over a floor and a wall, against a 1,000 cd light; floor
  and wall agree within about 10 %). It is not one curve scaled by size²: a larger fireball's hot core lies under more
  of its own smoke, so its light falls sooner (at half its life 0.0090 cd per m² of size² per nit at 2.2 m, 0.0020 at
  8.8 m). `voxel/fire-light` checks it in the renderer: the fire light alone gives a floor 0.987 of what the fireball's
  own emission gives it. The light, the parking of idle objects and the rest are temporary ways round the shared live renderer (the table
  below, "Temporary ways round on the engine's shared paths").

Caps, and what happens past them (node tests in `packages/voxel/test/explosions.test.ts`):

| what | cap | past it |
| --- | --- | --- |
| fireballs | `fireballs`, default 8, at most 32 (`MAX_FIREBALLS`): the kernel draws at most 32 volumes (`MAX_MEDIA`; a scene with more is refused by `scene::build`, no longer drawn without the last) | the oldest burning or waiting one is taken over: it ends early, the new one starts (`stats.reused`) |
| fire lights | `lights`, default min(fireballs, 8), at most 8 (`MAX_FIRE_LIGHTS`: the kernel evaluates up to 8 shadowed local lights exactly, `RIS_EXACT`) | the youngest fires have them; the rest burn without (`stats.unlit`: the most at once) |
| fireball size | 40 m (`MAX_FIREBALL`: r over about 18 m) | the largest; `voxelExplode` keeps its own limits |

Arguments: every number is checked before anything changes, since the scene sync checks only grid sources and
`customData` (not transforms or lights): a position outside ±1,000 km (finite numbers past f32 included) or not three
finite numbers, a radius not finite or not above 0, a time outside ±1e9 s, or the pool moved off the world's frame,
throws a `RangeError` from `fire`, `explode` (before its voxel call) and `step`; a huge radius (1e30) gives the largest
fireball, a tiny one (1e-46) the smallest; the constructor checks every option (ranges in the code; `intensity: 1e308`, `life: 61`, a
smoke grid are refused). 1,000 fires in one frame, at one time or 1 µs apart, make no object (the pool cycles; the
last 8 burn). An idle light has a reach of 1 mm where
it is parked (a distance of 0 is unbounded reach: the sphere of its group of 16 lights held every pixel, 0.9 ms of GPU
with nothing burning).

**Offline, the fire's light counts twice unless the game passes `lights: 0`**: an offline path reaches the
fire's emission, and the stand-in light is there as well. The bench's references are made with `lights: 0`
(`fireShot({ light: false })`); `level-destruction-fire` rendered offline has both. Live, surfaces see only the light
(their rays have no media code).

#### Measured: fire and smoke on the benchmark level (2026-11-12)

`tools/bench-voxel-fire.ts` (games/voxel-level/level-fire.game.ts): the 4 m explosion at the destruction game's
`crater` spot, by day (fireballs, no grid), the live frame at render scale 0.75 (never 1: at exactly 1 the live frame has no edge
anti-aliasing), 1280 x 720, lockstep at 60 fps from the start, against our offline render of the same scene time
(2,048 spp, no clamps, no denoiser; 1,024 against 2,048 spp: ΔE 0.40, SSIM 0.993), which has the fireball's own light
and no fire light. Live in lockstep is bit-reproducible (A/A ΔE 0.00). Targets ΔE ≤ 3, SSIM ≥ 0.9. Measured with the
live view telling media that change with time from the surfaces behind them (below), on main `230b0f3`; beside them
main before that (`0a89089`):

| after the fire starts | whole image ΔE / SSIM | the fireball (its box) ΔE / p99 / SSIM / luminance | the street below it ΔE / SSIM / luminance | main before: whole / fireball / street |
| --- | --- | --- | --- | --- |
| 0.2 s | **1.77 / 0.964** | 6.28 / 30 / 0.832 / 1.01 | **1.34 / 0.972** / 0.99 | 1.86 / 0.963; 7.32 / 36 / 0.822 / 1.04; 1.34 / 0.972 / 0.99 |
| 0.5 s | **2.54 / 0.942** | 5.56 / 30 / 0.855 / 1.18 | **0.98 / 0.981** / 1.03 | 3.21 / 0.930; 7.81 / 39 / 0.815 / 1.27; 0.99 / 0.981 / 1.03 |
| 1.0 s | 2.99 / 0.895 | 3.69 / 26 / 0.861 / 1.08 | (the fireball's box reaches the image's foot) | 4.39 / 0.851; 5.71 / 41 / 0.796 / 1.16 |

Where it falls short, and why:
- **The fireball** misses at every age. Most of it is the live media's own picture: the same moment held (time all
  but stopped, 120 frames) against offline reads ΔE 3.64 / 4.32 / 2.70, SSIM 0.903 / 0.881 / 0.903: the live view
  marches 8 steps through a fireball whose noise is finer than that, and lights it from the radiance cache's voxels
  (isotropic); its cooled smoke reads lighter and more see-through than offline. The rest is the temporal filter,
  the real-time frame against the held one: ΔE 5.33 / 2.71 / 1.57 (5.74 / 4.65 / 3.51 on main before). At 0.2 s the
  fireball is 12 frames old and grows fastest; a pixel there holds four samples of history.
- **What the temporal filter does with a medium** (the shared live path; `pathtrace.wgsl` `media_composite`,
  `live.wgsl` `compose`). Two faults were in it, both measured here:
  - The medium's transmittance was multiplied into each frame's diffuse and specular lighting before those were
    accumulated, so the surface behind kept the fireball's shadow of the frames before. It now goes with the sample
    and is applied to the denoised lighting where the frame is composed (alone: the fireball 6.78 / 6.35 / 4.59).
    `volume/animated-clears-live`: 16 frames after a medium ends, the wall behind it read 0.88 of 2 nits; now 2.00.
  - Nothing told the filter that a medium changes. A volume function is `animated` when its code reads `i.time` (or
    says so), a simulated grid always; the march gives the share of each pixel such media take, and the upscaler
    holds four samples of history there, as it does for a surface with animated shading (alone: 7.02 / 7.24 / 4.91).
  The fire light no longer matters to the fireball: without it ΔE 6.36 / 5.62 / 4.19. It lights the street.
- **How the numbers moved** (the fireball's box at 0.2 / 0.5 / 1 s): 6.90 / 6.61 / 4.36 when step 10 b merged, with
  the fire light's change cutting the whole frame's history to 16 frames (without the light 11.2 / 9.9 / 9.5); 7.18 /
  7.53 / 5.25 after #30 (the frame at render resolution and its upscaler, which holds 16 samples of its own;
  without the light 7.51 / 8.47 / 7.56); 7.32 / 7.81 / 5.71 after #33 (the cut gone, each pixel bounded at 32 frames
  while the lighting changes); 6.28 / 5.56 / 3.69 now.
- **The whole image** meets at 0.2 and 0.5 s; at 1.0 s its ΔE meets and its SSIM is 0.005 short (the fireball's box
  spans the image's height, its smoke over the top cells). Its p99 (16 / 21 / 24) grows with the fireball's box; away
  from the fire the cells' p99 is the level's own at 0.75 (foliage, about 10).


#### Why volumes must not overlap (measured 2026-11-12, kept as the evidence)

(Since 2026-11-13 the kernel draws overlapping media together, offline and live: ROADMAP § 1a "Overlapping media".
What follows is of the kernel before, and is why `Explosions` was built without a grid.)

The first `-fire` had the fireballs inside the smoke grid's box and puffed hot smoke at each fireball's centre. Live
and offline both take the media whole in the order a ray enters them: the smoke box first, so each fireball was drawn
behind all the smoke on that ray. With the grid (fire at 0 s offline, at 2 s live, the same age;
`out/voxel-fire/sheet-grid-vs-none.jpg`) the fire was not seen at all at 0.2 and 0.5 s, offline or live:

| after the fire starts | the fireball's region, no grid: ΔE / SSIM / luminance | with the grid around it |
| --- | --- | --- |
| 0.2 s | 6.90 / 0.846 / 1.02 | 19.93 / 0.664 / 2.40 |
| 0.5 s | 6.61 / 0.850 / 1.22 | 9.36 / 0.820 / 1.30 |
| 1.0 s | 4.36 / 0.839 / 1.09 | 6.14 / 0.851 / 1.14 |

Burning the fire in the grid instead (hot smoke that flames, one medium; fires in the grid, fireballs outside it) was
tried and set aside, on its measurements (`out/voxel-fire/sheet-gridfire.jpg`): at 0.2 s a large white ball without
detail, at 0.5 and 1 s its region ΔE 17.0 and 10.8 with live 1.5 times offline's luminance (at 1 s offline dark smoke,
live light flecks), against the fireball's 6.9 / 6.6 / 4.4 with a flame's shape; and a grid covers its box only (16 of
the level's 160 wall spots), a small fire is one to four cells, every cell is simulated every frame, and offline has
no reference for a fire that does not start at 0 s (`out/web-probe/step10/b-structure-data.md`). Until the engine
draws overlapping media together (the last temporary way round below), the fire stays out of the grid.

Cost: `--only cost`, the destruction game's routes at 0.75, lockstep (frames as fast as the GPU goes), GPU ms p50 over
2 s before and 4 s after the explosion; no rebuild in any run.

| route | before | after | the explosion frame's CPU |
| --- | --- | --- | --- |
| the level (`level-destruction`) | 8.2 | 8.4 | 25.5 |
| with the smoke grid (`-smoke`, 709,632 cells; no fire) | 9.5 | 10.2 | 27.5 |
| with fire (`-fire`: fireballs and their lights, no grid) | 8.5 | 9.0 (10.3-10.6 while it burns, then 8.7) | 25.7 |
| fire, `sustained` (7 a second for 10 s) | 8.6 | 11.8 (p95 12.4, max 12.8); at most 8 burning, 8 lit | |
| fire, `peak` (20 within a second) | 8.8 | 11.3 (max 12.7); 8 burning (the pool) | |
| fire, software traversal (loop guard on: no stop) | 13.0 | 13.7 (max 18.0 while it burns) | 28.9 |

### Joints, the level's anchor and the blast (ROADMAP § 6 step 10 c1; design: `out/web-probe/step10/c-design.md`)

Engine-side physics the vehicles and ropes of (c2) stand on, each the standard structure where the engine's code did
otherwise (the user's decisions 1-4, 2026-11-12; the independent review of #31).

- **A joint's parameters change in place.** A hinge's or a slider's limits and motor, a rope's length: `needsUpdate`
  sends `joint_set` (physics-core `Physics::set_joint`), the same Rapier joint; its frames are not taken anew from the
  bodies' poses, its warm-started impulses are kept. Before, every change removed the joint and made it again (a motor
  set each frame remade it each frame). A change of its bodies, anchors or axis makes it anew; the engine refuses a
  `joint_set` that names any of them (the rule is the engine's, not only the binding's). A hinge's limits are
  re-centred through its first frame with libm (as when it is made: the platform's `sin_cos` of a non-zero centre is
  not on our path).
- **Joints to the level hold to the world's anchor.** One fixed engine body with no collider (made at load, at the
  origin, unturned), as engines give joints to the static world (Rapier's static colliders are on no body; a joint
  needs a body): a joint whose object is `engineColliders` holds to it (a rope tied to a wall), a point of it the same
  point of the world. The level's colliders stay static, on no body, as before: to queries and contacts the level has
  no body (`hit.object` is `engineColliders`, `hit.body` undefined). Scene code cannot move the anchor (`pose` refused);
  a query's `skip` of it skips nothing (it has no collider; the level's colliders are not a body's to skip). An earlier
  draft hung the level's colliders on the fixed body instead, which made Rapier sum the body's colliders' mass
  properties on every edit (O(colliders): a tick after a small carve grew from 0.03 to 1.2 ms over 64 to 10,000 level
  colliders, and an explosion's frame took 26 s before a special case hid it); with the anchor the tick after a carve
  is main's, flat (0.02-0.05 ms at 64 to 10,000 colliders), and no special case is left.
- **A joint belongs to the material at its anchor.** Each joint on a voxel body or on the level is held by the cell its
  anchor is in (or the nearest solid of its 26 neighbours): cut so that the cell is in a part cut off, the joint moves
  onto that part's body (the same joint, its anchor where it was in the world); the cell carved, in debris, or its body
  carved away whole, retired, fallen or taken out by the world (past its edge), the joint breaks. The engine reports
  both (`joint_changes`): a moved joint's `bodyA` (or `bodyB`) becomes the part's `VoxelBody` and its `anchorA` (or
  `anchorB`) the same point in the part's frame, so a rope drawn from them follows and a body of the joint made anew
  joins it to the part; a broken one has `joint.broken` true and a "broken" event (every joint of a batch is marked
  before any event is dispatched; a listener that throws does not stop the rest), and is not made again until it leaves
  the scene. The engine finds a body's joints on the body (Rapier's joint graph), a carve's joints by the cells it took
  (a level carve looks only at joints within a cell of what it cleared): costs go with what an edit touches, not with
  the joints in the world (40,000 joints moved with a cut part: 8-10 ms; before, 6 s and more).
- **The blast.** An explosion (`voxelExplode`, `voxelEdit` "explode", `Explosions.explode`) pushes every dynamic body
  within 3 radii (voxel bodies, the parts it just cut loose, scene code's bodies) away from its centre, through each
  one's centre of mass (no turn, as the industry's radial impulse by default):
  J = push x r x min(1, (r / d)²) x (1 - (d / 3r)²)² x A, with d the distance from the centre to the body's box (0
  inside) and A its box's cross-section across the direction. r is the radius the carve took (at most `MAX_RADIUS`, 128
  voxels: a 600 m explosion pushes as far as the 12.8 m one it carves). `push` defaults to 400 N s / m² per m of radius
  (a 1 m blast throws a 100 kg, 1 m crate whose face is 1 m away at 3.16 m/s), 0 to 10,000, 0 none; out of range
  refused before anything changes. A light, broad body flies faster than a heavy compact one (the impulse goes with
  area, not mass).
- **The debris' kick.** The parts an explosion itself cuts loose also leave at a speed of their own, whatever their
  mass: push x 0.02 m/s (8 m/s at the default push) x min(1, (r / d)²) x (1 - (d / 3r)²)², the same fall-off, added to
  the push's, a body's whole change at most 30 m/s (what the solver and CCD follow). A charge throws what it breaks off
  at a speed of the charge's (as in the Gurney model of fragments: set by the charge and what holds it, not by each
  fragment's size); by area
  alone a 1 m³ chunk of stone (2,400 kg) took 0.1 m/s and stayed where it was cut. `push` 0: neither. On the
  benchmark level (`tools/bench-voxel-fire.ts --only cost --scenario topple | sustained`, against the push alone): a
  tree a 0.6 m explosion cuts loose leaves at 6.5-8.2 m/s (0.2-4.7), a part of a wall a 1 m one cuts loose at 7.9
  (0.75-1.24; the run's top speed 7.9 for 3.06); frame times within run-to-run spread. The level's 4 m explosions (the
  crater on the road, the wall spots of `singles`) cut nothing loose: there is no debris for them to kick. Only +, -, x, /
  and square roots: the same bits natively, in parallel and in wasm32. **Not shielded** (decision 4): a body behind a
  wall is pushed too; one voxel ray per body would shield (ROADMAP Open).

Checks: physics-core `joints_change_in_place`, `a_blast_pushes_by_area_and_distance`,
`a_blast_kicks_its_debris_whatever_their_mass`, `a_bodys_joints_are_its_own`, `the_anchor_and_the_engines_joints`; the
renderer's `the_level_has_an_anchor_that_joints_hold_to`, `joints_follow_the_cells_their_anchors_are_in`,
`an_explosion_pushes_the_bodies_near_it`, `an_explosion_kicks_its_debris_and_reaches_what_it_carves`; the binding's
joint, moved and broken-joint tests (the engine faked), and test-render's `voxel/joint-moved` and
`voxel/joint-moved-body` (the binding on the real engine); recorded hashes with a blast, its kick and an in-place joint
change (physics-core's script) and with a pushing explosion and one that cuts a body in two (the voxel bodies' script):
native = parallel = wasm32 = recorded; `physics_fuzz` with `joint_set` (and refused fields), joints to the level, the
anchor among every call's bodies (`pose` refused, the anchor checked unmoved), the joint reports and pushes (sane and
wild).

### Temporary ways round on the engine's shared paths

`Explosions` (step 10 b) works round five faults of the shared renderer, in its own code, until they are fixed
there (the user, 2026-11-12: these are temporary; the shared paths are fixed in their own PRs, then these go, in a PR
of their own). Each place in `packages/voxel/src/explosions.ts` points here. (A sixth is closed at its source: the
fireball's ghosting, once held down by its light's change cutting the whole frame's history; "Where it falls short"
above.)

| What | Now | The fault at its source | Known side effects | It can go when | How to tell |
| --- | --- | --- | --- | --- | --- |
| The stand-in light | a point light at each burning fire, its intensity and colour from `FIRE_LIGHT`, measured against offline | live secondary and shadow rays have no media code: `pathtrace.wgsl` `media_shadows()` is false live (line 796), GI and reflection rays never meet a medium | offline the fire's light counts twice unless the game passes `lights: 0` (`level-destruction-fire` rendered offline has both); live, `media_train` adds the light to the radiance cache's voxels (`point_direct`, `point_sphere`): the fireball (and its smoke) is lit by it a second time; the table holds for the defaults (density, life scaled; measured by day over a floor and a wall, the fireball 0.6 of its size above it; past 8.8 m the 8.8 m values) | live rays see media: a medium's emission lights surfaces and shadows pass through it | `lights: 0`, then `tools/bench-voxel-fire.ts --only quality`: the street and the fireball's regions at least as close to offline as with the light; `voxel/fire-light` replaced by a live test of the fire's own light |
| Idle fireballs parked | at y = −10,000, scale 1e-3, instead of hidden | `World::set_visible` (`world.rs` line 750): hiding anything but particles rebuilds the live scene | parked fireballs keep their slots among the kernel's 32 volumes, and every pixel's march still tests their boxes | hiding a volume or a light is done in place | `visible = false` instead of parking; the bench's cost runs report no rebuild after an explosion |
| Idle lights reach 1 mm | an idle light's `distance` is 1 mm where it is parked | `light_reach` (`lib.rs` line 211): a black light without a range reaches without limit, and the kernel does not skip black lights before the group test (`pathtrace.wgsl` line 1578) | parked lights 10 km away share a group of 16 with lit ones, and groups are not sorted again when lights move (`lib.rs` line 277): the group's sphere is about 5 km wide while any fire burns (the per-light range test still works) | black lights reach nothing (or are hidden in place) and moved lights are grouped again | distance 0 on idle lights; the fire route's GPU time before any explosion within 0.1 ms of the smoke route's |
| The smoke kept by never rebuilding | the pool's objects movable from the start; idle ones parked, not hidden (`Explosions` has no grid now: this keeps a game's own `SimulatedSmoke`) | `LiveCore::rebuild` (`live.rs` line 3474) → `load_scene` → `Gpu::upload` makes new grid textures (`gpu.rs` line 1664): the simulated state is not carried over | any other structural change in the game (an object added, a texture, an edit's fallback) wipes the smoke and the fire burning in it | a rebuild carries every simulated grid's state over | the bench's smoke check with a rebuild forced at 1 s (an object added): the smoke at 3 s as without it |
| Volumes must not overlap | `Explosions` has no smoke grid (it refuses one); the smoke is each fireball's own. Two fires close together still make two fireballs that overlap: nothing keeps them apart (no distance rule) | `media_track` (offline) and `media_march` (live) in `pathtrace.wgsl` take the media whole, in the order a ray enters them (at most 32, a 32-bit mask), not their coefficients summed along the ray; hence the rule (`core.ts` `VolumeMesh`, MATERIAL_FUNCTIONS.md) | a fire inside smoke is drawn behind all of it (measured: not seen at all at 0.2 and 0.5 s, "Why volumes must not overlap"); neighbouring fireballs hide each other in the same way; no smoke grid around explosions | **now** (2026-11-13: the kernel takes the ray a stretch at a time and sums the coefficients of the media it is in, offline and live; `volume/overlap-emission`, `-live`). What is left is `Explosions`' own: taking a grid again, in a PR of the voxel line's | puff a `SimulatedSmoke` around fireballs again (`Explosions` taking a grid) and run the comparison that found it (`bench-voxel-fire` at d7b4e4a, `--only smoke --ages 0.2,0.5`): the fire seen in its smoke offline and live; two overlapping fireballs both seen |

### Where the engine works round its dependencies

Each is a way round a fault or a gap of Rapier 0.36.1, parry 0.31.1, glamx 0.3.1 or rayon 1.12, in our code (no
crate is patched but glamx and parry3d, in `renderer/third_party`). When a new release comes, this says what can go.

| What | Way round | Where | It can go when | How to tell |
| --- | --- | --- | --- | --- |
| The parallel step can hang: Rapier joins its deferred BVH pass with a blocking `recv` on a pool worker, and rayon does not promise another worker takes the job (ROADMAP § 6 step 8) | the wait for a step injects an empty job into the pool every 0.5 ms (`THREERT_PHYSICS_KICK=off` waits without) | `renderer/src/physics.rs` `in_pool` | Rapier joins that pass without blocking a worker (or runs it inline) | `cargo run --release --example physics_fuzz -- --runs 0 --stress 10000000` with `THREERT_PHYSICS_KICK=off`: no hang (today it hangs within 10⁵–10⁶ steps); then delete `in_pool`'s kick |
| A ball on voxels panics the solver ("stale ContactRef manifold ordinal"): parry's voxels-ball contact keeps no workspace, so Rapier takes the pair for a simple one | the pair goes through parry's general voxels-shape function (a query dispatcher) | `renderer/physics-core/src/world.rs` `VoxelsWithBalls` | parry's voxels-ball contact keeps a workspace (or Rapier stops deciding by it) | take the dispatcher out and run `check::cases` (its voxel kind), `voxel_worlds_never_panic` (20,000 worlds, ignored: `-- --ignored`) and `random_calls_on_voxel_shapes_never_panic`: none panics |
| parry's ray misses voxels when it lies on a lattice plane and runs along it (8-22 % of rays straight down on a plane: a solid cell's box is asked for the hit again, and a ray in its face's plane grazes it) | **fixed at the source**: parry's chunk walk takes a solid cell as hit where the ray entered it (2026-11-12; the world's own walk is gone, every ray goes through Rapier's query) | `renderer/third_party/parry3d` (`PATCH.md`: `ray_voxels.rs` only), `[patch.crates-io]` in the three manifests | a parry release has the fix (upstream report: `out/upstream/parry-voxel-ray/DRAFT.md`, not sent) and Rapier uses it | delete the copy and its patch lines; `rays_at_voxel_shapes` (brute force, the lattice's planes, 0 misses) and the recorded hashes hold |
| parry's `Voxels::voxels()` leaves cells out: it takes its range from the shape's box in metres, and f32 rounding can drop a layer (one cell at (-1000, 9, -673) of 0.1 m is listed as none; found by the fuzz, a body's count 29 short) | the world lists a shape's cells over its exact `domain()` (`voxels_in_range`) | `world.rs` `listed` | `Voxels::voxels()` takes its range as `domain()` does (from voxel centres) | `every_voxel_is_listed` with `voxels()` in place of `listed` |
| A voxel shape edited where it sits (`shape_mut`) leaves stale recycled contacts on pairs that do not move | `edit_voxels` takes the collider out and puts it back under the same id | `world.rs` `Physics::edit_voxels` | Rapier refreshes a collider's manifolds when its shape is edited in place | `voxel_shapes_are_edited` and the voxel fuzz with the edit done through `shape_mut` |
| glamx's eigen decomposition calls the platform's `acos` / `cos` when `std` is in the crate graph: other bits natively than in wasm32 | the three calls through `simba`'s libm, by name | `renderer/third_party/glamx` (`PATCH.md`), `[patch.crates-io]` in both manifests | a glamx release has the fix (dimforge/glamx #9, #10) and Rapier uses it | delete the copy and the patch tables; `tools/physics-determinism.ts` (its "std in the crate graph" run) and the recorded hashes hold |
| Rapier takes the sine and cosine of a hinge's limits' centre with the platform's methods (0.024 % of angles differ in the last bit between macOS and wasm32) | limits reach Rapier centred, [−h, h], and the centre turns the first body's joint frame | `world.rs` `Physics::add_joint` | Rapier computes them with libm under `enhanced-determinism` (dimforge/rapier #1054 lists such calls) | `revolute_limits_off_centre` and `tools/physics-determinism.ts` (cases with joints) with the limits passed as given |

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
  touched and not the scene (in a session and in a browser; an offline render under software traversal flattens
  them, as before, and is byte for byte the reference render it was). Measured natively under software traversal
  (2026-11-11), live: the level's triangle data 310.9 MiB (flattened: 2,098), an explosion 2.5–4.5 ms in place
  (6.9–7.9 s), and a still frame's trace 7.85 ms where the flattened level's is 4.58 (the chunks' boxes overlap; the
  top tree is not tuned yet).
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

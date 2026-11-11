# Roadmap

## Goal

ThreeRT's goal is AAA real-time game rendering: on consumer GPUs (Apple M series first), dynamic, open-world-scale game
scenes (characters, vehicles, crowds, weather, day and night, all changing) at a steady 60 fps, with image quality close
to an unbiased path-traced truth: ΔE ≤ 3 and SSIM ≥ 0.9 against our offline render, and no visible ghosting in motion.

Every new scene or feature maps to a capability in this goal that has not been proven yet, with measurable acceptance.
No demos that repeat what is already proven.

## Direction

Direction of 2026-11-10 (the user's, relayed by the supervisor, who rewrites the roadmap by it): the day / night cycle
and the weather are developed and measured on Tokyo, in this phase (§ 0 steps 1a and 1b), after step 1 and before
streaming; there is no separate scene for them. The main line takes over the time of day the supervisor brought
forward (#121, `time-of-day`); the supervisor is back to its two-hourly supervision.

Direction of 2026-11-09, evening (the user's, relayed by the supervisor, who rewrites the roadmap by it): the main
line moves to an open city at the scale of Marvel's Spider-Man and GTA VI, Tokyo compiled from Japan's open 3D city
model (§ 0); the dynamic content phase closes (its record is the history under § 0). Two lines share this repository
and each merges its own PRs: this main line (Tokyo; the supervisor accepts and merges) and the voxel engine (§ 6,
Yujia's line, which merges its own). This line does not start § 6 steps; a [ThreeRT] contract either line adds is
there for the other, through its gate. Bistro and Zero Day stay the regression benchmarks.

Direction of 2026-11-09 (the user's): a voxel game engine built around ThreeRT, for voxel games in general, measured
on a destructible benchmark level (§ 6, `docs/VOXEL.md`). A capability every kind of game needs that ThreeRT lacks
goes into ThreeRT first, as a generic contract through its own gate, then the engine continues. Bistro stays
ThreeRT's benchmark.

Direction of 2026-11-08, later the same day (the user's, relayed by the supervisor): the next phase is dynamic content
(§ 0). Static Bistro night at 60 fps is proven; characters, vehicles and moving lights at scale are not. The static
quality targets continue alongside.

Direction of 2026-11-08 (the user's, relayed by the supervisor, who rewrites the roadmap by it): the industry-standard
scenes Amazon Lumberyard Bistro (night) and Beeple Zero Day replace Kabukichō as the scenes we are judged on. Each item
has acceptance metrics; its status line is updated with the measured numbers when it lands. Merging PRs and deploying
to production stay with the user.

Earlier direction (2026-11-05, kept for the history below): finish the three.js baseline and the UE mannequin → 2a / 2b
→ the A layer of the test suite (3) and the dev-loop tools (4) → the decision experiment (1) → split signals + caches
(1, step 1) → the GTA gaps (5), all on Kabukichō.

## How we are judged (2026-11-08, the user's decision, revised the same day)

- Day-to-day truth is our own unbiased offline render: no clamps (`clampIndirect` off), no denoiser, high spp (4096 at
  1280 × 720 for milestone images, 16 k at 320 × 180 for estimator studies). Together with layer A's analytic tests it
  is the reference for every PR, every evaluation and every report.
- UE leaves the day-to-day loop. It is a milestone audit: once per milestone or per new scene, with the user present,
  small UE Path Tracer runs compared with our offline render, to confirm our path tracer and our reading of the assets
  (materials, emission, exposure). UE runs never happen while the user is away.
- three.js is a real-time renderer we are compared with, not a target to approach. Every real-time renderer (ThreeRT
  live, three.js strongest / tuned) reports two kinds of numbers:
  - frame rate / GPU ms;
  - ΔE and SSIM against our offline render, overall and by region.
- Budget: 60 fps on Bistro night, which means a frame interval ≤ 16.7 ms as a player sees it: along Bistro's FBX
  camera path, p95, at the 0.5 dynamic-resolution floor. GPU time at one pose within 16.7 ms is not enough (the first
  pose reads 16.1 ms, the path 18.6 ms p50). The perf smoke and perf suite measure both (`docs/WORKFLOW.md`).
  Since 2026-11-09 the budget is read on frame starts (`start` in the frame log: what the pacer sets); the open
  city's drive and flight (§ 0) are judged the same way.

## 0. Current work: an open city, Tokyo

- Why (the user's, 2026-11-09): the main line needs the scenes of Marvel's Spider-Man and GTA VI, a dense real city at
  open-world scale that the player crosses fast, not Bistro copied again (the 3 × 3 district). It comes before day /
  night and weather, which this city then carries (its source app already has a time of day, a physical atmosphere and
  volumetric clouds).
- What is proven and what is not:
  - Proven: one resident scene at a time, at most the 3 × 3 district (Bistro nine times, one copy in memory, 11.3 GB;
    GPU p95 14.1 ms along its path); crowds and traffic on one street at 60 fps (frame start p95 16.67 ms).
  - Not proven: a world larger than memory; kilometres of distinct geometry streamed in and out while the camera moves
    fast; a city's worth of emitters at once (lit windows, signs, screens, street lamps, headlights); traffic and
    trains on a real network.
- The benchmark:
  - **The city: Tokyo**, from the pipeline of [jeantimex/tokyo](https://github.com/jeantimex/tokyo) (MIT, three.js).
    - An offline compiler turns three sources into 256 m binary tiles:
      - Project PLATEAU, MLIT's open 3D city model: real footprints, heights, roof shapes and photo textures, roads,
        bridges, street furniture (CC BY 4.0, multi-licensed with ODbL);
      - OpenStreetMap: the road graph, railways, places (ODbL);
      - GSI: elevation and aerial photos.
    - The client streams the tiles and adds generated detail: facades and windows, signs, road paint, traffic on the
      real road graph, trains on the real lines.
    - Areas of ~3.4 × 2.8 km: Shinjuku (the skyscrapers, Kabukichō), Shibuya (the Scramble), Tokyo / Marunouchi, Shiba
      (Tokyo Tower), Chiyoda (Akihabara), Chuo (the waterfront and canals). PLATEAU covers ~300 cities, all 23 wards of
      Tokyo among them; an area is one config entry.
  - **Scale and density targets** from UE's City Sample (The Matrix Awakens). Its assets are licensed for Unreal only,
    so it serves as numbers and as the UE audit's look reference: 4.1 × 5 km (15.8 km²), 7,000 buildings, 260 km of
    roads, 1,248 intersections, 45 k parked cars, 17 k traffic vehicles, 35 k pedestrians, a night lit almost entirely
    by emissive windows.
  - **Traversal targets** from Marvel's Spider-Man 2: the city crossed at speed, with no loading screen.
  - **Comparators:**
    - the source app itself (three.js WebGL, on the same compiled tiles): frame time, and ΔE / SSIM against our
      offline render at the same poses, as milestone 1 did with three.js;
    - UE 5.8 with the official PLATEAU SDK for Unreal (MIT) on the same city, for the milestone audit with the user
      present.
  - Truth: our unbiased offline render, as everywhere.
  - Attribution and licences: PLATEAU (MLIT), GSI, © OpenStreetMap contributors, Poly Haven (CC0), ez-tree (MIT) and
    takram three-geospatial (MIT) are credited wherever the city is shown. Compiled tiles redistributed outside the
    team are a derived database under the ODbL: do not publish them without that.
- The source's raster tricks become physics in ThreeRT, which is the point of the comparison:
  - lamp light-map quads → street lamps and headlights as emitters in the light tree;
  - the mirrored-city water and the screen-space window reflections → traced reflections;
  - n8ao → path-traced indirect light; the shadow map → shadow rays.
- Bistro and Zero Day stay the regression benchmarks (layer B, perf suite); Tokyo's lines are added next to them.
- Steps, one PR per item, in order; the supervisor accepts and merges. Each lands with its numbers in its status line.
  1. **Import, one area resident.** The pipeline brought in (`tools/city/`: compiler and tile format, its MIT notice
     and the data attributions kept) and Shinjuku compiled. A loader turns tiles into `@threert/scene` content:
     buildings (photo roofs and walls, generated facades; glass as glass, lit windows as emitters), ground (terrain
     under the aerial photo, roads with kerbs, paint), props, signs, water. Static (no traffic), the whole area
     resident, night and day under the existing sky.
     - Acceptance:
       - Shinjuku renders offline and live; counts stated (triangles, buildings, emitters, textures, memory, load
         time);
       - 5 poses (street level at Kabukichō's gate and at the station's west exit, a skyscraper canyon, two aerial):
         live vs our offline render ΔE / SSIM, and the source three.js app at the same poses (frame time, ΔE / SSIM);
       - live at the 0.5 floor, frame start p95 ≤ 16.7 ms on a scripted 60 s flight over the area.
     - Status, part 1 (2026-11-09, branch `tokyo-import`):
       - The pipeline is in `tools/city/`, from jeantimex/tokyo `1984760`. It keeps its MIT notice and the data
         attributions. Paths were changed, nothing else. Downloads and tiles go to `assets/city/`, outside git:
         - Shinjuku's raw data is 1.5 GB;
         - compiled, it is 168 tiles and 68 MB;
         - fetching took 34 min, most of it on busy Overpass servers; compiling took 24 s.
       - `tools/tokyo/to-glb.ts` turns each tile into a GLB through the source's own mesher (`tools/city/mesh`).
         `games/tokyo` places the GLBs; room `shinjuku-day` / `shinjuku-night`. It carries:
         - terrain under GSI's aerial photo (tiles past the photo's edge padded with the land's colour);
         - road surfaces, kerbs, parks, water and paint, coloured as the source colours them;
         - buildings, PLATEAU's LOD2 roofs under their photo atlases (the atlas's v, up as GL reads it, turned to
           glTF's), and PLATEAU's models;
         - windows as geometry, cut from the source's facade rules (bays, floors, shopfronts): unlit ones as dark
           glass, 42 % lit (45 nits) as emitters.
       - By day the sun stands where it does over Tokyo at 12:00 on 2026-11-09, 36.8° high and 10.5° west of south
         (`SUN` in games/tokyo): where the source app puts it for that day's ?time=12.
       - Counts: 13.6 M triangles, 21,576 buildings, 1.32 M windows (584 k lit), 1.64 M emitter triangles, 215
         textures, 1.44 GB of GLB. Offline: loaded in 1.6 s, acceleration structures in 1.0 s. Live: first frame
         3.9 s, physical footprint ~15 GB.
       - Live at the 0.5 floor (64 frames) against our offline render (1024 spp, no clamps, no denoiser), ΔE / SSIM;
         the live GPU time at the pose, p50 / p95 ms; and the truth's own noise, a 256 spp render against it, as
         milestone 1 shows it (`tools/tokyo/step1.ts`):

         | pose | day: live | GPU | truth's noise | night: live | GPU | truth's noise |
         |---|---|---|---|---|---|---|
         | Kabukichō's gate | 0.93 / 0.967 | 12.0 / 12.8 | 0.48 / 0.980 | 1.22 / 0.938 | 10.8 / 11.6 | 0.90 / 0.925 |
         | west exit, Chūō-dōri | 1.04 / 0.938 | 11.5 / 12.1 | 0.64 / 0.953 | 2.78 / 0.902 | 10.3 / 10.9 | 0.94 / 0.939 |
         | skyscraper canyon, Tochō-dōri | 1.55 / 0.894 | 10.6 / 11.2 | 0.67 / 0.924 | 2.61 / 0.914 | 9.9 / 10.4 | 0.99 / 0.943 |
         | aerial east | 1.86 / 0.944 | 11.5 / 11.9 | 0.52 / 0.990 | 4.63 / 0.879 | 9.9 / 10.2 | 0.58 / 0.994 |
         | aerial west | 2.13 / 0.954 | 11.6 / 12.0 | 0.50 / 0.994 | 4.69 / 0.871 | 10.9 / 11.6 | 0.64 / 0.990 |

         - By day, in sun at street level, the truth is specked with the sun's glints off window glass (paths that
           reach the sun through a mirror bounce, not converged at 1024 spp). They are most of the canyon's SSIM:
           its own noise reads 0.924. Live reads 1–5 % dark there; the glints' energy, which live does not carry, is
           the likely part of it.
         - By night the aerial views read 11 % bright, against a truth that is quiet there (0.99): hundreds of
           thousands of lit windows in view, which is step 3's subject.
         - Crops (offline | live): `docs/img/tokyo-shinjuku-day.jpg` (aerial west, Kabukichō's gate, the canyon),
           `docs/img/tokyo-shinjuku-night.jpg`.
       - The 60 s flight, once round the area at 300 m and 1.2 km out: frame start p95 16.67 ms by day and by night,
         within the budget. GPU p50 / p95 is 12.1 / 12.9 ms by day and 10.5 / 10.8 by night.
         - Two frames in each run take 70–120 ms, at the same two points of the orbit by day and by night (about
           17 s and 27 s in). The time is the frame's CPU (80–104 ms) with the GPU at its usual 10–12 ms. Step 2 allows
           no frame over 50 ms: find them there.
       - Still to do for part 2:
         - props: street lights as emitters, trees, signals, vending machines, parked cars;
         - signs and road decals;
         - memory: the meshes are triangle soups with full attributes; step 2's 1.5 × bound needs them indexed and
           lean.
     - Status, part 1b (2026-11-09, branch `tokyo-three-app`): the source three.js app at the same poses.
       - `node tools/tokyo/three-app.ts` runs the app, served by its own vite dev server on the same compiled tiles
         (its `public/tiles` linked to `assets/city/tiles`), in headless Chrome at 1280 × 720 over the DevTools
         protocol (`tools/three/browser.ts`). It holds the GPU lock.
         - It waits until every tile is in (`#loader.done`), sets each pose on the app's camera, and waits for the
           tiles round it.
         - Traffic, birds and clouds are off. Day is the app's `?time=12`, with its sun where ours is (`SUN`);
           night is its `?night=1` (22:00, moonlit).
         - The shot is the canvas alone, without the app's buttons and credits.
         - The app keeps its camera 2 m above the ground; ours stands at 1.7 m, so its street poses sit up to
           0.4 m higher.
       - The app's frame time is taken two ways: its own frame loop (rAF intervals, what its user sees, capped at
         60 Hz), and its frame function run back to back with `gl.finish()` ("finished"). Against our offline render
         it reads ΔE / SSIM / luminance, as live does. ThreeRT live's GPU time and distance are from part 1's table.

         | pose | three.js frame (rAF) ms | finished ms | three.js ΔE / SSIM / luminance | ThreeRT live GPU ms | ThreeRT live ΔE / SSIM |
         |---|---|---|---|---|---|
         | day: Kabukichō's gate | 16.7 / 33.3 | 1.6 / 18.4 | 30.4 / 0.345 / 2.37 | 12.0 / 12.8 | 0.93 / 0.967 |
         | west exit | 16.7 / 33.4 | 4.4 / 23.2 | 25.3 / 0.300 / 3.24 | 11.5 / 12.1 | 1.04 / 0.938 |
         | skyscraper canyon | 33.3 / 33.4 | 6.9 / 30.7 | 27.6 / 0.284 / 1.60 | 10.6 / 11.2 | 1.55 / 0.894 |
         | aerial east | 33.4 / 50.1 | 37.1 / 37.9 | 28.0 / 0.514 / 1.94 | 11.5 / 11.9 | 1.86 / 0.944 |
         | aerial west | 50.0 / 66.7 | 52.5 / 53.6 | 26.4 / 0.477 / 1.74 | 11.6 / 12.0 | 2.13 / 0.954 |
         | night: Kabukichō's gate | 16.7 / 33.3 | 4.5 / 18.4 | 23.5 / 0.373 / 0.60 | 10.8 / 11.6 | 1.22 / 0.938 |
         | west exit | 16.7 / 33.4 | 21.2 / 23.7 | 31.4 / 0.219 / 0.56 | 10.3 / 10.9 | 2.78 / 0.902 |
         | skyscraper canyon | 33.3 / 33.4 | 28.6 / 29.9 | 25.2 / 0.228 / 0.77 | 9.9 / 10.4 | 2.61 / 0.914 |
         | aerial east | 33.4 / 50.1 | 37.6 / 38.2 | 18.7 / 0.132 / 0.29 | 9.9 / 10.2 | 4.63 / 0.879 |
         | aerial west | 50.0 / 66.7 | 52.6 / 53.7 | 18.9 / 0.163 / 0.23 | 10.9 / 11.6 | 4.69 / 0.871 |

       - Frame time:
         - The app runs at 60 fps at street level, except in the canyon (30). It drops to 30 and 20 fps in the
           aerial views, where its frame function takes 37 and 53 ms to finish: the whole area is drawn.
         - ThreeRT live stays at 10–12 ms of GPU at every pose, and its 60 s flight paces at 60 fps.
         - The work differs: the app rasterises at full 1280 × 720, while ThreeRT path traces at the 0.5 floor
           (640 × 360) and upscales.
       - The image: ΔE 19–31 and SSIM 0.13–0.51 against our offline render. Milestone 1 read three.js at ΔE 10–16,
         but there both drew the same scene; here the app tones its own way and draws content we do not.
         - Exposure and tone account for much of it: the app is 1.6–3.2 × brighter by day and 0.23–0.77 × by night.
         - The rest is what the app draws and we do not yet (`docs/img/tokyo-shinjuku-three.jpg`: ours offline |
           ThreeRT live | the app):
           - trees (street trees, the park's canopy), poles and wires, shop signs and neon;
           - facades with window frames and wall finishes from its facade shader, where ours are flat colour and
             dark glass;
           - by night, street lights on the roads, and far fewer lit windows in warm and cool tones. Ours are 42 %
             lit and near white at EV100 5, and they wash the aerial views out;
           - the land beyond the area hazed into the sky, where ours is a brown band.
         - Part 2 takes the props (street lights as emitters, trees), the signs and the window mix. Facade finishes
           and the far haze are noted for step 3's look work.
     - Status, part 2 (2026-11-09, branch `tokyo-props`): street props, and the night's windows as the source lights
       them.
       - `tools/tokyo/props.ts` puts the client's street props into each tile's GLB as geometry. They are placed, turned
         and lifted as the client places them (`src/world/props.js`):
         - 4,421 street lights and 3,437 utility poles, each with an LED head;
         - 577 signals, red and green lit;
         - 514 vending machines, lit fronts, in the client's four body colours;
         - 2,152 parked cars;
         - 24,348 trees: a trunk and the client's far-tree crown. Park trees stand in clumps of three, as the client
           sets them.
         - Not yet: the mapped street furniture (bus stops, benches, bollards and the like). Near trees are still the
           far shape; the client draws ez-tree models up close.
       - The lamp heads are emitters by night only. Each tile's GLB holds them twice: lit in its "night" node, unlit in
         its "day" node (`tools/tokyo/glb.ts`). The scene loads "city" plus the room's node (`GLTFModel`'s `nodes`).
         - A street light's head (0.3 × 0.75 m, 6000 nits) gives ~20 lux on the road 8.5 m below.
         - A utility pole's lamp gives ~19 lux 5.6 m below.
         - Both are within JIS Z 9111's range for such roads.
       - Lit windows follow the source's facade shader at night, with our hashes in place of its own:
         - the lit share by kind (shops 0.75, offices 0.32, homes 0.22), times 0.35–1.9 for the building;
         - most offices lit by the floor;
         - towers mostly dark glass, with a few storeys lit as bands;
         - warm bulbs, cool tubes and the odd screen blue;
         - brightness by room and building: 45 nits × 0.2–1.7, in three bins.
         - 34 % of windows are now lit (444 k), against 42 % before.
       - Counts:
         - 17.7 M triangles (+4.1 M, mostly trees), 1.29 M emitter triangles by night, 1.89 GB of GLB;
         - offline: loaded in 2.0 s, acceleration structures in 1.8–1.9 s;
         - live: first frame 5.1–5.3 s, physical footprint ~18 GB (15 GB before). The trees are triangle soups, one
           copy per tree; instancing them is part of step 2's memory work.
       - The tile photos and roof atlases carry a 16-texel margin: what lies beyond each edge (the neighbouring photo;
         the atlas's own edge texels), with the UVs inside it. The renderer repeats every glTF texture, so filtering at
         an edge used to reach across to the texture's other side, a seam along every tile boundary from above.
       - The truth is now 4096 spp, and its noise is measured with the 1024 spp render. At 1024 spp the night streets'
         truths read SSIM 0.80–0.87 against 256 spp, and comparing live with them said little.
       - Live at the 0.5 floor against the 4096 spp truth, with the live GPU time (p50 / p95 ms) and the truth's noise:

         | pose | day: live ΔE / SSIM | GPU | truth's noise | night: live ΔE / SSIM | GPU | truth's noise |
         |---|---|---|---|---|---|---|
         | Kabukichō's gate | 0.85 / 0.975 | 11.8 / 12.6 | 0.30 / 0.992 | 1.27 / 0.940 | 10.6 / 11.3 | 0.72 / 0.948 |
         | west exit, Chūō-dōri | 0.90 / 0.951 | 11.2 / 12.1 | 0.37 / 0.973 | 2.18 / 0.842 | 10.2 / 11.0 | 1.34 / 0.875 |
         | skyscraper canyon, Tochō-dōri | 1.69 / 0.806 | 10.8 / 11.8 | 0.80 / 0.866 | 1.94 / 0.825 | 10.5 / 11.2 | 1.26 / 0.873 |
         | aerial east | 1.86 / 0.947 | 11.7 / 12.1 | 0.31 / 0.996 | 2.92 / 0.894 | 10.1 / 10.4 | 0.29 / 0.993 |
         | aerial west | 2.19 / 0.953 | 11.8 / 12.1 | 0.36 / 0.994 | 2.54 / 0.883 | 10.2 / 10.5 | 0.31 / 0.989 |

         - All ten poses are within ΔE ≤ 3. Five are within SSIM ≥ 0.9.
         - Three of the misses are bounded by the truth itself: the canyon by day, and the west exit and the canyon by
           night. Each truth's own noise reads 0.87 there.
           - By day this is the sun's glints off window glass, now also on the tree crowns. Live does not carry them
             and reads 7 % dark in the canyon.
           - By night it is the light of many small, bright sources on the road. Live shows it as blotches on the
             asphalt that the truth does not have.
         - The night aerials are a real gap with a quiet truth (0.99): SSIM 0.88–0.89, 3–4 % bright. They read
           11 % bright with the old windows.
       - The 60 s flight: frame start p95 16.67 ms by day and by night, within the budget. GPU p50 / p95 is 12.4 /
         13.0 ms by day and 10.5 / 10.9 ms by night.
         - The two CPU stalls are still there, at the same points, now 100–155 ms (the frame's CPU 88–138 ms): they
           grow with the scene.
       - The source app at the same poses (`docs/img/tokyo-shinjuku-props.jpg`: ours offline | ThreeRT live | the app):
         - by night ΔE 14–24 (19–31 before): our windows no longer wash the views out;
         - by day ΔE 25–30, unchanged. The app's exposure, 1.8–3.7 × brighter, and its facade finishes dominate.
     - Status, part 3 (2026-11-10, branch `tokyo-signs`): signboards and the road's painted symbols.
       - `tools/tokyo/signs.ts` letters the compiler's signboards as the client does (`src/world/signs.js`):
         - fascias flat on the wall, blades standing out from it (lettered on both faces), titles on top;
         - each tile's names in one atlas, drawn as SVG through sharp with the system's Japanese fonts: vertical
           writing on blades, long names squeezed to fit, the chains' stripes;
         - 3,746 boards, every board the tile data letters, on 140 atlas pages (2048 wide, up to 4096 high; two busy
           tiles take a second page). Billboards and screens (the client's poster atlas) are not drawn yet.
           - The first version dropped the boards that did not fit one page: 38 in Shinjuku, and 117 once the cells
             had their margins (Bugbot on #123).
         - The boards glow through their atlas: 200 nits by night (the GLB's "night" node), 20 by day ("day").
         - Each cell has an 8-texel margin of its own edge texels, and the boards' UVs sit half a texel inside the
           cell, so filtering at an edge, and the mips down to the fourth level, read the board alone. The renderer
           repeats every glTF texture whatever the sampler says, so the margin, not CLAMP_TO_EDGE, keeps the edge
           clean. Before (Bugbot on #123), the edges picked up the atlas's grey ground.
       - `tools/tokyo/decals.ts` paints the road's symbols: arrows, 止まれ, speed numbers, 1,919 of them.
         - They are drawn as the client draws them, then traced into outlines and triangulated: 12–159 triangles per
           symbol. Each is laid where the mesher lays its quad and draped as the road's paint is: split on the
           terrain's cells and diagonals, each piece in the plane of the road under it, 0.17 m above it.
           - With one height per vertex, as first done, a symbol stood between −0.015 and +0.311 m over the road
             (Bugbot on #123). Draped, it stands at 0.17 m everywhere: 140 k triangles instead of 75 k.
         - As alpha cutouts they cost 1.0–1.4 ms a frame over the whole view: one cutout in the scene turns on the
           kernel's alpha-test loop (F_MASKED) for every ray. Alternated A/B at the west exit and aerial east, by day:
           12.10–12.15 against 11.07 ms and 13.05–13.06 against 11.65–11.66 ms. As geometry they cost nothing
           measurable: 11.07 against 11.02 ms and 11.61 against 11.64 ms.
       - Counts: 17.9 M triangles, 355 textures, 1.93 GB of GLB. Live: first frame 5.9 s (5.3 before: the atlases),
         physical footprint ~16 GB.
       - Shiba's GLBs carry them too: 4,416 boards and 2,225 symbols.
       - Live against the 4096 spp truth (GPU p50 / p95 ms):

         | pose | day: live ΔE / SSIM | GPU | truth's noise | night: live ΔE / SSIM | GPU | truth's noise |
         |---|---|---|---|---|---|---|
         | Kabukichō's gate | 0.86 / 0.976 | 11.9 / 12.7 | 0.30 / 0.992 | 1.70 / 0.878 | 10.8 / 11.5 | 1.21 / 0.884 |
         | west exit, Chūō-dōri | 0.91 / 0.950 | 11.3 / 12.2 | 0.37 / 0.973 | 2.02 / 0.868 | 10.5 / 12.0 | 1.14 / 0.900 |
         | skyscraper canyon, Tochō-dōri | 1.70 / 0.804 | 10.8 / 11.9 | 0.81 / 0.865 | 1.87 / 0.836 | 10.5 / 11.5 | 1.20 / 0.877 |
         | aerial east | 1.86 / 0.947 | 11.8 / 12.1 | 0.32 / 0.996 | 2.95 / 0.894 | 10.7 / 10.9 | 0.29 / 0.993 |
         | aerial west | 2.19 / 0.953 | 11.8 / 12.2 | 0.35 / 0.994 | 2.57 / 0.882 | 10.2 / 10.5 | 0.31 / 0.989 |

         - All ten poses stay within ΔE ≤ 3, as in part 2. By day the numbers are part 2's: the boards glow at 20
           nits and the symbols are paint. Four poses are within SSIM ≥ 0.9.
         - By night the boards are new small light sources, the more so round Kabukichō's gate. There the truth's own
           noise rose (ΔE 0.72 to 1.21, SSIM 0.948 to 0.884), and live reads ΔE 1.70 / SSIM 0.878 (1.27 / 0.940 in
           part 2): under 0.9, as its truth is.
       - The 60 s flight: frame start p95 16.67 ms by day and by night. GPU p50 / p95 is 12.4 / 13.1 ms by day and
         10.5 / 11.0 ms by night.
         - No frame over 50 ms in either run. The two CPU stalls of parts 1 and 2 did not come back, in these
           runs or in a run before the symbols became geometry; what they were is still open for step 2.
       - Step 1's items are all in: the area resident, its props, signs and paint, measured live against the truth and
         beside the source app. What step 1 leaves to later steps:
         - memory (~16–18 GB, the meshes as soups, the trees uninstanced) and the first frame: step 2;
         - the sun's glints off glass (the canyon by day reads 7 % dark live), the blotches on the night streets, and
           the night aerials' 3–4 % excess: step 3's quality work;
         - near trees as the client's ez-tree models, billboards and screens, the street furniture: with step 3's
           look.
  1a. **Time of day, completed** (on #121's dynamic sky, which the main line takes over from the supervisor):
     - the land beyond the area hazed into the sky by the atmosphere's aerial perspective, not a brown band at the
       horizon;
     - windows that switch on and off with the clock (by building use, from late afternoon into the night, staggered by
       building, as the source app's facade shader does), in place (power or material);
     - moonlight (a second directional light: the moon's position and phase on the date), and the night sky's glow set
       against photographs of Tokyo at night.
     - Acceptance:
       - the Shiba fly-past (the supervisor's clip: 540–760 km/h past Tokyo Tower, the clock 15:30 → 17:45) and a
         Shinjuku drive at 60 fps: frame start p95 ≤ 16.7 ms, no frame over 50 ms, no rebuild;
       - live against our offline render at 5 poses × 4 times (15:30, sunset, civil dusk, night): ΔE ≤ 3, SSIM ≥ 0.9
         where the truth's own noise allows (stated per pose);
       - in place against a fresh load of the same state (`tools/bench-sun.ts`, extended to Tokyo): ΔE ≤ 0.1;
       - a clip posted in the session.
     - Found by the acceptance runs (2026-11-11, branch `light-tree-switchable`): light sampling was set up at load for
       the lights on at that moment.
       - A room loaded by day (its lamps and windows off) left them out of the light tree. Switched on at dusk, they were
         drawn from the flat CDF beyond it, by power share alone, with no importance at the shading point and no
         distance cut: shadow rays across the city.
       - The sky's light-sampling floor (1 % of the emitters' mean radiance) also came from the lights on at load. The
         room kept light-sampling a night sky that a room loaded at night does not.
       - Both now count a switched-off emitter at full power (`switchable_power`); the tree's node powers are refit to
         the objects' powers now. The 12 catalog scenes and 6 game rooms with every emitter on at load render
         byte-identical; Tokyo's 15:30 room does not.
       - GPU p50 at the 0.5 floor, main → this branch, per second of path (alternated runs):
         - the Shiba fly-past: 14.6–17.9 → 8.9–12.5 ms by day, 20.0–21.7 → 12.2–12.8 ms from dusk;
         - the Yasukuni-dōri drive: 15.0–18.4 → 10.7–13.5 ms once the first lamps are on (16:09).
       - In place against a fresh load (Shinjuku's east aerial, on main + ThreeRT#3 and #4): ΔE 0.49 → 0.08 at sunset,
         0.15 → 0.01 at civil dusk, 0.34 → 0 at night (bit for bit).
     - Status, part 1 (2026-11-10, branch `aerial-perspective`): the land beyond the area hazed into the sky.
       - `SkyAtmosphere({ aerialPerspective })` in `packages/scene`: the atmosphere's own haze over the surfaces behind it, at
         a multiple of its density. It is off by default (0); the kernel's `F_AERIAL` is compiled out where no scene
         has it.
         - On every path segment that ends on a surface: transmittance through Rayleigh (RGB) and Mie with sky.rs's
           Earth scale heights (8 km, 1.2 km), in closed form.
         - The light scattered toward the viewer is the sky's own radiance a little above the horizon in that
           direction (2.3°), (1 − T) of it: a horizontal ray through the same air ends on that horizon, so far land
           fades into the sky behind it.
         - Camera rays see the sky near the horizon at that same elevation: the baked sky's fade to its black lower
           half would otherwise leave a dark seam between the sky and the hazed land.
         - Live: the camera ray's haze joins the emission signal. In the diffuse signal, which the denoiser divides by
           the albedo, dark glass with haze in it read 9–18 % bright. The cache's gather paths take a grey
           transmittance.
       - Tokyo's sky (`tokyoSky` in `games/tokyo`): Mie at 10× the clean-air default (a visual range near 70 km,
         Koschmieder), aerial perspective 1, in the static rooms and the time-of-day ones.
       - Check (`tests/render/sky-aerial-perspective.ts`): a black wall 5 km away reads (1 − T) of the sky above it,
         0.3805 measured against 0.38 analytic.
       - Shinjuku by day, live against our 4096 spp offline render, both with the haze (ΔE / SSIM / luminance; part 3 of
         step 1 without it):

         | pose | without the haze | with it |
         |---|---|---|
         | Kabukichō's gate | 0.86 / 0.976 / 0.986 | 1.04 / 0.976 / 0.968 |
         | west exit, Chūō-dōri | 0.91 / 0.950 / 0.955 | 1.00 / 0.953 / 0.922 |
         | skyscraper canyon, Tochō-dōri | 1.70 / 0.804 / 0.932 | 1.92 / 0.810 / 0.917 |
         | aerial east | 1.86 / 0.947 / 0.994 | 1.63 / 0.951 / 0.995 |
         | aerial west | 2.19 / 0.953 / 0.990 | 1.95 / 0.952 / 0.990 |

         - The aerial views come closer to the truth. At street level live reads 2–3 % darker than before.
           - The haze is not why: with the same Mie and the haze off, live reads the same against a 256 spp
             render (west exit 0.922 and 0.923, the canyon 0.941 and 0.941, off and on).
           - The hazier sky is: a dimmer sun and more of the light from the sky, which live underestimates in the
             streets' shade (step 3's quality work).
       - Cost, alternated A/B with the same Mie, live GPU p50: +0.11–0.15 ms at the west exit, within ±0.05 ms over the
         city from the air.
       - The 12 catalog scenes render byte-identical between main and this branch (offline, software traversal). The
         kernel's `Params` grow by 16 bytes at their end.
       - Before / after: `docs/img/tokyo-aerial-perspective.jpg` (no haze | with it, offline | with it, live).
     - Status, part 2 (2026-11-10, branch `windows-clock`): the windows come on with the clock.
       - `tools/tokyo/to-glb.ts` writes each building's lit windows to one of 6 GLB nodes (`win0`…`win5`), its switch
         group: the sun's elevation at which its rooms come on (`games/tokyo/area.ts`), within its use's span.
         - Offices come on from 12° to 0°, shops from 6° to −2°, homes from 4° to −8°, each building at its own moment
           within that span (a hash). Which rooms are lit, their colour and glow, are as before.
         - A lit room's window is the same glass as the panes around it (albedo 0.02, roughness 0.04, ior 1.5) with
           the room's light behind it. While its group is off by day, it reads as glass. It had been an opaque grey
           panel, which caught the sun as a cream patch amid the glass (Bugbot). Crops at 14:00, every group off:
           `docs/img/tokyo-windows-glass.jpg`.
       - The static rooms load every node. The regrouping alone changes nothing they render: Kabukichō's gate by night,
         at 1024 spp with the regrouped GLBs against the 4096 spp truth of the old ones, reads ΔE 1.30 / luminance 1.002,
         against its truth's own noise of 1.21. The glass changes the lit windows little by night: ΔE 0.15 / luminance
         1.001 at the gate (256 spp, before against after).
       - The time-of-day rooms (`tokyo-tod`) load each tile's groups as objects of their own and switch their power by
         the clock, as the street lamps, with no rebuild. The frame script touches only what switches: setting every
         lamp's and group's power every frame cost it 15–20 ms.
       - Shiba from 16:18 to 17:36 in 20 s, live at the 0.5 floor (r1300: 106 tiles): no rebuild, frame start p95 16.67
         ms and p99 17.1, no frame over 50 ms, GPU p50 / p95 13.7 / 13.9 ms. Crops at 16:30, 16:54 and 17:24:
         `docs/img/tokyo-windows-clock.jpg`.
       - A power change costs the live view's commit ~13 ms of CPU, whatever its size: `write_emitters` rebuilds the
         emitter CDF and refits the light tree over all 1.4 M emitters. A group therefore switches on every tile in one
         frame; making that update incremental is the next item.
       - Next for step 1a: the commit's cost, moonlight and the night sky.
     - Status, part 3 (2026-11-10, branch `moonlight`): moonlight and the night sky.
       - The moon (`games/tokyo/moon.ts`): a second directional light in the time-of-day rooms, where the moon stands
         over Tokyo at that moment.
         - Position by Schlyter's method: mean elements and the main perturbations, within a few arcminutes.
         - Its light by phase: Allen's magnitude against the sun's, through the same air as the sun's (Rayleigh, Mie at
           10×, ozone; Kasten and Young's air mass).
         - Checked at November 2026's phases (`tools/tokyo/test/moon.test.ts`, USNO times): new on the 9th, first
           quarter on the 17th, full on the 24th, 0.25 lx above the air.
         - The rooms take a date (`-d20261124`); 2026-11-09 is a new moon.
         - It is black while it is down, until civil dusk, and under starlight's 1 mlx.
         - The full moon high over Shinjuku at 23:00 changes the city by ΔE 0.05 at EV 5 (0.2 lx against its lit
           streets), as in photographs: the moonlight is lost in the city's light.
       - The kernel skips a black directional light (no BSDF, no shadow ray), after the cone's random numbers, so
         every other light sees the same ones.
         - The moon while down, and the sun at night, cost nothing. Shiba from 16:18 to 17:36 live, GPU p50 with a
           black moon: 14.30 / 14.33 ms without the skip, 13.58 / 13.61 with it (serverless-agent-test#130: 13.7).
         - The 12 catalog scenes render byte-identical.
       - The night sky: `SkyAtmosphere` `horizonGlow`, a light dome over the glow (nits at the horizon, falling to
         1/e at `horizonGlowHeight`). `tests/render/sky-night-glow.ts`: 1.349 measured, 1.349 analytic.
         - Tokyo's (`NIGHT_GLOW` in `games/tokyo`):
           - 27 mcd/m² at the zenith (16.5 mag/arcsec², Tokyo's clear-night zenith measured from Sumida, 2010–11);
           - a dome of 0.26 cd/m² more at the horizon, falling to 1/e by 3.5°;
           - a violet-blue zenith and a warm dome.
         - Set against photographs of the Shinjuku skyline at night (Wikimedia Commons, 2004 and 2005; not
           committed):
           - the sky at 1°, 7° and 15° reads 7.3 : 1.95 : 1, and the dome is shaped on that;
           - the colours are the 2004 photograph's, white-balanced on its office windows;
           - at the 2005 photograph's exposure (EV 3) our sky reads 4× brighter than its top of frame, whose blacks
             are crushed, and darker than the 2004 one (a longer exposure).
         - The static night rooms now have the same sky as the time-of-day rooms: the atmosphere with the sun 40°
           down, and the glow. Before, a uniform blue sky lit the city from below the horizon too.
         - The land beyond the area now fades into the dome by night (the aerial perspective's haze carries its light),
           not a black band under the sky. At EV 5 the sky stays near black, as in photographs at that exposure.
           Before and after, at EV 5 and as a long exposure: `docs/img/tokyo-night-sky.jpg`.
     - The commit's cost (ThreeRT#3, first serverless-agent-test#135): a power edit rebuilt the emitter CDF and refit
       the light tree in the frame, ~13–15 ms over Tokyo's 1.4 M emitters, ~90 times in the Shiba fly-past (the
       street lamps and the window groups switching).
       - Scenes with many emitters now rebuild both on a worker thread (`EmitterWorker`). The edits land with the
         result, a few frames later.
       - The fly-past's frame interval p95 went from 29.7 / 28.5 to 17.20 / 17.17 ms, and its commits over 5 ms from
         91 to 0.
     - Acceptance (2026-11-11, branch `tokyo-1a-acceptance`; `node tools/tokyo/accept-1a.ts`, clips by
       `node tools/tokyo/clip.ts tower|drive`), on main + ThreeRT#6 (light sampling set up for the lights switched off at
       load, found by these runs):
       - The Shiba fly-past (15:30 → 17:45 in 10.5 s, 540–760 km/h) and a Shinjuku drive (Yasukuni-dōri eastbound from
         `roads.json` at 60 km/h, 16:00 → 18:00 over 67 s), real time at the dynamic resolution:

         | run | frames | scale | frame start p95 / p99 | interval p95 / max | over 50 ms | rebuilds | GPU p50 / p95 |
         |---|---|---|---|---|---|---|---|
         | fly-past | 648 in 10.5 s | 0.70 | 16.67 / 23.73 | 17.5 / 27.1 | 0 | 0 | 14.24 / 16.55 |
         | drive | 4,086 in 67.1 s | 0.625 | 16.67 / 16.95 | 16.83 / 20.0 | 0 | 0 | 14.40 / 16.25 |

         Both pass. Before #6 neither did: frame start p95 19.92 and 17.21 ms at the 0.5 floor.
       - Live (the 0.5 floor, 64 frames after a reset) against our offline render (4096 spp, no denoiser, clamps off): ΔE
         / SSIM / luminance (the truth's own noise, its 1024 spp render against it, ΔE / SSIM; SSIM with both halved, at
         the resolution live renders):

         | pose | 15:30 | sunset (16:40) | civil dusk (17:07) | night (19:30) |
         |---|---|---|---|---|
         | Kabukichō's gate | 0.50 / 0.983 / 0.976 (0.19 / 0.996; 0.990) | 0.60 / 0.984 / 1.004 (0.23 / 0.998; 0.992) | 0.62 / 0.966 / 0.989 (0.29 / 0.980; 0.979) | 1.73 / 0.877 / 0.994 (1.20 / 0.884; 0.936) |
         | west exit | 0.98 / 0.942 / 0.945 (0.29 / 0.975; 0.968) | 0.91 / 0.965 / 1.011 (0.27 / 0.997; 0.980) | 0.75 / 0.954 / 0.989 (0.26 / 0.982; 0.970) | 2.01 / **0.869** / 0.993 (1.14 / 0.900; 0.916) |
         | skyscraper canyon | 1.09 / 0.960 / 0.961 (0.28 / 0.987; 0.982) | 0.88 / 0.965 / 1.011 (0.26 / 0.996; 0.985) | 0.60 / 0.958 / 0.973 (0.22 / 0.978; 0.975) | 1.88 / **0.835** / 0.994 (1.19 / 0.878; 0.905) |
         | aerial east | 1.56 / 0.933 / 0.993 (0.29 / 0.991; 0.976) | 1.83 / 0.916 / 0.963 (0.25 / 0.998; 0.964) | 1.75 / 0.931 / 0.898 (0.13 / 0.998; 0.981) | 2.89 / **0.896** / 1.037 (0.28 / 0.993; 0.962) |
         | aerial west | 2.26 / 0.938 / 0.985 (0.34 / 0.993; 0.976) | 2.07 / **0.880** / 0.975 (0.29 / 0.998; 0.939) | 1.17 / 0.930 / 0.879 (0.12 / 0.997; 0.977) | 2.52 / **0.884** / 1.030 (0.30 / 0.989; 0.951) |

         - ΔE ≤ 3 at all 20. SSIM ≥ 0.9, or within 0.02 of the truth's own, at 15 (bold: the misses).
         - The night's numbers are step 1's static night rooms' (gate 1.70 / 0.878, west exit 2.02 / 0.868, canyon
           1.87 / 0.836, aerials 2.95 / 0.894 and 2.57 / 0.882): the time-of-day room at 19:30 renders as they do. Their
           misses are step 1's, left to step 3.
         - Sunset's west aerial misses on the far city's detail at the 0.5 floor: halved, 0.939.
         - At civil dusk the aerials read 10–12 % dark (0.898, 0.879): the far lamps and windows the live view does not
           sample (`LT_FAR`, 120 m) and finds only by BSDF rays. Step 3.
       - In place against a fresh load (Shinjuku's east aerial; the clock run in lockstep to the time, held 200 frames,
         reset, 128 frames): ΔE 0.03 at 15:30, 0.08 at sunset, 0.01 at civil dusk, 0 at night (bit for bit). All pass.
         Before #6: 0.29 / 0.49 / 0.15 / 0.34.
       - Clips, posted in the session (animated WebP: the real-time run's frames, the lockstep snapshots of each):
         - the fly-past, 637 frames in 10.5 s at scale 0.70;
         - the drive, 4,078 frames in 67.1 s at scale 0.625.
  1b. **Weather on Tokyo.** States, and transitions between them, all in place (no rebuild):
     - rain: wet streets and puddles reflecting the city, and the rain curtain (Bistro's), measured on Tokyo;
     - fog and mist: height fog over the city, thickening and clearing;
     - overcast: cloud cover that dims and softens the sun and the sky;
       - Engine side done (2026-11-11, branch `sky-cloud-cover`): `scene.cloudCover` (0..1), changed in place.
         - The atmosphere's sky goes over to the CIE standard overcast sky, white. Its horizontal illuminance is 0.3 of
           what the clear sky and sun give the ground (stratus pass 0.1–0.4).
         - The sun's direct light is × (1 − cover), its mean share of time out of cloud.
         - A night glow is 3× brighter off the cloud base (Kyba et al. 2011).
         - The live view re-bakes the dynamic sky and relights the sun on a change, as for a sun move.
         - Unit test `full_cover_is_the_cie_overcast_sky_lit_by_the_clear_day`.
     - volumetric clouds in the sky, their shadows moving over the city (the sky's bake and the sun's visibility
       through them);
     - transitions in real time: clear → overcast → rain → clearing, with the clock running.
     - Acceptance:
       - each state and each transition with no rebuild; frame start p95 ≤ 16.7 ms on the Shiba fly-past, no frame over
         50 ms; each effect's cost stated;
       - live against our offline render ΔE ≤ 3 per state at 5 poses (rain: the wet-street poses);
       - in place against a fresh load ΔE ≤ 0.1;
       - Tokyo lines in the perf suite: the Shiba fly-past clear and in rain;
       - a clip (a clear afternoon into a storm at dusk) posted in the session.
     - Status, part 1 (2026-11-11, branch `gltf-material-functions`): the engine side of the rain on the city.
       - `GLTFModel.functions`: material functions on a glTF file's own materials, by name prefix (the compiled tiles'
         ground, roads and roofs are glTF materials with the aerial photos as maps). Set at build; the model has its
         own copies of the file's materials and meshes. Their triangles are flattened per placement, as every
         material-function triangle is: on Tokyo 15.2 M of 17.8 M.
       - `wetLayer()`: `SurfaceMaterial`'s wetness (Lagarde; puddles on up-facing ground, rain ripples) as a layer on
         another material's shading. `surface/wet-layer-gltf`: 0.556 against 0.549 analytic.
       - The translucency pass compiles only its particles' material functions: with every function of Tokyo's kernel
         in it, the Metal compiler failed on the `translucent` pipeline once the ground had wetLayer and the rain
         curtain was in the scene.
  Order: 1 (complete with its part 3, signs and road markings) → 1a → 1b → 2 → 3 → 4 → 5 → 6 → 7.
  2. **Streaming.** Tiles resident by distance:
     - geometry, textures and BLASes per tile, in a bounded pool;
     - the far field as proxies (merged, simplified tile meshes; roofs and facades as photo-textured blocks);
     - the TLAS and the light tree updated as tiles come and go, without whole-scene rebuilds (wgpu 30 rebuilds only:
       measure per-tile BLAS builds and the TLAS rebuild, then choose);
     - uploads spread over frames.
     - Acceptance, on a contiguous region of two areas (~20 km², Shinjuku through Shibuya):
       - resident memory ≤ 1.5 × one area's, whatever is loaded; first frame ≤ 3 s;
       - a drive at 60 km/h through the streets and 100 km/h on the expressway, and a flight at 150 km/h at roof
         height: frame start p95 ≤ 16.7 ms, p99 ≤ 25 ms, no frame over 50 ms;
       - streaming invisible: 5 poses against everything resident at full detail ΔE ≤ 1.0, SSIM ≥ 0.98 (§ 5 item 5's
         bar); popping in motion within the bench-ghosting limits.
  3. **The city's lights.** Lit windows that switch on and off, signs, LED screens, street lamps and headlights as
     emitters, 10^5–10^6 across the loaded tiles, in the light tree. Switching an emitter on or off costs no rebuild.
     If the voxel line's "dynamic emission" contract (§ 6 step 3) lands first, use it rather than duplicate it.
     - Acceptance: Shinjuku at night (Kabukichō, the station, the skyscrapers) live vs our offline render no worse
       than Bistro night's layer B (ΔE 3.3 / SSIM 0.82); the emitter count stated; step 2's frame budget held.
  4. **Traffic, trains and pedestrians.** The source's traffic (signals, one-way streets, expressway ramps) and its
     trains on the real network brought into `@threert/gameplay`; the walkers of `games/bistro-crowd` on the
     sidewalks; simulation and detail by distance. Deterministic (the frame clock), so lockstep runs repeat.
     - Acceptance: City Sample's densities (≈ 1,100 traffic vehicles and 2,200 pedestrians per km²) within the view
       range, or the largest that holds the budget, stated; frame start p95 ≤ 16.7 ms on step 2's drive; ghosting on
       moving objects within the bench-ghosting limits.
  5. **The whole city.** A contiguous stretch of central Tokyo of at least 30 km² (twice City Sample's 15.8 km²),
     covering the six areas, compiled from PLATEAU's 23-ward data. `tools/bench-city.ts`: a 5-minute drive and flight
     across it (frame start p95 / p99, frames over 50 ms, memory, streaming bandwidth, popping), added to the perf
     suite; a Tokyo pose added to layer B.
     - Acceptance: step 2's numbers across the whole city; memory still bounded.
  6. **Street level.** Hero detail for what the camera sees closest: storefronts, signage, vending machines, street
     furniture, vegetation (library or generated assets; Meshy is the natural source), and material upgrades where the
     generated facades read flat.
     - Acceptance: street-level poses live vs our offline render no worse than step 1; crops beside photos of the same
       places (qualitative, in the PR); the budget held.
  7. **Milestone 2, the open city.** Shinjuku and Shibuya at night, and by day:
     - a montage per view: our offline render | ThreeRT live | the source three.js app, each tile with its frame rate
       and ΔE / SSIM;
     - a side-by-side clip of a fast flight and drive (ThreeRT live | three.js), fps on each tile, posted in the
       session;
     - then the UE audit with the PLATEAU SDK, with the user present.
- Not in this phase: the voxel engine (§ 6, its own line). The day / night cycle and the weather are developed in this
  phase, on Tokyo (steps 1a and 1b, the user's, 2026-11-10): there is no separate scene for them.
- Brought forward (the user's, 2026-11-10; branch `time-of-day`, the supervisor's): **time of day in the live view**,
  for a clip that flies past Tokyo Tower at 540–760 km/h while the clock runs from 15:30 to 17:45.
  - Before it, every move of the sun rebuilt the scene (the sky is baked per sun direction): on games/sun-sweep (the
    sun room, its sun swept from 40 deg to −8 deg in 8 s) main read 166 frames in 9 s, 111 rebuilds (≤ 82 ms each),
    frame interval p50 71 ms; on a Tokyo area a rebuild takes 0.5–0.7 s.
  - Now (`SkyAtmosphere({ sun, dynamic: true })`): the sun's light is rewritten in place every move (its direction,
    and its colour through the atmosphere, `sky::sun_illuminance`), the sky is re-baked at 256 × 128 on a worker
    thread with three cores (~10–40 ms behind the sun; lockstep waits for it), a manual exposure that changes value is
    applied in place, and lamps switch through object power (in place already). `glow` adds the night sky's light
    pollution above the horizon.
    - games/sun-sweep (`node tools/bench-sun.ts`): 553 frames in 9 s, no rebuild, frame interval p50 / p95 / p99
      16.7 / 17.6 / 18.2 ms, none over 50 ms; GPU p50 / p95 15.3 / 17.6 ms.
    - In place against a fresh load of the same state at 2, 5 and 7.5 s (the lamp switched on at 6.3 s), after the
      lighting-change transients and a history reset, drawing the same random numbers: ΔE 0.02 / 0.01 / 0.06, the
      same image. Right after a change the temporal pass's anti-lag reads 1–4 % bright for ~100 frames: the live
      pipeline's response to lighting changes, as before.
    - Crops along the clip (afternoon, the pass, sunset, dusk, night): `docs/img/time-of-day-tokyo-tower.jpg`.
    - Tokyo, Shiba within 1.3 km of Tokyo Tower (106 tiles, 17.4 M triangles), the clock 15:30 → 17:45 over 7 s with
      lamps switching tile by tile around sunset (`games/tokyo/tokyo-tod.game.ts`, the solar position of 2026-11-09):
      after the first second (dynamic resolution settling) 61 fps, frame interval p95 / p99 17.7 / 18.6 ms, max 21.4,
      GPU p50 / p95 15.6 / 16.9 ms at scale 0.62, no rebuild. The fly-past clip in real time: 60.7 fps, interval p95
      18.2 ms, GPU p50 14.4 ms.
  - Shiba is compiled (180 tiles, 23 M triangles, Tokyo Tower built member by member); to-glb lights the lattice
    tower's bulbs and beacons in the "night" node, as street lamps are, and paints its steel (it was metal).
  - Still for the day / night phase: weather, volumetric clouds, the haze over the land beyond the area (a brown band
    at the horizon today), moonlight, windows that switch on at dusk.
- Carried over from the dynamic phase (taken up when they block a step, or between steps):
  - live quality in motion: the crowd's ΔE at 10 s is 10.4 against static Bistro's 5.8, and the vans' roofs still
    sparkle;
  - the unbiased replacement for the foliage-shadow bias (an opacity micromap, or emitter shadow rays compacted into a
    pass of their own);
  - the perf suite's first-frame recheck (alternate the two builds before reporting a first-frame regression).

### History: the dynamic content phase (2026-11-08 → 2026-11-09)

Closed on 2026-11-09 by the user's call (the goal's coverage is enough to move on):
- the frame budget met: w150-v60 frame start p95 16.67 ms along Bistro's path, with the accepted foliage-shadow bias;
- live quality in motion short of its acceptance: the crowd's ΔE at 10 s 12.7 → 10.4 over the phase against static
  Bistro's 5.8; luminance 0.67 → 0.89 of the truth; trails 297 → 82 px (bench-crowd `--quality`, b20d791 → 20533d3);
- ghosting within the bench-ghosting limits; the scaling curve recorded.

What is left is carried over in § 0. The phase's plan and results, as they were measured:

- What is proven and what is not:
  - Proven: static Bistro night (2.8 M triangles, 20 k emitters) at 60 fps along its camera path at the 0.5 floor.
  - Also proven, at small scale: the C slice, with four walkers, one van, rain, an explosion and smoke, at 59–60 fps.
  - Not proven: dynamic content at the scale of the goal, with crowds, traffic and moving lights at once. The history
    below holds the static phase.
- Continuing quality targets, static scenes:
  - Live against our offline render: SSIM ~0.81 -> 0.90 (Bistro 3.62 / 0.812, Zero Day 3.47 / 0.805, 64 frames after a
    reset at the 0.5 floor; layer B).
  - The mid-tones: lit surfaces read 7–13 % darker than the reference (§ 2d, § 0 history's live energy).
- Steps, one PR per item; the supervisor accepts and merges.
  1. Stress scene first, measured before anything is optimised. Bistro night plus:
     - 100–200 walking skinned characters (Quaternius' mannequin with its existing LOD);
     - 50–100 vehicles driving with their lights on (rigid instances; head and tail lights as moving emitters);
     - some debris and moving props.

     Requirements:
     - Deterministic: driven by the frame clock, so lockstep runs repeat.
     - The camera follows Bistro's FBX path.
     - Measured:
       - frame interval p50 / p95;
       - GPU time split: BLAS rebuild, TLAS, trace, filters;
       - CPU time split: skinning, frame script, uploads;
       - live against our offline render, by segment;
       - ghosting on moving objects.
     - Added to the perf suite.
     - Also measured: the scaling curve, i.e. the cost of each additional walker and each additional vehicle.
     - Done (2026-11-08, branch `bistro-crowd`): `games/bistro-crowd` and `node tools/bench-crowd.ts [--curve]
       [--quality]`.
       - The scene. Walkers on the 240 straight walks and vans on the lanes of `games/bistro-crowd/paths.ts` (one each
         way, broken where the street narrows; they hold about 60 vans nose to tail, so the curve's 120 overlap),
         generated by `tools/bistro-crowd-paths.ts` from Bistro's walkable ground (`renderer/examples/walkable.rs`:
         1 m cells, clear from 0.15 to 2.2 m above the ground). Bistro's take swings its lanterns; debris is thrown
         every 8 s. Counts come from the room id (`w150-v60` by default).
       - The perf suite gains "bistro crowd w150-v60" drive lines. The frame log gains `js`, `skin`, `upload`, `wait`
         and, with THREERT_PROFILE_BLAS=1, `blas` (the dynamic BLAS / TLAS rebuild timed alone: Metal writes no
         timestamps between acceleration-structure builds).
       - Measured, w150-v60, along the path's first 50 s at the 0.5 floor:
         - Frame interval p50 / p95 35.9 / 36.5 ms (28 fps). GPU passes p50 / p95 16.2 / 17.9 ms (trace 9.2), plus
           the dynamic BLAS / TLAS rebuild, 10.1 ms.
         - The frame is CPU-bound: the main thread never waits for the GPU (`wait` 0).
         - CPU split of the 36 ms (frame log, confirmed by `sample`):
           - skinning and re-transforming the dynamic triangles (`dynamic_tris`, `tri_attr`): 18.1 ms;
           - uploading them (`write_buffer`, ~100 MB a frame): 8.7 ms;
           - collecting the moved subtrees (`World::subtree` scans every object for each moved node): ~4.8 ms;
           - the frame script's JavaScript: 4.2 ms.
         - Per-pass profiling (THREERT_PROFILE) slows this scene's frame by ~20 ms (59.8 ms), so the drive runs
           without it and a separate 10 s run gives the pass split.
       - Scaling curve (the path's first 20 s; frame interval p50):

         | walkers / vans | 0 / 0 | 50 / 0 | 100 / 0 | 200 / 0 | 0 / 30 | 0 / 60 | 0 / 120 |
         |---|---|---|---|---|---|---|---|
         | interval (ms) | 16.7 | 19.2 | 22.7 | 36.3 | 16.9 | 17.3 | 23.2 |

         - A walker costs +0.18 ms of CPU and +0.10 ms of frame interval; its GPU passes stay flat (+0.002 ms).
         - A van costs +0.18 ms of CPU and +0.06 ms of frame interval. Its CPU is mostly the frame script and the
           change tables, not its triangles.
       - Live against our offline render (lockstep along the path, 2048 spp truths), ΔE / SSIM / luminance. The
         truths need the offline render to run the frame script at its time (main's 0d01f1c; without it Bistro's
         take is caught at another moment):

         | time | w150-v60 | static Bistro, same path |
         |---|---|---|
         | 10 s | 15.3 / 0.38 / 0.51 | 8.0 / 0.63 / 0.69 |
         | 25 s | 8.2 / 0.60 / 0.66 | 5.6 / 0.75 / 0.77 |
         | 40 s | 11.4 / 0.49 / 0.57 | 7.1 / 0.69 / 0.72 |

         - The van headlights (2500 nits) light the next vans' roofs and the shop fronts far more in the truth than
           live: moving emitters sit outside the light tree and are proposed by their share of power. Trails behind
           the vans read 410 px at 10 s.
         - Static Bistro in motion is itself far from its still pose (ΔE 3.4 / luminance 0.92): short histories.
       - Order for step 2, by the measurements:
         1. the CPU per moving object: skinning on the GPU, the moved-subtree scan, uploads;
         2. the 10 ms BLAS rebuild;
         3. moving emitters in light sampling;
         4. quality in motion.
       - Step 2, item 1, the CPU per moving object (2026-11-09, branch `crowd-cpu`):
         - Moved subtrees through a parent → children index (`World::children`, `subtree_with`): O(subtree), where
           `subtree` scanned every object for each moved node.
         - Skinning and re-transforming in parallel, one task per object, straight into its own slices of the dynamic
           tail and of the CPU triangles. Objects whose triangles fall in more than one traversal group (opaque and
           cutout materials) are done one by one, as before.
         - Measured on main + this item, w150-v60 along the path's first 50 s at the 0.5 floor: frame interval p50 /
           p95 35.8 / 36.6 → 26.4 / 27.9 ms (38 fps). Skinning 18.3 → 6.0 ms; uploads unchanged (8.1 ms); the
           JavaScript 4.0 ms.
         - The frame is now GPU-bound: the main thread waits 8.1 ms for it. GPU passes p50 16.0 ms, plus the dynamic
           BLAS / TLAS rebuild, 10.4 ms.
         - Scaling curve (the path's first 20 s; frame interval p50, ms):

           | walkers / vans | 0 / 0 | 50 / 0 | 100 / 0 | 200 / 0 | 0 / 30 | 0 / 60 | 0 / 120 |
           |---|---|---|---|---|---|---|---|
           | before | 16.7 | 19.2 | 22.7 | 36.3 | 16.9 | 17.3 | 23.2 |
           | after | 16.7 | 19.3 | 22.7 | 30.7 | 16.9 | 17.2 | 17.8 |

           - CPU per walker 0.18 → 0.087 ms, per van 0.18 → 0.061 ms.
           - At 50 and 100 walkers the frame was already GPU-bound: the passes plus the BLAS rebuild, which grows by
             ~0.07 ms a walker (3.5 k triangles). That rebuild is item 2.
       - Step 2, item 2, the BLAS rebuild (2026-11-09, branch `crowd-pose`): walkers in one pose share one skinned
         mesh.
         - The game gives each walker one of 8 walk phases, as crowd systems share animation (`-p<n>` in the room id;
           p0: every walker its own phase, skinned alone as before).
         - Posed skinned models in the same pose (model, nodes, clip, time) share one model-space mesh
           (`scene::PoseGroup`): skinned once per new pose (the groups in parallel) and its BLAS rebuilt, built fast
           rather than traced fast as the dynamic groups are (8 poses 1.04 → 0.88 ms, the passes unchanged); each
           model is an instance of it, moved by its transform. Models whose poses part are regrouped by a rebuild.
           Meshes with glass, emissive or function triangles are not shared.
         - Motion vectors on them: the instance's previous transform applied to the mesh's previous pose. The
           deforming meshes come first in the instanced pool, so the previous-frame vertices (`prev_verts`) cover
           them, as they cover the dynamic triangles. Motion blur now also follows moved instances (it followed
           re-posed triangles only).
         - Measured on main + items 1-2, w150-v60 along the path's first 50 s at the 0.5 floor: frame interval
           p50 / p95 26.4 / 27.9 → 17.1 / 18.7 ms (58 fps at p50). The dynamic BLAS / TLAS rebuild 10.4 → 1.8 ms. CPU
           18.2 → 8.0 ms: skinning 6.0 → 1.9, uploads 8.1 → 1.5, the JavaScript 4.1. GPU passes p50 / p95 15.8 /
           17.4 ms.
         - Scaling curve after (frame interval p50, ms): 16.7 / 16.9 / 16.9 / 17.1 (0 / 50 / 100 / 200 walkers),
           17.0 / 17.2 / 17.8 (30 / 60 / 120 vans). A walker now costs +0.002 ms of frame interval and +0.012 ms of
           CPU; a van +0.009 ms and +0.049 ms.
         - What is left of the rebuild (10 s runs, timed alone): ~0.06 ms a pose mesh plus ~0.4 ms of fixed cost;
           the vans' dynamic triangles ~1 ms. The vans also add 0.7 ms to the passes (14.6 → 15.3 ms with 60 vans);
           walkers add nothing there.
         - Tried and dropped: van parts as instances (movable meshes moving a TLAS transform). The rebuild fell only
           2.03 → 1.89 ms and the CPU rose 8.0 → 9.1 ms (one instance write per part), the frame interval unchanged.
         - Quality unchanged (measured before the fast build): live against our offline render (lockstep, 2048 spp
           truths), ΔE at 10 / 25 / 40 s 15.2 / 8.2 / 11.5 (was 15.3 / 8.2 / 11.4), trails 405 / 70 / 131 px (was
           411 / 70 / 133); static Bistro 8.0 / 5.6 / 7.1 as before.
       - Step 2, item 3, moving emitters in light sampling (2026-11-09, branch `crowd-lights`):
         - Moving emitters (dynamic triangles: the vans' lights, the lanterns Bistro's take swings) were outside the
           light tree, picked anywhere by their share of power. They now have a subtree of their own, the root's right
           child (the static emitters' subtree, unchanged, its left). After a commit moves them, their subtree is
           refit around their triangles (`LightTree::refit_moving`: leaf spheres, then unions up to the root), keeping
           its topology, so the codes their triangles carry stay (re-skins keep them). Scenes without moving emitters
           build the same tree as before. Particles still go by their share of power.
         - Measured on main + items 1-3, against our offline render (lockstep, 2048 spp truths), ΔE / luminance:

           | time | w150-v60 before | after | static Bistro (its take) before | after |
           |---|---|---|---|---|
           | 10 s | 15.2 / 0.51 | 15.3 / 0.48 | 8.0 / 0.69 | 8.0 / 0.69 |
           | 25 s | 8.2 / 0.66 | 7.1 / 0.73 | 5.6 / 0.77 | 5.0 / 0.82 |
           | 40 s | 11.5 / 0.57 | 9.5 / 0.64 | 7.1 / 0.72 | 6.0 / 0.76 |

           At 10 s the vans queue just in front of the camera: their headlights light the bakery front in the truth,
           and live it stays dim and blotchy, with sparkles on the van roofs. That is the live filters with many
           close, bright, moving lights (item 4).
         - Cost: the frame interval unchanged (17.2 / 18.9 ms); the vans' share of the trace 0.83 → 0.15 ms (10 s,
           60 vans; 0.47 ms of it was their lights proposed from anywhere); the take's lanterns, sampled through the
           tree as static Bistro's are, +0.4 ms of GPU on static Bistro with its take; the refit +0.5 ms of CPU.
         - Unbiased check (hardware offline against the software BVH) passes, Zero Day's moving parts now in its
           tree: mean 1.0006, worst region 1.004.
       - Where step 2 stands after items 1-3 (2026-11-09; first 20 s of the path unless stated, frame interval p50):
         - The crowd costs ~1.1 ms over its static baseline, and that baseline is itself over budget: static Bistro
           with its take 16.7 / 17.6 ms over 50 s (p50 / p95), w150-v60 17.2 / 18.9. Per 5 s of the path the crowd's
           GPU passes are 0.5-0.9 ms above static's; the square (15-30 s) is the worst for both (static passes
           16.6 ms there).
         - Real costs: 150 walkers in one shared pose +0.32 ms (the TLAS, instance writes), each further pose mesh
           ~0.06 ms (8 poses +0.73); 60 vans +0.48 (0.25 of it in the passes); motion vectors 0.1. Bistro's take
           (50 s): static 16.69 / 17.60 → 16.65 / 17.31 ms frozen, the crowd unchanged; its cost is CPU (3.7 ms),
           hidden behind the GPU.
         - So the budget is the renderer's: the crowd's frame interval p95 ≈ GPU p95 + ~1.3 ms, and static Bistro's
           passes alone reach ~16.8 ms at p95 along the path. p95 ≤ 16.7 needs ~2 ms (~12 %) off every frame's GPU.
         - Tried, no gain: one build and one submit for the pose meshes, the dynamic BLASes and the TLAS (A/B within
           noise); wgpu 30 has no BLAS or TLAS refit (`PreferUpdate` logs "only rebuild is implemented"). Specular
           filters off saves 0.55 ms but breaks layer B (Bistro 3.62 / 0.812 → 4.67 / 0.718); caching their taps in a
           local array doubled their cost (Metal spills it).
         - Quality at the crowd's 10 s pose (vans queued before the camera): the error is the vans' glossy roofs
           (luminance 0.27 of the truth, which shows a smooth sheen of the string lights; live, sparse sparkles) and
           the shop front (0.48). Not the sample clamp (raising it changes nothing); 8 / 16 light candidates instead of
           4: ΔE 15.3 → 14.5 / 14.0; the reservoir experiment (THREERT_RESTIR=1) is worse (17.5).
         - Held still there (time frozen, 64 frames) the frame recovers half of it: ΔE 15.3 → 12.1, luminance 0.48 →
           0.62 (roofs 0.26 → 0.58). Half the loss is motion (specular histories held to 4 frames while the view
           translates), half survives convergence. The specular filters are not it (off: held 0.62 → 0.64).
         - Item 3 costs this pose a little: held, the pavement under the headlights reads 0.34 of the truth (item 2,
           moving emitters by their share of power: 0.41; no light tree at all: 0.59). Not the root's split between
           the two subtrees: a 25 % floor on either side of it changed nothing. Open (item 4); no light tree also means
           one candidate and no far-node culling, so that comparison is not one change.
       - The GPU time's outliers (2026-11-11, branch `crowd-timer`): 3–4 % of the crowd's frames read 70–80 ms of GPU
         while frames came every 16.7 ms.
         - The live view times a frame from its first pass to its post pass's end. A slot of the timer read, in those
           frames, the post pair of the frame that used the slot next, 4 frames on: the same timestamps, bit for bit.
         - A post pass that begins more than 2 ms after the trace ends (it follows within ~0.2 ms) is now counted by its
           own length. Every other frame reads as before.
         - w150-v60, 30 s of the path, 3 alternated pairs: frames over 2 × their interval 3.4–4.2 % → 0–0.1 %, GPU max
           81–82 → 20.4–20.6 ms, p95 15.3–15.4 → 14.95–15.0 ms. Frame interval p50 / p95 16.65 / 16.9 ms, frame start
           p95 16.67 ms and scale (the 0.5 floor) unchanged. GPU p50 reads 0.13–0.15 ms higher in every pair (Bistro's
           perf smoke: +0.14), though the change can only lower a reading; not explained.
  2. Then the costs, in measured order. The method is chosen per measurement. Candidates:
     - GPU (compute) skinning;
     - the dynamic BLAS update strategy:
       - a refit (wgpu 30 has none: a workaround, or through Metal);
       - per-character BLASes against merged ones;
       - an update rate scheduled by distance;
     - character LOD by distance;
     - moving emitters in the light tree;
     - how fast the radiance cache follows moving occluders and lights;
     - motion vectors and the temporal pass for deforming meshes;
     - the frame script's CPU scaling with hundreds of agents;
     - § 1 step 2's rasterized G-buffer for deforming content, only if the measurements show the ray-traced route
       cannot fit the budget.
     - Item 4, quality in motion, first finding (2026-11-09, branch `firefly`): the diffuse history's firefly cut
       (`dtemporal`) clipped the rare bright samples of small strong lights for good. It is tied to the history's
       frame count, which is capped (48 frames while the view moves, 24 in scenes with sparks), so it never let them
       through. Now it acts on young histories only (4-15 frames: after a disocclusion or a lighting change).
       - The reflection resolve's lost share (2026-11-09, branch `resolve-share`): the trace drops each sample's
         specular share for the resolve to estimate from the specular-lobe rays around it. A sample that took the
         diffuse lobe has no such ray; when none of its neighbours had one either (rough surfaces: likely), the resolve
         returned nothing and the share was lost. It now returns that sample's own share then. Static Bistro's
         specular 0.84 → 0.89 of our offline render (0.91 with no resolve at all); layer B Bistro 3.55 / 0.810 → 3.50
         / 0.811, Zero Day 3.48 → 3.44; bench-crowd ΔE 13.1 / 6.9 / 8.4 → 13.0 / 6.8 / 8.3, static 6.9 / 4.9 / 5.6 →
         6.8 / 4.8 / 5.5; ghosting better on every check.
       - Found by splitting live and offline into diffuse / specular / emission and comparing them linearly
         (`render --signals`, live `.signals` snapshots, `tools/signals-compare.ts`). Bistro crowd at 10 s, time and
         camera held 256 frames, live diffuse against our offline render: 0.70 → 0.82 overall, the shop front 0.63 →
         0.84, the pavement under the vans' headlights 0.49 → 0.70.
       - In motion (bench-crowd --quality, ΔE at 10 / 25 / 40 s): the crowd 15.3 / 7.0 / 9.5 → 13.1 / 6.8 / 8.4,
         static Bistro with its take 8.0 / 5.0 / 6.0 → 6.9 / 4.9 / 5.5 (luminance at 10 s 0.69 → 0.80). Layer B
         Bistro 3.62 / 0.812 → 3.56 / 0.813, Zero Day 3.48 → 3.46. Without any cut: about the same (crowd 12.9 /
         6.8 / 8.1); the young-history cut keeps restarts from sparkling.
       - The radiance cache's training clamp (2026-11-09, branch `train-clamp`): training samples were clamped at the
         indirect clamp, so the voxels near bright small lights stored their light clamped and every pixel reading them
         inherited the loss. Now at 4 x (`CACHE_TRAIN_CLAMP`, the headroom the fixed-point sums already have). The
         Bistro crowd's held frame: diffuse 0.81 → 0.86 of our offline render (the shop front 0.83 → 0.91), specular
         0.74 → 0.78. bench-crowd ΔE 13.0 / 6.8 / 8.3 → 12.7 / 6.8 / 8.2, static 6.8 / 4.8 / 5.5 → 6.6 / 4.8 / 5.4
         (luminance at 10 s 0.81 → 0.85); layer B Bistro 3.50 → 3.47, Zero Day 3.44 → 3.42; ghosting unchanged.
       - Light sampling at camera hits picks by the BSDF (2026-11-09, branch `bsdf-target`). The camera hits' light-tree
         candidates were resampled by emitted luminance × geometry, with a plain cosine on the receiving side. A glossy
         surface's highlight from one of many small lights (the string lights on the crowd's moving van roofs) was
         found only when that light happened to be drawn: sparse sparkles, which the denoisers spread thin or kept out.
         The receiving side is now the surface's BSDF × cosine toward the candidate (`eval_bsdf`), so the lights in the
         specular lobe win. Any target keeps RIS unbiased: tools/unbiased-check.ts passes, means within 0.1 %, regions
         within 0.4 %.
         - Measured on main + `mirror-history`, `sample-ceiling`, `frame-start`. The Bistro crowd in motion (lockstep
           at 10 s, live / our offline render): specular 0.88 → 0.91, the vans' roofs 0.90 → 0.94, the facades 0.95 →
           0.96. bench-crowd ΔE at 10 s 10.95 → 10.61, static 5.94 → 5.82 (25 / 40 s unchanged). Layer B Bistro 3.47 /
           0.812 → 3.34 / 0.819, Zero Day unchanged (3.42 / 0.806). bench-ghosting better on every check: turn-mid 0.75
           → 0.66, strafe-mid 2.10 → 1.74, turn-return 1.01 → 0.88, strafe-return 1.29 → 1.11, stop + 0.1 s 1.82 →
           1.54.
         - Cost: perf smoke drive GPU p50 / p95 +0.04 / +0.07 ms, the still and moving poses −0.01 / +0.05.
         - 8 candidates instead of 4 (branch `ris8`, `RIS_LIVE`): with the BSDF in the target, a glossy surface chooses
           among more of the small lights in its lobe. bench-crowd ΔE at 10 / 25 / 40 s 10.61 / 6.09 / 7.12 → 10.31 /
           6.00 / 7.02, static 5.82 / 4.23 / 4.81 → 5.71 / 4.13 / 4.73; layer B Bistro 3.34 / 0.819 → 3.23 / 0.825,
           Zero Day 3.42 → 3.43; bench-ghosting better on every check (turn-mid 0.66 → 0.56, strafe-mid 1.74 → 1.53,
           stop + 0.1 s 1.54 → 1.35). No measurable cost: perf smoke drive GPU p50 / p95 −0.02 / −0.28 ms (noise), the
           crowd's trace 8.59 → 8.53 ms.
       - The other two cuts cost more and stay: without the indirect clamp (`clampIndirect`, depth > 0) and the per-sample
         ceiling, the held crowd frame reads diffuse 0.92 / specular 0.75 of the truth (0.82 / 0.67 with them), but in
         motion the crowd's ΔE rises 13.1 → 15.5 and layer B (64 frames) goes Bistro 3.56 → 4.5, Zero Day 3.46 → 12.2.
         What is left of the bias is sampling variance under many small strong lights: next, light sampling that finds
         them more often, not cutting less.
       - The mirror-image specular history, not on moving objects (2026-11-09, branch `mirror-history`): the temporal
         pass also reprojects every reflection through its mirror image and keeps whichever history is closer to the
         1-spp neighbourhood mean. The camera-only reprojection ignores a moving reflector's motion: on the crowd's vans
         it landed on other surfaces (a level street passes the normal test) and, picked against a mostly dark mean,
         replaced the roofs' reflections with darker ones. Moving objects no longer take it.
         - Measured on main (`spec-clamp`, `frame-start`). The Bistro crowd in motion (lockstep at 10 s, live / our
           offline render): specular 0.55 → 0.83, the vans' roofs 0.46 → 0.85 (their total 0.54 → 0.82).
         - bench-crowd at 10 / 25 / 40 s: the crowd's ΔE 11.23 / 6.01 / 7.16 → 10.86 / 6.04 / 7.19, luminance 0.77 /
           0.92 / 0.81 → 0.86 / 0.94 / 0.83, trails at 10 s 125 → 77 px; static unchanged (5.96 / 4.24 / 4.86).
           Layer B unchanged (3.47 / 0.812, 3.42 / 0.806); bench-ghosting unchanged on Bistro (strafe-mid 2.19, stop
           + 0.1 s 1.86) and on Kabukichō (strafe-mid 3.30, 62 px; stop + 0.1 s 2.59).
         - Restricted to near mirrors as well (roughness < 0.2, the first version of this branch), Bistro's glossy
           surfaces read a little brighter (static luminance at 25 s +1 %), but Kabukichō's wet street lost the
           history that carries it through strafes: strafe-mid 3.3 → 4.3, trail 62 → 94 px, failing the check. Every
           static surface keeps it.
     - The frame budget, first finding (2026-11-09, branch `cold-fallback`): the radiance cache's cold fallback.
       Pixels whose voxel was cold traced a fresh estimate each frame and inserted the voxel. In steady motion the cold
       voxels left are mostly where bounce rays land off screen: scattered, sparsely trained. Their traces cost each
       SIMD group nearly a whole extra trace, and the low-sample entries they inserted kept later frames slower until
       evicted. Now cold pixels trace only for 30 frames after a reset or a camera cut (`COLD_FALLBACK_FRAMES`), and
       without inserting; later they read the voxel as it is.
       - Found after a camera cut (the supervisor's clip: ~30 frames at ~35 ms after a jump): the cost decayed only as
         fast as the cache evicted, and stayed ~2 ms in the square.
       - Bistro crowd w150-v60 along the path's first 50 s at the 0.5 floor: frame interval p50 / p95 17.16 / 18.91 →
         16.69 / 17.25 ms, GPU 15.85 / 17.65 → 15.09 / 15.97 ms; every room of the scaling curve at the 60 fps cap.
         Static Bistro's drive (perf smoke) GPU p50 / p95 15.33 / 16.04 → 14.42 / 14.92 ms.
       - Quality: bench-crowd ΔE at 10 / 25 / 40 s 13.1 / 6.8 / 8.4 → 13.3 / 6.9 / 8.5 (static 6.9 / 4.9 / 5.5 →
         6.9 / 5.0 / 5.6); layer B unchanged (3.57 / 0.813, 3.46 / 0.805); ghosting unchanged. Right after a cut (frames
         10 / 30 / 60) the same ΔE as before (9.1 / 7.2 / 6.3).
       - The pacer floors the frame interval's p95: static Bistro reads 16.91 ms with its GPU p95 at 14.9 (wall-clock
         jitter around 16.67). The crowd now meets the budget on the GPU (p95 15.97 ms); its interval p95 is ~0.35 ms
         above static's floor.
     - The frame budget read on frame starts (2026-11-09, branch `frame-start`). The frame log's `interval` runs from
       the end of one frame's work to the next's, so it carries the variation of the work itself: static Bistro's
       drive read p95 16.9 ms with its GPU p95 at 14.9 and every frame starting on its slot. The log now also has
       `start` (frame start to frame start: what the pacer sets, the frame rate a player sees); perf smoke reads the
       budget on it and bench-crowd prints it.
       - Static Bistro's drive (perf smoke): frame start p95 16.67 ms, within the budget (interval p95 16.92).
       - The crowd w150-v60 along the path's first 50 s: frame start p95 17.44 ms (interval 17.46), GPU-bound. The
         timed passes read GPU p50 / p95 15.09 / 16.27 ms; the dynamic BLAS / TLAS rebuild adds ~2.0 ms outside them,
         and the CPU waits ~8 ms a frame for the frame in flight. About 0.8 ms of GPU at p95 is missing.
       - Plan, in order:
         1. The trace pass: 8.5 of the crowd's 15 ms. Foliage traversal in the square (static Bistro's trace 8.7 →
            10.4 ms there).
         2. The specular filters: 1.3 ms at output resolution, three passes that mostly copy. Not by moving them to the
            traced grid: the `spec-internal` prototype (−0.62 ms) lost the sharp reflections and the wet street's
            ripples for structural reasons (the traced grid's history averages each jittered sample over its 2 × 2
            output footprint, and two stacked histories add their lag), not only through the clamp `spec-clamp`
            removed. Cheaper passes at output resolution instead.
         3. Pose groups re-skinned at half rate, alternating (animation LOD): measured, small. The crowd's 20 s drive
            frame start p95 17.75 → 17.60 ms, frames over 17.2 ms 232 → 185 of 1140. Kept as a fallback.
     - The crowd within its budget (2026-11-09, branch `emitter-shadow-cutouts`), by a bias the user accepted: live
       emitter shadow rays at camera hits take alpha-cutout triangles as opaque (`occluded_emitter`,
       `TRACE_ANY_OPAQUE`), as deeper vertices already did. The leaves of a tree block them as whole cards.
       - Found by attribution along static Bistro's path:
         - every live cutout opaque: trace −2.1 to −2.5 ms;
         - only camera-hit shadow rays: −1.5 to −2.1;
         - only the emitter ones: ~−0.6 (on the crowd −1.2 ms of GPU p50).
       - What these rays skip is mostly the first hit's alpha test on a leaf; the candidate loop past it is ~0.3 ms.
       - Unbiased tries that did not pay:
         - a roulette past cutouts, per pixel (q = 0.5) or per SIMD group (`subgroupBroadcastFirst`): no saving;
         - ordered opaque steps instead of the candidate loop: +1.5 ms at the worst stretch;
         - the alpha test without material functions: no change;
         - skipping the first hit's separate alpha test: −0.15 ms.
       - Measured on main + `mirror-history`, `sample-ceiling`, `bsdf-target`, `ris8`:
         - The crowd w150-v60 along the path's first 50 s: frame start p95 17.39 / 17.19 → 16.67 / 16.67 ms (two
           alternated runs each), GPU p50 15.1 → 13.9 ms.
         - Static Bistro's drive (perf smoke): GPU p50 14.34 → 13.51 ms.
       - Cost: the live image under foliage loses the light through the leaves' gaps.
         - Static Bistro at 25 s: luminance 0.993 → 0.976 of our offline render.
         - bench-crowd ΔE at 10 / 25 / 40 s: 10.31 / 6.00 / 7.02 → 10.39 / 6.10 / 7.23; static 5.71 / 4.13 / 4.73 →
           5.81 / 4.22 / 4.92.
         - Layer B: Bistro 3.23 / 0.825 → 3.30 / 0.823, Zero Day 3.43 → 3.44.
         - Ghosting unchanged. Offline rendering always tests.
       - Follow-up: remove the bias without the cost. Either:
         - an opacity micromap: per masked triangle, cells known opaque or transparent skip the texture and mixed
           ones test it;
         - or the cutout continuation rays compacted into a pass of their own.
       - The specular filters' luminance edge stop, widened (2026-11-09, branch `spec-edge`): it weighed each tap by its
         distance from the local mean at 2 x strength, so a sparse bright reflection was kept out of its neighbours'
         means and, as a centre far from its own mean, out of its own: its energy was lost, not spread. At 8 x:
         static Bistro's specular 0.80 → 0.84 of our offline render (layer B pose, 64 frames; 0.85 with no luminance
         stop at all, which blurred Zero Day's sharp reflections: layer B 3.46 → 3.53). bench-crowd ΔE 13.3 / 6.9 /
         8.5 → 13.1 / 6.9 / 8.4; layer B Bistro 3.57 / 0.813 → 3.55 / 0.810, Zero Day 3.46 → 3.48; ghosting a little
         better on every check (strafe 5.27 → 5.17, stop 4.59 → 4.47).
       - The specular history's neighbourhood clamp, removed (2026-11-09, branch `spec-clamp`): while the view moved,
         the temporal pass clamped specular history to 4-8 σ of the nine 1-spp samples around the pixel. Around sparse
         bright reflections and highlights (string lights on glossy roofs, headlights on wet stone) those nine are
         mostly all dark, so the box collapsed and clipped converged history every frame: the light was lost, not
         delayed. The history cap stays (12 frames while the view moves, 4 for 8 frames after it translates), and so
         does the choice between the surface and the reflection-hit history.
         - The Bistro crowd in motion (lockstep snapshot at 10 s, live / our offline render, linear): specular 0.26 →
           0.55; the facades 0.35 → 0.92, the pavement 0.34 → 0.75, the vans' roofs 0.24 → 0.46 (the rest of the
           remaining loss is on the moving vans).
         - bench-crowd ΔE at 10 / 25 / 40 s: the crowd 12.7 / 6.8 / 8.2 → 11.2 / 6.0 / 7.2 (luminance at 10 s 0.68 →
           0.77, trails 298 → 129 px), static Bistro 6.6 / 4.8 / 5.4 → 6.0 / 4.2 / 4.9 (0.85 → 0.91). Layer B
           unchanged (the view holds still, where the clamp was already off).
         - The clamp was the ghosting guard, yet bench-ghosting is better on every check: each shot is compared with
           its converged frame, and the clipped (darkened) reflections were most of the error. Excess turn-mid 3.43 →
           0.76 (trail 22 → 4 px), strafe-mid 5.14 → 2.19, turn-return 1.40 → 1.01, strafe-return 1.51 → 1.30,
           stop + 0.1 s 4.41 → 1.86.
       - The per-sample ceiling, 64 → 256 display units (2026-11-09, branch `sample-ceiling`, `SAMPLE_CEILING`): with
         the specular history no longer clipped (`spec-clamp`, `mirror-history`), the ceiling was what still cut the
         light of small strong lights in motion. Measured on main + both: the Bistro crowd at 10 s (lockstep, live /
         our offline render, linear) diffuse 0.87 → 0.90, specular 0.83 → 0.89, total 0.93 → 0.95; the pavement under
         the headlights 0.71 → 0.77, the vans' roofs' specular 0.85 → 0.91. 1024: the same within 0.01.
         - bench-crowd luminance at 10 / 25 / 40 s 0.87 / 0.96 / 0.84 → 0.89 / 0.97 / 0.85 (static 0.91 / 0.99 / 0.94 →
           0.92 / 0.99 / 0.95); ΔE 10.9 / 6.0 / 7.1 → 11.0 / 6.1 / 7.1 (a little more noise from the brighter
           samples), static unchanged; layer B and bench-ghosting unchanged.
         - Live snapshots held still (time frozen, N frames) do not repeat run to run: the same binary gave diffuse /
           specular 0.86 / 0.78 and 0.90 / 0.69 an hour apart. Judge such changes on the lockstep moving phase, which
           repeats to three digits.
  3. Acceptance:
     - The stress scene, with its walker and vehicle counts stated, along Bistro's path at the 0.5 floor: frame
       interval p95 ≤ 16.7 ms.
       - Met 2026-11-09 on frame starts (`start`, what the pacer sets): w150-v60, frame start p95 16.67 ms along the
         path's first 50 s, with live emitter shadows taking cutouts as opaque (item 2's frame budget notes).
     - Live against our offline render: no notable regression from static Bistro. The number is set once measured.
     - Ghosting on moving objects within the bench-ghosting limits.
     - The scaling curve recorded here.

### History: the Bistro night and Zero Day phase (2026-11-08)

The static phase's plan and results, kept as they were measured.


- The scenes, both from NVIDIA ORCA, CC-BY 4.0, FBX + DDS:
  - Amazon Lumberyard Bistro, exterior: 2.83 M triangles. Night only: gates, metrics, UE references, comparisons and
    reports use the night version. The small scenes cover daylight.
  - Beeple Zero Day, Measure One: 1.37 M triangles by ORCA's README; the FBX holds 5.87 M (the room 3.14 M, the two
    instrument groups 1.34 M + 1.32 M). 10,103 emissive triangles; 551 objects move (13.7 s of node animation).
- Decisions (the user's):
  - Scenes are written in `@threert/scene`, the three.js-like scene code. The same code runs on both engines: real
    three.js in a browser through `tools/three/engine.ts` (degraded), the full effect in ThreeRT.
  - three.js lacks the light that emissive surfaces give in ThreeRT. The adapter adds it by itself: a few point lights
    from the brightest emissive clusters, capped in number, listed in the adapter's dropped / notes. Scene code states
    physical intent only and adds no lights for three.js.
  - Kabukichō's roles in gates, metrics and reports pass to Bistro. Its code stays.
  - Tests and reference images use the manual exposure written in the scene. Night auto exposure reads ~4 stops bright;
    whether that becomes an item is decided after milestone 1.
- Order: one line of work, serial (one GPU, the same files).
  1. This roadmap rewrite.
  2. The asset pipeline, both scenes at once: FBX + DDS converted offline to GLB.
     - Why GLB: ThreeRT, three.js and UE (Interchange imports glTF) read the same data, and the materials become one
       PBR model.
     - Textures to PNG / JPEG, resolution capped (e.g. 2K, by GPU memory).
     - Falcor's material texture conventions mapped once, in the conversion script, to metallic-roughness. Emission
       kept; Zero Day keeps its node animation.
     - Tools: Blender headless + Python, wrapped in a Bun Shell script (`brew install --cask blender`).
     - Output in agent3d-assets, fetched by `scripts/setup.ts`; the CC-BY attribution in the docs.
     - If ORCA's download needs a login, the user downloads it; other work goes on meanwhile.
     - Done (2026-11-08, branch `orca-assets`): `bun scripts/orca.ts`, run by `scripts/setup.ts`. No login needed
       (developer.nvidia.com/bistro and /beeple redirect to the archives). Two minutes on the M5 for both scenes.
       - Output: `assets/orca/bistro/BistroExterior.gltf` (2.83 M triangles, 1296 meshes, 132 materials, 405 textures,
         438 MB) and `assets/orca/zeroday/MEASURE_ONE.gltf` (5.87 M triangles, 7293 meshes, 166 materials, 352 textures,
         434 MB), each with a report of what was converted.
       - glTF with separate .bin and texture files, not GLB: the exporter keeps the converted PNG / JPEG files as they are
         (no second encoding, no packing pass), and ThreeRT, three.js and UE's Interchange read .gltf as they read GLB.
       - Falcor's conventions as Falcor renders them: the Specular map's G / B are roughness / metalness, which is glTF's
         metallicRoughnessTexture as is (its R, occlusion, is 0 in every map of both scenes: unused); DirectX normals
         flipped to OpenGL, BC5's z rebuilt; BaseColor alpha is alpha-tested (MASK, 0.5) where any texel is below 250,
         else opaque, so Bistro's window glass stays opaque as in Falcor; double-sided only for `*.DoubleSided`
         materials; emission = the Emissive texture x 1 (Falcor's factor), the scene scales it to nits (GLTFModel
         `power`); the FBX sun is dropped (night = emitters only); meshes without UVs get a zero UV set (Falcor samples
         (0, 0)).
       - Animation: each FBX take is one glTF clip named after the file, over the whole take (Bistro: the camera path and
         15 swinging lanterns / string lights, 154 s; Zero Day 13.7 s).
       - Checked: both render in ThreeRT offline from their FBX cameras (Bistro at power 1000, EV100 2: lamps, string
         lights, signs light the street; 64 spp in 10 s). Bistro's offline run peaks at 21.8 GB of memory (the 405
         textures at <= 2K as RGBA8 pages): a texture budget may be needed in step 3. Dark speckles on Bistro's shaded
         walls at 64 spp, to look at in step 3.
  3. Bistro night: `scenes/bistro.ts`, the adapter's automatic lights, a three.js page that opens in an ordinary
     browser.
     - Done (2026-11-08, branch `bistro-scene`):
       - `scenes/bistro.ts`: the converted exterior, emitters x 1000 (street lamp globes ~420 nits, ~370 lm each), an
         urban night sky glow (~0.02 nits), the FBX camera's first pose, manual EV100 2, 1280 × 720.
       - ThreeRT: offline 256 spp in 20 s; live at the 0.5 floor 14.75 ms GPU (trace 9.1, temporal 2.0), first frame
         5.2 s. Within the 16.7 ms budget before any Bistro work.
       - three.js adapter: automatic point lights (COMPARE.md, rules). By raw power the 16 lights went to the bright
         square down the street and the near street stayed black; ranked by power / distance² from the camera they
         light what the view shows (27 % of the emitted power, 4 of them shadowed). WebGL baseline 16.6 ms GPU, 8,792
         draws (cube shadows are half of it).
       - `node tools/three/open.ts scenes/bistro.ts`: the scene in three.js in an ordinary browser (the measured
         baseline's mapping and post, orbit controls, an overlay with fps, GPU time, lights, dropped, notes).
       - The dark speckles on Bistro's shop-window panes (offline, hardware and software traversal alike; three.js
         clean) are ours, and not geometry: without the denoiser the 256 spp image is full of bright fireflies there
         (glossy panes reflecting thousands of small emitters, found by BSDF rays), and the à-trous denoiser leaves dark
         holes around them. Zero Day's glossy machine tops do the same. Not the normal maps, the glass layers or the
         alpha masks (each removed in turn). The cure is light sampling for many small emitters (step 8's question).
  4. Bistro's UE reference: the same GLB imported into UE 5.8, the Path Tracer as truth, Lumen as the real-time
     comparison. Small runs, step by step (UE's volume renders once hung the GPU).
     - Revised (2026-11-08, "How we are judged"): this becomes the milestone audit, UE Path Tracer vs our offline render
       of Bistro night and Zero Day, with the user present. No Lumen. Branch `bistro-ue` is prepared, not run.
  5. Milestone 1: ThreeRT live vs three.js (strongest / tuned) vs our offline render, Bistro night and Zero Day (below),
     shown to the user. It does not wait for UE.
     - Done (2026-11-08, branch `judge-offline`, `node tools/milestone1.ts`; sections in `docs/COMPARE.md`, montages
       `docs/images/milestone1-{bistro,zero-day}.jpg`). ΔE / SSIM against our unbiased offline render, frame cost at
       the still pose and along the first 6 s of the scene's FBX camera move:

       | | ThreeRT live | three.js strongest (WebGPU) | three.js tuned (WebGL) | reference's own noise |
       |---|---|---|---|---|
       | Bistro night (ref 4096 spp) | ΔE 3.8 · SSIM 0.812 · 16.5 ms at 0.57; move p50 16.4 ms (60 fps) | ΔE 10.1 · 0.663 · 35.0 ms (29 fps) | ΔE 12.1 · 0.613 · 15.9 ms; move 18.3 ms (55 fps) | 1024 spp: ΔE 1.8 |
       | Zero Day (ref 16384 spp) | ΔE 3.6 · SSIM 0.802 · 16.1 ms at 0.57; move p50 15.7 ms (60 fps) | ΔE 14.4 · 0.440 · 180 ms (6 fps) | ΔE 16.2 · 0.319 · 79.7 ms (13 fps) | 4096 spp: ΔE 2.1 |

       (Refreshed 2026-11-08 at local-main f2815a1, after item 3 part 2; the first run, before it, read Bistro ΔE 4.0 /
       SSIM 0.795 at scale 0.5 and 54 fps along the move. The dynamic resolution now settles at 0.57 on both scenes.)

       - ThreeRT live is 2.5–4.5× closer to the reference than either three.js configuration on both scenes. Where it
         is farthest: Bistro's square (ΔE 7.6; the string lights) and Zero Day's floor (4.9; glossy reflections of
         small emitters). Its image is 10–12 % darker than the reference (luminance 0.88 / 0.90): the live clamps.
       - Bistro along its camera path was below 60 fps in the first run (GPU 16 → 22 ms as the view opened towards the
         square; the supervisor's 8 s capture: frame interval p50 18.9 / p95 23.1 ms). The gates measured only the first
         pose. Item 3 part 2 (below) brought the path to 60 fps.
     - Our side measured (2026-11-08, `node tools/compare/strongest.ts --scenes bistro`, M5, 1280 × 720). ΔE / SSIM
       here are against our offline render at 2048 spp in its default configuration (clamped, denoised), not yet the
       unbiased reference milestone 1 uses:

       | renderer | GPU ms / frame | ΔE | SSIM | luminance vs reference |
       |---|---:|---:|---:|---:|
       | ThreeRT live, 60 fps pacing (dynamic scale 0.52) | 15.47 | 5.76 | 0.795 | 0.68 |
       | ThreeRT live, full resolution | 39.07 | 5.11 | 0.835 | 0.71 |
       | three.js strongest (WebGPU + SSGI + SSR + TRAA), sky | 35.76 | 10.06 | 0.676 | 1.05 |
       | three.js tuned (WebGL, 16 automatic lights), sky | 16.27 | 12.06 | 0.616 | 0.81 |
       | ThreeRT offline 256 spp | 20 s | 1.54 | 0.959 | 0.95 |

     - Found: ThreeRT live is ~30 % darker than offline at Bistro night. Most of it in the square down the street: the
       60 m emitter cull dropped its string lights from light sampling (§ 5 item 5, "the view cone"). Fixed on branch
       `view-cull`: live raw radiance 0.75 → 0.85 of offline (square 0.40 → 0.70) for +0.5 ms; the table above is
       from before the fix.
     - UE Lumen and the UE Path Tracer wait for UE runs (branch `bistro-ue`, prepared, not run while the user is away).
  6. Zero Day, static: its UE reference and the same comparison.
     - Our side done (2026-11-08, branch `zero-day-static`): `scenes/zero-day.ts` (Measure One, emitters x 1000, no
       sky, the FBX camera's first pose; EV100 3: its median radiance is twice Bistro's, 0.24 vs 0.12 nits). Blender,
       ThreeRT and three.js frame it the same (checked against a Blender render of the converted glTF).
       `node tools/compare/strongest.ts --scenes zero-day`, interim ΔE against our offline at 2048 spp:

       | renderer | GPU ms / frame | ΔE | SSIM | luminance vs reference |
       |---|---:|---:|---:|---:|
       | ThreeRT live, 60 fps pacing (dynamic scale 0.6) | 15.65 | 3.13 | 0.859 | 0.97 |
       | ThreeRT live, full resolution | 35.16 | 2.80 | 0.872 | 0.96 |
       | three.js strongest (WebGPU + SSGI + SSR + TRAA), sky | 185.18 | 13.99 | 0.463 | 1.30 |
       | three.js tuned (WebGL, 16 automatic lights), sky | 81.26 | 15.44 | 0.351 | 0.95 |
       | ThreeRT offline 256 spp | 16 s | 2.61 | 0.910 | 0.87 |

       three.js draws 7,293 meshes, and the 4 shadowed automatic point lights render them 6 times each: 37,515 draws a
       frame. The UE side waits for UE runs, as Bistro's.
  7. The gates move to Bistro:
     - perf smoke and perf suite, baselines measured again (the rain / sparks / smoke combinations on Bistro too);
     - bench-ghosting's camera moves and their UE MRQ references;
     - rain against UE; layer B.
     The scene names in WORKFLOW and here change with them. Then Kabukichō leaves the gates and reports.
     - Perf gates moved (2026-11-08, branch `gates-bistro`): `tools/perf-smoke.ts` measures Bistro night (still at its
       camera, then 2.5 m down the street with 12° of yaw; `--scene kabukicho` keeps the old gate); `tools/perf-suite.ts`
       measures Bistro dry and in the rain, rain + sparks and rain + smoke on Bistro (`tools/perf/bistro-*.ts`), and
       the district drive until Bistro tiled replaces it. Baselines at local-main eb7ee8b (GPU ms at the 0.5 floor,
       still / moving): Bistro 15.10 / 14.78, rain 16.94 / 16.57, rain + sparks 17.40 / 16.95, rain + smoke 17.20 /
       16.86; district p50 / p95 16.78 / 17.31. Bistro in the rain is over the 16.7 ms line (rain costs 1.8 ms).
     - Waiting for UE: bench-ghosting's moves on Bistro and their MRQ references, rain against UE, layer B with Bistro
       and Zero Day (their UE Path Tracer images).
     - bench-ghosting on Bistro (2026-11-08, branch `ghosting-bistro`; it compares moving frames with the same build's
       converged ones, so UE only adds the optional `--ue` comparison). `node tools/bench-ghosting.ts --scene bistro`
       is the default; `--scene kabukicho` keeps view v1 and its limits.
       - Bistro's moves are smaller (its street is narrow): a 20° turn, a 2.5 m strafe to the left, the same 4 m dolly.
         Regions: ground (the street), shop (the shop and its lamp), facades (the lamp-lit left facade).
       - --quick limits from three runs of main 87b7ef7, which repeated to 0.01 dE and 1 px. At the 0.5 floor: turn-mid
         ground 3.95 dE / 29 px, strafe-mid ground 5.70 / 46 px, stop+0.1s ground 5.11 / 31 px, returns 1.6 dE.
       - Sensitivity: with the reflection resolve off, Bistro's check fails on the turn's trail (38 px against 34),
         Kabukichō's on three shots (wet signs). The R2 gate runs both (WORKFLOW).
  8. Zero Day animated: static and moving parts apart, only what moves in the dynamic BLASes. Measure first, then decide
     whether light sampling needs another method.
     - Done (2026-11-08, branch `zero-day-animated`): `games/zero-day/zero-day.game.ts` plays the take (13.7 s, looped).
       - Measured first: the posed model re-skinned whole each frame (5.87 M triangles on the CPU, all of them in the
         dynamic BLAS) ran at ~540 ms a frame (CPU 310 ms in the commit, GPU 44.5 ms).
       - Posed rigid models (no skins) are now split by node at build (`scene::RigidModel`): the nodes the clip leaves
         alone are static triangles (3.46 M); each moving mesh node is an instance of its own mesh in node space (3,991
         instances, 2.40 M triangles, BLASes built once; mirrored nodes keep their winding front-facing); their
         emissive / glass / function triangles stay flattened and dynamic (9,359, re-transformed in place, the only
         dynamic BLAS). A new pose samples the clip (0.3 ms for 8,641 nodes), re-places the instances (one buffer
         write per model) and rebuilds the TLAS.
       - Zero Day animated at the 0.5 floor: frame interval 16.68 ms (p95 17.25), GPU 12.5 ms, commit 2.0 ms; the
         moving light columns light the room through emitter sampling as before.
       - Light sampling stays as it is for now: the moving emitters are ~10 k triangles and sampled like the static
         ones. Many small emitters still make fireflies in glossy reflections (step 3's speckles), so a light tree or
         grid remains the candidate, measured on both scenes when it is taken up.
       - Instances that stop moving catch their previous-frame rows up once per commit now (two updaters in one
         commit, movable models and rigid models, would otherwise reset each other's).
  9. The C slice (§ 3) on Bistro.
     - Part 1 done (2026-11-08, branch `bistro-slice`): `games/bistro-slice/bistro-slice.game.ts`, one 60 s shot from
       Bistro's camera: four walking mannequins, three potted plants in the wind, Bistro's own lanterns and string
       lights swinging (its FBX take on the frame clock, through step 8's rigid-node path: 13 moving nodes as instances,
       18,400 bulb triangles re-transformed), rain from 15 s, an explosion (fireball, sparks, debris) at 40 s and its
       smoke after. Frames at 5 / 20 / 41 / 46 s: agent3d-scratch-backup-20261107/pin/bistro-slice-4.jpg.
     - Cost at the 0.5 floor (live, 66 s): GPU 18.4 ms before the rain, 19.5 ms in the rain, with the explosion and the
       smoke; frame interval 23.1–24.7 ms (the per-frame BLAS / TLAS rebuilds of the walkers and the swinging bulbs are
       outside the GPU profile). Over the 16.7 ms budget, as the Kabukichō slice was: part 2 takes it down and adds the
       UE side (waits for UE runs).
     - A van (2026-11-08, branch `slice-car`; § 3 C lists a car): a plain delivery van (body, cabin, wheels, head and
       tail lights at 2,500 / 300 nits) drives down the street toward the camera and past it from 25.5 to 34 s, on a
       lane 1.5 m right of the axis, passing the crossing pedestrian when that one is on the far side. A movable group,
       as Kabukichō's taxi: moved every frame, no rebuild; its lights are moving emitters. Live at the 0.5 floor the
       25–35 s stretch reads frame interval p50 16.91 ms (16.85 without it); the shot 16.80 / 17.28 ms p50 / p95.
  10. The 3 × 3 district (§ 5 item 5) becomes Bistro tiled.
     - Done (2026-11-08, branch `bistro-district`): `games/bistro-district/bistro-district.game.ts`, nine copies of
       the exterior on a 180 × 190 m pitch (25.3 M instanced triangles: one mesh per node shared by the copies;
       185,742 emitter triangles, flattened per copy). Driven along Bistro's own FBX camera path through the middle copy
       (`tools/lib/gltf-camera.ts`; the first 50 s, ~2 m/s): `node tools/bench-district.ts` (`--scene kabukicho` keeps
       the old district and its straight drive; both drives are `tools/lib/live.ts` `Drive`s). The perf suite's district
       lines are now Bistro's ("bistro district ...").
     - At the 0.5 floor: GPU p50 17.7, p95 24.2 ms, frame p99 25.1 ms, no frame over 50 ms, first frame 5.0 s, 11.3 GB
       footprint (the offline render peaks at 21.8 GB, the same as one copy). One copy alone on the same path: 17.5 /
       24.1 ms: the other eight cost ~0.2 ms (instancing, the emitter cull). The p95 is Bistro's own: 5–20 s into the
       path the view faces the square and its string lights (~24 ms), the many-small-emitters cost seen in steps 3 and
       8 (and the old Kabukichō district reads 16.8 / 17.3 ms on the same build).
- While the UE runs wait (the supervisor's list of 2026-11-08, in order): 1. light sampling for many small emitters;
  2. the denoiser's dark holes around fireflies; 3. Bistro night's budgets back under 16.7 ms; 4. milestone 1's output
  as one command.
  1. Light tree — done (2026-11-08, branch `light-tree`; `renderer/src/ltree.rs`).
     - What it fixes: emitters were drawn from one power CDF over the whole scene (20,638 triangles in Bistro). The lamp
       beside a shop window or the string lights over the square had a tiny share of the samples, so their light in
       glossy reflections came from rare BSDF hits: fireflies, which the clamp cuts (live too dark) and the à-trous
       denoiser turns into dark holes (step 3's speckles).
     - How: a binary tree over the static emitter triangles (single-emitter leaves, median splits; a node = one vec4:
       half-float sphere, power, child word). A shading point descends it by importance: power x the best cosine of
       the node's sphere over its hemisphere / max(distance², radius²). The selection probability of an emitter is the
       product of the choices down its path (a 26-bit code in its triangle's attrs x.w), and MIS against BSDF rays uses
       that pmf (`emit_mis` at the vertex that sampled, kept in `mis_x` / `mis_n`). Moving emitters (dynamic triangles,
       particles) stay in the CDF, after the tree's entries, chosen with their share of power. Scenes with fewer than
       256 static emitters keep the CDF alone (byte-identical). Camera hits resample 6 tree candidates offline, 2 live (4 since 2026-11-08, below).
       Live, the light LOD and the emitter cull are off with the tree (it weighs distance itself); instead nodes wholly
       beyond 120 m from the shading point are not proposed (never within one Bistro; the district's far copies).
     - Judged against our own unbiased offline reference (raw HDR, clamps and denoiser off, 320 × 180; references at
       16 k spp from both estimators agree within ±0.2 % (Bistro) / ±0.6 % (Zero Day) in every region):

       | 256 spp, 320 × 180 | relMSE | fireflies | dark spots after the denoiser | path tracing time |
       |---|---:|---:|---:|---:|
       | Bistro, CDF | 0.0959 | 14 | 555 | 1.48 s |
       | Bistro, light tree | 0.0406 | 7 | 65 | 1.61 s |
       | Zero Day, CDF | 0.826 | 394 | 3,047 | 0.95 s |
       | Zero Day, light tree | 0.632 | 282 | 2,448 | 1.04 s |

       At equal time Bistro is 2.2 x as efficient, Zero Day 1.2 x (its remaining noise is light found only through
       glossy reflections, which no light sampling reaches). Crops: agent3d-scratch-backup-20261107/pin/light-tree-windows.png.
     - Live (Bistro, 1280 × 720, raw HDR against the offline reference at 1024 spp): energy 0.85 -> 0.95 of the
       reference (the square 0.70 -> 0.93), relMSE after 16 frames 0.207 -> 0.105, after 128 frames 0.324 -> 0.117,
       dark pixels 13,932 -> 3,720.
     - Cost at the 0.5 floor (main and branch alternated): Bistro 15.10 / 14.79 -> 16.03 / 15.81 ms (+0.9); the
       district drive p50 17.7 -> 18.8, p95 24.3 -> 23.9, p99 25.15 -> 24.45 (its worst views, the square's string
       lights, got cheaper); Zero Day static +0.9 ms, animated unchanged (its emitters move). Perf smoke fails by
       design (R3).
     - Tried and dropped: stopping the descent after 6 levels at deeper vertices (no saving: the descents at deeper
       vertices cost nothing measurable, the camera hits' do), leaves of 4 emitters (-0.1 ms, Bistro relMSE 0.052),
       2 candidates offline (Zero Day relMSE 0.82), an importance by distance to the sphere floored at 0.25 / 0.1
       radii (cheaper in the district but the near lamp lost samples to nodes merely holding the point: live energy
       0.92, relMSE 0.155). Without the 120 m cut the district paid +2.9 ms (shadow rays to its far copies).
     - Live candidates 2 -> 4 (2026-11-08, branch `live-ris4`), from the study of live's brightness against the
       unbiased render (linear HDR, Bistro's first pose, direct light alone: live with one bounce and no cache against
       offline with one bounce). Live's light came out lower the fewer the candidates: mid-tones 0.85–0.88 with 1,
       0.90–0.92 with 2, 0.94–0.96 with 6. The estimator weights check out (one function gives both MIS pdfs). What
       loses light is the live filters on noisy signals: bright outliers lose weight in the specular filter's
       luminance edge-stop (off: +1–2.6 points, but ΔE 4.05 -> 5.15) and in the reflection resolve's ratio estimate
       (off: +2 points and ΔE 3.95 on Bistro, but bench-ghosting fails on Kabukichō's wet street). Less noise is more
       light: 4 candidates cost nothing measurable (Bistro's trace 8.20 / 9.93 -> 8.19 / 9.89 ms at 0 / 13 s into its
       path; the descents overlap the shadow ray). Bistro ΔE 4.11 -> 3.96, luminance 0.871 -> 0.889, SSIM 0.791 ->
       0.797; Zero Day ΔE 3.85 -> 3.77 (6 candidates: 3.83 / 3.81). bench-ghosting quick check ok, offline unchanged.
  2. The denoisers' dark holes around fireflies — done (2026-11-08, branch `denoise-holes`).
     - Both à-trous filters set their luminance edge-stopping width from each pixel's own variance. Among fireflies, a
       pixel that drew none has a low mean and a low variance, rejects all its neighbours and stays a dark hole. They
       now take the variance over 3 × 3 pixels (offline: a Gaussian of it, as SVGF; live: the neighbourhood mean the
       stage already reads, when larger than the pixel's).
     - Offline à-trous, dark spots after the denoiser at 256 spp (320 × 180, against the reference): Bistro 65 -> 54
       with the light tree (555 -> 298 with the CDF), at 64 spp 648 -> 319; Zero Day 2,448 -> 1,591. Live (Bistro, 128
       frames) 3,720 -> 3,662: what remains live is thin geometry the 0.5 render scale loses against a full-resolution
       reference (leaves, string-light wires, sign edges), not holes; the shop windows no longer show any.
     - Perf smoke +0.09 / +0.15 ms; bench-ghosting ok; layer B unchanged.
  3. Bistro night's budgets (16.7 ms at the 0.5 floor; the suite at 7249202: rain 16.80 / 16.45, rain + sparks
     17.24 / 16.87, rain + smoke 17.19 / 16.85, district p95 24.3, first frame 5.2 s, C slice 18.4–19.5 ms; the light
     tree since adds ~0.9 ms to every Bistro frame).
     - Part 1 (2026-11-08, branch `budgets`):
       - First frame 5.2 -> 2.0 s: the glTF loader decodes the images in parallel (the `gltf` crate decoded Bistro's
         405 one after another: load 3.3 -> 0.83 s); the ORCA conversion shrinks power-of-two textures of 512+ by the
         page gutters itself (2048 -> 2016), which the texture pages did at every load (pages 1.2 -> 0.63 s).
       - Rain -0.57 ms: streaks behind the camera are left out (they only cost the streaks' BLAS), and below full
         resolution the translucency pass draws per traced pixel and lays the result over the output bilinearly
         (`translucent_up`, as UE draws translucency at the screen percentage): translucent 0.99 -> 0.49 ms, its voxel
         training 0.29 -> 0.21. Bistro + rain 17.85 -> 17.28 ms (17.30 / 17.07 still / moving), + sparks 17.73 /
         17.42, + smoke 17.65 / 17.48.
       - Gate (R2): offline 16 / 16 byte-identical, perf smoke ±0.01, ghosting ok, layer B unchanged.
     - Where the rest is (Bistro dry, 16.0 ms at 0.5): trace 10.4 (the light tree ~0.9 of it), and passes that run at
       the output resolution whatever the render scale: temporal 2.0, specular filters 1.3, à-trous 0.8. Rain costs
       1.3 ms now (translucency 0.5, the streaks' BLAS 0.35, training 0.2, wet shading 0.1). Next: the output-resolution
       passes (the largest fixed cost), the C slice's per-frame BLAS rebuilds, the district's square views.
     - Measured for part 2 (at local-main 878e19c, 0.5 floor):
       - C slice: GPU profile 19.6 ms, frame interval 24 ms (GPU-bound: the TLAS rebuilds are outside the profile). The
         dynamic BLAS is rebuilt every frame: 2.44 ms, of it the four skinned walkers 1.44 ms (13,744 triangles each,
         + 0.36 ms of tracing) and Bistro's swinging lanterns / string lights 0.4 ms (18,400 flattened bulb triangles).
         wgpu 30 has no BLAS refit. Levers: a walker LOD (~3 k triangles; Blender's decimation keeps the skin
         weights), the moving bulbs' emitters inside their rigid instances (emitter sampling through the instance
         transform) instead of flattened.
       - Output-resolution passes, 4.1 ms at any render scale: temporal 2.0 (several histories, Catmull-Rom),
         specular filters 3 x 0.44 (real filtering for most pixels: their gate passes almost everywhere at night),
         à-trous 0.8. The largest fixed cost of every scene; a traced-resolution specular filter or one fused pass is
         the candidate.
       - Rain: 1.3 ms left (translucency 0.49, the streaks' BLAS 0.35 with 16,000 drops, voxel training 0.21 (not the
         point count: halving the cap changed nothing), wet shading 0.1). Fewer drops would trade the density UE's
         rain has.
       - District p95 23.9 ms: Bistro's own square view (one copy on the same path reads the same); light sampling for
         many emitters is in, what remains is the trace of that view.
     - Part 2a, foliage (2026-11-08, branch `foliage-trace`). Bistro along its camera path is GPU-bound below 60 fps
       (the supervisor's capture; the gates now drive the path): per-pass profiles at 0 / 7 / 15 s into it put the
       growth in the trace (10.5 / 13.3 / 14.7 ms), and nearly all of it in alpha-cutout foliage (285 k masked
       triangles: leaves 158 k, hedges 53 k, the square's lindens 41 k, ivy 21 k). With the alpha test off the trace is
       6.3 / 6.8 / 7.5 ms. The light tree is 0.9–1.3 of it.
       - Rearranging the queries changed little. Each cutout step had been a whole-scene query (up to 16 a ray).
         Stepping on the cutout instances alone after one opaque query, or one candidate loop on them, or one candidate
         loop on everything, all came out within ±0.4 ms. The alpha tests and the traversal past the leaves cost it,
         however the queries are arranged (alpha texture fetches ~1.4–2.2 ms of it). Culling the cutout geometry by its
         non-opaque flag is slow on Metal (+1.7 ms).
       - Done: live rays past the camera ray and its own light samples, off mirror-like paths, take cutouts as opaque
         (`cutouts_opaque`). Leaves are solid cards to indirect light and to later bounces' light samples. Offline
         rendering always tests: offline renders byte-identical (16 / 16 catalog scenes, Bistro with hardware RT).
       - Cost (perf smoke, 0.5 floor): still / moving 15.98 / 15.77 -> 14.04 / 13.85 ms; the first 20 s of the path GPU
         p50 / p95 22.2 / 25.8 -> 16.8 / 19.0 ms, frame interval p95 25.8 -> 19.3 ms (39 -> 52 fps). District drive
         (A/B alternated, twice each): GPU p50 / p95 18.8 / 23.8 -> 15.7 / 18.1 ms, interval p50 16.7 (60 fps).
       - Quality against the unbiased offline render (live, 64 frames, fixed 0.5; live is deterministic, two runs of
         one build read the same): first pose ΔE 4.08 -> 4.14, luminance 0.873 -> 0.868 (the top-right third, the
         square's trees, 0.86 -> 0.84); 15 s in ΔE 4.59 -> 4.62, luminance 0.912 -> 0.908. Treating only bounces from
         the second on as opaque saved half as much (15 s in 19.9 -> 18.3 ms) for the same image.
       - Perf suite at local-main 9093e7d (against 7249202, before the light tree): every line of the supervisor's list
         is within 16.7 ms at the still and moving poses: Bistro 13.95 / 13.86, rain 14.70 / 14.67, rain + sparks
         15.09 / 15.00, rain + smoke 15.71 / 15.20; first frame 1.99 s; district GPU p50 / p95 15.9 / 18.3 (was 24.3).
         Over: the 50 s path (GPU p95 18.6, interval p95 20.2 ms) and the district's p95.
     - Part 2b, foliage shadows (2026-11-08, branch `shadow-cutout`). At the path's worst stretch (12–14 s, the square)
       the alpha cost left was the first hits' light samples: camera rays taking cutouts as opaque saved nothing (trace
       11.0 vs 11.1 ms), their shadow rays 2.3 ms. Shadow rays used to step through leaves in order like closest hits:
       up to 16 whole-scene queries, and past the 16th leaf the ray counted as unblocked.
       - Done: after a first any-hit query finds a leaf its alpha lets through, one query whose candidate loop
         alpha-tests the cutouts and stops at the first blocker. Revised the same day (branch `shadow-determinism`,
         Known issues): the candidate loop made offline renders differ from run to run, so it is live only; offline
         shadow rays step through cutouts in order, up to 64 (16 before), and match the software BVH within 0.2 %. The candidate loop that lost hits on Metal was for
         closest hits (ground missing behind foliage); for shadow rays a lost hit would be light in a shadow. Against
         the software BVH (exact, no step limit; Bistro 15 s in, 16 spp, raw HDR, 4 x 4 regions) the old offline
         render was up to 2.3 % bright (light through dense foliage), the new within 0.3 %.
       - Perf smoke: path GPU p95 19.2 -> 17.8, interval p95 19.3 -> 17.9 ms (56 fps), still unchanged (13.9). District
         A/B (alternated, twice): GPU p95 18.4 / 18.5 -> 17.9 / 18.1 ms. Offline renders change where shadows cross
         more than 16 leaves (Bistro's first pose: luminance 0.999 of before); software traversal byte-identical.
       - Live against the unbiased render: ΔE 4.21 -> 4.25 at the first pose (against a reference rendered the new
         way), luminance 0.875 -> 0.870: the added (correct) occlusion darkens a live image the clamps already darken.
     - Part 2c, the radiance cache while moving (2026-11-08, branch `cache-levels`). Driving the worst stretch the
       trace cost 2 ms more than at the same poses held still (11.7–12.1 vs 9.8–10.4 ms). Voxel sizes follow the
       distance to the camera in powers of two: a surface the camera approaches gets a new, cold voxel one level finer,
       and every pixel on it traced a fresh estimate until it held CACHE_WARM samples, while the voxel a level coarser
       was warm.
       - Done: a cold lookup takes the warmer of the neighbouring levels (coarser, then finer). The training tiles warm
         the new voxel.
       - Driving 9–17 s: trace 11.7 / 12.1 / 11.7 -> 9.9 / 10.3 / 10.3 ms, as held still. Perf smoke: path GPU p50 / p95
         16.7 / 17.8 -> 15.3 / 16.0 ms. District A/B (alternated, twice): GPU p50 / p95 16.0 / 17.9 -> 15.0 / 16.2 ms.
       - Quality in motion (drive 9 -> 13 s, against the unbiased render at 13 s): 1 frame after stopping ΔE 8.06 ->
         7.96, 16 frames after 7.07 -> 6.83, luminance 0.80 -> 0.81. A warm neighbour beats a one-sample fresh estimate.
         Live only: offline byte-identical. Ghosting quick check ok (unchanged).
       - The frame interval did not follow: p95 19.6 -> 21.9 ms with GPU p95 16.0 (next item).
     - Part 2d, the headless viewport's pacing (2026-11-08, branch `live-pacing`): the supervisor's second question,
       the gap between frame interval and GPU time. With the GPU now under the frame, `live` paces by sleeping, and
       its sleeps overslept by half their length: macOS gives a process that is not a foreground app timer leeway of
       50 % (plain Python from the same terminal: sleep 1 / 5 / 11 / 16 ms returned 0.5 / 2.5 / 5.5 / 8 ms late).
       Frames with GPU 14 ms and CPU 1 ms came 20–23 ms apart. Not App Nap (an NSProcessInfo latency-critical
       activity), thread QoS (user-interactive) or the timer call (mach_wait_until): each tried, none changed it.
       - Done: `sleep_until` sleeps 60 % of what is left, again, and yields through the last 1.5 ms.
       - Perf smoke: path frame interval p95 19.96 -> 16.84 ms (59 fps; GPU p95 16.0 unchanged). District A/B
         (alternated, twice): interval p95 21.3–22.1 -> 16.8, p99 24 -> 17.0 ms.
       - A windowed viewport in the foreground has no such leeway; the benchmarks and the supervisor's capture ran
         headless, so the gates measured the leeway too.
     - Item 3 status (2026-11-08, after parts 2a–2d): every line of the supervisor's list is within 16.7 ms at the
       still and moving poses (perf suite at 9093e7d, before 2c / 2d), and Bistro's camera path runs at 60 fps: GPU
       p50 / p95 15.3 / 16.0 ms, frame interval p50 / p95 16.6 / 16.8 ms (the interval p95 against the 16.7 line is
       the pacer's own 0.2 ms of jitter). The district: GPU p95 16.2, interval p95 16.8 ms. Left: the C slice (BLAS
       rebuilds of the walkers and bulbs) and the rain combinations on the path.
     - Part 2e, the C slice's walkers (2026-11-08, branch `walker-lod`). Measured first at local-main cb80825: GPU 15.0
       ms (p95 15.2), frame interval p50 / p95 18.0 / 18.3 ms: the per-frame BLAS rebuilds are not in the GPU profile.
       - Done: the walkers use a level of detail of Quaternius' mannequin, 3.5 k of its 13.8 k triangles, same skeleton
         and 43 clips (Blender's collapse decimation keeps the skin weights; `tools/quaternius/lod.py`, made by
         `node tools/fetch-quaternius.ts` when Blender is there). At the slice's distances (10 m and more) it reads
         the same (offline frame at 8 s, side by side).
       - Slice, 60 s live at 0.5: frame interval p50 / p95 18.0 / 18.3 -> 16.8 / 17.2 ms (GPU unchanged). Before the
         rain (0–15 s) 16.65 ms: 60 fps; in the rain the GPU's whole frame (profile 15.0 + the rebuilds of the bulbs,
         0.4, and the rain streaks, 0.35, + the TLAS) slightly passes 16.7. What the LOD saved was mostly CPU skinning
         (four walkers re-skinned on the CPU every frame), not BLAS time (below).
     - Part 2f, the swinging bulbs (2026-11-08, branch `lantern-emitters`, measured, not landed). Bistro's 13 moving
       nodes carry 18,400 emissive bulb triangles, flattened into the dynamic BLAS so light sampling sees them in world
       space, and re-transformed and rebuilt every frame.
       - Tried: the bulbs in their node instances for rays, with untraced world-space twins for light sampling (a
         TRI_UNTRACED flag keeps the twins out of every BLAS; a hit through an instance finds its twin for MIS).
       - Same image (offline, 256 spp at 8 s: every 4 x 4 region within 0.2 %); offline trace 18.3 -> 16.8 s.
       - Live, alternated runs: the frame's profile 0.2 ms less (16.4 -> 16.2 dry, 16.85 -> 16.65 in the rain), the
         slice's frame interval in the rain p50 16.9 -> 16.8 ms; Zero Day animated unchanged (16.66 ms, 60 fps already).
       - The profile's BLAS entry stayed at 1.35–1.6 ms with 32 k or 14 k triangles in the dynamic BLAS: on Metal it is
         mostly a fixed cost per rebuild, not per triangle. Not landed: too little for a second path through light
         sampling. The slice stays at 60 fps dry and 16.8 ms (59 fps) in the rain, at the 0.5 floor.
  4. Milestone 1 in one command — done (2026-11-08, branch `milestone1-cmd`): `node tools/milestone1.ts`.
     - Reads the UE side when it is there (`out/ue/pathtracer/bistro.png`, `out/ue/lumen/bistro.png`, Lumen's frame
       time and settings in `out/ue/lumen/bistro.perf.json`, optionally Lumen frames of the camera move in
       `out/ue/lumen/bistro-move/`); measures ours (ThreeRT live and full resolution, three.js tuned and strongest,
       our offline path tracer at 1024 spp); ΔE76 / SSIM / per-region ΔE against UE PT (against our offline render,
       labelled interim, until it exists); writes the four-way montage (UE PT | ThreeRT live | Lumen | three.js, each
       with frame rate and ΔE; copied to `docs/images/milestone1-bistro.jpg`), a side-by-side GIF of Bistro's camera
       path (ThreeRT live at 60 fps pacing, Lumen, three.js, each with its FPS; shrunk until <= 5 MB: 60 frames, 921
       px, 4.95 MB), `metrics.json` and the `docs/COMPARE.md` section between its milestone-1 markers.
     - Run now with the UE tiles pending: ThreeRT live 15.72 ms at scale 0.5, ΔE 3.8 (interim); three.js strongest 34.4
       ms, ΔE 10.0; tuned 15.9 ms, ΔE 12.1. When the UE runs are done: `node tools/ue-bistro.ts` (branch `bistro-ue`),
       Lumen's perf JSON, then `node tools/milestone1.ts --reuse`.
     - Revised for "How we are judged" (2026-11-08, branch `judge-offline`): no UE inputs, Bistro night and Zero Day
       (`--scenes`, both by default). The reference is our unbiased offline render (no clamps, no denoiser; Bistro 4096
       spp, Zero Day 16384: at 4096 its glossy machine tops left the reference about as noisy as ThreeRT live is far from
       it), cached by bundle + renderer tree; a render at a quarter of the samples shows the reference's own noise.
       Montage per scene: our offline render | ThreeRT live | three.js strongest | three.js tuned, each with its frame
       rate (at the still pose and, where measured, along the camera move) and ΔE / SSIM. The clip is an animated WebP
       (ThreeRT live | three.js tuned along the scene's FBX camera move, the frame times measured along it; 60 frames
       at 1280 px in ~3 MB, where the GIF had to drop to 30 frames at 818 px). `tools/three/page.ts` times each pose of
       a path; headless Chrome runs without its crash reporter (its crashpad handler outlived Chrome holding our stderr
       pipe, and `strongest.ts` never exited).
- Milestone 1, Bistro night and Zero Day (revised 2026-11-08: no Lumen, no waiting for UE):
  - Conditions: one machine, each scene's own camera, 1280 × 720, the scene's exposure, the GPU otherwise idle.
  - Reference: our unbiased offline render (no clamps, no denoiser, 4096 spp).
  - Renderers:
    - ThreeRT live: 60 fps pacing, dynamic resolution.
    - three.js through the adapter: the strongest WebGPU configuration and a tuned WebGL one.
  - Each reports GPU ms / fps and ΔE, SSIM and per-region numbers against the reference.
  - Images per scene: one montage, our offline render | ThreeRT live | three.js strongest | three.js tuned, each tile
    labelled with its frame rate and ΔE / SSIM; a side-by-side animation of the scene's camera move (ThreeRT live |
    three.js) with FPS on each tile, ≤ 5 MB.
  - Delivered as sections of `docs/COMPARE.md`, the image paths given in the session.
  - The UE audit (UE Path Tracer vs our offline render) follows with the user present; it confirms the reference and
    does not change the milestone's tiles.
- Live against the unbiased render, its brightness (2026-11-08, branch `live-energy`). Milestone 1's live images read
  0.87 / 0.90 of the reference's luminance (Bistro / Zero Day). Measured in linear HDR (live `.raw` snapshots, offline
  `--hdr`, Bistro's first pose, 1024 spp, clamps off; 8 x 8 blocks binned by the reference's luminance):
  - Live at the 0.5 floor has 94.5 % of the offline radiance, at full resolution 98.5 %. Display luminance weighs the
    mid-tones, hence 0.87 there; the same tone map on both HDRs gives the same 0.87, so post-processing is not it.
  - Not the clamps: the indirect clamp (`clampIndirect`) and the sample ceiling (`sample_clamp`) off, alone or
    together, change nothing (every bin within 0.002). Not the radiance cache: off, the image is darker (0.941) and
    farther (ΔE 4.42). Not the light tree's live cut: the CDF instead is far darker (0.75).
  - Where: of the 5.5 % missing, 2.8 points in 1–53 nit blocks (69 % there) and 1.75 in the emitters themselves.
    Those are directly seen emissive surfaces (lamp globes, string-light bulbs, signs) smaller than the traced grid.
    The temporal pass clamped their emission history into the 3 x 3 box of the frame's traced samples, which the
    jitter fills with them only some frames.
  - Done: on surfaces, the emission is clamped only while something changes (view, object, lighting, animated shading),
    as the specular is. Bistro 0.945 -> 0.982 of the offline radiance (1–53 nit blocks 0.69 -> 0.89, emitters 0.98 ->
    1.00); display luminance 0.873 -> 0.893, ΔE 4.12 -> 4.05, SSIM 0.791 -> 0.792. Zero Day 0.902 -> 0.930, ΔE 3.91 ->
    3.85, SSIM 0.785 -> 0.789. Background pixels kept the clamp until the live volume scattering was fixed (Known
    parity gaps).
  - Left: the mid-tones, 0.87–0.93 at either resolution (the diffuse and glossy light of the street and facades).
    - Their cause, found after: the live filters darken noisy signals (bright outliers lose weight), so the noisier
      the estimate the darker (more tree candidates: more light; § 0 item 1).
    - Tried and dropped: the specular filter keeping the centre's weight, or edge-stopping against the centre instead
      of the local mean (Bistro ΔE 3.89 -> 3.90 / 3.94); the existing ReSTIR DI experiment (`THREERT_RESTIR=1`, not
      built on the light tree: Bistro ΔE 7.09); alpha-testing the rays that leave leaves (the square's tree, +1.2 ms
      for ΔE 4.11 -> 4.09).

### History: the Kabukichō phase (2026-11-05 → 2026-11-08)

The sections below keep their Kabukichō data as history. Where they name Kabukichō as a gate or a target, the gate is
now Bistro night (step 7 above moves them); their numbers stay as they were measured.

#### 0.1 three.js baseline — done (numbers below, rerun 2026-11-06 with the OIDN build and this round's live viewport)
The same scene code drives three.js r187: `tools/three/engine.ts` implements the engine API on WebGLRenderer, so scenes,
games and `SceneBinding` are unchanged (`node tools/bench-three.ts`). Mapping: physical light units 1:1, exposure
2^-EV100, ACES, the preset vignette, shadow maps, PMREM image lighting, the Preetham sky scaled to nits by one global
constant; a "+ scene probe" variant adds a two-bounce cube capture. Playground: "three.js" A/B split.

Mean ΔE against the UE5 Path Tracer (Kabukichō v1: against UE Lumen of the real level):

| scene | three.js (better variant) | ThreeRT live, 1 / 16 / 256 frames | ThreeRT offline (256 spp, OIDN) | UE Lumen |
|---|---:|---:|---:|---:|
| cornell-box | 13.10 | 2.12 / 2.33 / 1.44 | 0.95 | 3.27 |
| material-grid | 5.84 | 2.21 / 2.02 / 2.04 | 1.96 | 1.44 |
| glb-asset | 15.13 | 5.00 / 4.26 / 4.20 | 4.17 | 1.85 |
| neon-reflections | 5.63 | 2.72 / 2.24 / 2.19 | 2.04 | 2.31 |
| sun-room | 18.81 | 10.14 / 8.32 / 4.49 | 3.02 | 4.22 |
| light-room | 7.22 | 2.03 / 1.69 / 1.72 | 1.58 | 3.08 |
| kabukicho v1 | 29.07 | 12.50 / 10.55 / 10.18 | 11.32 | (reference) |
| mannequin | 9.47 | — | 3.81 | 1.51 |

After 16 frames the live viewport has 2.3–5.6× lower ΔE than three.js on every scene. Where three.js wins: SSIM on the
sun room at 16 frames (0.908 with its scene probe vs our 0.781: our image is still noisy there; 0.964 at 256 frames).
Full tables: `out/bench/three.md`.

three.js is 10–40× cheaper on the small scenes (0.4–1.2 ms vs 7–17 ms per frame) but drops to ~10 fps on Kabukichō
(99 ms: 147 lights, 4.5 k draw calls). Correction: ThreeRT does not hold 60 fps there either: 18–19 ms of GPU even at the
lowest dynamic render scale (0.525), about 51 fps on the M5 (measured with the stall-free timer, § 4).

#### 0.2 UE-comparable mannequin — done (ground truth in UE; live game; merged 2026-11-06)
- Engine: glTF skins and clips (`renderer/src/skin.rs`: step / linear / cubic-spline channels, linear blend skinning);
  `GLTFModel.animation = {clip, time, loop}` from scene code, per frame from frame scripts. Posed models are dynamic
  triangles in their own BLASes: a pose or move re-skins them in place (no scene rebuild); 2 ms per frame for one 4.7 k
  triangle character in the live loop (0.2 ms of it is skinning). Static scenes unchanged (glb-asset bit-identical).
- Asset: Quaternius Universal Animation Library (CC0): a UE-style mannequin, 13.7 k triangles, 65 bones with UE5
  mannequin bone names, 43 clips (`node tools/fetch-quaternius.ts`).
- Scene: `scenes/mannequin.ts` (four poses on a UE default-level setup), rebuilt natively in UE for ground truth
  (`ue5/mannequin.ts`: Interchange import of the same GLB, animation assets at the same times, MRQ Lumen + Path Tracer).
  Poses line up pixel for pixel. Against the UE Path Tracer: ThreeRT offline ΔE 3.81 / SSIM 0.990, three.js ΔE 9.47 /
  SSIM 0.939 (vs UE Lumen: 3.98 / 0.983 and 9.12 / 0.946).
- Game: `games/mannequin/mannequin.game.ts` (mannequin plaza: click to cycle idle / walk / jog / dance / sit; walkers
  circle). Four animated mannequins: frame script + re-skinning 2.5 ms per frame, clicks commit in place.
- Requirement for later (decision 1): skinned meshes must work in both paths — the dynamic-BLAS path (secondary rays)
  and a raster G-buffer path (primary hits); don't make per-frame BLAS rebuilds the only way to draw characters.

## 1. Real-time pipeline architecture

Targets (2026-11-08): the frame-time and quality gates below that name Kabukichō now apply to Bistro night (§ 0); the
Kabukichō numbers stay as measured.

Path tracing stays, in three roles: ground-truth reference, offline renders, and the secondary-ray engine of a hybrid
pipeline. The real-time pipeline moves toward Lumen's structure — split signals + caches, not rasterization first.

- **Step 1, split signals + caches.** Most direct light deterministic (deferred lighting; round 3's exact unshadowed
  local lights at camera hits are already this); indirect diffuse from a world-space radiance cache fed by path-traced
  rays; reflections ray traced, denoised by hit distance.
- **Step 2, rasterized G-buffer for primary hits.** Cheaper; dynamic content (skinning, particles, translucency, decals,
  foliage vertex animation) drawn every frame without acceleration-structure updates; platforms without hardware RT
  (WebGPU, lavapipe). The material-function contract (MatIn → MatOut) compiles to both a fragment shader and hit-point
  evaluation. The agent-facing DSL and gameplay API do not change.
- **Decision experiment (before step 2).** On Kabukichō, only the primary hit becomes a G-buffer, plus deterministic
  direct light; everything else unchanged. Measure with bench-temporal: ΔE / SSIM in motion and frame time, against our
  converged references and UE (2b). Proceed if, at equal or better motion quality vs UE, frame time drops by ≥ 20 %, or
  motion ΔE vs UE improves by ≥ 15 % at equal frame time.
  - Done (`THREERT_RASTER=1`: triangle id + depth rasterized at the frame's Halton jitter, the kernel intersects the camera
    ray with that triangle; cutout triangles that fail the alpha test fall back to tracing). Quality unchanged within
    noise (move40 / move59 5.76 / 5.92 vs 5.59 / 6.01 traced; stopped 0.5 s 5.05 vs 5.03; vs UE unchanged); GPU −0.75 ms
    (−3 %) at 60 %, −2.3 ms (−5 %) at 100 %. Deterministic direct light changed nothing here: Kabukichō's 4 shadowed
    lights were already evaluated exhaustively (RIS starts above 8) and unshadowed ones at camera hits.
  - **Conclusion: below the bar; step 1 next.** On the M5 with hardware RT, primary rays are cheap; the raster path stays
    (behind the flag) for dynamic content and platforms without hardware RT.
- **Step 1 acceptance** (measured before / after with the same tools; baselines from the 2026-11-06 build):
  - Disocclusion (k = 1, first frame of fresh history, `bench-three` live rows): sun-room SSIM vs the UE Path Tracer
    ≥ 0.90 (baseline 0.849, ΔE 10.1); Kabukichō v1 SSIM vs UE ≥ 0.68 (baseline 0.595, ΔE 12.5).
  - Motion: bench-temporal `excess_over_ue` (§ 2b: our temporal error minus UE's own) at move40 / move59 / stopped-0.5s
    −30 %: from 3.39 / 3.69 / 3.83 to ≤ 2.37 / 2.58 / 2.68.
  - Lighting changes: closing 4 s `excess_over_ue` ≤ 1.08 (today), closing 0.3 / 1 s vs converged no worse than
    9.79 / 9.44 + 5 % (a cache must not add lag; closing 2 s is too run-dependent to gate on).
  - Frame time: Kabukichō at 1280×720, dynamic resolution, GPU ≤ 16.7 ms (60 fps); today 18–19 ms at the 0.525 floor.
    Path: at 60 % the trace costs 7.4 ms with one bounce and 14.3 ms with four (THREERT_PROFILE, bounces 1 / 2 / 4: 7.37 /
    12.09 / 14.33), so ending paths in the cache after the first bounce saves ~7 ms; budget for the cache: update
    ≤ 1.5 ms, lookups ≤ 0.5 ms → ~16.2 ms at 60 %, ~13 ms at the floor. Plus: no shader-variant or rebuild hitch
    (bench-hitches p99 ≤ 26.7 ms baseline).
  - Tests: layer A stays green; new atomic tests for the cache (energy of a one-bounce diffuse box vs the path tracer).
- Status (2026-11-06): step 1 in progress. Landed: the world-space radiance cache and an albedo-free diffuse history.
  Not yet: deterministic direct light for the many emissive lights (next).
  - Radiance cache (pathtrace.wgsl, live only; offline renders stay pure path tracing). Spatially hashed voxels sized
    at 2 % of their distance to the camera (power-of-two levels, ≥ 6 cm) × 6 normal directions; each holds irradiance / π
    (total, and its indirect part). Live paths end in it at their first bounce: the surface reflects (diffuse +
    directional specular albedo) × the cached value. Surfaces seen in reflections below roughness 0.4 (where UE traces
    reflections) get their own direct light at the hit, resampled, plus the cache's indirect part: UE's hit lighting.
    One in eight 8 × 4 tiles (whole SIMD groups) trains it per frame with two-vertex paths of white-Lambert direct light
    plus a cosine-sampled ray each, ending in the cache again, so it carries every bounce. Pixels that find their voxel
    cold trace one vertex and use that meanwhile. Per voxel the history is capped at 256 samples and 32 frames (4 for
    45 frames after a lighting edit); voxels unused for 120 frames are freed. 2^18 entries, 13.6 MB, resolve 0.14 ms.
    `THREERT_CACHE=0` turns it off; `THREERT_CACHE_FLAGS` switches its parts for cost attribution.
  - Diffuse history holds irradiance (lighting / albedo, smooth), remodulated by albedo. Albedo and emission history
    is resampled with Catmull-Rom and clamped into this frame's 3 × 3 range (TAA: both are noise-free). It is held to
    4 frames while the view moves and 16 when still, so texture and sign detail no longer blur through repeated
    resampling.
    - Bilinear history at 2 frames scores better (move59 excess 2.72) but shimmers, which single-frame metrics don't
      see.
    - Catmull-Rom costs +0.18 ms.
  - Acceptance so far (before → after):
    - Frame time, met: Kabukichō trace at 60 % 15.5 → 11.9 ms; GPU at the 0.52 floor 18.7 → 16.3 ms. The default view
      settles at 15.7 ms (bench-three @256). Rain keeps it above: bench-hitches GPU p50 17.0 ms (more mirror pixels).
      Cost attribution at 60 %: lookups alone 8.1 ms, hit lighting at reflection hits 1.6 ms, cold fallback 0.5 ms,
      training tiles 0.8 ms. Exact light loops at the hits would cost 3.3 ms (mirror pixels are scattered); two-vertex
      training for the scattered cold pixels another 0.6 ms.
    - Hitches, met: bench-hitches p99 20.3 ms (≤ 26.7), no new spikes.
    - Disocclusion k = 1 on sun-room, met: SSIM vs the UE Path Tracer 0.849 → 0.916, ΔE 10.1 → 4.0. @16 frames
      0.781 → 0.958 and @256 0.964 → 0.972, at 93–100 % scale (was 77–80 %) in 14.4–15.3 ms.
    - Disocclusion k = 1 on Kabukichō v1, not met: 0.595 → 0.612 (target 0.68); ΔE 12.5 → 12.2. @16 0.693 → 0.703,
      @256 0.733 → 0.741. Its noise is direct light from 138 emissive signs sampled stochastically (RIS, 6 candidates,
      one shadow ray), which the cache does not touch.
    - Motion, not met: `excess_over_ue` move40 / move59 / stopped-0.5s 3.39 / 3.69 / 3.83 → 2.69 / 2.86 / 3.20, i.e.
      −21 / −22 / −16 % (target −30 %).
      - The cache alone changed it by < 0.1.
      - The albedo-free diffuse history and the sharp, clamped albedo / emission history did the rest. Ours vs
        converged: move59 5.91 → 5.08, stopped 4.95 → 4.32.
      - Switching the specular or diffuse spatial filter off makes it worse (noise), and so does a tighter specular
        clamp.
    - Lighting changes, met: closing 0.3 / 1 s 9.67 / 9.34 (≤ 10.28 / 9.91), closing 4 s `excess_over_ue` 1.14
      (1.08 before; closing runs through random switch-off orders, ± 0.1 between runs).
    - Tests: layer A 16 / 17 (glass transmission as before). New `cache/furnace-box`: an albedo-1 open box in a 1-nit
      sky through the live renderer (`serve` jobs take `live: frames`) reads 0.973 after 600 frames (1 is exact). It
      reaches 0.997 by 4000 frames, so energy is conserved. For comparison: offline at its bounce cap 0.78, live without
      the cache 0.86. Multi-bounce in deep concavities builds up over seconds: two bounces per 32-frame history turnover.
  - Emitter reservoirs (ReSTIR DI at camera hits: 4 candidates, the previous frame's reservoir at the reprojected
    pixel + 2 neighbours, one shadow ray). Kept as an experiment (`THREERT_RESTIR=1`), off by default:
    - Kabukichō v1 k = 1: 0.608 → 0.613. A history reset drops the reservoirs, as a real disocclusion does.
    - Motion: `excess_over_ue` +0.1 (3.12 / 3.29 / 3.53).
    - Cost: +0.44 ms at 60 %. No global bias (luminance ratio 0.995).
    - Reuse among this frame's reservoirs, which a disocclusion needs, takes a pass of its own after the trace.
  - Measurement correction (2026-11-06, second round). bench-temporal's cached references were stale: rendered by an
    older build, they differed even from the pre-step-1 build's own converged frame by 3.5 ΔE. Every "ours" ΔE above,
    and the 3.39 / 3.69 / 3.83 baseline the targets were derived from, carried that static offset (part of it: the
    change in what each build converges to, which is bias, not temporal error). bench-temporal now keys references by
    renderer build, bundle and THREERT_* switch, so each build is measured against its own converged frames. Two converged
    references of one build agree to 0.07–0.16 ΔE. Bias is judged against UE / the path tracer (bench-three @256:
    Kabukichō v1 0.733 → 0.740).
  - Re-measured, each build against its own converged frames. Baseline (b5566bd, before step 1): `excess_over_ue`
    move40 / move59 / stopped-0.5s 3.20 / 3.34 / 3.21. The −30 % targets become ≤ 2.24 / 2.34 / 2.25.
  - Where the remaining error was. Per signal (`THREERT_VIEW_MODE` 6 / 7, each against its own converged image), move59
    vs converged: specular 3.18, diffuse 2.73. Diffuse barely changes from moving to still (2.55 after 4 s still: slow
    convergence of emitter-lit diffuse). Specular is the motion part, and it was 8 % too dark while moving (luminance
    ratio 0.918). The 1σ clamp box, built from raw 1-spp reflection samples (mostly dark, sometimes a bright sign),
    clipped bright history: that is the smear on the glossy facades. UE clamps against its spatially resolved
    reflections.
  - Specular clamp box ×8 (UE's 1σ × 8; only while moving, as before):
    - Specular alone, move59 vs converged: 3.18 → 2.59 (luminance ratio 0.989).
    - Box ×2: 2.93. Box ×4: 2.71. Unclamped: 2.61, but bright ghosts (ratio 1.031).
  - Fresh history (< 4 frames) gets every specular filter pass and a loose normal test: 1-spp reflections on rippled
    wet ground are speckle that the normal-map detail otherwise keeps apart. Kabukichō k = 1 +0.02.
  - Results (2026-11-06; run-to-run variation ± 0.1–0.2):
    - Motion, met: `excess_over_ue` move40 / move59 1.87 / 1.97 (−42 / −41 %).
    - Stopped 0.5 s, borderline: 2.33 (−27 %). Other runs of the same build: 2.19–2.23.
    - Kabukichō v1 k = 1, not met: 0.612 → 0.632 (target 0.68).
    - Hitches p99 20.4 ms.
    - Lighting changes unchanged: closing 4 s excess 0.37.
  - Tried and dropped:
    - Motion history cap 48 → 96 / 192 frames: no change.
    - Fresh history seeded from the radiance cache (a prior worth 3 frames): Kabukichō k = 1 +0.014 only, and it blurs
      sharp sun patches at disocclusions (sun-room k = 1 ΔE 3.86 → 4.75).
    - ReSTIR DI, re-measured with correct references: no gain (1.87 / 1.85 / 2.25 vs 1.78 / 1.85 / 2.23).
  - What's left at k = 1 is the wet street's first-frame reflections (blotches of neon), not diffuse. That needs a
    reflection resolve that reuses neighbouring pixels' rays weighted by each pixel's BRDF (Lumen's spatial reuse),
    not a blur.
  - Reflection resolve (2026-11-06, third round; live.wgsl pass `refl`, Stachowiak 2015 / Lumen's spatial
    reconstruction). The trace kernel keeps each camera hit's specular-lobe ray (direction, lobe pdf, radiance it
    brought back, the hit's specular albedo E); the per-pixel specular signal keeps only light-sampled specular.
    Each traced pixel then reuses its own and up to 8 neighbours' rays on the same surface, re-aimed at their hits
    and weighted by its own BRDF over their pdf: E × Σ w·L / Σ w. 0.29 ms at the 0.5 floor.
    - The clamp box tightens toward UE's again. Excess move40 / move59 / stopped-0.5s by box:
      - 8σ: 1.68 / 1.84 / 2.19;
      - 4σ: 1.72 / 1.87 / 2.21;
      - 2σ: 1.75 / 1.92 / 2.24;
      - 1σ (UE): 1.82 / 2.01 / 2.28.
      Before the resolve, halving the box cost ~0.3 ΔE. Now 4σ.
    - Results with 4σ, three runs (they agree to ± 0.01):
      - Motion, met: move40 / move59 1.71 / 1.86, −47 / −44 % from 3.20 / 3.34.
      - Stopped 0.5 s, met: 2.20, −31 % (≤ 2.25).
      - Closing 4 s excess 0.28.
      - Sun-room k = 1 SSIM 0.915 → 0.950.
      - Kabukichō v1 k = 1, not met: 0.632 → 0.640 (target 0.68).
      - Hitches p99 21.4 ms. Kabukichō GPU at the 0.5 floor 16.4 ms.
    - Why Kabukichō k = 1 stays short. More or wider reuse (16 neighbours, 1.5× radius, looser normal test) moves it
      by +0.002, and resolution doesn't matter: at a fixed 100 % scale k = 1 is 0.646 vs 0.641 at 55 % (`bench-three
      --scale`). The k = 1 error is spread over every edge and sign (one jittered sample per pixel, no history),
      emitter-lit diffuse (one emitter sample) and reflections together. Closing it means reuse within the frame for
      emitter samples too (a shading pass after the trace: ReSTIR's spatial pass), the analogue of what makes Lumen's
      disocclusions clean (deterministic direct light, interpolated probes).
  - Frame time by state (Kabukichō at the dynamic-resolution floor 0.5 = UE's default minimum, GPU timer, 12 s after
    each change). It is not rain:
    - Within 16.7 ms: rainy night (default) 15.8, rain on / off 15.7 / 15.8, clear 16.2, viewpoints 16.0–16.4.
    - Over it: the fog-heavy moods while they ramp in, mist 17.1 and 3 AM 17.2. A mist timeline reads 17.3 / 17.2 /
      17.1 at +2 / 4 / 8 s, then 15.7 at +15 s.
    - Where the transitional cost is: the denoiser. Changing fog keeps anti-lag restarting history where fog-lit
      light changes, so the wide à-trous stages run there.
    - Tried: a 3 × 3 à-trous kernel saves 1.3–1.7 ms everywhere, but sun-room @1 / @16 SSIM fell 0.915 / 0.957 →
      0.831 / 0.885. Only on long-history pixels it keeps quality and saves nothing in transitions. Capping
      environment ramps at 32 history frames instead of 16: no measurable change.
    - Status: met in steady states; fog transitions exceed the budget by ≤ 0.5 ms for ~10 s.
    - Since the diffuse denoiser runs at traced resolution (§ 5, item 4), the same states at the 0.5 floor: rainy night
      14.8, mist 14.9, clear 14.4, rain off / on 14.8 / 14.8, viewpoints 15.3–15.6 ms (the #18 build: 17.2–18.6). The
      mist ramp reads 15.0 / 15.0 / 15.0 / 14.8 at +2 / 4 / 8 / 15 s (#18: 18.4–18.5). Met in transitions too.

- **Direct light for camera hits in its own pass (MegaLights route; next, before streaming, § 5 item 5).** The path-tracing
  kernel is one megakernel, and code in it costs GPU time even when it never runs. Camera-hit direct light (the exact
  light loop, emitter resampling) should become separate passes with per-screen-tile light lists (coherent reads)
  and MegaLights-style sampling, so the kernel shrinks instead of growing.
  - Evidence (2026-11-06): 3 × 3 Kabukichō district, lockstep drive down the middle street (`live` lockstep, one
    camera step per frame at 15 m/s), GPU p50 at the 0.5 floor; main 23.75 ms.
    - A light grid plus per-cell light resampling, compiled in but switched off: 25.31 ms (+1.6 ms) for code that
      never runs.
    - The per-pixel grid-cell lookup itself costs nothing: 25.30 ms with the lookup on and the old loop kept.
    - Resampling 8 candidates from the pixel's cell everywhere: 25.68 ms, 0.4 ms slower than the old loop at the
      same code size. That loop over all 1,211 lights reads the same light in every thread, so it is cheap.
    - Capping that loop at 150 lights (a one-line change) measured 25.29 ms: the kernel's code generation swings by
      more than the work saved.
  - What the 3 × 3's extra cost over its middle row (19.34 ms) is: the outer rows' light sources, not their geometry.
    - Removing their analytic lights and emissive triangles, all 37.5 M triangles kept: 19.38 ms.
    - Removing only the lights, only the emitters, only geometry (props, procedural meshes or both): 23.3–23.8 ms.
    - Capping secondary rays at 60 m: 23.57 ms.
    - Far light sources take samples (and long, mostly occluded shadow rays) whichever kind is left.
  - Light LOD (§ 5 item 5) took the district to 19.3 ms lockstep p50 (19.5 on bench-district; about 19.4 was the
    goal), with no new cost when switched off. Its order and what is left:
    1. Light LOD, unbiased: emitters sampled by distance to the camera, local lights in spatial groups. Done.
    2. UE strafe reference and the sign-band comparison (§ 2c). Done.
    3. Motion blur length and the night auto exposure measured against UE (§ 2c, Known parity gaps). Done.
    4. This pass (moved before streaming on 2026-11-07: the wet street after a strafe is what the user notices most).
       Not pursued for now (decided 2026-11-07). Inline, the camera-hit direct light overlaps the traversal's memory
       latency and costs almost nothing; on its own it is paid in full (the split and tile-culling data below). It
       reopens only when MegaLights-style sampling actually needs a pass of its own.
       - With it, one more emitter sample at near-mirror camera hits, reusing `refl_rays` (branch `glossy-refl-eval`,
         § 2c): reflection rays and emitter sampling both have to halve their noise there.
       - Acceptance:
         - Kabukichō and 3 × 3 district GPU at the 0.5 floor no worse;
         - the street after a strafe +0.5 s ≤ 1.33 (UE50 + 0.3) and the +0.1 s gap to UE50 halved;
         - the sign band's trail no longer.
       - It also covers the rest of the way to 16.7 ms on the district, about 2.7 ms. Geometry LOD does not change
         the GPU time: 19.23 ms with the outer blocks gone entirely vs 19.38 with only their light sources gone.
    5. Streaming (§ 5 item 5): first frame ≤ 3 s, no frame over 50 ms. The first frame is met without it (§ 5).
  - Tried 2026-11-07, not merged.
    - The split itself (branch `direct-pass`):
      - What it does: the trace leaves each camera hit's surface in a buffer and skips direct_light there; a pass of
        its own (`direct`) lights it with the same estimators.
      - Kabukichō is even (moving 15.42 → 15.35 ms: trace 8.89 → 6.79, `direct` 2.03).
      - The district is worse: drive p50 19.36 → 20.22, p95 20.87 → 21.42; mid-street 15.95 → 17.17 (trace 9.93 →
        8.56, `direct` 2.58).
      - Taking the camera-hit light code out of the megakernel altogether saves only 0.06–0.12 ms more. The cost is not
        code size: inline, that work overlapped the kernel's other latency-bound work, and on its own it is paid in
        full.
      - `direct` on the district: 0.68 ms fixed (the surface, the moon's shadow ray), local lights 0.89, emitters 0.86
        (6 candidates, each a 19-level binary search over 376 k entries).
      - It was to be reconsidered once emitter sampling and the local lights got cheaper:
        - Emitter sampling: done. The CDF search goes through a guide table (§ 5 item 5, #49): district drive p50
          −0.4 ms inline. An alias table, the first plan, was slower.
        - Local lights: per-tile culling has nothing to save (next item).
    - Per-tile culling of the camera-hit local lights: measured first, not built. Inline, the light loop's work costs
      nothing measurable, so tile lists have nothing to take away. 3 × 3 district at the 0.5 floor:
      - The same binary with every light group skipped (data only: no camera-hit local light evaluated): drive p50 /
        p95 19.30 / 20.57 → 19.54 / 21.38 ms, slower.
      - The same, with the unshadowed lights scaled by 1e-20 in both arms, so the image is the same and only the
        loop's work differs: mid-street trace 10.8–11.0 → 11.4–11.5 ms, still slower.
      - Without groups (`THREERT_LIGHT_GROUPS=0`, every light's range tested at every pixel): trace +4.6 ms. The groups
        already remove what culling would.
      - Shader variants that skip the loop on a runtime condition: trace +0.2–0.7 ms (code generation).
      - In the `direct` pass the same local lights cost 0.89 ms. Inline, that work overlaps the kernel's
        latency-bound traversal. Why skipping it makes the trace slower (scheduling, clocks) was not measured.
    - Specular denoising at the internal resolution, UE's structure (branch `spec-internal`, one time-boxed prototype on
      top of #54): the specular history, its clamp and the spatial filter on the traced grid (a pass `stemporal` with
      the surface and mirror-image reprojections, UE's 4 sigma clamp and frame caps; the three filter steps after it),
      and the output grid upsampling the result like the irradiance, with a short history (16 frames still, 4 moving)
      clamped to the four traced values.
      - Time: −0.62 ms on the district drive (specular filter 1.28 → 0.30, temporal 2.00 → 1.78, `stemporal` +0.35, a
        final-image copy +0.22 that reordering buffers would remove).
      - Ghosting much worse, every bench-ghosting row fails:
        - turn-mid ground excess 2.83 → 3.55, trail 65 → 123 px;
        - strafe-mid trail 50 → 134 px;
        - stop +0.1 s 2.78 → 2.99, trail 28 → 76 px.
      - The wet street vs UE50 is worse at all three times: 3.27 / 2.67 / 1.62 → 3.56 / 3.00 / 2.03.
      - Converged frames lose the wet street's ripples and the sharp sign reflections.
      - Why:
        - The traced grid's history adds each frame's jittered sample at its pixel centre, so it averages over the
          pixel's 2 × 2 output footprint. The output-grid history placed every jittered sample where it fell (TSR's
          reconstruction); now it only sees the averaged input and cannot get the detail back.
        - Two histories in a row (12 frames moving on the traced grid, then 4 on the output grid) add their lag.
        - The traced history reprojects at 2-pixel granularity.
        - The output clamp spans the four traced values, already filtered, so it is wide.
        - The filter steps (1, 2, 4 traced pixels) are twice as wide in output pixels.
      - UE avoids it because TSR accumulates the final lit image on the output grid from the jittered internal frames.
        Lumen's denoiser output keeps each frame's sample detail; it does not hold a second long history of its own on
        the internal grid. Doing it our way would mean the output grid accumulating the composed image (TSR proper),
        a larger restructure. Stopped after the first prototype, as agreed.
    - A second emitter sample at near-mirror camera hits, added to `refl_rays`' extra reflection ray (branch
      `refl-rays-nee`):
      - The trace weights its own emitter sample 1/2 on those pixels.
      - Street after a strafe vs UE50, 3.28 / 2.70 / 1.63 → 3.02 / 2.50 / 1.57, the same as the reflection ray alone.
      - Kabukichō moving +1.07 ms, still +0.12 ms.
      - The two-samples-per-frame ceiling (2.96 / 2.27 / 1.34, § 2c) comes from whole extra path samples, not from
        NEE:
        - more samples per frame with one reflection ray gained 7–12 %;
        - one more reflection ray alone 3–7 %;
        - both together 9–17 %;
        - a second emitter sample nothing.
        A second jittered camera hit and everything behind it is the part that helps.
      - Rendering the street with two samples would cost roughly 3–4 ms, beyond the budget. The wet street's
        +0.5 s line (1.33) stays open; what is left is the research item in § 2c (reprojecting glossy reflections
        so their history survives a strafe).
    - Biased light culling or merging only if the unbiased LOD were not enough. It was enough. Any far light it drops
      would still have to reach the image through the radiance cache, as Lumen keeps far emissive objects in its GI,
      checked against UE.
  - Since then the merge gate reports what pathtrace.wgsl additions cost compiled in but off (docs/WORKFLOW.md).

## 2. Temporal fixes

Targets (2026-11-08): bench-temporal, bench-ghosting and their UE MRQ references move to Bistro night (§ 0 step 7); the
Kabukichō shots below are history.

### 2a. Reflection temporal parameters at UE 5.8 defaults
Reference: `Engine/Shaders/Private/Lumen/LumenReflectionDenoiserTemporal.usf`, `LumenReflections.cpp:153-162`,
`LumenReflectionDenoiserSpatial.usf:92`.
- Neighbourhood clamp always on (not only in motion), in YCoCg, 1σ (`NeighborhoodClampScale` = 1).
- At most 12 accumulated frames (`MaxFramesAccumulated`); at full resolution, specular with roughness < 0.05 drops to 2
  frames (residual noise left to TSR).
- Virtual-image (mirror reprojection) and surface histories are clamped separately; keep the one closer to the
  neighbourhood (`...Temporal.usf` ~l.495).
- Spatial filter radius × saturate(roughness × 8).
- Today `live.wgsl` clamps only in motion, at 2–3σ in RGB, with 8–48 frames of history.
- Also: at long history the live image is the raw accumulated average (all à-trous stages gated off by history length
  since round 3: Cornell box at 256 frames SSIM 0.912 vs the UE Path Tracer, 0.98 on Nov 4). Gate on the remaining
  variance instead.
- Acceptance: bench-temporal motion ΔE / SSIM (vs converged and vs UE) no worse, closing 0.3–2 s better, crops of
  reflective surfaces in motion attached; GPU cost ≤ +0.5 ms at 1280×720.
- Status: done (`d100b57`). bench-temporal at 100 % on Kabukichō vs our converged references: motion move40 / move59
  5.76 / 6.22 → 5.59 / 6.01, stopped 0.5 s 5.34 → 5.03, closing unchanged within noise; GPU +0.23 ms at 60 %. Grain:
  the Cornell box at 256 frames vs the UE Path Tracer SSIM 0.912 → 0.978 (ΔE 1.68 → 1.44): the à-trous stages 0–1 now
  run while the accumulated mean is visibly noisy (the wide stages keep their history limits: +1.9 ms otherwise).
- Deviation from UE's defaults, kept on purpose: the specular neighbourhood clamp applies only while the view moves.
  UE clamps against Lumen's spatially resolved specular; ours comes from nine raw 1-spp samples, and a static 1σ box
  kept resetting converged history (stopped 0.5 s ΔE 6.05 with the static clamp vs 5.03 without; closing 4 s 5.19 vs
  4.10). The 12-frame cap also lifts when the view holds still (UE's TSR does that accumulation; our temporal pass is
  both). `THREERT_TEMPORAL_FLAGS` switches each piece back for experiments.

### 2b. UE motion references
The same camera move (v1: 2.5 m forward and 12° right over 1 s) and the closing sequence rendered in UE (Sequencer + MRQ
at one temporal sample per frame, or the editor's real-time viewport) as a second bench-temporal reference.
- Acceptance: `out/bench-temporal/refs-ue/` for all eight shots; bench-temporal prints vs-UE columns.
- Status: done. `ue5/temporal.ts`: one 60 fps Level Sequence on the UE level (MRQ, 1 temporal sample per frame,
  TSR / Lumen / eye adaptation history as in real time, 3 s lead-in), the camera keyed per frame to bench-temporal's
  `pose(k)`; closing time driven per frame (the Director has no closing function) on the same groups as our "lights"
  handler with the same delay / flicker distribution. `--refs-ue` columns in bench-temporal.
- Measurement fix (2026-11-06): compared directly, every variant sat at the port's floor (our converged frames vs UE
  ΔE 10.7–12.7: our own gate, UE ~10 % darker), and subtracting that floor left only +0.0–0.5 ΔE (errors don't add on top
  of an 11 ΔE difference), too flat to judge by. Now each renderer is measured against its own converged frame: the UE
  sequence also holds poses 20 / 40 / 59 for 3 s (settled frames), and bench-temporal reports `ue_self` (UE's motion frame
  vs UE's settled frame) next to ours (our frame vs our converged reference) and `excess_over_ue` = ours − UE's.
  Baseline (`r5-step1-baseline`): move20 / 40 / 59 ours 5.22 / 5.55 / 5.91 vs UE 2.29 / 2.16 / 2.22; stopped 0.5 s 4.95
  vs 1.12; closing 0.3 / 1 / 2 / 4 s 9.79 / 9.44 / 8.45 / 4.03 vs 10.36 / 8.70 / 5.69 / 2.95. In motion our temporal error
  is ~2.5× UE's, 4.4× just after stopping. Closing 0.3–2 s run through random switch-off orders in both renderers, so
  they vary between runs (closing 2 s: 7.61 and 8.45 in two runs of the same build); judge closing by 4 s.

### 2c. Ghosting and trails while the camera moves (bench-ghosting)
Reported 2026-11-06: trailing (拖影) is very obvious when playing. Measured where it is played: the 0.5
dynamic-resolution floor, 60 Hz camera input.
- `tools/bench-ghosting.ts`, Kabukichō view v1. Three moves:
  - turn 40° away and back (a pure rotation, which reprojects exactly);
  - strafe 3 m at 7.5 m/s and back;
  - a sudden stop after 8 m/s forward.
- Each shot is compared with a converged frame of the same pose, per region (whole frame, the wet street, the sign
  band, a facade). Metrics: mean dE, p95, p99, and `trail_px` (p95 over rows of the longest horizontal run of pixels
  off by dE > 10; `threert-render compare --region`). Each is reported as excess over the settled frame, the live
  noise floor.
- Animation is frozen (`THREERT_FREEZE_TIME`: shader time and the frame script's clock, so signs do not switch between
  the shot and its reference) and exposure snaps to its target (`THREERT_EXPOSURE_SNAP`), so neither reads as error.
- Findings (0.5 floor):
  - The trail that jumps out during fast turns is the motion blur. With UE's real-time defaults (on in Kabukichō
    since § 5 item 4) the sign band mid-turn is +7.2 dE over the floor (p95 +41), +11.5 mid-strafe: neon text smears
    unreadably. With it off: +0.4 / +3.2.
  - Decided (2026-11-06): keep UE's default. The supervisor's rule was "on if ours matches UE". It does.
    - `node tools/bench-motion-blur.ts` measures what blur adds to the sign band of move v1 in each renderer, the same
      frame with vs without it. UE: MRQ refs-ue-mb vs refs-ue. Ours: lockstep, 1/60 s frames.
      - Ours over UE, dE: 0.96 / 1.12, 0.83 / 1.00, 0.75 / 0.95 at frames 20 / 40 / 59, ratio 0.79–0.86.
      - p95 is a little lower; the trail is the same 3 px.
      - Ours is slightly weaker than UE's, not stronger.
    - bench-temporal `--motion-blur --region signs` (excess_over_ue, sign band, 100 %, wall-clock paced):
      - 2.87 / 2.91 / 2.60 with blur on both sides;
      - 3.26 / 3.29 / 2.81 with it off;
      - both within the mid-move limit of § 2c (3.2).
  - UE reference for the strafe (2026-11-06). `MOVE=strafe bun ue5/temporal.ts` (MB=1 with blur) renders
    bench-ghosting's strafe: 3 m right in 24 frames, 30 held, back in 24, then the midpoint held 3 s. It is MRQ at
    60 fps, 1 temporal sample per frame, 100 % screen percentage, 26 s.
    - `bench-ghosting --quick --ue out/bench-temporal/refs-ue-strafe` (`--mrq-blur --ue …-mb`) puts each renderer's
      moving frame against its own converged frame of the pose, per region.
    - UE's own floor (two still frames 10 apart): 0.63 dE overall, 0.54 ground, 0.76 signs, 0.31 facades.
    - At 50 % screen percentage, as ours runs at the floor (`MOVE=strafe bun ue5/temporal.ts <project> 50` ->
      refs-ue-strafe-sp50): street excess 2.61 / 1.92 / 1.03 at mid / +0.1 s / +0.5 s (floor 0.45), against 2.15 /
      1.28 / 0.50 at 100 %.
    - Excess over each renderer's floor, motion blur off, ours / UE:

      | shot | all | ground | signs | facades | ground trail |
      |---|---|---|---|---|---|
      | strafe-mid | 2.81 / 2.38 | 3.82 / 2.15 | 2.94 / 3.57 | 1.04 / 1.36 | 65 / 49 px |
      | return+0.1s | 2.06 / 1.16 | 2.79 / 1.28 | 1.99 / 1.63 | 0.69 / 0.39 | 62 / 38 px |
      | return+0.5s | 1.11 / 0.34 | 1.65 / 0.50 | 0.87 / 0.36 | 0.44 / 0.12 | 18 / 36 px |

    - The sign band mid-strafe is closer to converged than UE's: UE's TSR softens the whole frame while it moves,
      ours stays sharp. After the return ours takes longer to settle, about 0.5 dE more in the sign band at +0.5 s.
    - The wet ground is the gap: 1.7 dE over UE mid-strafe and 1.2–1.5 after the return, the same reflections item
      as below.
    - Motion blur at the strafe midpoint adds +0.23 dE overall / +0.77 in the sign band to ours and +0.57 / +1.31
      to UE's. So ours adds about half of what UE's does here (0.79–0.86 on move v1). Part of that is UE's softer
      frame without blur. Crops (`docs/img/strafe-mid-signs-ours-ue.jpg`: ours off / on, UE off / on): ours shows
      little visible blur at 7.5 m/s. A direct blur-length measurement is needed before changing ours.
  - Blur length, measured (2026-11-07). The 居酒屋 sign's left edge at the strafe midpoint moves 7.3 px per frame:
    88 px over UE's 1.5 m of travel between its settled start and midpoint frames. With MRQ's 0.5 shutter that is a
    3.7 px smear.
    - Measured with `threert-render edge-width <png> 353 379 70 200` (red channel, 10–90 % rise across the edge,
      median of 130 rows), blur off → on:

      | | off | on | added | added in quadrature |
      |---|---|---|---|---|
      | ours, 0.5 floor | 1.50 | 2.26 | +0.76 | 1.69 |
      | UE, 50 % | 2.83 | 3.58 | +0.75 | 2.19 |
      | ours, 100 % | 1.49 | 2.23 | +0.74 | 1.66 |
      | UE, 100 % | 2.61 | 3.53 | +0.92 | 2.38 |

    - At the floor the game runs at, our blur adds as much as UE's. Taken in quadrature, since UE's edge is softer
      to start with, ours is 23 % shorter.
    - At 100 %, ours is 20 % shorter by the direct difference and 30 % in quadrature.
    - Under the 25 % rule at 0.5, borderline at 100 %: unchanged. If blur comes up again, a synthetic bar at a known
      speed in both engines would separate the edge's own softness from the blur.
  - The wet street after a strafe (2026-11-07). Ours vs UE at 50 % screen percentage, as both run at the 0.5 floor
    (UE at 100 % in brackets); street excess at mid / +0.1 s / +0.5 s:
    - Before: 3.73 / 2.81 / 1.65 vs 2.61 / 1.92 / 1.03 (2.15 / 1.28 / 0.50). Acceptance for this item: +0.5 s ≤ UE50 +
      0.3 (1.33), the mid and +0.1 s gaps at least halved, the sign band and still-frame noise no worse.
    - Where it is lost:
      - All of it is specular: without specular the street's excess is 0.44–0.55.
      - Nothing stale is left: after the strafe the street converges exactly like a fresh start with history reset
        (dE vs converged at 6 / 18 / 30 / 60 / 120 frames: 3.63 / 2.81 / 2.45 / 2.05 / 1.73 after it, 3.44 / 2.90 /
        2.52 / 2.11 / 1.78 fresh; UE50 after it 2.37 / 1.78 / 1.48 / 1.13 / 0.93). The 4-frame hold while the view
        translates is a restart.
      - It is noise: two independent fresh runs differ from each other as much as from the reference (2.77 at 30
        frames). It falls slower than 1 / √N (×0.68 from 30 to 120 frames), as the spatial filter is strongest on
        young history.
    - What did not help (street excess mid / +0.1 / +0.5 against 3.81 / 2.79 / 1.66 then):
      - Longer specular history while translating (8 or 12 frames at 4σ; 12 at 2σ or 1σ; no hold): all worse,
        4.0–4.2 / 2.8–3.6 / 1.9–2.2. Glossy reflections do not reproject under translation, and the kept history
        smears.
      - 4 samples per frame: −10 %. Twice the resolve neighbours or radius, a filter twice as strong: under 0.1.
      - The radiance cache's total at reflection hits: −10 %.
      - Tracing at 100 %: −24 % at 30 frames fresh (GPU cost not yet measured).
    - Done: mirror-image reprojection through the resolve's BRDF-weighted hit distance on near-mirror surfaces
      (roughness < 0.2). It was a pixel's own single lobe sample, so the mirror image landed anywhere along the ray.
      - Street: mid 3.73 → 3.28 (the gap to UE50 1.12 → 0.67), +0.1 s 2.81 → 2.70; +0.5 s unchanged (1.63).
      - Trail 66 → 49 px; sign band unchanged (on the signs, roughness 0.22, it cost +0.1 dE: hence the limit).
      - Crops: `docs/img/strafe-mid-street-resolved-distance.jpg` (main | this | converged).
    - Tried next, not merged (2026-11-07):
      - A variance-guided spatial filter for young specular history (σ = 4 × the local standard deviation; also with
        doubled steps or a 16-frame threshold) made it worse: 6 frames after a reset 3.42 → 3.71, and two
        independent runs then differed by 4.19 instead of 3.62. The street's noise is sparse bright speckle, the
        signs' reflections, and a wider filter smears each speckle into a blob instead of averaging it away.
      - More reflection rays. The ceiling, with every sample of a frame giving its reflection ray to the resolve
        (rays per traced pixel 1 / 2 / 4, street excess mid / +0.1 / +0.5 s): 3.27 / 2.67 / 1.62, 2.96 / 2.27 / 1.34,
        2.71 / 1.95 / 1.17 (UE50 2.61 / 1.92 / 1.03). Tracing everything at 100 % instead costs Kabukichō 45–48 ms
        (the path-tracing kernel 34–36 ms against 8.5–8.9).
      - A pass of its own, `refl_rays` (branch `glossy-refl-eval`):
        - What it does: one more specular-lobe ray per near-mirror traced pixel (roughness < 0.2) while the view
          translates and for 1 s after. It starts from the trace pass's g-buffer, and its hit is shaded from the
          radiance cache's total or, as an option, with hit lighting.
        - Street: 3.28 / 2.70 / 1.63 → 3.04 / 2.55 / 1.58; with hit lighting 3.02 / 2.51 / 1.55.
        - Cost: Kabukichō moving +0.56 ms (the pass 0.39, the resolve +0.12). Still frames and the path-tracing
          kernel are unchanged.
        - Why it falls short: the ceiling above doubled more than the reflection rays; every sample also brought its
          own emitter sampling, whose specular part (signs reflected through NEE) is the street's other noise
          source. Halving either alone gains 3–10 %, both together 16–30 %: the two variances add.
        - Its +0.5 s stays at 1.58, short of the 1.33 acceptance line, so it is not merged.
        - The option left: a second emitter sample at near-mirror camera hits in the same pass. It needs the camera
          hit's surface (a primary ray and a material evaluation), an estimated +0.6–1.0 ms more, so Kabukichō
          moving would be ~16.6–17.0 ms.
      - UE 5.8's own temporal rules for reflections (`LumenReflectionDenoiserTemporal.usf`), applied while the view
        moves (2026-11-07):
        - What changed: no hold after translating; 12 frames; a clamp at 1 σ, doubled for samples reconstructed
          from neighbours (all of ours below 100 %), also tried at 4 and 8 σ; the reflection-hit history only for
          narrow lobes (`GetSpecularDominantDirFactor` > 0.5). Still frames kept accumulating, uncapped.
        - Street excess vs UE50, mid / +0.1 / +0.5 s: 3.59–3.75 / 3.14–3.20 / 2.07–2.08, against 3.28 / 2.70 / 1.63
          with the hold.
        - Trails get longer: turn-mid street 65–108 px (65 with the hold), stop+0.1s 48–59 px (30). bench-ghosting
          `--check` fails.
        - Why: without the hold, the history a translation misplaces survives (neither reprojection is exact for a
          glossy lobe). Our samples (1 spp at the 0.5 floor, one lobe sample each) are too noisy for a σ box to
          reject it: the box admits the misplaced history, which smears. UE clamps against its spatially resolved
          reflections and TSR finishes after it.
        - The hold stays. Temporal rules alone do not reach the +0.5 s line at our sample density.
    - Research, later: reprojecting glossy reflections correctly under translation, so that their history survives
      a strafe. Planar-mirror reprojection is exact for mirrors only; a glossy lobe's reflection moves somewhere
      between surface and mirror image depending on roughness and distance.
    - Not to be done: capping the specular history while the view holds still, as UE's ~12 frames effectively do.
      bench-ghosting measures each renderer against its own converged frame. UE gets back to its steady state within
      ~0.5 s, but that steady state is noisy (two still frames differ by 0.45 on the street). Ours converges for
      seconds toward a cleaner image. A cap would bring our metric down by making every still image noisier.
  - Reflections: the wet street is 3–4 dE off while moving and after a stop, and almost all of it is specular
    (diffuse +0.1–0.2).
    - A single frame of reflections at quarter resolution is very noisy (dE 5.4 vs 0.58 converged), so they need
      many aligned frames.
    - A rotation reprojects them exactly (one jump, history only: dE 1.07).
    - A translation does not, for glossy reflections: under a strafe they are back to single-frame quality.
    - History that lands wrong passed the loose clamp (8σ below 100 %) and smeared into streaks that stayed after a
      stop, because the specular clamp switched off with the motion.
  - No effect: Catmull-Rom instead of bilinear specular history, counting only disocclusions as fresh history,
    picking one of the surface / mirror-image histories instead of blending them, a 4σ box everywhere.
- Fix: for 8 frames after the view last translated (the stop included), the specular history is clamped at 4σ and
  held to 4 frames; a rotation keeps the long history (`THREERT_TEMPORAL_FLAGS=16384` switches it off). Excess mean dE /
  trail, old → new:
  - strafe mid, street: 4.10 / 79 px → 3.82 / 58 px;
  - strafe back + 0.5 s, whole frame: 1.47 → 1.13;
  - stop + 0: 3.85 / 68 px → 3.24 / 38 px;
  - stop + 0.1 s: 3.19 / 40 px → 2.69 / 26 px;
  - stop + 0.3 s: 2.57 → 2.10;
  - turn mid unchanged (2.94 → 2.88).
  - After a stop the smeared streaks are gone (crops: `docs/img/ghosting-stop.jpg`, old | new | converged).
- Costs nothing: Kabukichō at the 0.5 floor 15.04 ms still, 15.86 moving. bench-temporal at 100 %: excess move40 /
  move59 / stopped 1.75 / 1.75 / 2.01 → 1.65 / 1.69 / 2.09.
- Middle loop: `node tools/bench-ghosting.ts --quick --check` (0.5 floor, references from 256 frames at that scale; every
  move five times, medians checked; ~75 s, ~100 s when a new build renders its references). It fails above these limits (excess mean dE / p95 dE / trail px),
  which the previous renderer fails on the strafe and stop shots:

  | shot | region | mean | p95 | trail |
  |---|---|---|---|---|
  | turn-mid | street | 3.2 | 12.5 | 80 |
  | strafe-mid | street | 4.1 | 17.5 | 75 |
  | turn-return+0.5s | all | 1.3 | 4.2 | 30 |
  | strafe-return+0.5s | all | 1.35 | 4.5 | 28 |
  | stop+0.1s | street | 2.95 | 11.2 | 35 |

- Made load-independent (2026-11-06), because it is a merge gate now. Shots and waits were timed by the wall clock and
  camera commands paced at 60 Hz, so a busy machine (the user working on it) rendered fewer frames per move and per
  wait. The return shots then caught less converged history, and identical code read 0.97–1.87 excess
  (turn-return), failing the check about half the time.
  - The camera now steps once per rendered frame (`live` `sync` command: answers after the n-th frame).
  - Shots and waits are frame counts (`snapshot` `frames`): +0.1 / 0.3 / 0.5 / 2 s = 6 / 18 / 30 / 120 frames;
    startup 150 frames, settle 240.
  - Mean dE is then stable to ~0.1 between runs, but `trail_px` (a longest-run statistic) still moves by ±20 px per
    shot (turn-mid 52–96 px on one build). The first turn after startup often reads ~96 px.
  - `--quick` therefore does every move five times and checks the median: 75 s instead of ~50.
  - Lockstep (2026-11-06): commands still raced the frames. A snapshot or sync sent right after a camera step could
    land in the next frame's batch, capturing a frame the camera had not moved in (no blur, one more frame of
    history), and the free-running renderer could hold or merge steps.
    - `live` now has a lockstep mode (`{"cmd":"lockstep","dt":1/60}`): it renders exactly the frames the commands
      ask for, at that frame time.
    - A camera step carries its sync / snapshot in the same command.
    - bench-ghosting and bench-motion-blur run in it, and two runs of either now agree to 0.01 dE.
    - The five repeats of a run still differ from each other (each starts from another history state), so the median
      stays.
    - stop+0.1s reads 2.84 now (2.62 before): the old timing included held frames after the stop.
  - Medians on a busy machine, all checks passing:

    | shot | excess | trail | limits (excess / trail) |
    |---|---|---|---|
    | turn-mid | 2.85 | 69 px | 3.2 / 80 |
    | strafe-mid | 3.73 | 63 px | 4.1 / 75 |
    | turn-return | 0.94 | 20 px | 1.3 / 30 |
    | strafe-return | 1.06 | 21 px | 1.35 / 28 |
    | stop+0.1s | 2.62 | 25 px | 2.95 / 35 |

    Three runs with three repeats gave turn-mid trail medians of 60 / 75 / 64 px.
- Next (the remaining 3–4 dE on the street in motion): reflections with fewer, better frames. Either a stronger
  spatial resolve at quarter resolution (more and wider neighbours for glossy surfaces while the view moves), or
  reflections traced above the diffuse resolution. A UE reference of the same moves at 50 % screen percentage to
  calibrate the limits against.

### 2d. The diffuse history's reprojection (2026-11-08, branch `dtemporal-centre`)

- Found while tracing the live image's dark mid-tones (§ 0, live energy). Measured in linear HDR: Bistro's first pose,
  live at the 0.5 floor after 512 lockstep frames, against the offline render at 1024 spp. 8 x 8 blocks were binned by
  the reference's luminance and split by whether directly seen emission makes up more than 30 % of the block.
  - Emitter blocks read 0.996–1.0. Lit surfaces were darker the brighter they were: 0.1–1 nit 0.908, 1–10 nit 0.732,
    10–53 nit 0.458. The worst blocks were the facade around a lantern 31 m away: the offline render shows a warm glow
    there of up to 19.6 nits, nearly all of it direct light (18.3 with one bounce); live showed 6.9.
  - Ruled out, each made no difference: the à-trous and specular filters, the reflection resolve (some), the firefly
    cut (some), the emission clamp past glass, the bounce count (2 / 4 / 8), the emitter cull, the light LOD, the
    radiance cache, opaque cutouts on secondary rays, and 512 vs 2048 frames. The undenoised history read as low as
    the final image.
  - It depended on screen size: 3.5 m from the same lantern live read 0.90–0.95. An offline render at half
    resolution matched full resolution in every bin.
  - Cause: `dtemporal` fetched the traced grid's history where the jittered sample's hit point was the frame before.
    That is a bilinear resample at a random sub-pixel offset every frame, even holding still. The blur grew with the
    history: light bled from lit surfaces into the shadow beside them (dark blocks up to 9x too bright), and a small
    lamp's steep glow flattened.
- Done: once a pixel has HIST_CENTRE (16) frames of history, its history is fetched where the pixel's centre was,
  moved by the sample's motion. A young history keeps the old blur, which denoises it: always fetching at the centre
  read Bistro ΔE 5.43 at 16 frames against 4.34 before. Thresholds 8 / 32 read worse at 16 / 64 frames.
  - Layer B real scenes (ΔE / SSIM against the offline render, live at 0.5 after a reset; frames 16 / 64 / 256):

    | | before | after |
    |---|---|---|
    | Bistro | 4.34 / 0.776, 3.89 / 0.799, 3.99 / 0.806 | 4.34 / 0.776, 3.72 / 0.806, 3.44 / 0.819 |
    | Zero Day | 3.73 / 0.790, 3.77 / 0.790, 4.11 / 0.789 | 3.73 / 0.790, 3.46 / 0.805, 3.27 / 0.818 |

    Before, both scenes got worse after 64 frames as the blur grew; now they keep converging.
  - Linear radiance (512 frames): lit 1–10 nit 0.732 -> 0.785, 10–53 nit 0.458 -> 0.723. Shadow beside lit surfaces
    no longer gains bled light: the 0–0.1 nit bin reads 0.992 -> 0.931, because before, the bleed had offset losses
    elsewhere in that bin.
  - C slice against the offline render, ΔE: 5 s 4.43 -> 4.34, 20 s 4.49 -> 4.52, 41 s 6.06 -> 5.96, 46 s 4.36 ->
    4.25, 55 s 4.34 -> 4.23 (SSIM up in every segment).
  - Ghosting quick check, Bistro (excess / trail): turn-mid 3.55 / 23 -> 3.37 / 22, strafe-mid 5.38 / 33 -> 5.15 / 32,
    turn-return 1.52 / 18 -> 1.34 / 13, strafe-return 1.55 / 18 -> 1.43 / 14, stop 4.56 / 24 -> 4.47 / 27. Kabukichō
    ok.
  - Other gates: perf smoke within 0.1 ms; `test:render` 90 / 90.
  - Left: the lit 1–10 nit blocks at 0.785 (0.85 at full resolution) and 0.1–1 nit at 0.91. The traced grid's
    upsampling and the young-history blur are next to look at.
- Follow-up (2026-11-08, branch `dtemporal-normals`): the centre fetch made histories restart.
  - Holding still, the centre lands on one history pixel exactly. Its stored normal is another jittered sample's, and
    on normal-mapped stones and facade detail it often failed the 0.8 cosine test, so the history restarted. The
    history-length view at 256 frames showed Bistro's street speckled and its facades at 8–30 frames; before #78 the
    street was uniform (bilinear between four pixels, one of which usually passed).
  - Done: the centre fetch accepts 0.5 (depth test unchanged).
  - Also tried: falling back to the sample's own position. It was better on Bistro but brought back the resampling
    blur on Zero Day (+0.06 ΔE at 1024 frames).
  - Also tried, on top: when none of the four pixels shows the surface (foliage, where the jitter alternates leaves
    and what is behind them), take the history of the nearest 3 x 3 pixel that does. Bistro at 256 frames 3.27 ->
    3.14, but Zero Day 3.28 -> 3.33 (SSIM -0.007). Limiting it to cutout surfaces needs a cutout flag in the samples,
    which they do not carry yet.
  - Layer B real scenes, frames 64 / 256 / 1024, ΔE / SSIM: Bistro 3.72 / 0.806, 3.44 / 0.819, 3.45 / 0.813 ->
    3.62 / 0.812, 3.27 / 0.828, 3.23 / 0.827; Zero Day 3.49 / 0.804, 3.30 / 0.818, 3.20 / 0.824 -> 3.47 / 0.805,
    3.28 / 0.819, 3.18 / 0.825.
  - Ghosting quick check ok on both scenes. Bistro's return shots read 1.34 / 1.43 -> 1.47 / 1.58 against their own
    converged frames, which improved too. Against the offline render the return shot itself got better
    (turn-return+0.5s ΔE 4.35 -> 4.22, luminance 0.849 -> 0.865), as did the settled frame (3.47 -> 3.27).
  - Other gates: perf smoke within 0.11 ms; `test:render` 90 / 90.
  - Measurement note: a "frames after a reset" snapshot is only fair if every accumulation honours the reset. A
    per-output-pixel raw diffuse accumulation (PR #79, closed) did not; it read back the 6 s warm-up and faked
    a 0.4 ΔE gain. Measured honestly, it gained nothing over these histories.

## 3. Rendering test suite (Test262-style)

- **A, atomic tests:** 60–100, one feature each. Header metadata: id, features, reference, metric, temporal.
  Result: pass / fail / unsupported. ~300 ms each; the whole layer under one minute.
- **B, composite scenes** (2026-11-08): the Khronos glTF Sample Assets, Bistro night and Zero Day.
  Earlier the list was Khronos, Bistro, Kabukichō.
- **C, a GTA-like slice:** one block with day / night, rain, a drivable car, pedestrians, foliage, a waterfront and an
  explosion; a 60 s fixed-camera shot. Output: a side-by-side video with UE plus per-segment metrics. Since 2026-11-08 it
  is built on Bistro (§ 0 step 9); the Kabukichō version below (part 1) is history, its general fixes kept.
  - Status, part 1 (2026-11-08): our side of the shot, `games/slice/slice.game.ts`, without day / night yet.
    - What it is: Kabukichō Ichiban-gai at night seen across a canal cut into Yasukuni-dori in front of the gate.
      The Kabukichō game builds the street and runs its frame script; the slice adds the waterfront (a `WaterBody`
      between embankments, railings), potted plants in the wind, four walking mannequins, and an explosion (fireball,
      sparks, debris, rising smoke).
    - Timeline: taxi at 10 s, rain at 15 s, explosion at 40 s, 60 s shot. It is a function of the frame script's clock
      that hands Kabukichō's frame script the state its handlers would set; lockstep captures repeat exactly.
    - `docs/img/slice-shot.jpg`: 5 / 12 / 20 / 40.8 / 44 / 55 s.
    - Live at the 0.5 floor over the 60 s:
      - GPU 18.0–18.5 ms a frame (profile total), interval p50 18.1–19.4 ms (52–55 fps), one frame over 50 ms (the
        explosion's first frame); frame script 6.7–7.3 ms.
      - With the movable plants as TLAS instances (§ 5 item 1): GPU 17.0–17.4 ms, interval p50 17.1–18.3 ms (55–58
        fps), BLAS rebuild 2.1 ms. Still ~0.5 ms over in the rainy segments.
      - Over budget by ~1.5 ms. Against Kabukichō alone (15.2–15.5): the moving triangles' BLAS rebuild +2.7 ms
        (3.1 against 0.42; ~100 k triangles: four skinned walkers, four swaying plants, debris, the taxi), the trace
        +0.6, the waterfront ~1.2 of it (water surface and body).
    - Found on the way:
      - `Object3D.lookAt` recomputed the whole scene's world matrices: 12 ms per call on Kabukichō (the frame script
        19.0 → 6.8 ms). It now updates the ancestors only, as three.js does.
      - Dynamic BLASes are built with PREFER_FAST_BUILD: 4.14 → 3.1 ms, trace unchanged. wgpu 30 has no BLAS refit
        ("only rebuild implemented").
    - Stopped on 2026-11-08: the slice moves to Bistro (§ 0 step 9). Not pursued on Kabukichō: day / night (a WIP branch
      `sun-move`, sun moves without a rebuild, is kept unmerged), the walkers' BLAS rebuild, the UE side.
  - Per-segment metrics on Bistro (2026-11-08, branch `slice-eval`; our offline render as the truth, "How we are
    judged"): `node tools/slice-eval.ts`. Live plays the shot in lockstep (dynamic resolution) with snapshots at 5 / 20
    / 41 / 46 / 55 s; the truth is an offline render of each moment (1024 spp, no clamps, no denoiser, cached in
    `out/cache/truth`). Writes `out/slice-eval/{metrics.json, montage.jpg}`.
    - Found on the way: an offline render at a scene time (`render --time t`) ran shaders and sway at t but not the
      game's frame script, so the slice's walkers, smoke and rain stood at 0 s in every frame. The frame runner now has
      `seek(t)` (its clock set, the scene shown settled at that moment); `render` seeks to the job's time (0 without
      one), serve's live jobs to their first frame. Renders without a time are byte-identical (the four catalog games
      with frame scripts); bundles built before `seek` skip it.
    - At c690e06 (live with 2 tree candidates): ΔE / SSIM / luminance 5 s 4.64 / 0.743 / 0.85, 20 s (rain) 4.78 /
      0.735 / 0.83, 41 s (explosion) 6.02 / 0.706 / 0.98, 46 s (smoke) 4.50 / 0.739 / 0.88, 55 s 4.53 / 0.739 / 0.86;
      GPU p50 15.9–16.1 ms. Above the still scene (3.9) by the moving content's short histories and the truth's own
      noise at 1024 spp (Bistro: ΔE 1.8). Live's smoke reads denser and brighter than offline (Known parity gaps).
- A "feature × pass rate" table, run once per release.
- Reference source: test assets are glTF + a small JSON sidecar, imported into UE via Interchange. Only for test assets:
  games and scenes stay code; no scene IR comes back.
- Status: A layer skeleton running (`npm run test:render`, `tests/render/*.ts`): 21 tests (light units,
  Lambert, furnace, Fresnel, shadows, sky light, the sky's horizon, sRGB decode, fog transmittance, rect / point / spot /
  directional, skinning, the live radiance cache, glass transmission, clearcoat): 20 / 21 pass + 1 depending on an asset
  (animation/skinned-pose needs the Quaternius mannequin, assets/quaternius/ual/UAL1_Standard.glb, which the default
  workspace's asset root does not have yet; with it, 21 / 21), under 3 s for the layer. Tests can run in the real-time
  renderer (`live: frames` in the test metadata).
- Layer B started (2026-11-06): three Khronos glTF Sample Assets.
  - The models: TransmissionTest, ClearCoatTest, MetalRoughSpheres (`node tools/fetch-khronos.ts`), as
    `scenes/khronos-*.ts`.
  - The setup: the plaza's neutral daylight studio (sun + sky atmosphere, grey ground, EV 14.6), which three.js, UE and
    ThreeRT all express.
  - UE 5.8 references come from `node tools/ue-glb-scene.ts`: Interchange scene import, path tracer + Lumen through
    MRQ, ~2 min per scene. Reading the GLBs needed KHR_materials_transmission / _ior / _clearcoat factors in the
    renderer, and grey + alpha PNGs decoded as such.
  - three.js r187 comes from bench-three as for the other scenes.
  - `npm run test:layer-b` judges them against the UE Path Tracer. Pass: ThreeRT offline and live @256 frames both
    ΔE ≤ 5 and SSIM ≥ 0.90; three.js is the baseline.
  - Results, ΔE / SSIM vs UE Path Tracer (ThreeRT is 1.4–1.8× closer to UE than three.js on all three):

    | scene | three.js (best tonemap) | ThreeRT offline | ThreeRT live @256 | UE Lumen | |
    |---|---|---|---|---|---|
    | ClearCoatTest | 7.27 / 0.929 | 4.22 / 0.964 | 4.41 / 0.947 | 1.85 / 0.976 | pass |
    | MetalRoughSpheres | 9.00 / 0.872 | 4.92 / 0.921 | 4.85 / 0.912 | 3.03 / 0.923 | pass |
    | TransmissionTest | 7.84 / 0.892 | 5.32 / 0.929 | 5.60 / 0.922 | 3.21 / 0.931 | fail |

  - What separates us from UE there:
    - All three, before 2026-11-06: the horizon. Beyond the 2 km ground plane, a fraction of a degree below the
      horizon, ours was black; UE's path tracer fades there over one texel of its sky light's cubemap (below). With
      that: MetalRoughSpheres offline / live ΔE 5.01 / 4.97 → 4.92 / 4.85 (passes), ClearCoatTest 4.32 / 4.43 →
      4.22 / 4.41, TransmissionTest 5.73 / 5.75 → 5.69 / 5.79.
    - UE's MetalRoughSpheres are faceted (its import of the GLB doesn't keep the smooth normals); ours are smooth.
      That costs us more ΔE there than anything left in our renderer; not ours to copy.
    - TransmissionTest (2026-11-08): the `transmissionTexture` is read now (it stripes the red spheres, as in UE), and
      shadows through glass are tinted by the glass's colour offline, as UE's path tracer tints them. Offline ΔE / SSIM
      / PSNR 5.66 / 0.918 / 23.59 → 5.32 / 0.929 / 24.89 dB, live @256 5.67 → 5.60. What is left is mostly not glass:
      the top third of the frame (the sky) reads ΔE 6.7, the middle 5.9, the bottom 3.1. Live shadow rays still pass
      glass unattenuated.
  - Feature × pass rate, layers A + B (`npm run test:layer-b` prints it): clearcoat A 2 / 2, B 1 / 1; transmission
    and refraction A 1 / 1, B 0 / 1; fresnel A 3 / 3, B 1 / 1; metal-rough B 1 / 1; sky-atmosphere A 2 / 2; every other
    layer-A feature passes; animation / skinning 0 / 1 (asset missing).
  - The Khronos models sit outside the default asset root for now, like the mannequin (§ 3 status above, waiting on
    the asset-root decision): `--assets` points the tools at them.
- Layer B, real scenes (2026-11-08, branch `layer-b-real`; ROADMAP "How we are judged"): Bistro night and Zero Day,
  ThreeRT live (the 0.5 floor, 64 frames after a history reset, the scene's camera held still) against our unbiased
  offline render (no clamps, no denoiser; Bistro 4096 spp, Zero Day 16384), in `npm run test:layer-b` after the
  Khronos rows (`--no-real` skips them).
  - The truth renders are cached in `out/cache/truth` by bundle content and `TRUTH_VERSION` (bump it when the offline
    estimator changes what it converges to): once, ~5 / ~15 min. Then a run takes ~19 s.
  - Limits: a regression guard, live not being within the composite scenes' absolute bar. Set at main e98236f, whose
    runs repeat exactly (Bistro ΔE 3.89 / SSIM 0.799, Zero Day 3.77 / 0.790), at + 0.1 ΔE / - 0.005 SSIM. The build
    before #69 and #70 (4.12 / 0.791, 3.91 / 0.785) fails both.

### 3a. Standard material library (2026-11-06)
- `packages/scene/src/materials.ts`: nine Three-like constructors (`WetGroundMaterial`, `FacadeMaterial`,
  `TriplanarMaterial`, `PaintMaterial`, `NeonSignMaterial`, `LedScreenMaterial`, `LightPanelMaterial`,
  `ShopWindowMaterial`, `RoadMarkingMaterial`), each one of Kabukichō's nine master materials taken line for line as a
  MaterialFunction. Metres and three.js parameter names; neutral maps bound for empty slots; `set()` for later edits.
- Kabukichō builds its materials from them. Offline renders at three viewpoints match the previous ones within the
  renderer's own repeat noise (dE 0.01 / 0.02 / 0.04, the same as rendering the old bundle twice). GPU unchanged (0.5
  floor 14.94 ms still, 15.90 moving).
- Layer A: six atomic tests (`std/*`):
  - sign emission = intensity;
  - a blinking light's off phase is dark;
  - LED screen picture = intensity;
  - a road-marking mask cuts out;
  - grime darkens paint by 30 %;
  - dry wet-ground = the standard material.
  Layer A 25 / 25.
- Built-in material fields moved into the library (2026-11-06). `wet`, `puddles`, `grime`, `windows` and `mapping`
  are `SurfaceMaterial`: the kernel's code moved line for line into a material function, with its noise and hash
  helpers. `emissiveAnimation` maps onto `NeonSignMaterial` (flicker, scroll, new `blink`), `LightPanelMaterial` and
  `LedScreenMaterial`.
  - Removed from the renderer core: the fields on `MeshStandardMaterial` and in the engine's material description,
    96 bytes of the GPU material (288 instead of 384), five kernel feature bits (no wet-street kernel variant
    compiled when `scene.wetness` first rises above 0), and 276 lines of WGSL (17 added in their place).
  - Material functions get `i.near` (camera hit or mirror-like reflection), so library code can keep fine detail
    where it shows, as the kernel did.
  - Pinned first: five layer-A tests measured on the built-in code (world mapping, wet, puddles, grime, windows).
    They pass unchanged on the library port, to every printed digit (`surface/*`).
  - Combined check beyond the pins: scanned maps with normal maps, world-mapped, wet ground with puddles and rain
    ripples (t = 1.3 s), grime and windows on two walls. The old fields on the old build and `SurfaceMaterial` on the
    new one render the same image (PSNR 95 dB, dE 0.0).
  - Found on the way, in the old code as well as the port: lit windows seen in a mirror or glossy reflection with no
    other emitter in the scene came out at ~1e31 nits. The BSDF hit's MIS weight divided by the total emitter power,
    which is 0 when only material functions emit. Fixed (the BSDF ray gets the full weight);
    `surface/windows-mirror` guards it.
  - Two more tests: a sign's scroll at t = 1 s and its blink (`std/sign-*`). Tests can set the scene time (`time`).
    A ratio test whose reference region is black now fails instead of passing 0 / 0. Layer A 33 / 33.
  - Kabukichō doesn't use the fields. Renders at three views match the previous build within its repeat noise
    (dE 0.01 / 0.03 / 0.04). GPU at the 0.5 floor is unchanged: 14.88–14.89 ms still, 15.64–15.69 moving (the
    previous build in the same runs: 14.86–14.90 / 15.62–15.65).
  - Left as it was: rain ripples on a material function don't mark pixels as animated (the kernel's built-in did),
    so the live history smooths them, as it already did on `WetGroundMaterial`. A `MatOut` flag for animated
    non-emissive output would fix both.

## 4. Dev loop (the 300 ms rule)

- Throughput benchmarks size their load from a time budget so one measurement takes ~300 ms. Hitches are measured
  separately: frame-time p95 / p99 and spikes (first compile of a shader variant, the 0.7 s structural rebuild).
- Drop warm-up frames, use GPU timestamps, cache reference images by scene hash + renderer version, never benchmark
  while the GPU runs something else.
- Three loops: inner < 1 s, middle 30–60 s, outer minutes. Every benchmark prints one summary line "×N vs baseline".
  Middle loop, every change to the live passes: `node tools/bench-ghosting.ts --quick --check` (§ 2c).
- Gate tiers (2026-11-07, [WORKFLOW.md](WORKFLOW.md)): before merging, each PR runs only its tier's checks (D docs,
  T tools, R1 feature-gated renderer, R2 changes existing scenes, R3 hot path or perf claim) plus a ~30 s perf smoke
  against the cached main baseline. Full A/B timing only for R3. The full perf suite runs on main every ~3 h or after
  4 renderer merges, with a bisect; the UE-parity suite every ~12 h. Next tooling: `tools/perf-smoke.ts`,
  `tools/perf-suite.ts`.
- Status: started. `tools/lib/bench.ts` (quiet-GPU guard, renderer-version reference cache, p95 / p99 / spikes,
  time-budgeted runs, "×N vs baseline" lines in out/bench/baselines.json); `tools/bench-hitches.ts` (frame-time
  percentiles while game events play, spikes attributed to events; `live --frame-log`). First finding: the live loop
  drained the GPU twice a second to time a frame (a 53 ms hitch every 0.5 s on Kabukichō); GPU time now comes from
  timestamp queries read back without waiting. Two more pacing faults followed: a fixed 60 Hz sleep made a GPU-bound
  loop alternate long and short frames, and the two-frames-in-flight limit waited for all GPU work (the GPU idled while
  the next frame recorded). Kabukichō, dynamic resolution, events lights / mood / rain / viewpoint: frame p99
  44.7 → 26.7 ms, interval p50 = GPU time 19.4 ms, no spikes (`bench-hitches`).

## 5. GTA gaps, in order

1. Movable nodes — done (2026-11-06; TLAS instancing not needed so far).
   - `Object3D.movable` is UE's Mobility "Movable": scripts move the object, and everything under it, every frame
     without rebuilding the scene. Its triangles go to the dynamic BLASes with the posed models, and a move
     re-transforms them on the CPU, uploads the touched span and rebuilds the dynamic BLASes + TLAS (hardware RT; the
     software BVH still rebuilds).
   - Lights move in place through the light table; a visible rect light's card moves with its group.
   - A fixed object that moves becomes movable at its first move: one rebuild and a log line naming it. Placing a
     newly created object is not a move.
   - Moved surfaces keep a short history, like posed models: there are no per-object motion vectors yet.
   - Acceptance, bench-hitches:
     - Kabukichō with a taxi driving down the street (T / `traffic`; parked under the street when off, so the
       default views are unchanged): frame p99 769.8 ms → 18.3 ms. Before, every frame rebuilt the scene (767 ms);
       now there are no spikes.
     - Mannequin plaza with a car driving a loop behind the mannequins (a movable group of 9 meshes, wheels
       turning): p99 21.6 ms. The previous build manages 19.7 ms there only because the small plaza rebuilds in
       ~2 ms (interval p50 18.4 ms then, 16.7 now).
     - The same car not marked movable: one rebuild at its first move (22.8 ms), then p99 22.3 ms, no spikes.
   - Step 1's numbers are unaffected: bench-temporal excess 2.70 / 2.86 / 3.18, hitches p99 21.3 ms.
   - Movable glTF models as TLAS instances (2026-11-08, hardware RT).
     - A movable model that is not posed, and has no triangles flattened per placement (glass, emissive, material
       functions), is placed as an instance. A move rewrites its kernel record, its object pivot and its TLAS transform,
       and rebuilds the TLAS. No triangles are re-transformed, no BLAS is rebuilt.
     - The record keeps the previous frame's transform, so the kernel's motion vectors follow it. A fast sway (0.4
       rad, 2 Hz): moving vs converged ΔE 1.86 with them, 1.98 without.
     - The C slice: BLAS rebuild 3.1 → 2.1 ms a frame, frame script 7.0 → 3.7 ms. The skinned walkers are what is left.
     - Meshes (scene-code geometry) and posed models still re-transform / re-skin their triangles. Software traversal
       flattens instances and rebuilds, as before.
     - Kabukichō perf smoke −0.03 / +0.02 ms. District drive GPU p50 / p95 16.75 / 17.28 → 16.77 / 17.29 ms: the
       record grew from 112 to 160 bytes.
2. Skinning — done (0.2): glTF clips sampled on the CPU, linear blend skinning, posed models re-skinned in place
   (dynamic BLASes), the plaza game.
3. Glass refraction and car paint — first version done (2026-11-06).
   - Glass: MeshPhysicalMaterial `transmission` / `ior`. The transmitted share replaces the diffuse lobe and is
     sampled as a delta event: refraction through the shading normal, or through a VNDF-sampled microfacet normal
     above roughness 0.05 (Walter et al. 2007); total internal reflection reflects. F0 comes from the IOR.
     - Seen through glass counts as seen directly: exact lighting, no indirect firefly clamp, the live viewport's
       specular signal.
     - Glass lets shadow rays through. Offline (since 2026-11-08) each glass surface keeps its transmission share and
       tints by its colour, as UE's path tracer does; live they pass unattenuated (no caustics, like UE's raster
       translucency shadows). `transmissionMap` / glTF `transmissionTexture` scale the transmission.
     - Live paths don't end in the radiance cache on glass.
   - Car paint: `clearcoat` / `clearcoatRoughness` (UE ClearCoat). A GGX coat with F0 0.04 over the base; the base
     gets what the coat's directional albedo lets through, so energy is conserved.
   - Both are feature bits: scenes without them compile them out (Kabukichō's cost is unchanged).
   - Tests (layer A):
     - `material/glass-transmission` 91.86 vs 92.31 analytic (it failed before: transmission was ignored);
     - new `material/clearcoat-f0` 0.0400 vs 0.04;
     - new `furnace/clearcoat` 1.000.
   - The mannequin plaza's car now has clearcoated paint and a glass cabin with seats inside.
   - Not yet:
     - Thin-walled glass: a single-plane pane refracts as if solid.
     - Tinted shadows.
     - Light through windows in the radiance cache: it treats glass as a surface, not a window.
     - glTF KHR_materials_transmission / clearcoat.
     - Metallic flakes.
     - A UE reference shot of glass and paint.
4. Motion blur and depth of field.
   - Per-object motion vectors first: done (2026-11-06).
     - The renderer keeps the previous frame's vertices of the dynamic triangles (posed models, movable objects;
       `prev_verts`).
     - The trace kernel writes each camera hit's world-space offset to where it was. The temporal pass reprojects
       moving objects through it, so their history is no longer cut short.
     - `THREERT_NO_MV=1` restores the short history.
   - `tools/bench-motion.ts` measures moving objects: the plaza's movers freeze; the frame right after (history built
     while moving) is compared against the same pose converged. With motion vectors vs without:
     - ΔE 1.30 vs 1.28, p95 5.81 vs 4.96, SSIM 0.962 vs 0.957.
     - The movers are a small part of the frame. They are visibly cleaner (no grain on the walking / jogging
       mannequins), but their lighting lags a little as they turn.
     - Their shadows smear on the ground either way.
     - Both come from the sun's direct light riding the long diffuse history.
   - Directional lights' direct light in a signal of its own: done (2026-11-06).
     - It holds the camera hits' direct light from the sun (a tiny disk, so nearly deterministic). It bypasses the
       denoisers.
     - Its history: Catmull-Rom at the motion-compensated position. While anything changes (view, object,
       lighting) it is clamped to this frame's 3 × 3 range and held to 4 frames; otherwise it accumulates like the
       diffuse.
     - bench-motion ΔE / p95 / SSIM: 1.30 / 5.81 / 0.962 → 0.73 / 1.87 / 0.982. The moving shadows are crisp and the
       movers' lighting follows them.
     - The exact local-light loop went there too at first and was taken out again. On Kabukichō its sphere lights'
       soft shadows are noisy, and sign flicker keeps the history short, so without a denoiser:
       - excess move40 / move59 / stopped 1.71 / 1.86 / 2.20 → 1.92 / 1.95 / 2.33;
       - k = 1 SSIM 0.640 → 0.620.
     - Directional only, on Kabukichō (after fixing stale direct light in pixels that turned to sky, which drew
       trails of moving figures across the sky):
       - excess 1.81 / 1.79 / 2.14 (step 1 targets still met), closing 4 s excess 0.27;
       - k = 1 0.637, @256 0.738;
       - sun-room k = 1 0.950 → 0.959; mannequin still @16 / @256 0.987 / 0.983; bench-motion 0.73 / 1.78 / 0.980.
     - Cost: +0.9 ms on Kabukichō at the 0.5 floor (trace 8.46 → 8.90, temporal 1.93 → 2.23 ms). The trace's share
       is the kernel as a whole (register pressure / occupancy), not the work: switching the write or the bookkeeping
       off at run time changes nothing. So Kabukichō is 17.3 ms at the floor, over the 16.7 ms budget again
       (16.46 before).
     - The structural way back: the à-trous (3.7 ms) runs at output resolution whatever the render scale. Running
       the diffuse denoiser at traced resolution, as UE denoises before upscaling, saves ~2.7 ms at the floor.
   - The diffuse denoiser at traced resolution: done (2026-11-06).
     - The diffuse temporal accumulation (`dtemporal`) and the à-trous stages run on the traced grid, on
       irradiance, with edge stops from each sample's own surface. The temporal pass then upsamples per output pixel
       (joint bilateral on depth and normal) and remodulates by the sharp albedo. UE also denoises before it upsamples.
     - Edges. Each frame the upsampled irradiance is that of the surface the pixel's nearest jittered sample hit; at
       an edge that flips with the jitter while the albedo history holds a blend of both sides, so edges dithered
       and stair-stepped. The temporal pass keeps a short history of it (clamped into the range of the traced values
       around; 16 frames still, 4 moving), like the albedo's.
     - Bandwidth. The stages first read the 80-byte samples for their edge stops and cost twice as much per pixel;
       `dtemporal` now keeps a compact copy of each sample's surface in a buffer that is free until the specular
       passes.
     - Kabukichō GPU at the 0.5 floor 17.37 → 14.95 ms (trace 8.93; dtemporal 0.41, à-trous 0.69 for five stages,
       was 3.68; temporal 2.23 → 2.03; the compose moved into the last specular pass, +0.34). Every mood and viewpoint
       is within budget, fog ramps included (§ 1).
     - Above about 0.85 the separate pass costs more than it saves: sun-room at a fixed 0.93 is 18.2 vs 17.1 ms.
       Dynamic resolution absorbs it (sun-room settles at 0.85 instead of 0.93, with better SSIM, below).
     - Quality, same tools against the #18 build:
       - bench-temporal at 100 %: excess move40 / move59 / stopped 0.5 s 1.79 / 1.79 / 2.14 → 1.75 / 1.71 / 1.98;
         closing 4 s 0.27 → 0.25; closing 0.3 / 1 s vs converged 9.06 / 9.00 (limits 10.28 / 9.91).
       - bench-temporal at 0.5: move20 / move40 / move59 / stopped 2.61 / 3.09 / 3.20 / 3.17 → 2.67 / 3.03 / 3.07 /
         3.00; closing 4 s 1.00 → 1.10.
       - Sun-room vs the UE Path Tracer at a fixed 0.93, k = 1 / @16 / @256 SSIM: 0.958 / 0.959 / 0.970 → 0.960 /
         0.970 / 0.971. Dynamic resolution (0.85): 0.958 / 0.970 / 0.972.
       - Kabukichō v1 vs UE Lumen at a fixed 0.5: 0.633 / 0.708 / 0.738 → 0.633 / 0.721 / 0.726. @256 is the one
         regression: irradiance detail finer than the traced grid. The old output-resolution history super-resolved
         it from the jittered samples, but needed two output-resolution à-trous stages to hide its noise (the cost
         removed here). A raw output-resolution accumulation that takes over at long history stays visibly grainy
         at 256 frames (@256 0.663); a longer edge history changes nothing (0.726).
       - Mannequin vs offline @16 / @256 0.987 / 0.983 → 0.988 / 0.983; bench-motion ΔE / p95 / SSIM 0.73 / 1.78 /
         0.980 → 0.74 / 1.91 / 0.980; hitches p99 25.0 ms; layer A 18 / 19 + 1 asset-dependent.
   - Motion blur at UE's defaults: done (2026-11-06).
     - Parameters: amount 0.5, max 5 %, target 30 fps. The presets switch it off for their stills; Kabukichō and the
       mannequin plaza switch it on, as UE's level has it.
     - Velocity: the temporal pass computes each pixel's motion from the camera and the motion vectors.
     - Reconstruction: a gather after McGuire et al. 2012 on the HDR image before bloom. A moving object smears over
       a static background; a static foreground stays sharp.
     - Length, as UE's (PostProcessMotionBlur.cpp, UpdateMotionBlurTimeScale): ± amount / 2 × the frame's motion ×
       (1 / target FPS) / frame time. At 60 fps that is one frame of motion in total, capped at 5 % of the width.
     - Camera cuts (a jump of 2 m or 30° in one frame) don't blur (UE: bCameraCut). Without that check, Kabukichō's
       viewpoint switch smeared the whole frame (horizontal gradient energy 102 vs 363).
     - Cost at the 0.5 floor, Kabukichō:
       - 0.44 ms while anything moves: 15.78 ms total vs 15.36 with it off.
       - 0.01 ms while nothing moves: the pass is skipped and the last specular pass composes straight into the
         final image.
       - A separate copy pass (0.33 ms) is gone: the last specular pass composes into the buffer the blur reads.
       - Within budget either way.
     - UE reference: bench-temporal `--motion-blur` against UE renders with the same settings (`MB=1
       ue5/temporal.ts`). Movie Render Queue replaces the amount with its shutter fraction (0.5) and the target FPS with
       the output rate (60) (MoviePipelineImagePassBase.cpp), which is half the real-time blur. Ours at UE's real-time
       defaults read about twice UE's blur against those frames. Ours at what MRQ rendered (`THREERT_MOTION_BLUR=0.5,5,60`):
       - excess over UE move20 / move40 / move59 / stopped 1.23 / 1.51 / 1.60 / 2.01 (unblurred 1.43 / 1.75 / 1.75 /
         2.01);
       - strong vertical edges widen by +0.03–0.10 px (UE +0.06–0.21).
       - Both blurs are short: the move turns 0.2° per frame, about 2.4 px.
   - Depth of field at UE's defaults: done (2026-11-06).
     - UE's default is off: no focus distance on the camera. A camera's `dof` (focus distance, f-stop, sensor width;
       UE's defaults f/4, 24.576 mm) switches it on.
     - CoC: UE's physical model (DiaphragmDOFUtils.cpp, FPhysicalCocModel):
       - focal length from the sensor height (width / aspect) and the vertical FOV;
       - CoC of infinity f² / (N (focus − f));
       - per pixel (z − focus) / z of that, capped at 2.5 % of the width (r.DOF.Kernel.Max{Fore,Back}groundRadius).
     - Live: a gather on the composed HDR image, before motion blur as in UE.
       - The CoC is computed in the compose pass.
       - A 16 × 16 tile maximum bounds each pixel's search.
       - 48 Vogel-disk samples, scatter as gather. A foreground layer (samples in front, coverage from their spread) is
         composited over the rest. Samples behind count only as far as the pixel's own CoC reaches, so blurred
         background doesn't cover a sharp edge.
     - Offline: the path tracer's thin lens (unchanged).
     - Cost: 1.67 ms at 1280 × 720 while on (tiles 0.11, gather 1.56); off, 0.01 ms. Kabukichō at the 0.5 floor
       15.03 ms still, 15.83 moving (motion blur included).
     - UE reference: `scenes/mannequin-portrait.ts`, the mannequin scene through a portrait lens (CineCamera, 36 mm
       filmback, 20° vertical FOV = 57 mm, f/1.2, focus 5.9 m on the dancer), and `PORTRAIT=1 bun ue5/mannequin.ts` in UE
       (Lumen + Path Tracer). The plaza's F key swaps the same lens in.
       - Ours vs the UE Path Tracer: live @256 ΔE 3.34 / SSIM 0.982 (the unfocused scene: 3.94 / 0.982), offline thin
         lens 3.31 / 0.987, UE Lumen 1.58 / 0.991.
       - Blur widths (10–90 %) of the blocks behind the dancer, ours live / ours offline / UE Lumen / UE PT: tall block
         2.3 / 3.0 / 3.1 / 3.5 px, low box 4.2 / 3.8 / 4.7 / 5.4 px.
       - UE's horizon is softer still: its haze below the horizon (next item).
     - Fixed on the way: a camera description change (lens, DOF) rebuilds the scene, and the view kept its old camera.
       It now takes the scene's camera when that changed.
   - The atmosphere's haze below the horizon followed in #22 (see Known parity gaps).
5. LOD and streaming. In progress: steps 1 and 2 done (#30, #31), light LOD added (below); 3 and 4 next.
   - Since 2026-11-08 the district becomes Bistro tiled (§ 0 step 10); the Kabukichō district numbers below are history.
   - Bistro district against the targets below (2026-11-08, local-main f2815a1, `node tools/bench-district.ts`, the
     first 50 s of Bistro's camera path at the 0.5 floor): GPU p95 16.2 ms (≤ 16.7: met, was 24.2 when the district
     landed), frame interval p99 17.0 ms with no frame over 50 ms (≤ 25 / 0: met), first frame 2.1 s (≤ 3 s: met),
     footprint 11.3 GB for nine copies, the same as one (≤ 1.5 x one: met). What got it there: the light tree, live
     secondary rays taking foliage cutouts as opaque, shadow rays' single candidate-loop query, the radiance cache's
     neighbouring levels, the viewport's pacing (item 3 of the supervisor's list, § 0). Geometry LOD and streaming
     (implementation steps 3 and 4) are not needed for these targets on Bistro; the "invisible" criterion applies when
     they are taken up.
   - Scene: `games/kabukicho/district.game.ts`, the Kabukichō district.
     - Nine Kabukichō streets on a 3 × 3 grid (175 m along, 90 m across), each from its own seed (4..12):
       different buildings, shops, signs, props and lights. No two blocks alike.
     - The night sky, fog and rain of the game.
     - About 37 M triangles: one block ~4.1 M; the middle row alone, `district-row.game.ts`, is 12.27 M triangles
       and 254 textures.
     - Textures come from the same Poly Haven pool in every block. The implementation PR widens the pool per block,
       so streaming has to handle textures too, not only geometry.
   - Drive (`node tools/bench-district.ts [--row] [--scale 0.5|auto]`): eye height, down the 520 m middle street at
     15 m/s, back at 30 m/s, camera commands at 60 Hz; `live --frame-log` per frame; physical footprint at the end.
   - Targets. "Before" is the renderer without LOD or streaming; "now" is the 3 × 3 district with light LOD
     (2026-11-06, `bench-district.ts`):

     | target | goal | before | now (3 × 3) |
     |---|---|---|---|
     | the 3 × 3 district loads | yes | **no**: `Tlas … invalid custom index (more than 24 bits)`, triangle offsets cap a scene at 2^24 = 16.7 M triangles | yes (#30, #31) |
     | GPU at the 0.5 floor, p95 along the drive | ≤ 16.7 ms | row: 20.6 ms (p50 19.7, p99 20.9) | 21.2 ms (p50 19.5); 24.6 without light LOD; 18.5 ms (p50 17.9) with far emitters culled; **17.9 ms** (p50 17.4) with the temporal and specular passes lighter (2026-11-07): the current limit, 1.2 ms short |
     | GPU at dynamic resolution, p95 (scale ≥ 0.5) | ≤ 16.7 ms | row: 21.8 ms at 0.50..0.75 | **21.2 ms** at 0.50..0.80 |
     | frame interval p99 / frames over 50 ms | ≤ 25 ms / 0 | row: 21.0 ms / 1 (51 ms); dynamic 23.2 ms / 1 (85 ms) | 22.1 ms / 0; dynamic 22.8 ms / 0 |
     | first frame | ≤ 3 s | row 6.5 s; one street 5.4–6.1 s | 5.8–6.0 s; **2.7–3.0 s** since 2026-11-07 (decoding in parallel) |
     | memory (physical footprint) | 3 × 3 ≤ 1.5 × one street | one street 8.6 GB, row 9.8 GB, 3 × 3: does not load | 7.2 GB (instancing) |
     | LOD / streaming invisible | 5 poses along the drive vs everything resident at full detail: dE ≤ 1.0, SSIM ≥ 0.98; while driving, popping within the bench-ghosting limits (§ 2c) | (no LOD yet) | light LOD: see below |

   - Where the far blocks' GPU time goes. Variants of the district, lockstep drive (x −10 → 465 m, 0.5 floor),
     GPU p50:
     - 23.75 ms for the full district.
     - 23.08–23.83 ms with the outer blocks' meshes, props, geometry, local lights alone or emitters alone removed,
       or rays capped at 60 m.
     - 19.38 ms with the outer blocks' local lights and emitters both removed (geometry kept); 19.23 ms with the
       outer blocks gone entirely.
     - So the cost is far light sources, not geometry, and only both kinds together. Every camera-hit pixel tested
       all 1174 unshadowed local lights' ranges (the row has 391), and emitters were drawn by power wherever they
       were. Making either cheaper alone did not help: culling the light loop alone made it 25.1 ms.
   - Light LOD (live, 2026-11-06):
     - The emitter CDF weighs emitters by power × (d0 / max(d, d0))², d = distance to the camera, d0 = 10 m. It is
       rebuilt every 15 m of travel (1.2–1.5 ms CPU for the district's 376 k emitters).
     - Emitter draws (2026-11-07) search the CDF through a guide table (the cutpoint method, 64 k buckets): a draw u
       only searches between the entries where u's bucket and the next one begin. It finds the same entry as the
       full 19-level search (unit test), so the estimator is unchanged.
       - GPU at the 0.5 floor, main → guide table: district drive p50 / p95 19.35 / 20.97 → 18.93 / 20.30 ms (4 k
         buckets: 19.02 / 20.38); Kabukichō still / moving 14.64 / 15.42 → 14.39 / 15.14 ms.
       - An alias table (Vose's method, O(1) per draw, same distribution) was slower: district drive p50 21.6 ms,
         20.05 with the keep probability and the alias packed into one 8-byte slot; Kabukichō even. Each draw starts
         at a uniformly random slot of the 376 k-entry table, nearly always a cache miss. The binary search's reads
         stay in few cache lines: every thread shares its top levels, and light LOD concentrates the draws on the
         nearby emitters. Branch `emitter-alias`.
     - The weighting only decides which emitters are proposed. Every surface vertex resamples the proposals (RIS: 6
       candidates at camera hits, 2 deeper) toward power × geometry term, and every MIS weight uses power alone.
       So the split of each light between emitter sampling and BSDF rays is the same as without light LOD.
     - A first version put the light LOD into the MIS weights and drew single samples at deeper vertices. It was
       unbiased offline, but live it moved far signs' reflections onto the BSDF rays and made the radiance cache's
       training samples heavy, which the live clamps cut. Kabukichō's street came out 2 % farther from the truth.
     - Local lights at camera hits are sorted into spatial groups of 16, each with a sphere bounding the lights'
       reach; a point outside the sphere skips the group. Same result as testing each light.
     - `THREERT_LIGHT_LOD=0` and `THREERT_LIGHT_GROUPS=0` switch the two parts off for A/B. `THREERT_SEED` gives another sample
       sequence for noise floors. Renders repeat exactly only with software traversal (Known issues).
     - GPU at the 0.5 floor, main → light LOD:
       - District, lockstep p50 / p95: 23.91 / 24.27 → 19.30 / 20.61 ms.
       - Middle row, lockstep p50 / p95: 19.55 / 19.92 → 17.84 / 18.30 ms.
       - Kabukichō, still / moving: 15.00 / 15.83–15.89 → 14.59–14.62 / 15.42–15.51 ms.
       - Both parts compiled in but switched off: district 23.30 ms, Kabukichō 14.88–14.90 / 15.69–15.71 (faster than
         main).
     - Against the truth (criteria below; 480 × 270; regions sign band / street / facades / sky):
       - (a) Offline with every clamp off (16 k spp), LOD on vs the truth (two LOD-off seeds): within 0.32 % in every
         region of the district and Kabukichō. The two seeds differ by up to 0.57 %.
       - (b) Live converged frames (1024 frames) vs that truth, off → on:
         - District: −18.4 → −17.5 / −16.7 → −16.3 / −0.1 → +0.3 / −48.6 → −34.1 %.
         - Kabukichō: −6.2 → −5.5 / −6.9 → −6.8 / −1.2 → −0.4 / −21.8 → −15.5 %.
         - Closer or equal everywhere. The district facades' 0.2-point difference is inside their seed spread.
         - The live pipeline itself sits well below the truth: on Kabukichō's street the radiance cache alone accounts
           for −6.6 % (−0.25 % with the cache off).
       - (c) Live seed spread, the same build with two seeds: 0.1–0.3 % on the street and facades; 1–5 % in the sign
         band and sky.
     - Temporal, Kabukichō (bench-temporal at 0.5, medians of 3 runs, dE vs our converged frames, main → LOD):
       - Closing 0.3 / 1 / 2 / 4 s: 9.84 / 9.78 / 7.64 / 4.06 → 9.79 / 9.62 / 7.48 / 3.88.
       - Moving: move20 / 40 / 59 4.89 / 4.87 / 5.10 → 4.87 / 4.97 / 5.02; stopped 3.93 → 3.91. move40 is worse:
         sign band +0.28 dE, sky +0.46.
       - Excess over UE moves the same way.
     - bench-ghosting: turn 2.90 → 2.86, strafe 3.82 → 3.73, returns equal, stop+0.1s 2.87 → 2.83.
     - It needed #41 first: emitter shadow rays had skipped `castShadow = false` geometry (Known parity gaps). The
       light LOD's larger share for near emitters turned that leak into +16 % on the near facades.
   - Still open against the targets:
     - GPU p95: 20.6 → 18.5 ms (bench-district, 0.5 floor) by culling far emitters (2026-11-07, below); 1.8 ms short.
       - Far emitters cost, not far analytic lights (lockstep drive p50 / p95, main 18.87 / 20.17): the outer rows'
         emissive meshes removed 17.96 / 18.38, their analytic lights removed 19.02 / 20.41, both 17.52 / 17.93.
         Light LOD still proposes many far emitters (summed over the district, (d0 / d)² is mostly far mass), and
         each candidate reads a triangle, its material and an emissive texel somewhere in the district: incoherent
         misses, though RIS rarely picks them.
       - Live emitters farther than 60 m from the light LOD's reference are not proposed (`emitter_lod`, weight 0;
         `THREERT_EMITTER_CULL` = metres, 0 = off). BSDF rays that hit them get their whole light (`emit_mis` returns 1
         for them), so every emitter's MIS weights still sum to one and the estimator stays unbiased; their light
         just arrives by BSDF rays (and so the radiance cache) alone. The one bias: fog in-scattering finds
         emitters only by sampling them, so the culled ones' glow in the fog is missing (the far signs below).
       - Lockstep drive p50 / p95 by radius: off 18.89 / 20.13, 120 m 18.16 / 18.64, 60 m 17.84 / 18.26, 30 m
         17.54 / 17.95.
       - Invisible (5 poses along the drive, x = 20 / 120 / 250 / 380 / 480, 1600 still frames at the 0.5 floor):
         - vs the same render without culling, same seed: dE 0.62–1.01, SSIM 0.986–0.992. Two seeds without culling
           differ by dE 1.32–2.37, SSIM 0.94–0.96, so most of that is noise.
         - Region means vs the offline truth (16 k spp, clamps off), culled − exact: far street +0.14 to +0.70 %,
           far signs −0.06 to −0.66 %, sign band, near street and facades within ±0.25 %. The live seed spread is
           0.07–1.0 %: on average about 1 ×, at most 4.4 × (far street at x = 250). Live itself sits 12–31 % below
           the truth there.
       - Kabukichō even (still / moving 14.35–14.43 / 15.07–15.08 ms, main 14.36–14.38 / 15.09–15.10); compiled in
         but off +0.07 ms. Wet street after a strafe vs UE50 the same (street 3.28 / 2.67 / 1.61 vs 3.28 / 2.68 /
         1.62).
       - Where the rest goes (drive, mean per pass): trace 11.06 ms, temporal 2.07, specular filter 1.78, à-trous
         0.70, the other passes ~1.5. Culled, the district is at the middle row's level (row alone 17.4 / 17.8 ms
         lockstep): what is left is the street's own content. The temporal and specular passes run at output
         resolution, so the render scale does not shrink them.
       - The view cone (2026-11-08, branch `view-cull`, found on Bistro): the cull keeps emitters ahead of the camera.
         Culled emitters' light arrives by BSDF rays alone, which is unbiased but rare; the firefly clamp cuts those
         rare rays, so a lit region whose emitters are all culled goes dark. Bistro's square (40–90 m down the street,
         lit by its own string lights) had 40 % of its offline light in the live image, the frame 75 %. Now emitters
         inside a cone around the view (the frustum's half-diagonal + 20°, rebuilt after a 15° turn as after 15 m)
         are kept out to 2 × the radius (120 m; `THREERT_EMITTER_CULL_VIEW` = the multiple, 0 = the old cull); the
         kernel's `emitter_culled` makes the same test.
         - Bistro, live raw radiance / offline 1024 spp (99th-percentile cap): frame 0.75 → 0.85, square 0.40 → 0.70,
           street 0.91 → 0.95, left facade 0.94 → 0.97 (= no cull at all). GPU at the 0.5 floor 14.62 → 15.1 ms.
         - Reach: 1.5 × / 2 × / 4 × give Bistro the same (square 0.69 / 0.70 / 0.70); the district drive (GPU p50 /
           p95, cull alone 16.6–16.7 / 17.1–17.2 ms) pays nothing / +0.1–0.2 / +0.4–0.5 ms. 2 × keeps margin for
           Bistro views down its longer streets.
         - Kabukichō perf smoke +0.14 / +0.09 ms (+0.03 / +0.01 at 4 ×: run noise; its far emitters are out of view
           or occluded); offline unchanged
           (16 / 16 catalog scenes byte-identical).
         - Measured, not taken: light LOD d0 20 / 30 m on top: frame 0.87 / 0.88, square 0.76 / 0.79, +0.25 / +0.4 ms;
           LOD off: square 0.81. The rest of the live deficit (~12 %, everywhere) is the live estimator's: fewer
           bounces, the clamp on noisier samples, temporal clamping.
     - The temporal and specular passes, lighter (2026-11-07): p95 18.5 → 17.9 ms (p50 18.0 → 17.4), 1.2 ms short.
       - Measured first (district drive, timing-only switches): the specular filter's three passes cost 0.35 of their
         0.48 ms copying (memory traffic), spec2 mostly read four buffers to compose the image; in the temporal pass
         the Catmull-Rom albedo / emission / sun history 0.38 ms, the 3 × 3 gather 0.25, the irradiance upsample
         0.15, the mirror-image fetch 0.15, the rest mostly its output-grid histories (~104 bytes written per pixel).
       - Kept: the temporal pass writes the composed rest (diffuse, emission, sun light; w: depth) so spec2 reads one
         vector (0.83 → 0.71 ms); the specular ping-pong in f16, 8 bytes per pixel (−0.27); the output-grid g-buffer
         in 8 bytes with roughness (octahedral normal, f16 depth and roughness: −0.2 over the passes).
         Temporal 2.08 → 2.00, specular 1.78 → 1.28, dtemporal 0.41 → 0.38 ms. Converged frames at 5 poses vs main:
         dE 0.05–0.09 (two seeds of main: 1.3–2.4); bench-ghosting and the wet street vs UE50 unchanged; Kabukichō
         −0.55 ms still and moving.
       - Tried, no gain: one exp per tap and pow by squaring in the filter (latency-bound on its loads, not math);
         loading its 3 × 3 once into per-thread arrays (spilled: 0.43 → 0.79 ms); the three steps fused in one pass
         through workgroup memory (16 × 16 tiles with a 7-pixel halo, 25 KB: occupancy fell, 25 ms).
       - The histories that accumulate long (specular, diffuse, sun light: up to 4096 frames still) need f32; the live
         bind group is at Metal's 29-buffer limit, so new typed buffers need old ones merged first.
       - What is left at 17.4 ms (mean): trace 11.05, temporal 2.00, specular 1.28, à-trous 0.70, dtemporal 0.38,
         refl 0.27, the rest ~0.7.
     - 17.9 ms is the current limit (decided 2026-11-07), 1.2 ms short of 16.7. Tried and not pursued further:
       - the camera-hit direct light in its own pass (§ 1): the district +0.86 ms;
       - per-tile light culling (§ 1): the loop it would cull costs nothing inline;
       - an alias table for emitter sampling: slower than the guide table that replaced the search;
       - specular denoising at the internal resolution (§ 1): −0.62 ms, ghosting and the wet street worse.
     - Candidate structural project, not scheduled: full TSR, the output grid accumulating the composed image instead
       of each signal's history. The gain is uncertain, since our per-signal histories already accumulate at the
       output resolution, and it touches every ghosting acceptance.
     - First frame: met on 2026-11-07 (5.8–6.0 → 2.7–3.0 s; Kabukichō 4.8 → 1.7 s), without streaming.
       - Where the 5.8 s went (3 × 3 district, launch to first frame):
         - the game's setup in the script runtime, 1.25 s;
         - decoding images one after another, 2.0 s: ~200 texture files, and 53 models with 219 embedded images,
           all 2 k;
         - packing the texture pages, 0.9 s, nearly all of it resizing each 2048² texture to 2016² so its gutters
           fit; the image crate's horizontal pass walks a whole-image f32 intermediate column by column;
         - copying the 32 pages to the GPU, 0.3 s. Building the BLASes and the TLAS takes 0.13 s.
       - What changed:
         - `scene::build` decodes up front, in parallel: the texture files the materials use, and the models with
           their images converted to RGBA;
         - `texpack::resize_triangle` gives the image crate's Triangle resize bit for bit (unit test), a row at a
           time and in parallel.
         - Renders are bit-identical to before (software traversal, district and Kabukichō).
       - So streaming by distance is not needed for the targets yet: memory is met, and nothing on the GPU side
         (BLAS builds, 0.13 s) is worth spreading across frames. It becomes necessary once the texture pool widens
         per block (§ 5, the scene above), since decoding grows with it.
       - Left on the way to the first frame: the setup script (1.25 s), the page copies (0.3 s), the rest of
         `scene::build` (0.5 s).
     - Frame interval in dynamic resolution: the controller acted twice a second. At the drive's far end the camera
       looks out of the district, the scale rises to 0.85–0.88, and on the turn back the GPU took 35–48 ms for up to
       30 frames (p99 22.2–28.6 ms, 0–21 spikes). Fixed in #52: two frames well over the frame time act at once;
       p99 21.5–21.7 ms, no spikes.
   - Order (decided 2026-11-06, from the numbers above; the far blocks' GPU time is their light sources):
     1. Light LOD. Done.
     2. UE strafe reference with the sign-band comparison (§ 2c). Done.
     3. The direct-light pass (§ 1) for the GPU p95, with one more emitter sample at near-mirror camera hits for the
        wet street (moved ahead of streaming on 2026-11-07).
     4. Streaming: first frame ≤ 3 s, no frame over 50 ms. Memory is met already. The first frame was met by
        decoding in parallel instead (2026-11-07, above); streaming waits for a wider texture pool.
     - Simplified meshes and impostors (step 3) come later. They help neither the GPU time nor memory here.
   - Which "invisible" criterion applies:
     - Unbiased estimator changes (light LOD): judged against the truth, not against the image without them.
       a. Offline with every clamp off, the region means with and without the change are within the noise floor.
       b. Live converged frames are no farther from that offline truth with the change than without it, per region
          (sign band, street, facades, sky); compared with UE too.
       c. The noise floor is the same build run with two seeds (`THREERT_SEED`).
       d. bench-ghosting and closing 0.3 / 1 / 2 s no worse than main.
     - Biased geometry LOD (simplified meshes, impostors): dE ≤ 1.0, SSIM ≥ 0.98 at 5 poses along the drive vs
       everything resident at full detail, as in the table above.

   - Implementation order:
     1. Lift the 2^24 cap: per-instance offsets in a table instead of the TLAS custom index.
     2. Instance repeated props: one BLAS per model, TLAS instances. Each block places ~350 GLB props from a shared
       set.
     3. Distance LOD: simplified meshes for far blocks (meshoptimizer), impostors beyond.
     4. Streaming by distance: block BLASes and texture mips made resident ahead of the camera, built and uploaded
       across frames so no frame goes over 50 ms.
6. Particles, volumes, foliage wind, water. Plan (2026-11-07); one sub-item per PR, in this order.
   - Rules for all of them:
     - Agents write Three-like DSL; the mechanisms are the existing engine paths (material functions, the standard
       library, movable nodes, instancing, frame scripts).
     - No built-in effect fields in the kernel. Where the kernel needs something new, it is a generic contract with
       WGSL bodies, like material functions, and the PR reports its cost compiled in but off.
     - Every PR: the full gate, crops for visual changes, and the gate scene (Kabukichō then; Bistro night once § 0 step 7
       lands) at the 0.5 floor ≤ 16.7 ms with the effect on
       (now 13.9 still / 14.5 ms moving: ~2.2 ms for effects).
     - UE references come from the same tools as before:
       - `ue5/capture.ts` stills of a level;
       - `ue5/temporal.ts` MRQ sequences at 60 fps (frames for side-by-side videos);
       - scratch projects built natively from a script (`ue5/glb-scene.ts`, `ue5/mannequin.ts`) for test scenes.
   - Order and why:
     - Rain first. Kabukichō's rainy night has wet streets and ripples but no falling rain, and the UE level has it
       (below). It is also the C slice's rainy night street.
     - Then sparks and debris (the C slice's explosion), on the same particle path.
     - Volumes next, for smoke and the explosion's fireball, and because water's body needs a medium.
     - Water after that (the C slice's waterfront).
     - Foliage wind last: no current scene has foliage; it is for the C slice only.
   1. Particles: rain.
      - How agents use it: `new Particles({ count, emitter, material })`, a node in the scene like a Mesh.
        - The emitter is a `ParticleFunction`: a WGSL body under a small fixed contract. From (particle index, time,
          camera, seed, its parameters) it returns a quad (centre, axes, size) and a per-particle vec4 for the
          material.
        - The standard library has presets: `RainCurtain({ drops, box, speed, length, wind })` and a
          `RainMaterial` (streak texture, alpha-tested, lit by the scene).
        - Particles are a function of time and camera, so they stay deterministic and need no game state (the scene
          is a function of the state).
      - Mechanism:
        - A compute pass evaluates the particle function into a dynamic triangle span (this frame's quads and last
          frame's for motion vectors), and the dynamic BLAS is rebuilt on the GPU: the posed models' path, no CPU or
          script cost.
        - The material is a material function; the per-particle vec4 reaches it as a new `MatIn` field (the
          generic per-instance data channel, as UE's PerInstanceCustomData), so no effect fields.
        - Why not frame scripts moving 16 k instances: QuickJS per frame, and wgpu builds TLAS instances on the CPU.
      - UE reference:
        - The level's own rain, `AKabukichoRain` (~/code/KabukichoNight, read only): 16 000 streak quads that
          M_Rain's world position offset wraps into a 32 × 32 × 22 m box around the camera, drops and billboards.
          Our preset takes the same numbers.
        - The captures hide it today (`KabukichoRain` in the hide lists): stills and MRQ sequences with it shown,
          rain on.
      - Metrics:
        - Rain is random, so per-pixel ΔE against UE means nothing for the streaks themselves.
          - Rain on vs off: the added luminance per region (sign band, street, facades, sky) against UE's same
            difference.
          - Streak density, length and angle against UE's frames.
          - Behind the rain, ΔE / SSIM to our rain-off image ≤ the two-seed noise floor.
        - In motion:
          - bench-ghosting with rain on: the scene's rows no worse than without rain;
          - streaks leave no trails: residual luminance one frame after a streak passes ≤ its 1-spp noise.
        - GPU: Kabukichō with rain ≤ 16.7 ms at the 0.5 floor; the particle pass and BLAS rebuild reported.
        - Layer A:
          - `particles/quad-placement`: a particle function's quad at an analytic position and size covers the
            expected pixels;
          - `particles/instance-data`: the per-particle vec4 reaches the material.
      - Status (2026-11-07): the mechanism is in, without rain (docs/MATERIAL_FUNCTIONS.md, "Per-instance data and
        particles").
        - Per-instance data: `customData` on any node, `i.custom` in any material function (objects table stride
          3 vec4). Layer A `material/custom-data`.
        - `ParticleFunction` / `Particles`, the particle pass and the BLAS rebuild; `THREERT_PROFILE` times both, and the
          frame timer starts at the particle pass. Layer A `particles/quad-placement`, `particles/instance-data` and
          `particles/lit-normal` (the quad's own shading frame).
        - Existing scenes are unchanged: the 15 catalog scenes render byte-identical to main offline with software
          traversal (4 spp).
        - Compiled in but unused: Kabukichō 13.76 → 13.84 ms still, 14.51 → 14.53 ms moving; district drive p50
          17.31 → 17.36, p95 17.74 → 17.78 ms (0.5 floor, two alternating runs each).
        - With 16 000 particles over Kabukichō: particle pass 0.12 ms, BLAS + TLAS rebuild 0.6 ms.
      - Status (2026-11-07): rain is in. `RainCurtain` (standard library) on Kabukichō, key P toggles the streaks.
        - The translucency pass (docs/MATERIAL_FUNCTIONS.md, "Translucent particles") draws `transparent`
          particles over the image:
          - depth: each pixel's ray stops at the opaque surface it shows;
          - overlap: ordered blending, the 4 nearest hits front to back, not weighted OIT;
          - lighting: the radiance cache's volume voxels at the hit (live), a fresh estimate per sample (offline).
          They are absent from reflections, shadows and GI (Known parity gaps).
        - Layer A:
          - `translucent/order`: 2.5 nits, against 1.5 for the reversed order;
          - `translucent/depth`: live, a quad behind a wall stays hidden while one in front shows (ratio 10);
          - `volume/lighting` (live, 1.021) and `volume/lighting-offline` (0.997), against the analytic 1: a white
            particle in a cube with one lit wall.
        - Rain against UE50 at the bench start pose, 60 frames:
          - added luminance:

            | region | UE | ours |
            |---|---|---|
            | all | +0.15 % | +0.17 % |
            | facades | +2.04 % | +1.86 % |
            | right facades | +0.49 % | +0.47 % |
            | signs | +0.03 % | +0.00 % |
            | street | +0.08 % | +0.25 % |

          - streak angle: −1.1° (UE −2.6°);
          - run length: 9.1 px (UE 10.9);
          - streak coverage: 0.98 % of pixels, against UE's ≈ 0 (below).
          - Crops: `docs/img/rain-facade-ue-ours.jpg` and `docs/img/rain-street-ue-ours.jpg` (UE on | off | ours
            on | off).
          - Our on / off frames come from one run (THREERT_TL_HIDE=odd hides the layer on odd frames). Shader time
            follows the wall clock even in lockstep, so separate runs differ in sign flicker and rain phase: ±2 %
            on the facades between runs.
        - Behind the rain: the layer hidden vs rain off is within the two-seed noise floor in every region (ΔE / SSIM,
          10 frames):
          - all: 0.78 / 0.981 against a floor of 2.54 / 0.934;
          - street: 1.12 / 0.960 against 1.77 / 0.928;
          - facades: 0.46 / 0.994 against 0.69 / 0.987.
          The layer has no history, so a streak leaves nothing behind once it has passed.
        - bench-ghosting --quick --check: every row equals main's.
          - Under THREERT_FREEZE_TIME the frame script gets dt 0, so the bench's "rain off" event never fades
            `scene.rain`: the bench has always run with rain at 1 (ripples).
          - It now runs with the streaks as well, static in time.
        - Volume voxels after a light switches off (cube with one lit wall, white walls): the particle drops below
          10 % in 4 frames and below 5 % in 7. The floor and walls of the same view (surface cache + temporal filter)
          take 48–52 frames to drop below 10 %.
        - GPU at the 0.5 floor with rain:
          - Kabukichō: 15.22 ms still, 16.1–16.2 ms moving (≤ 16.7);
          - the particle pass 0.09 ms, the BLAS rebuild 0.42–0.49, the translucency pass 1.05, volume training 0.09.
          - Streaks past the 15.5 m fade are left out of the BLAS (−0.15 ms).
          - Offline: 124 ms for 32 samples.
        - Compiled in but unused (main 1791358 vs this, alternating):
          - Kabukichō without rain: 13.80 / 13.81 vs 13.80 ms still, 14.49 / 14.56 vs 14.48 / 14.54 ms moving;
          - district drive p50 17.32 vs 17.33–17.35 ms, p95 17.75 vs 17.77–17.78 ms;
          - the 14 catalog scenes without rain render byte-identical offline with software traversal.
        - Streak coverage, open. UE's streaks are invisible frame by frame; its +2 % is a veil.
          - UE draws translucency before TSR, without velocity: TSR spreads a thin streak that moves 10–50 px a
            frame thin over time.
          - Ours draws each frame's streaks after the temporal filter: crisp, about as bright on average.
          - Tried and reverted:
            - A history for the layer, reprojected with the opaque surface's motion and clamped to the current 3 × 3
              neighbourhood (no trails). Weight 0.5 / 0.25 / 0.1: coverage 0.73 / 0.44 / 0.16 %, facades
              +1.85 / +1.32 / +1.16 %. The clamp throws the streak's light away with its contrast.
            - Thin-line anti-aliasing (streaks at least 1.5 px wide, as much fainter): coverage unchanged (wider
              streaks are still detected).
          - Matching UE would take a history without the clamp, a veil as TSR makes, which trails by definition.
   2. Particles: sparks and debris.
      - How agents use it: the same `Particles` with burst emitters. Presets `SparkBurst({ count, speed, gravity,
        drag, life })` (emissive streaks that cool from white to red) and `Debris` (small meshes; few, so movable
        nodes moved by a frame script are enough).
      - Mechanism: the particle function gets the burst's start time and origin as parameters. Sparks are emitters:
        light LOD and the emitter table already handle moving triangles (`EmitterTri::moves`).
      - UE reference: a Niagara burst built from UE 5.8's own templates (Sprite Burst / Fountain, sparks) in a
        scratch project, captured as an MRQ sequence.
      - Metrics:
        - Burst statistics against UE (count over time, spread, light cast on the ground: luminance added per
          region).
        - No trails after the sparks die out.
        - GPU at the 0.5 floor during the burst.
        - Layer A: `particles/emissive-spark` (an emissive particle lights a white floor by the analytic amount).
      - Status (2026-11-07): in.
        - Particles as lights: `emitterArea` puts a particle's triangles in the emitter table with a nominal area.
          - The emitter entry carries it negated, and the triangle attribute `x.z` holds it.
          - The kernel samples the actual quad and corrects the area pdf for the nominal one in the RIS sampler,
            `emit_mis` and the reservoir evaluation.
          - Particles without it are marked: their BSDF hits get full weight. Before, any other light-sampled emitter
            made their light vanish: 0.002 instead of 2.418 nits in `particles/emissive-unsampled`.
          - Existing scenes stay byte-identical offline (software traversal, all 15 catalog scenes). Forms of the same
            code that kept the static path identical were chosen after a bisect: an extra early return in
            `sample_emitter`, or a reordered `emit_mis`, changed Metal's fast-math codegen by 1 LSB in emitter scenes
            (PSNR 92–112 dB).
        - The emitter node's `customData` reaches its particle function as `i.custom`, set live by the game (no
          rebuild): a burst's start time.
        - Live scene time is now the game's clock.
          - `LiveCore::advance_time`, with the dt the frame script gets, replaces wall-clock time since start. A burst
            fired at `f.time` starts then.
          - Lockstep runs now repeat exactly: before, sign flicker and rain phases differed per run, ±2 % on region
            means.
        - Scenes with light-sampled particles cap the diffuse history at 24 frames. Fading sparks darken the floor too
          gradually for anti-lag: the floor glow was 0.0033 (display) 30 frames after the last spark, 0 with the cap.
        - `SparkBurst` and `Debris` in the standard library (docs/MATERIAL_FUNCTIONS.md).
        - Against UE: `bun ue5/burst.ts` captures UE 5.8's SimpleExplosion template, unchanged, in a scratch project
          ("a sprite core, velocity aligned sparks, and oriented meshes"). `node tools/bench-burst.ts` renders
          `tools/burst-scene.ts`, the same floor, camera and exposure, with `SparkBurst` set like the template's sparks,
          and compares the sequences frame by frame:

          | | ours | UE |
          |---|---|---|
          | burst pixels, peak | 13.7 k at frame 19 | 16.5 k at frame 21 |
          | below 5 % of peak | frame 87 | frame 90 |
          | separate sparks, frame 20 | 83 | 108 |
          | top (px above the base), frame 45 | 478 | 477 |
          | RMS spread, frames 20 / 60 | 175 / 257 px | 186 / 264 px |

          - Mid-flight (frames 30–45) ours shows fewer burst pixels (3.6 k vs 13.5 k): our streaks fade with age and
            cool to red, UE's stay grey.
          - Crops: `docs/img/burst-ours-ue.jpg` (frames 15, 30, 60; ours | UE).
          - Light on the floor: ours up to +0.0027 (display), UE 0. UE's burst templates (DirectionalBurst,
            RadialBurst, SimpleExplosion, UpwardMeshBurst) have no light renderer, so their sparks light nothing; ours
            are emitters (UE: a Niagara Light Renderer). Known parity gap.
          - No trails: the floor is back at the background (0.0000) from frame 90.
        - The kernel's particle-emitter code sits behind a feature bit (F_PEMIT, scenes with opaque particles).
          - Compiled in, it cost Kabukichō's trace 0.12 ms (8.20 vs 8.08 ms).
          - Behind the bit, the compiled-in but unused cost (main vs this, alternating) is:

            | | main | this |
            |---|---|---|
            | Kabukichō still, ms | 15.20 / 15.23 | 15.22 / 15.22 |
            | Kabukichō moving, ms | 15.02 / 15.00 | 15.01 / 15.02 |
            | district p50, ms | 17.34 / 17.31 | 17.33 / 17.35 |
            | district p95, ms | 17.79 / 17.75 | 17.77 / 17.77 |

        - GPU during bursts: Kabukichō with its rain and a `SparkBurst` firing every 1.5 s ahead of the start pose
          (sparks always in the air, the worst case) is 15.86 ms still and 16.68–16.70 ms moving at the 0.5 floor.
          - The sparks add 0.24 ms of rebuilds: opaque particles rebuild the scene's dynamic BLASes and TLAS every
            frame.
          - They add 0.3–0.4 ms of trace.
          - Spark streaks are plain quads: an alpha-tested outline (an any-hit test on every ray crossing one) cost
            another 0.05 ms and was dropped.
        - Layer A:
          - `particles/emissive-spark` and `-nominal`: 1.579 / 1.576 against the analytic 1.586;
          - `particles/emissive-unsampled`: 2.399 against 2.418;
          - `particles/node-custom`: live.
   3. Volumes: local fog, smoke, the explosion's fireball.
      - How agents use it:
        - `new FogVolume({ shape: "box" | "sphere", density, falloff, albedo })`, UE 5.8's Local Fog Volume.
        - `new VolumeMesh(boundsGeometry, volumeFunction)`: a `VolumeFunction` is a WGSL body under a small fixed
          contract. At a point and time it returns extinction, albedo and emission.
        - Presets `SmokeColumn` and `Fireball` (procedural noise density, emission from temperature); a particle
          system can place them.
      - Mechanism:
        - The kernel's fog path becomes bounded media: rays find the volume's bounds as flagged triangles, ratio
          tracking for shadow transmittance, delta tracking for scattering (Novák et al. 2018), emission
          accumulated along the path.
        - A generic contract like material functions, not an effect field. A feature bit compiles it out where no
          scene uses it; the PR reports its cost compiled in but off (Kabukichō and the district).
      - UE reference: UE 5.8 Local Fog Volumes with Volumetric Fog (Lumen) and Heterogeneous Volumes (path tracer).
        A scratch-project test scene: a fog box lit by a point light, and a smoke column.
      - Metrics:
        - Stills vs the UE Path Tracer: ΔE / SSIM;
        - in motion: bench-temporal-style convergence in the smoke while the camera moves;
        - GPU with a volume in Kabukichō's street at the 0.5 floor;
        - layer A: `volume/box-transmittance` (Beer–Lambert through a homogeneous box, analytic) and
          `volume/box-single-scatter` (a point light's in-scattering vs the analytic integral).
      - Status (2026-11-07), part 1: the media integrator and the contract.
        - `VolumeFunction` / `VolumeMesh` (box or sphere under the node's transform, majorant, HG anisotropy) and
          `FogVolume` (UE's Local Fog Volume) in the standard library (docs/MATERIAL_FUNCTIONS.md, "Volume functions").
        - The kernel holds a media table (binding 18), with the volume functions spliced in like material functions.
          - Paths: delta tracking per segment, with emission at every tentative collision.
          - Real collisions: phase-function NEE (directional, local, emitter, environment lights, MIS against phase
            sampling), then a Henyey-Greenstein sample.
          - Shadow rays: blocked with probability 1 − transmittance (ratio tracking); only `occluded()` changes.
          - All behind F_MEDIA: the 15 catalog scenes render byte-identical offline (software traversal).
          - Compiled in but unused (main vs this, alternating):
            - Kabukichō: 15.20 / 15.22 vs 15.22 ms still, 15.06 / 14.99 vs 14.98 / 15.00 ms moving;
            - district p50 17.34 / 17.32 vs 17.31 / 17.33 ms, p95 17.77 / 17.75 vs 17.73 / 17.75 ms.
        - Layer A against the analytic values:
          - `volume/box-transmittance`: 0.7362 vs 2e⁻¹ = 0.7358;
          - `volume/box-emission`: 1.264 vs 2 (1 − e⁻¹) = 1.264;
          - `volume/box-single-scatter`: 6.349 vs the single-scattering integral 6.349.
      - Status, part 2: the live estimator and the presets.
        - Live media are marched (docs/MATERIAL_FUNCTIONS.md, "Volume functions"); delta tracking stays offline.
          - Camera segments: 8 steps of transmittance, emission and in-scattering from the volume voxels (jittered
            lookups), into the signal of what the camera sees directly.
          - Later segments: 4 steps of transmittance.
          - The voxels are trained by `media_train`, 4096 points a frame, a pass of its own. Inlined in the kernel,
            `point_radiance` copied surface shading and all of Kabukichō's material functions into it: the first frame
            took about a minute.
          - With delta tracking out of the live kernel, Kabukichō with a volume reaches its first frame in 2.0 s
            (1.9 s without).
        - Live layer A, the same analytic values:
          - `volume/box-transmittance-live`: 0.7354 vs 0.7358;
          - `volume/box-emission-live`: 1.265 vs 1.264;
          - `volume/box-single-scatter-live`: 6.25 vs 6.35, −2 %, voxel quantization;
          - `volume/follow-node` and `volume/node-custom` (live pose and customData edits): 0.7355.
          - `volume/box-sun-scatter-live` (new, a 1000-lux sun): 13.4 vs 13.72, −2 %; offline 13.72.
          - `volume/box-transmittance-lit-live` (new, a lit wall behind the absorber): 0.7361 vs 0.7358.
        - Two bugs these tests found, fixed:
          - Live, the camera hit's direct light (a signal of its own) did not carry the transmittance to the camera, so
            lit surfaces behind media were not attenuated (2.00 for 0.7358). Height fog went through the same split:
            `fog/transmittance-lit-live` (new) reads 2.0 on main for 0.7358, now 0.7354. Cost: Kabukichō (height fog)
            +0.05 ms still (15.26 / 15.32 vs 15.22 / 15.24 ms; without this fix 15.21 / 15.21).
          - Volume-voxel training clamped direct light with the firefly clamp, cutting bright suns and lamps (21 % low in
            `volume/box-sun-scatter-live`); now only the sphere ray is clamped. Translucent particles share it.
        - `SmokeColumn` and `Fireball` in the standard library; `tools/volume-scene.ts` shows them with `FogVolume`.
          Its smoke column in front of the wall, lit by the moon alone, reads +5.7 % live over offline: two thirds
          from the isotropic phase (the smoke's is HG 0.2), a third from the 8-step march's transmittance; a lamp inside fog is a soft disc live (voxel detail), a peak
          offline.
        - GPU, a 2 × 5 × 2 m `SmokeColumn` in Kabukichō's street at the 0.5 floor:
          - rain off: 15.89–15.90 ms moving (14.86–15.01 without the smoke);
          - with Kabukichō's rain as well: 16.06 ms still, 17.01–17.02 ms moving, over 16.7.
          - The smoke costs about 1.0 ms (trace), from the march and from shadow rays through the smoke.
          - Tried: 12 → 8 camera steps and transmittance-only later segments (kept, −0.4 ms); three noise octaves
            instead of four (kept, −0.03 ms); no media in the surface cache's training shadows (−0.08 ms with a bias:
            dropped).
        - Next (part 3): UE references and the motion metric; cheaper media shadow rays to fit rain + smoke.
      - Status, part 3a: UE references, and `FogVolume` is UE's Local Fog Volume.
        - `FogVolume` takes UE 5.8's Local Fog Volume density as UE voxelizes it into volumetric fog
          (VolumetricFog.usf), with UE's settings and defaults (docs/MATERIAL_FUNCTIONS.md, "Volume functions").
          `volume/local-fog` (new): 0.2823 against the numerically integrated 0.2863.
        - UE references, `bun ue5/volumes.ts`: `tools/volume-scene.ts` in a scratch project, path-tracer and Lumen stills
          as linear EXR.
          - Every preset is a Heterogeneous Volume: a Volume-domain material whose Custom nodes are our volume
            functions in HLSL. The path tracer renders no Local Fog Volumes, and a Local Fog Volume through Lumen's
            volumetric fog hung UE 5.8 on this Mac twice (the whole GPU, until a reboot).
          - Measured UE conventions:
            - a Heterogeneous Volume's extinction and emission are per local cm, before the actor's scale;
            - its phase function in the path tracer is isotropic;
            - point and spot lights shadow through volumes only with Cast Volumetric Shadow, which is off by
              default. Without it, a lamp inside a sphere of medium read 11.3 against the analytic 4.23 for single
              scattering; with it, 4.07.
        - Against the UE Path Tracer (256 spp), at 1280 × 720 through the same display transform
          (`docs/img/volumes-ours-ue-pt.jpg`). Ours is offline at 1024 spp, with isotropic phase like UE's.
          - Mean ΔE 1.89, luminance ratio 0.995. SSIM is 0.77, but neither image is denoised.
          - Regions, ours / UE: wall 1.01, floor 0.99, smoke 1.01, fireball 0.99, and the fog 0.94 outside the lamp's
            own sphere (UE's path tracer shows it).
          - The fog's −6 % has a counterpart in a lamp inside a sphere of medium (σ 2 / m, albedo 1, 64 bounces): 23.8
            ours, 24.9 UE.
        - Next: Lumen and motion comparisons; the rain + smoke budget (part 3b).
      - Status, part 3b: rain + smoke fits the budget.
        - Live media are a pass of their own after the trace (`media_composite`):
          - the camera-to-first-hit march, as before, applied to the pixel's signals;
          - live, reflection, GI and shadow rays do not see media; offline paths still track them;
          - the media voxel training keeps media in its shadow rays, so smoke still shadows itself.
        - The megakernel holds no media code any more. Kabukichō + rain with a smoke column, trace pass:
          - 8.16 ms (rain alone), 8.58 with the column 50 m underground, 9.01 with it in view.
          - So 0.42 ms was inlined code no ray used, and the column's shadow-ray tracking cost 0.35 ms more.
          - Tried and not kept:
            - 2 fixed steps instead of ratio tracking on shadow rays: −0.1 ms;
            - 1 step on later segments, or 4 on camera segments: −0.05 ms each;
            - media shadows from directional lights only (one inlined copy): 16.70 ms moving, right at the budget.
        - Perf suite, local-main → this:
          - rain + smoke 15.99 / 16.96 → 15.53 / 16.48 ms still / moving (≤ 16.7);
          - every other line within the noise floor;
          - offline renders byte-identical, the volume scene's too.
        - **Live multiple scattering:** the media voxel training's sphere ray crosses the media (attenuation and
          in-scattering from the voxels), so the voxels converge to every scattering order.
          - `volume/lamp-inside{,-live}` (new): a lamp inside a sphere of albedo-0.9 medium reads 6.607 offline and
            7.61 live (+15 %). Single scattering alone had read 1.44 live, 22 % of it.
          - `volume/furnace{,-live}` (new, energy conservation): 0.999 offline, 0.963 live.
          - The volume scene's fog: live / offline 0.58 → 0.81, the lamp's surroundings 0.61 (voxel detail).
          - Against the UE stills (1280 × 720, same display transform; live with isotropic phase like UE's), live
            before → after: mean ΔE to the Path Tracer 2.54 → 2.20 and to Lumen 1.54 → 2.02.
          - The fog: 0.52 → 0.73 of the Path Tracer's. It is now 1.59 of Lumen's, whose fog lacks multiple scattering
            too (0.37 of the Path Tracer's 0.81 there). UE's Lumen against its own Path Tracer: mean ΔE 3.16.
          - Cost: within noise (rain + smoke 15.56 / 16.52 ms vs 15.54 / 16.57).
        - **Motion**, `node tools/bench-temporal.ts --shot volumes --region fog|smoke|fire`:
          - the volume scene at frozen time, the camera strafing 3 m in 1 s;
          - live frames against the same build's converged ones, mean ΔE / p95:

            | region | moving | stopped + 0.5 s |
            |---|---|---|
            | smoke | 0.51–0.81 / ≤ 2.84 | 0.50 |
            | fog | 1.21–1.40 / ≤ 4.52 | 1.15 |
            | fireball | 0.89–1.75 / ≤ 5.92 | 0.60 |

          - No trails: the media are the pixels' emission signal and follow the surface behind them.
        - **Known gap:** live, media cast no shadows on surfaces and do not show in reflections (homogeneous media,
          water, do since item 4 part 1).
          `docs/img/volumes-live-composite.jpg`: offline | live before | live after. A shadow-friendly structure
          (transmittance toward the main lights, cached per voxel) is left for later.
   4. Water: the waterfront.
      - How agents use it: `new WaterMaterial({ absorption, scattering, ior: 1.33, waves })`. A water body is a Mesh
        with it, plus a homogeneous `VolumeMesh` below the surface for the water itself.
      - Mechanism:
        - The surface uses the existing glass path (transmission, IOR) with animated normals from summed waves and
          scrolling normal maps in the material function.
        - The body is a medium from item 3, so light is absorbed and scattered along the refracted rays.
        - No geometric waves: material functions have no world position offset, and normal waves are what UE's
          Single Layer Water uses at street scale.
      - UE reference: UE 5.8 Single Layer Water (Water plugin) in a scratch-project canal at night: a pier, lights,
        a sign reflected in the water.
      - Metrics:
        - Stills vs UE Lumen and the Path Tracer: ΔE / SSIM;
        - reflections in motion with bench-temporal;
        - GPU with the canal in view;
        - layer A: `water/fresnel` (reflectance 0.020 at normal incidence for IOR 1.33) and `water/absorption`
          (transmittance through 1 m of water vs Beer–Lambert).
      - Status, part 1 (2026-11-08): the water itself, offline and live (local branch `water-part1`).
        - Landed: `WaterBody` (a `WaterMaterial` surface over a `HomogeneousVolume` box), `WaterMaterial` (glass of IOR
          1.33, four summed wave normals), `HomogeneousVolume` (a `VolumeMesh` of constant coefficients that live
          reflection, refraction, GI and shadow rays see, behind feature bit F_HMEDIA). `tools/water-scene.ts`: the
          night canal (pier, two lamps, a neon sign on a wall), the scene the UE reference will rebuild.
        - Layer A, offline / live: `water/fresnel` 0.0202 / 0.0200 (0.0201), `water/absorption` 1.192 / 1.190 (1.189),
          `water/furnace` 0.997 offline (1), `water/scatter` 0.1336 offline, 0.1279 live (−4.2 %). test:render 86 / 86.
        - Found and fixed along the way:
          - Live smooth glass lost its reflections. The reflection resolve dropped them and rebuilt them from
            neighbours that had almost all refracted: a glass sheet in a uniform sky read 0.762 at grazing incidence
            (offline 1.0). Transmissive camera hits now keep their reflections in the specular signal.
          - Offline, light under glass was counted twice. Shadow rays passed glass unattenuated, and the paths that
            refracted found the same sky at full weight. A white furnace through water read 1.53, then 1.25 with
            the MIS fixed (paths through glass that does not bend keep their vertex's MIS weights; behind glass that
            bends, the sky is theirs alone). With shadow rays now attenuated by each glass surface's transmission
            share, it reads 0.997. TransmissionTest against the UE Path Tracer: offline ΔE 5.69 → 5.66, PSNR 23.44 →
            23.59 dB; live @256f 5.68 → 5.67. Offline it takes 3.9 → 6.1 s: one more trace per shadow ray in scenes
            with glass. The 14 other catalog scenes are byte-identical; mannequin-plaza moves −0.01 %.
          - Live, the volume voxels never learned water. They were trained only where camera rays cross media before
            their first hit, which for water is its top layer, and their sphere rays stopped at the surface. Single
            scattering read 2.5× offline, then 0.79× with points down to the first opaque surface. With the
            sphere rays passing or reflecting at the surface and two lookups per crossing, it reads −4.2 %.
          - The radiance cache's training paths skipped homogeneous media (now attenuated and lit on every segment).
        - Canal, live against offline (frozen time, mean luminance by region): walls, walkways, pier, far water,
          under the pier and the sign's and wall's reflections within ±4 %. GPU at the 0.5 floor 13.1–14.1 ms, the
          same as the canal without water. Kabukichō perf smoke +0.02 / +0.04 ms still / moving; rain + smoke equal.
          `docs/img/water-canal.jpg`: offline | live, time frozen | live, waves moving.
        - Known gaps, for part 2:
          - The lamps' glints read 11 % of offline live: each lighting sample is clamped at 64× white, so a small
            light's mirror image is white but carries little energy for bloom.
          - Dark water near the camera reads +20 % live. Half of it is the missing Fresnel on live shadow rays
            through glass (+11 % with it; one more trace per shadow ray in glass scenes).
          - Live multiple scattering converges an order per training generation. Light trapped by total internal
            reflection needs many: albedo-1 water in a uniform sky reads 0.70 after 480 frames, 0.81 after 1920.
            Real water's albedo (0.1–0.3) converges within a few.
          - Extinction is achromatic (one coefficient, the colour from the albedo). Water's red absorption needs
            spectral tracking.
        - Next, part 2: the UE 5.8 Single Layer Water reference of the canal (Lumen and the Path Tracer, small careful
          runs), stills ΔE / SSIM against it, and reflections in motion with bench-temporal.
      - Status, part 2a (2026-11-08): reflections in motion (local branch `water-part2`).
        - `bench-temporal --shot water`: the canal with the camera still and the waves moving. At T = 6, 7 and 8 s,
          frames are compared with a reference converged 1024 frames at T. The live runner's new `time` command plus
          lockstep makes the scene time exact.
        - Before: long history smeared the reflections into streaks. Moving waves read ΔE 6.9 on the water and 20.6 on
          the reflections, SSIM 0.48 there; 240 frames held at T read 0.7 / 1.2.
        - Material functions now say where their shading moves with time (`o.animated`; the wave function sets it while
          its waves move), and the live view keeps 4 frames of history there, as for animated emission. Moving reads
          3.7 / 9.6 (SSIM 0.83 / 0.64): −47 % / −54 %. Its cost is noise: held still, 1.9–2.1 / 3.8–4.1.
          `docs/img/water-waves-motion.jpg`.
        - Not a detection of `i.time` in the code. Kabukichō's wet ground reads it for rain ripples, and a short history
          there would change the whole street. Catalog 16 / 16 byte-identical; Kabukichō perf smoke +0.00 / −0.03 ms.
        - Left for later: the excess over UE needs UE's water in motion (part 2b, with the stills).
   5. Foliage wind.
      - How agents use it: `windSway(model, { strength, frequency, stiffness })` from the standard library, on
        placed plant models.
      - Mechanism:
        - First, rigid sway: each plant (or branch node) is a movable node rotated about its pivot by a deterministic
          wind function. The existing path, cheap for street-scale counts.
        - Per-vertex bending (UE's WPO wind) only if that falls short. It would be a `VertexFunction` contract
          deforming the dynamic span before the BLAS rebuild, the posed models' path.
      - UE reference: UE 5.8 foliage with SimpleGrassWind / Pivot Painter and a Wind Directional Source, in a
        scratch project with the same plant models.
      - Metrics:
        - Sway amplitude and frequency vs UE (tracked points in MRQ frames);
        - ghosting on the foliage with bench-ghosting-style trails;
        - GPU with plants at the 0.5 floor;
        - layer A: `wind/pivot-sway` (a rigid plant's displacement at time t matches the wind function).
      - Status, part 1 (2026-11-08): rigid sway (local branch `foliage-wind`).
        - Landed:
          - `windSway(plant, { strength, frequency, stiffness, direction, phase })` sets a node's `sway`. The renderer
            leans it about its origin with scene time: every live frame, and for an offline still at its time.
          - The lean is A (0.6 + 0.4 sin(w t + p) + 0.2 sin(2.7 w t + 2 p)), A = strength / (1 + stiffness),
            w = 2 pi frequency sqrt(1 + stiffness). The phase p comes from the world position unless given.
          - The live runner's serve jobs take `time` as the last frame's scene time.
          - `tools/foliage-scene.ts`: a shop front in afternoon sun with potted plants (the pot fixed, the plant
            leaning about the soil) and ferns, Poly Haven CC0 models.
        - Layer A: `wind/pivot-sway` and `-live` read 10 of 10, the marker where `windAngle` puts it.
        - Cost: ~52 k swaying triangles, re-transformed in place on the CPU, take 2.4 ms of CPU a frame. GPU at the 0.5
          floor is within noise (10.4–10.7 ms swaying, 9.9–11.7 held); the scene stays at 60 fps.
        - Motion (`bench-temporal --shot foliage`): moving vs converged at the same time, ΔE 1.68–1.71 (plants) and
          1.60–1.67 (ferns), against 1.41–1.43 / 1.19–1.20 with time held. Motion vectors carry the history.
        - Found on the way: the hardware traversal's candidate loop lost hits near cutout foliage (black ground pixels).
          Fixed separately (`cutout-trace`, Known issues); it was also why hardware renders did not repeat.
        - Next: the UE reference (SimpleGrassWind / Pivot Painter in a scratch project), sway amplitude and frequency
          against it. Branch-level sway for trees, if rigid plants fall short.

## 6. Voxel engine (2026-11-09)

The rule (what goes into ThreeRT, what stays in the engine), the benchmark level, the research and the design are in
`docs/VOXEL.md`. One PR per step. **[ThreeRT]** steps take their tier's gate (R1 for a new contract: catalog scenes
byte-identical, perf smoke on Bistro with the contract compiled in but off). **[engine]** steps are measured on the
benchmark level. The order follows the dependencies; a step that meets a further gap in ThreeRT inserts it here.

1. **[ThreeRT] Procedural geometry.** AABB geometry plus a game-supplied intersection body (WGSL, like material
   functions) that returns t, a normal and a material, in both traversals (hardware: AABB BLASes and
   `rayQueryGenerateIntersection`; software BVH: AABB leaves). A hit kind of its own, and a `surface()` path that yields
   the same `Surface`. Instances move by transform, with motion vectors from the previous transform.
   - Acceptance:
     - Layer A: analytic spheres and boxes against their triangle versions.
     - The unbiased check passes on a procedural scene (hardware against software).
     - Existing scenes byte-identical; Bistro perf smoke within baseline + 0.3 ms with the feature compiled in.
   - Status (2026-11-09, branch `proc-geometry`): in, measured on the M5 Pro; the perf line is open (below).
     - The contract (docs/MATERIAL_FUNCTIONS.md, "Geometry functions"): `GeometryFunction` (WGSL `gf_<name>(GIn) ->
       GOut`: t, object-space normal, material index), `ProceduralGeometry` (object-space boxes and the game's u32
       data), `ProceduralMesh` (its materials). Behind F_PROC; `THREERT_FEATURES_ON` forces bits in.
     - Hardware: one AABB BLAS per mesh, a TLAS instance after the model instances. On Metal the AABB candidates come
       up under `RAY_FLAG_FORCE_OPAQUE` (a probe: 479 k of 1 M rays had one pending after the first proceed), so every
       query runs a candidate loop under F_PROC. One AABB-only loop repeats run to run (0 of 1 M rays differ in three
       runs); mixing cutout-triangle candidates into it does not (697–1,164 differ), the cutouts' own known issue.
     - Software traversal: each mesh's bounds, then its boxes in order, not AABB leaves in the BVH (the reference path;
       a voxel level needs a BVH over its bricks there, step 2).
     - `Hit` unchanged: `tri` = PROC_BIT | material << 23 | box, (u, v) the octahedral object-space normal, `inst` the
       record; `surface_proc` does not run the function again. No new binding (Metal allows 29 with the translucency
       pass, all taken): records in `instances`, boxes / materials / data after the environment CDF (`Params::procs`).
     - Moves rewrite the record and TLAS transform (motion vectors from the previous rows); raster primary visibility
       is off with procedural meshes; a click picks the boxes. Procedural surfaces: opaque, untextured, not
       light-sampled.
     - The geometry function is game code in a kernel compiled without bounds checks: its `gdata` / `gdataf` read only
       its own object's data (`i.data`, `i.count`; 0 outside). The same kernel splices material and volume functions,
       which can index any binding directly; that hole predates this step and stays open here (the PR's Open list).
     - Acceptance:
       - Layer A `procedural/*`, 11 tests, both traversals: sphere and box against SphereGeometry / BoxGeometry
         (halves 1.000), furnace 0.5005 (0.5), materials 2.5, customData 2.499, shadows, no-cast, with glass, a live
         move, data reads clamped (`data-bounds`: fails without the clamp), data bits kept (`data-bits`: 0xFFFFFFFF,
         0x7FC00001, 0x7F800001, 0xFF800000, 0x80000000, 0x00000001 through the f32 binding). `test:render` 103 / 103
         (main 92 / 92).
       - `unbiased-check --scenes procedural` (hardware 4096 spp against software 16384): mean 1.0001, worst region
         1.001.
       - Catalog offline, software traversal: 20 / 20 byte-identical with main `132ca2c`. A guard inside `glass_at`,
         compiled out, still changed Metal's code for the glass path (0.08 % of khronos-transmission's values): it
         sits in a wrapper (`glass_hit`). Hardware traversal: 14 / 20 byte-identical, the 6 others (cutouts) differ
         from main no more than main from itself (Bistro 0.05 % / 0.05 %, Zero Day 0.58 % / 0.80 %).
       - Bistro perf smoke with F_PROC forced in: not settled. Other applications kept the shared GPU at 20–56 % for
         hours (two runs: +0.36 / +0.25 ms forced, +0.23 / +1.42 ms not forced, whose code paths are main's). In the
         quietest window (2026-11-09 ~09:00 UTC, main 132ca2c re-measured, three configurations alternated, GPU lock
         held, quiet checked before and after each; 3 of 9 measurements discarded at 18–21 % busy) every measurement
         still fell on one of two levels, ~7.7 or ~9.6 ms still / moving, main's too: desktop compositing during the
         runs. On the low level: not forced = main (still 7.67 vs 7.69 / 7.73, moving 7.60 vs 7.52); forced +0.2 to
         +0.6 (still 7.95, moving 7.74 / 8.13), at the line. Drive GPU p50 on the low level 8.61 / 8.34 / 8.60 (main /
         not forced / forced).
         - The A/B the user settled on (one round, no quiet rule, GPU lock held; 2026-11-09 ~09:40 UTC): main 132ca2c
           against the branch with F_PROC forced in. Still 8.03 → 7.95 (−0.08), moving 7.38 → 7.78 (+0.40), drive GPU
           p50 11.54 → 9.90, p95 12.15 → 42.03, frame interval p50 / p95 16.65 / 16.92 → 16.65 / 16.89. Other
           processes' GPU time over the runs: mean 9.9 % / 11.4 %, peaks 76 % / 79 %. Read as over the +0.3 ms line
           (moving, drive p95) at first; withdrawn after the A/A below.
         - A/A, main 132ca2c against itself (the same binary, the same measurement, alternated, ~10:00 UTC):
           still 9.38 / 7.71 / 7.82 / 10.86, moving 11.04 / 7.70 / 8.35 / 10.71, drive GPU p50 8.12 / 11.49 / 8.09 /
           9.60, p95 8.12 / 12.36 / 12.05 / 12.22; other processes' GPU time mean 14.7 / 13.9 / 14.3 / 14.8 %, peaks
           75 / 78 / 69 / 76 %. One program spreads ~3 ms (still 3.2, moving 3.3, drive p50 3.4): on this machine, as it
           is used now, the measurement cannot resolve the 0.3 ms line.
         - Settled (the user's decision, 2026-11-09): R1. Existing scenes pay nothing: F_PROC is compiled out where no
           procedural geometry is (Bistro's code paths are main's) and the catalog is 20 / 20 byte-identical. The cost
           with F_PROC compiled in (+0.40 ms in the one A/B, inside the noise) is measured in step 2.
         - Step 2d (2026-11-09, 10:30–10:48 UTC; no dev servers running, screen not locked): one binary, main a6c2b8e
           (it has step 1), A = main, A' = main again, C = main with `THREERT_FEATURES_ON=262144`, alternated over
           5 rounds, GPU lock held, nothing discarded. Medians, still / moving / drive GPU p50 / p95 ms: A 8.48 / 7.55
           / 11.16 / 11.83, A' 7.76 / 8.49 / 10.25 / 12.85, C 9.92 / 9.99 / 8.72 / 11.34. The A/A difference, the
           noise: 0.72 / 0.94 / 0.91 / 1.02, above the 0.3 ms line; C against A and A' goes up on still and moving
           (+1.8, +2.0) and down on the drive (−2.0, −1.0), noise again. Other processes' GPU time 13.5–19.3 % on
           average per measurement, peaks 70–88 % (the desktop). Not resolvable here: medians of 5 still carry ~1 ms.
2. **[engine] Voxel objects and the benchmark level, static.**
   - Sparse 8³ bricks, palettes and `.vox` import.
   - The benchmark level (`docs/VOXEL.md`).
   - Measure first, too: step 1's cost with F_PROC compiled in (Bistro, `THREERT_FEATURES_ON=262144` against main),
     with the screen locked or off, main and branch alternated over several rounds, judged by the medians against the
     +0.3 ms line; an A/A of main beside it gives the noise (step 1's status: ~3 ms with the desktop in use).
     - 2d, run (step 1's status): 5 alternated rounds with an A/A, the desktop in use; the A/A medians differ by
       0.7–1.0 ms, so the 0.3 ms line cannot be judged from them. What could: the screen off (the desktop's GPU time
       gone), or many more rounds (the medians' spread falls only as the square root of the rounds: ~50 for 0.3 ms
       at this noise), or a measurement of the trace pass alone that the desktop does not reach.
   - 2a-1 (branch `proc-data-assets`, 2026-11-09): procedural data from asset files, and kept on the GPU through
     rebuilds.
     - The check this step asked first, measured (`tools/proc-data-scene.ts`, 256 MB of data, `bench-hitches` with a
       sky change every 3 s): every change was a 88–91 ms frame (73 ms of CPU); the same scene without data 28 ms.
     - Now each distinct geometry is stored once (SHA-256 of its boxes and data), and a rebuild with the same
       geometries copies their data GPU to GPU from the last upload's buffer; only the CDF and the headers are sent:
       33 ms (17 ms of CPU). Every upload still owns its buffer (the scene being replaced, or another room's on a render
       node, may still read the last one), so the CDF stays first and the env-sampling code is untouched; a different
       CDF length only moves where the copy lands. A first build with no matching geometries writes the data once into
       the new buffer's mapping (no intermediate copy).
     - `ProceduralGeometry.fromAsset(fn, uri)`: the boxes and data from a file the scene build reads (format and
       checks in docs/MATERIAL_FUNCTIONS.md, "Procedural geometry files"; written by tools/lib/proc-asset.ts), so the
       level's data never passes through QuickJS. Malformed files are refused (unit tests: magic, version, byte order,
       length both ways, counts against a short file, reserved bytes, hash, boxes).
   - 2a-2 (branch `proc-geometry-2`, 2026-11-09): the contract's improvements from step 1's review.
     - One AABB BLAS per distinct geometry, a TLAS instance per mesh, the BLASes kept across rebuilds by hash (a sky
       change builds none of them again).
     - `GOut.attr` (vec2; in the hit's v as two half floats rounded to 15 bits, exact integers up to 1024; the normal
       in u as two 15-bit components): the material function's `i.uv`. Both inside ordinary floats (exponent bits
       01xxxxxx): the first packing put raw snorm bits in u, and the +Y normal, 0x7FFF0000, is a NaN pattern WGSL does not
       promise to keep through a bitcast (Bugbot; `procedural/normal-up`).
     - Software traversal: a BVH over each geometry's boxes (bvh::build_boxes, the triangle BVH's SAH; the triangle
       path unchanged), built only when a software render needs it, and its nodes and leaf order in the blob.
     - Materials: one per material object already (a palette shared by every mesh is sent once; binding test).
     - Layer A: `procedural/attr-uv`, `normal-up`, `shared-geometry`, `many-boxes` (2048 boxes, both traversals
       0.9991 for 1; a software BVH that tests one box per leaf reads 0.27).
   - 2b (branch `voxel-package`, 2026-11-09): `packages/voxel`, the engine's voxel objects on the contract (TS and WGSL,
     no Rust; docs/VOXEL.md, "Voxel data").
     - `VoxelGrid`: sparse 8³ bricks, a byte per voxel (0 empty, 1..255 a palette entry); its layout: a dense brick grid,
       per brick 16 words of occupancy and a material word (uniform bricks: the material; else 128 words of bytes).
     - `VOXEL_BRICKS`: a DDA over the bricks inside the box the ray entered, then over the voxels of each non-empty brick;
       one body for boxes per brick, per 32³ region or per object (step 2's experiment switches `Granularity`).
     - `.vox`: `parseVox` (SIZE / XYZI / RGBA, every length checked), `voxGrid` (z up to y up), `encodeVox`,
       `paletteMaterials`; `tools/voxel-build.ts` writes a file's models as procedural geometry assets.
     - Found on the way: the voxel DDA's summed step times drift from the brick's exit time, so a step could leave the
       brick unnoticed and read another voxel's bit: phantom bricks outside a cuboid (`voxel/cube-mesh` 0.9938,
       granularity pairs 0.995). It now stops on the cells themselves; a JS port of the body against a cuboid's slab
       test, 10⁵ rays per granularity from outside: 0 missed, 0 extra, 0 wrong t (and 0 phantoms in 2 × 10⁵ rays from
       anywhere).
     - Also found (Bugbot): a ray with a zero direction component missed every voxel (the hit test's entry and exit used
       the reciprocal's 1e20 stand-in on that axis): ~91 % of axis-aligned rays in the port; `voxel/shadow-axis` (a
       point-like sun straight overhead) read 0.50 for 0. Such an axis now never bounds the hit test.
     - Layer A `voxel/*`, both traversals: cube against BoxGeometry 1.000, shadow rays exactly along +y 0 (0), object and region boxes against brick boxes
       0.9997, per-voxel materials in one brick 2.5, a single-voxel checker 0.9995 (1), a `.vox` file through the
       scene runtime 2.5. `unbiased-check --scenes voxel` (tools/voxel-scene.ts): mean 1.0000, worst region 1.002.
   - 2c-1 (branch `voxel-level`, 2026-11-09): the benchmark level, generated (docs/VOXEL.md, "The benchmark level").
     - 4172 objects (1600 terrain tiles, 172 buildings in 688 parts, 1884 props over 14 shared geometries), 514.8 M
       voxels in 1.32 M bricks: 440.6 k exposed, 888.7 k full; 270 MB; generated in ~4 s per box granularity; loaded
       in ~0.7 s (offline render's load).
     - Box granularity is one switch for every object (`generate.ts --granularity`, `level[-brick|-object].game.ts`):
       1.32 M / 46.9 k / 4172 boxes. The three render alike (overview at 32 spp: mean 0.99999 / 1.00000 of region's,
       99.8–99.9 % of pixels identical).
     - Not yet: software traversal walks every procedural object per ray (4172 here), too slow for a reference render of
       the level; a top-level BVH over the objects comes before one.
   - 2c-2 (branch `bench-voxel`, 2026-11-09): `tools/bench-voxel.ts`, the level's measurements (docs/VOXEL.md,
     "Measured: the box granularities").
     - Per variant: GPU memory (procedural BLASes at Metal's own sizes, `proc_blas_mb`; procedural data,
       `proc_data_mb`), distinct geometries / instances, load and first frame; live at the 0.5 floor along the 60 s
       camera path, trace pass and frame GPU p50 / p95 and frame start p95.
     - The trace pass per frame from the frame timer's timestamps (frame log `trace`): no THREERT_PROFILE stalls, so
       every variant is timed alike and frame start is the real one.
     - Variants alternate over rounds with an A/A of the first; other processes' GPU time is recorded per run, nothing
       is discarded (`--merge` puts runs together).
     - A control: every prop its own geometry (`--distinct-props`; 4100 distinct geometries instead of 2230), as BLAS
       sharing flatters the props.
     - Found on the way: 72 of the level's 2302 geometry files are byte-identical, and the GPU holds each once; the
       generator now reports distinct geometries and their data.
     - Measured, 4 rounds (medians; trace p50, frame start p95): region boxes 30.1 / 42.1 ms (A/A 30.2 / 43.3),
       brick 29.0 / 47.4, object 20.1 / 28.5, all props distinct 33.6 / 45.1. None is within 16.7 ms yet.
     - Step 1's F_PROC compiled in, on Bistro night's trace pass (20 s of its path, 5 rounds with an A/A): 4.40 →
       4.62 ms (+0.22 ms, every round 4.61–4.67 against 4.37–4.42; the A/A 0.01 ms). Only scenes with procedural
       geometry compile it in.
   - 2c-3 (branch `voxel-greedy`, 2026-11-09): greedy-meshed triangles (docs/VOXEL.md, "Measured: greedy-meshed
     triangles").
     - `greedyMesh` in `packages/voxel`: exposed faces merged per plane and material; `generate.ts --granularity mesh`
       writes a GLB per geometry (23 s; 1.90 M triangles, 13.25 M as placed); `level-mesh.game.ts` places them as
       models. The renderer reports the triangle BLASes (`tri_blas_mb`, Metal's sizes and the vertex buffer) and their
       data (`tri_data_mb`).
     - Measured, all five representations in one run (4 rounds, A/A of region; trace p50, frame start p95): region
       29.2 / 41.6 ms (A/A 29.4 / 41.6), brick 24.9 / 33.6, object 17.3 / 23.9, distinct props 31.5 / 42.5, **mesh
       2.19 / 16.67**: the only one within the frame budget. Its cost: 258 MB of BLAS against 3.6, and re-meshing on
       edits.
     - Follow-up (branch `voxel-dedupe`, 2026-11-09): 2c-3 found 72 geometry files with the same contents (building
       parts alike, the narrow tiles at the far edge), separate meshes since models are shared by file. The plan now
       names them alike (`buildingGeometry`, `tileGeometry`) and the generator writes each once, checking every
       instance gives the same file: 2230 files; the mesh 1.89 M distinct triangles (−11 k), BLASes 256.4 MB (−1.5);
       images unchanged.
   - The intersection experiment: AABB per object, per 32³ region, per 8³ brick; greedy-meshed triangles (2c-1 to
     2c-3). The compute DDA is deferred (2026-11-10): triangles trace 8× faster than the best box split for a static
     level, but only with ray-tracing hardware (native builds). Players play in the browser, and WebGPU has no ray
     queries: whether the browser traverses triangles or walks a voxel DDA in compute waits for the user's decision on
     a browser feasibility experiment. What decides the native representation now is what an edit costs.
   - 2c-4, the cost of destruction (the user's decision, 2026-11-09; replaces the compute DDA probe): the level under
     explosions, triangles against object boxes. Two PRs (the user's rule: one PR per step or feature, split only for a
     ThreeRT contract with its own gate):
     - The level's targets (docs/VOXEL.md, "The benchmark level"; the user's, 2026-11-10): ΔE ≤ 3 and SSIM ≥ 0.9 against
       our offline render, overall and by region, ghosting within bench-ghosting's limits, emissive voxels lighting
       their surroundings, no seam leaks, per-voxel colour; memory ≤ 2 GB per level (BLASes, triangle and procedural
       data together), which the benchmark level is accepted against (today: 516 MB as meshes, 267 MB as region boxes;
       nearing it brings compressed attributes, BLAS compaction and LOD forward).
     - A [ThreeRT, R1; part of step 5 brought forward], with this plan and the look's targets in docs/VOXEL.md:
       replacing, adding or removing one object's geometry at run time without rebuilding the GPU scene (until now
       every geometry change was structural: the whole scene was rebuilt).
       - Status (branch `geometry-edits`, 2026-11-10): done.
         - The world records geometry edits apart from structural changes: geometries replaced, objects added (or
           re-created with other geometry or materials) and removed, for objects with their own BLAS: procedural
           meshes, and meshes with the new `dynamicGeometry` (instanced like a model placement, shared by objects of
           the same geometry and material). Everything else stays structural.
         - The upload reserves room past each buffer's own data (triangles, kernel records, object entries,
           procedural words, TLAS instances; a quarter of the scene's, `THREERT_EDIT_RESERVE` scales it) for scenes
           with editable geometry under hardware RT only: other scenes' buffers are unchanged. An edit writes the
           touched objects' data into it as the build would (the build and the edit share the per-object code),
           builds BLASes for new geometry only (procedural ones kept by hash), sets the TLAS instances and rebuilds
           the TLAS: one submit. What an edit replaces stays until the next rebuild.
         - It falls back to the rebuild, and the frame log says why (`fallback`, `fallbacks`): software traversal, the
           room full, a kernel feature the scene lacks (its first procedural or instanced object, first cutout), an
           emitting / glass / function material, a new material, an object the build flattened, a move of an object
           added in place. Frame log `edit` / `rebuild` (CPU ms); with `THREERT_PROFILE_BLAS=1`, `blas` times the
           edit's builds on the GPU.
         - Checked: layer A `geometry/edit-live` (a box removed, a mesh's geometry swapped, a box and a mesh added,
           all in place; analytic 0.5) renders byte-identical in place, with every edit rebuilt
           (`THREERT_EDIT_REBUILD=1`) and with no room (falls back).
         - On the level (`bench-voxel --edits`: one object replaced every half second, camera still; 3 rounds,
           medians): a procedural tile (region boxes) 0.55–0.57 ms of CPU in place, its frame as any other (27.2–27.4
           ms), its BLAS and the TLAS 0.9–1.25 ms on the GPU, against 702–716 ms rebuilt; an 8192-triangle
           `dynamicGeometry` mesh on the mesh level (13.25 M triangles) 0.7–1.5 ms, its frame 17.2–17.8 ms (60 fps),
           1.7–1.9 ms on the GPU, against 483–530 ms rebuilt. No fallbacks.
     - B [engine and the experiment], one PR: what was planned as B, C and D.
       - Status (branch `voxel-destruction`, 2026-11-10): done. **The choice: triangles only, pieces batched**
         (docs/VOXEL.md, "Measured: destruction").
         - `renderer/voxel-core` (builds for wasm32, CI checks it): bricks, layout decode / encode, carve, binary greedy
           meshing (the same rectangles as `greedyMesh`), split. `renderer/src/voxel.rs`: `voxelLevel` /
           `voxelExplode` / `voxelFrame`. Loaded as triangles, the level has the TS generator's 13,251,212 triangles.
           Meshing a terrain tile 0.57 ms median, a building part 0.37–0.44 ms.
         - Batched pieces: an explosion's pieces one mesh per material, rebuilt every frame (a geometry of the same
           triangle count is rewritten in place, its BLAS rebuilt: a ThreeRT generic contract, three.js's dynamic
           draw usage), merged back into their objects 1.5 s after.
         - One explosion, frame p50, batched / each piece an object / boxes: 0.5 m 28 / 24 / 17 ms, 1 m 26 / 32 / 17,
           2 m 29 / 38 / 18, 4 m **38** (max 43.5) / 59 / 24. Sustained and peak, frame start p95: batched 16.67 / 16.67 ms
           (every run), each piece an object 16.67 / 16.67, boxes 20.1 / 18.9 (their trace alone 14–15 ms). Stress (one
           a frame): the batched route falls back from the 176th explosion (the triangle room).
         - The lines (≤ 50 ms for the 4 m explosion's frame, sustained and peak frame start p95 ≤ 16.7 ms): the batched
           route meets both. A piece as its own object costs ~23 µs of edit CPU and ~13 µs of GPU builds a mesh;
           batching pays that once per material.
         - Considered and dropped: the hybrid (pieces and what was just broken as boxes until settled): two
           representations to maintain, the browser's `web/` cannot draw WGSL boxes, and it leaves the cost per piece.
         - Fixed on the way: `World::set_geometry` scanned every object for a new id too (44 ms of a 4 m explosion); an
           edit that fell back left an unbuilt procedural BLAS cached (the rebuild failed wgpu validation); the frame
           log's `edit` / `rebuild` / `fallback` were reset by a frame's second commit.
       - Native voxel objects in `renderer/src/voxel/` (bricks from the `.tpg` files) with `carve` (a sphere;
         the changed bricks), binary greedy meshing (64-bit column masks) and a first `split` (carved voxels grouped by
         4³ into pieces). Scene code sends commands; QuickJS never touches voxels.
         - Acceptance: the same faces as `greedyMesh` (the TS tests' grids); meshing time, median and maximum, of a 64³
           terrain tile and of a building part (published binary meshers: 50–200 µs per 62³ chunk).
       - The experiment (`level-destruction.game.ts`, `tools/bench-destruction.ts`), at the 0.5 floor, with the GPU
         lock, ≥ 4 alternated rounds and an A/A.
         - Scenes: one explosion of radius 0.5 / 1 / 2 / 4 m (thousands to tens of thousands of voxels, across several
           tiles and building parts); sustained, 5–10 explosions a second for 10 s; a peak, 20 explosions within a
           second (a chain). Each explosion leaves tens to hundreds of new pieces, each its own geometry, at rest
           (bodies are steps 7 and 8); pieces under 8 voxels become no object. One explosion every frame is kept as
           a stress limit, to find where it breaks.
         - Triangles: the touched objects meshed again and their BLASes rebuilt; pieces meshed, a BLAS each. Object
           boxes: only the changed bricks uploaded (an object's BLAS rebuilt when its box changes); pieces as
           procedural geometry.
         - Figures: the explosion frame's time and frame start p95 while it goes on; split into the edit, meshing,
           upload, BLAS / TLAS rebuild and CPU wait; per touched object its meshing and BLAS time; per piece its cost;
           the scene's object count over time; the fallbacks to a full rebuild.
       - The results and the choice.
         - Lines: the explosion frame ≤ 50 ms; sustained, frame start p95 ≤ 16.7 ms; memory within the level's 2 GB.
         - Triangles meet both at 4 m: the engine renders triangles throughout.
         - Triangles miss and object boxes meet them: a hybrid. The static world as triangles; objects just broken or
           still changing, and pieces, as boxes; meshed on a background thread once they stay unchanged and swapped in
           (the swap measured too).
         - Neither: the split names the bottleneck and the next step; the largest radius that meets the lines is
           recorded as what the engine affords today.
   - Acceptance:
     - The experiment's table: GPU ms of the trace and of the frame at the 0.5 floor along the level's camera path,
       p50 / p95.
     - The choice is recorded here with its numbers, and frame start p95 on the level is measured.
2e. **[ThreeRT] `InstancedMesh`.** Many copies of one geometry as one object (the user's decision, 2026-11-09; started
   once step 2c is done).
   - Why:
     - ThreeRT's DSL follows three.js, where `InstancedMesh` is the standard way to draw many repeated objects
       (vegetation, crowds, debris, props, particles): agents write it, and the DSL lacks it.
     - N separate objects cost far more in QuickJS than one object and N matrices: memory, setup time, and the per-frame
       change tracking.
     - It meets the rule: every kind of game needs it, and it is a generic contract, not a voxel one.
   - Scope:
     - As in three.js: `new InstancedMesh(geometry, material, count)`, `setMatrixAt` / `getMatrixAt`, `setColorAt`,
       `instanceMatrix.needsUpdate`, `count` lowered at run time.
     - Triangle geometry and `ProceduralGeometry` alike.
     - Picking returns the `instanceId`.
     - Optional `customData` per instance.
     - The renderer: one shared BLAS, N TLAS instances; moving instances changes only the instance table, no rebuild.
     - Not `BatchedMesh`.
     - Step 5 (incremental structure) is the general case, any object added or removed; this item is the instanced
       object's API and its updates.
     - What three.js's usual instancing code needs besides (`dummy.position.set(...); dummy.updateMatrix();
       mesh.setMatrixAt(i, dummy.matrix)`), missing from the DSL today (only `updateMatrixWorld` and `Matrix4.compose`
       exist): `Object3D.matrix`, `updateMatrix()`, `matrixAutoUpdate`; `Matrix4.makeTranslation`,
       `makeRotationX` / `Y` / `Z`, `makeScale`, `invert`, `setPosition`.
   - Acceptance:
     - The typical code of three.js's official instancing examples runs unchanged, `tools/three`'s comparison renders
       included.
     - 10 k instances against 10 k separate meshes: setup time, and CPU and frame time with every instance moving each
       frame.
     - A scene: small debris (pieces under the voxel engine's threshold, step 2c-4) as instances of a unit cube.
     - The catalog's renders byte-identical; R1.
   - Status (branch `instanced-mesh`, 2026-11-10): done.
     - DSL: `InstancedMesh(geometry, material(s), count)` (triangle geometry or `ProceduralGeometry`),
       `InstancedBufferAttribute`, `setMatrixAt` / `getMatrixAt`, `setColorAt` / `getColorAt`, `setCustomDataAt` /
       `getCustomDataAt` (four numbers for `i.custom`), `instanceMatrix.needsUpdate`, `count` (up to the constructor's),
       `DynamicDrawUsage`; `Object3D.matrix` / `updateMatrix()` / `matrixAutoUpdate`; Matrix4 `makeTranslation`,
       `makeRotationX` / `Y` / `Z`, `makeScale`, `invert`, `setPosition`, `set`, `equals`, `fromArray` / `toArray`; Color
       `copy`, `equals`, `getHex`, `fromArray`; `Quaternion.random`. The binding sends one object and the instance
       table (`Engine.instances`: count, matrices, colours, data, each only when new or flagged).
     - Renderer: the build places one shared mesh (one BLAS) and every instance of the capacity as a TLAS instance
       with its own object entry (pivot, bounds, data, colour). Past `count`, or at a scale of 0 (three.js code hides
       instances so), an instance is a point with TLAS mask 0. A change rewrites the drawn range's records, object
       entries and TLAS transforms (one write per buffer) and rebuilds the TLAS: no rebuild of the scene. Scenes with
       instance colours get a 4th vec4 in each object entry and a kernel patched to multiply the base colour by it
       (F_TINT, gpu.rs `tinted_source`); every other scene keeps the object layout and the kernel source as they
       were. Picking returns `instanceId` (the click event's field). Flattened instead, and rebuilt on change: glass /
       emissive / function materials, mirrored instances, software traversal.
     - Checked: layer A `geometry/instanced` (colours), `geometry/instanced-procedural` (procedural boxes with
       per-instance data), `geometry/instanced-live` (moved, hidden by count and by scale 0, recoloured: in place,
       byte-identical to every change rebuilt); three.js's official instancing code in packages/scene's tests and in
       `games/instancing/instancing.game.ts` (catalog "instancing": 1,000 instances turning every frame, a click
       recolours one; 60 fps, CPU 3.6 ms), which `tools/three` also renders on three.js; the catalog byte-identical.
     - 10 k instances against 10 k meshes (`tools/bench-instancing.ts`, 4 rounds, medians; 0.2 m cubes, every one
       moving each frame the usual three.js way):

       | | setup (game) | first frame | moving: frame start p50 / p95 | CPU | QuickJS | renderer |
       | --- | --- | --- | --- | --- | --- | --- |
       | InstancedMesh | 10 ms | 106 ms | 16.7 / 16.7 ms | 11.8 ms | 9.9 ms | 1.0 ms (instance table) |
       | 10 k meshes (movable) | 145 ms | 262 ms | 52.0 / 53.6 ms | 52.0 ms | 42.7 ms (syncing 10 k transforms) | 7.3 ms (triangles) |

       GPU and trace the same (4.7 / 1.3 ms). The instanced frame's QuickJS time is the game's own loop
       (`dummy.position.set`, `updateMatrix`, `setMatrixAt` 10 k times: 6.3 ms, 19.8 ms before the DSL's fast paths:
       one change report per `set`, `updateMatrix` without accessors, `setMatrixAt` by `Float32Array.set`, the change
       trackers in an array).
     - Found and fixed on the way: generated names searched from 1 for every unnamed object (10 k meshes: 8.06 s of
       setup, now 145 ms); `World::note_move` scanned every object for each move (now only where it decides).
     - Debris (`games/voxel-level/level-destruction-debris.game.ts`): voxel-core's split hands the voxels too few to
       make a piece to scene code (`voxelDebris`); the game throws them as instances of a unit cube coloured from the
       palette, which fall, bounce and go after 4 s. Against the batched route alone (`bench-destruction --routes
       batch,debris`, 2 rounds with an A/A each, medians): one explosion of 0.5–4 m throws 30–617 (929 in flight at
       most), its frame p50 29.6 ms against 30.3 (max 44.6 / 39.9); 7 a second for 10 s throw 3,506 (up to 1,677 in
       flight), explosion frames 20.1 / 19.6 ms, frame start p95 16.67 ms in every run; no frame over 50 ms, no
       fallback. The instance update 0.05–0.11 ms of CPU a frame (the batches' edit builds the TLAS for both), 1.9–5.1
       ms when it builds the TLAS itself (the level's ~20 k instances).
2f. **[ThreeRT] three.js compatibility.** The DSL shares three.js's class names and constructors, not yet its everyday
   API (the user's decision, 2026-11-09; after 2e).
   - Missing today (packages/scene, 2026-11-09):
     - `BufferGeometry`: `setAttribute`, `BufferAttribute`, `Float32BufferAttribute`, `getAttribute`, `setIndex`,
       `addGroup`; `translate`, `rotateX`, `center`, `computeBoundingBox`, `dispose`.
     - `Object3D`: `clone`, `attach`, `removeFromParent`, `getObjectById`, `rotateX` / `Y` / `Z`, `rotateOnAxis`,
       `translateX` / `Y` / `Z`, `localToWorld`, `worldToLocal`, `getWorldQuaternion`.
     - `raycast` and `Raycaster`.
     - `Mesh` with an array of materials (with geometry groups).
   - Method: a compatibility test set of typical three.js snippets (official examples, the code LLMs usually write),
     with the share that runs unchanged recorded. The gaps are filled in order of how often they occur, and the share
     is recorded again after each.
   - Two semantics the user set:
     - Shadows stay on by default: the physically correct behaviour of a path tracer, documented as a deliberate
       difference from three.js. An explicit `castShadow = false` is still honoured. `renderer.shadowMap.enabled` /
       `.type` and the like are accepted and do nothing (no error).
     - Materials stay PBR only, mapped in the DSL, each mapping noted once on first use:
       - `MeshLambertMaterial`: standard, roughness 1, metalness 0.
       - `MeshPhongMaterial`: standard, roughness ≈ √(2 / (shininess + 2)).
       - `MeshBasicMaterial`: a surface showing its own colour only and lighting nothing else, which needs a generic
         kernel switch "emissive, not light-sampled" (R1).
       - `MeshToonMaterial`, `MeshNormalMaterial`, `MeshDepthMaterial`: standard, with a warning.
     - Order: Lambert and Phong first (TS only), then Basic.
   - Geometry that changes at run time (2c-4's PR A): three.js's `attribute.setUsage(DynamicDrawUsage)` maps to
     `dynamicGeometry` (its own BLAS ahead of the change). And without it, a mesh whose geometry changes at run time is
     promoted automatically, as a first move promotes to `movable`: its own BLAS from then on, that first change
     rebuilding once (frame log `fallback` "promote").
   - Acceptance: the test set's pass rate, recorded at the start and after each gap filled; the catalog's renders
     byte-identical.
   - Status (branch `three-compat`, 2026-11-10): done.
     - The set: tests/three-compat, 34 snippets (official examples' core code and LLM-typical code: geometry,
       Object3D, materials, lights, raycasting, the renderer's set-up, instancing), run by `tools/three-compat.ts`. A
       snippet passes when it runs unchanged (bound, synced, its frame stepped), its own check of the three.js
       behaviour it relies on holds, the DSL noted nothing `unsupported`, and the renderer draws it (not black, not
       flat, colourful where colour is the point). packages/scene's tests run every snippet too (without the render).
     - Pass rate: **1 / 34 at the start** (27 snippets stopped at `AmbientLight`, the most frequent gap; then
       `HemisphereLight` 5, `MeshPhongMaterial` 4, `MeshLambertMaterial`, `DoubleSide`, `Vector2`, `Raycaster` 3 each),
       31 / 34 after the DSL's gaps (run only), **32 / 34 at the end** with the renders. Still open: `Shape` /
       `ExtrudeGeometry` (polygon triangulation) and vertex colours (the renderer has none yet).
     - Filled: `BufferAttribute` / `Float32BufferAttribute` and the geometry's attributes (`setAttribute`,
       `getAttribute`, `setIndex` / `getIndex`, `addGroup`, `setUsage`), `translate` / `rotateX` / `scale` / `center`,
       `computeBoundingBox` / `Sphere`, `toNonIndexed`, `clone`, `dispose`; `CircleGeometry`, `RingGeometry`,
       `LatheGeometry`, `CapsuleGeometry`, the polyhedra; box groups. `Object3D`: `clone` / `copy`, `attach`,
       `removeFromParent`, `getObjectById` (three.js's number `id` until scene code sets a stable one), rotating and
       translating on axes, `localToWorld` / `worldToLocal`, `getWorldQuaternion` / `Scale` / `Direction`, the `is*`
       flags. `Raycaster` (scene code's own geometry, as three.js: meshes, groups, instances with `instanceId`,
       procedural boxes; it runs wherever the game runs, the browser too). Mesh material arrays (geometry groups, in
       the renderer). `Vector2`, `Box3`, `Sphere`, `Ray`, `Color.setHSL`. `AmbientLight` / `HemisphereLight` (the
       environment of a scene without one: radiance colour x intensity / pi; not seen), `scene.background`, `Fog` /
       `FogExp2` (uniform exponential fog), the lights' `shadow` settings and `WebGLRenderer` (accepted, ignored), the
       materials as the user set them, `flatShading`, `DynamicDrawUsage` and the automatic promotion.
     - Kernel: two switches compiled only into scenes that use them, the kernel source patched at checked anchors
       (gpu.rs `kernel_source`, as 2e's instance colours): MeshBasicMaterial (unlit: its colour to camera rays and
       mirror-like paths, albedo 0, in no emitter list) and the background (camera rays and paths bent straight
       through glass that miss see it, `Params::background`). renderer/shaders is as on main: every other scene's
       kernel is main's. Layer A: `material/unlit`, `material/unlit-lights-nothing`, `environment/background`,
       `environment/background-lights-nothing`, `geometry/groups`.
     - Chosen here (the user's semantics aside): in a scene lit by ambient lights the camera takes three.js's fixed
       exposure (EV 0) unless scene code set one, and the background defaults to black (three.js's clear colour);
       background and fog colours are shown at the level of the scene's light (its ambient and directional lights'
       irradiance / pi); light `decay` is always 2 (physical), noted as a mapping.
2g. **[ThreeRT] Vertex colours.** three.js's `color` attribute and `material.vertexColors` (planned after step 4; the
   supervisor's, 2026-11-10), and the voxel meshes' palette as vertex colours where that saves meshes.
   - Status (branch `vertex-colors`, 2026-11-10): done.
     - DSL: the geometry's `color` attribute (RGB; RGBA's alpha dropped; normalized integers to 0..1) goes to the engine
       with the geometry, `vertexColors` with the material; neither is noted `unsupported` any more. The compatibility
       set: **33 / 34** (`geometry-official-buffergeometry` now passes, its render coloured; `Shape` /
       `ExtrudeGeometry` still open).
     - Renderer: the build keeps three sRGB 8-bit colours per triangle of a mesh whose material has `vertexColors`
       (`scene::tri_colors`; the palette's colours exactly), `prepare` lays them out as the GPU's triangles. The kernel
       switch F_VCOLOR (gpu.rs `kernel_source`, as 2e's instance colours) interpolates them in linear at the hit and
       multiplies the base colour of surfaces and glass. They ride after the previous vertices in `prev_verts` (from
       vec4 `Params::procs.w`): the layout has no binding to spare (the translucency pass is at Metal's 29). Scenes
       without them: `prev_verts`, `Params`, the object table and the kernel source as on main. In-place edits write
       an added mesh's colours into the edit room (white where it has none); a scene's first coloured mesh rebuilds
       (a kernel feature it lacks).
     - Layer A: `material/vertex-colors` (0 .. 1 across a quad: 0.5 at its centre, 0.5012), `material/vertex-colors-
       instanced` (x an instance colour of 0.5: 0.2506), `geometry/vertex-colors-live` (added in place: 0.4992,
       byte-identical to every edit rebuilt and to no room).
     - Voxels: `voxel_core::merge_by_class` and `voxelLevel({ colors: "vertex" })`: one mesh per class of palette
       entries alike in all but their colour, each vertex its voxel's colour. On the benchmark level it saves little
       (docs/VOXEL.md, "Measured: vertex colours"): the same 13.25 M triangles and the same image, 9,916 meshes placed
       instead of 10,459, 27 MB of colours, the trace within the A/A's noise. The level keeps one mesh per entry;
       vertex colours are there for palettes of many colours in one material (`.vox` models) and the browser.
3. **[ThreeRT] Dynamic emission.** Any geometry, procedural included, can start or stop emitting without a rebuild,
   with light-tree entries for non-triangle emitters.
   - Acceptance: toggling emitters in Bistro costs no rebuild; ΔE against our offline render unchanged where nothing
     toggles.
   - Status (branch `dynamic-emission`, 2026-11-10): done for triangles; procedural geometry's emission is not done
     (procedural surfaces are not light-sampled at all yet: that needs non-triangle light-tree entries, later).
     - Emitting `own_blas` meshes (scene code's `dynamicGeometry`) are emitter meshes: their own BLAS, triangles in
       world space at an identity placement (never shared), so light sampling reads them as flattened triangles. The
       build gives them emitter entries (in the light tree with the static emitters); the live view's in-place edits
       add, replace and remove them: a placed mesh's entries go to area 0 (their weight; the tree keeps its shape and
       is refit), new ones are appended to the CDF's power-share part (path code 0, as the tree's moving emitters).
     - A material that starts emitting gets emitter entries in place from its flattened static triangles (`prepare`
       keeps each material's runs of them); one that also covers pool (instanced) or dynamic triangles still rebuilds.
     - The emitter buffer grows (and the scene's bind group is made again) only when entries are added past it: a
       scene that adds none keeps its layout. Intensity and power edits were in place already.
     - No kernel change: renderer/shaders as on main.
     - Bistro with a scene-code sign switched on (`tools/bench-emission.ts`, 2 rounds): its frame 18.6–19.1 ms, CPU
       2.9–3.3 ms, in place, against 615–618 ms when it rebuilt (`THREERT_EDIT_REBUILD=1`: 601 ms rebuild).
     - Layer A: `geometry/emitter-added-live`, `material/emitter-started-live` (0.49 against the analytic 0.5, in place
       and rebuilt alike), `geometry/emitter-removed-live`.
4. **[engine] Lighting on voxels.** Voxel surfaces through the live pipeline, emissive voxels as emitters.
   - Acceptance: ΔE ≤ 3 and SSIM ≥ 0.9 against our offline render on the level, overall and by region (the level's
     targets, step 2c-4), plus a layer B entry and ghosting within the bench-ghosting limits.
   - When to give emitters added in place a light-tree subtree of their own (the supervisor's rule, 2026-11-10; the crowd's
     moving emitters, sampled by power share alone, looked clearly worse): the same lamp measured in the static tree
     and after an edit moved it to the CDF's power-share part, at the same spp, against the offline reference over
     the region it lights. If after the edit ΔE is higher by more than 0.5, or the per-pixel variance by more than
     50 %, step 4's PR adds the dynamic subtree (rebuilt with each edit, as the moving one is refit).
   - Status (branch `voxel-lighting`, 2026-11-10): the lighting is in; two of the acceptance's items are open, the
     quality by region and ghosting's trail length (below; docs/VOXEL.md, "Measured: lighting on voxels";
     `tools/bench-voxel-light.ts`).
     - **Open, the user's decision (2026-11-10): the voxel line goes on; these two are left to later work on the shared
       live pipeline** (its temporal and denoising passes are § 0's, not changed here). Not met: the worst 3 × 3 region
       ΔE 2.87–5.0 and SSIM 0.79–0.89 (target ≤ 3 / ≥ 0.9; the floor and dynamic resolution, three shots and the
       exploded world), and ghosting's trail 51 / 34, 94 / 53 and 80 / 26 px (turn-mid, strafe-mid, strafe-return
       +0.5 s against the bench's limits). Why: the live pipeline smooths away voxel-scale detail, through its
       half-resolution floor, its temporal and denoising passes, and radiance-cache voxels coarser than the level's
       10 cm ones.
     - `threert-render render` (and `serve`) load the native voxel level the scene asks for (`voxelLevel`): the
       offline reference. The palette takes emission (`emissive`, nits): the lamps' voxels, as own-BLAS meshes, are
       emitter meshes (step 3), 2304 triangles of 192 lamps in the light tree. `games/voxel-level/level-night.game.ts`:
       the level at night, lit by them (and a faint sky glow).
     - Explosions in setup are queued until the level loads and done then, with their pieces' flight (`voxelFrame` in
       setup): the live view and the offline render start from the same exploded world. Batched pieces fly
       (`flight`: thrown out, gravity, a damped bounce on the terrain below them), so glowing ones light what they
       pass; an emitter mesh rewritten in place takes back its entries (the CDF does not grow frame by frame).
     - The rule above, measured (4 live shots of 64 frames each, over the ground and the first 3 m around the lamp's
       foot): the lamp moved to the power-share part by an edit read ΔE +0.92 and variance × 1.46 of the static
       tree's, over the line. So the dynamic subtree is in: in scenes the live view can edit (`EditRoom::editable`),
       the light tree's root holds the static subtree and one for emitters added in place (`ltree::build_slotted`),
       grown again at every such edit (`set_dynamic`; their entries right after the static ones in the CDF, their codes
       in their triangles' attrs); no kernel change. With it the edited lamp reads ΔE −0.08 and variance × 0.93.
       Offline renders, and live scenes that cannot edit geometry, build the tree as before (THREERT_LT_DYNAMIC=0: by
       power share, the A/B). Layer A `geometry/emitter-added-tree-live` (320 static emitter triangles, a panel added
       in place: 0.492 against 0.5, 0.4914 rebuilt).
     - Lamps: the ground around the lamp ahead of the camera reads 12.45 nits lit, 0.006 with the lamps off; blown up,
       its pieces 0.3 s into their flight light it at 12.81.
     - Destruction (the lamp blown up live, 4 runs): the explosion's frame 20.3–25.0 ms, frame start p95 16.67 ms in
       every run, no frame over 50 ms, no fallback or rebuild; its pieces' flight 2.7–3.0 ms of in-place edit CPU a
       frame.
     - Quality, live 64 frames after a reset against our offline render at 16384 spp (4096 still differs from 16384
       by ΔE 0.83 / SSIM 0.966), three shots and the exploded world: overall ΔE 2.34–2.68 / SSIM 0.887–0.907 at the
       0.5 floor, 1.92–2.26 / 0.906–0.928 at the dynamic resolution the game runs at; the worst region ΔE 3.8–5.0 /
       SSIM 0.79–0.84 at the floor, 2.9–4.4 / 0.81–0.89 at dynamic resolution. Overall met at dynamic resolution;
       by region not. Where the error is: the trees' canopies (voxels with holes, per-voxel shading) and the edges of
       voxel steps, which the live view blurs (half resolution at the floor, the temporal and denoising passes, the
       radiance cache's voxels coarser than 10 cm ones), and live reads 4–5 % darker than offline (the firefly clamp
       is not it: without it 1 % brighter and noisier, ΔE 2.10 -> 2.63). Open (above).
     - Ghosting (`bench-ghosting --scene voxel-night --quick`, against the bench's limits): the excess over the
       settled frame is within them everywhere (turn 0.74 / 4.25, strafe 2.24 / 6.0), but the trail length is over at
       three shots (turn-mid 51 / 34 px, strafe-mid 94 / 53, strafe-return +0.5 s 80 / 26): the wall beside a lamp
       and the canopies are still off by ΔE > 10 half a second after the camera stops. Open (above).
     - Layer B: `voxel-night` (shot 0, its limits from this branch: 2.52 / 0.893 -> ≤ 2.62 / ≥ 0.888), a regression
       guard; the level's target stays the bench's.
5. **[ThreeRT] Incremental structure.** Instancing of any object with a TLAS that grows; adding, removing or replacing
   an object's geometry without the full rebuild (today ~0.7 s on Kabukichō).
   - Acceptance: bench-hitches with objects spawned and removed every frame has no frame over 50 ms; Bistro unchanged.
   - Brought forward in 2c-4's PR A: objects with their own BLAS (procedural meshes, `dynamicGeometry` meshes) are
     replaced, added and removed in place, within room reserved at upload.
   - Status (branch `incremental-structure`, 2026-11-10): the rest, but flattened meshes, new materials and textures,
     and moving objects added in place.
     - Objects added in place are drawn: their new BLASes and the TLAS in one build (it was missing in most runs;
       Known issues). Clicks pick what is drawn: an object removed or hidden in place no longer, one added in place
       too.
     - Past the room: when no hole fits, the room's buffers (triangles' vertices, attributes and BLAS vertices, vertex
       colours; kernel records with the TLAS; object entries; procedural words) are reallocated at twice the room
       (the scene's own data before it as it is), copied on the GPU and submitted at once, the scene bind group made
       again; not with particles (their pass binds those buffers: a rebuild).
     - What edits leave behind: reused at once, not moved. Triangle ranges, records (a record's TLAS instance is
       `inst_base` + it) and object entries are taken from holes first (first fit, neighbours merged), the room's end
       after; a mesh that no object draws any more is freed (its triangles a hole, its slot and BLAS reused by the next
       new mesh), a removed object's record and entry go back. No pause: each edit pays for its own allocations.
     - New at run time, in place: an `InstancedMesh` (its mesh, records, TLAS instances and object entries from the
       room, contiguous; its instance updates as the build's), a copy of a static glTF model already in the scene
       (another instance of its mesh). A model new to the scene (materials, textures), animated or movable, or with
       glass / emissive parts, still rebuilds. Instanced meshes and static models are removed in place too (a model the
       build placed: a rebuild).
     - Layer A, each byte-identical in place, with every edit rebuilt and with no room (the room grown):
       `geometry/added-live` 0.4992, `procedural/added-live` 0.4992, `geometry/grow-live` (1,100 objects and 72,200
       triangles past the room) 0.4991, `geometry/instanced-added-live` 0.4989, `geometry/instanced-removed-live`
       0.4991, `geometry/model-added-live` (ratio to the build's copy) 0.9984; unit test `picks_follow_in_place_edits`.
     - Endurance (`bench-destruction --endurance`: the level, batched pieces flying, one 1 m explosion every frame for
       60 s of game time, the default room): 3,599 explosions, no fallback (before: from the 151st–176th). The room
       grew once (at 10 s, triangles 2.36 M -> 2.84 M held), then held: 2.16–2.25 M triangles in use, 159–214 holes,
       buffers 491.6 MB and live BLASes ~230 MB (at most 722 MB with the rest of the room's buffers; the level's line
       is 2 GB). Frame start p95 36.0 ms, the worst explosion frame 87.3 ms, 14 frames over 50 ms: the stress is CPU
       bound, 31 ms a frame (rebuilding every live batch natively 11.7 ms for up to 68 batches, their in-place rewrite
       10.3 ms; the trace 1.9 ms), as one explosion a frame keeps ~90 batches flying.
     - bench-hitches with objects spawned and removed every frame (`tools/perf/voxel-level-spawn.game.ts`: three
       boxes, an InstancedMesh of 16 and a glTF copy a frame on the level, each removed a second later): 796 frames,
       interval p50 / p95 / p99 / max 16.4 / 17.9 / 19.0 / 20.3 ms, none over 50 ms, no rebuild.
5a. **[ThreeRT] Stateful GPU particles.** Particles emitted, updated and retired on the GPU, with collision, including
   against a voxel occupancy grid; the same compute in the browser's WebGPU. A prerequisite of steps 6 (debris, dust)
   and 10 (effects). It meets the rule: every kind of game has particles, and the contract (emit, update, collide,
   retire, with a collision source a game supplies) is not voxel-specific.
   - Status (branch `gpu-particles`, 2026-11-10): done; collision with triangle scenes is later.
     - `GpuParticles` and `ParticleProgram` (docs/MATERIAL_FUNCTIONS.md, "Stateful particles"): the state (position
       and age, velocity and life, colour, size) in a GPU buffer, a ring of `capacity` slots; one compute pass a frame
       (`renderer/shaders/gpu_particles.wgsl`) emits, updates, collides, retires and places them as camera-facing quads
       of dynamic triangles, the dynamic BLAS rebuilt after it. The CPU keeps no particle: it sends each frame's birth
       counts and the game's spawns (`node.spawn`, 8 numbers a particle).
     - The contract is generic: an emit and an update function as WGSL bodies with `$Name` parameters, as material
       functions are; the engine integrates, collides and ages. WGSL compute only, so WebGPU can run it as it is.
     - Collision: a ground plane and the voxel level's occupancy grid (`voxelLevel({ occupancy: 0.2 })`: one bit a
       0.2 m cell, 32.9 MB on the level, built in 0.87 s at load; the cells in an explosion's box and a settle's box
       set again and their rows uploaded). Restitution along the normal, friction across, the particle put against the
       surface at its half size.
     - Lit like any surface; emissive materials glow (`emitterArea` as for sparks); the particle's colour is the
       material function's `i.custom`. Opaque or alpha-tested materials only. `Particles`, `SparkBurst` and the other
       stateless effects are unchanged.
     - Demo: `games/voxel-level/level-destruction-dust.game.ts`, the voxels too few to make a piece turned to dust (4
       particles each, their voxel's colour), thrown out from the explosion, landing on the road and the voxels.
     - Layer A: `gpu-particles/plane` (a particle dropped 2 m bounces and rests on the plane, its quad over it: 5.000
       nits against 5; fallen through or resting at its centre it reads 0 or 2.5), `gpu-particles/voxels` and its live
       twin `gpu-particles/voxels-live` (the same on a voxel slab), `gpu-particles/voxels-side` (thrown into the slab's
       side: stops against it). All four read 5; with collision off, or before it lands, they read 0.
     - GPU time on the level (`tools/bench-gpu-particles.ts`, a fountain on the voxels, 2 rounds, ms a frame):
       10,000 particles 0.14 (the pass) + 1.15–1.19 (BLAS); 100,000: 0.57–0.60 + 2.70–3.12. Frame start p95 16.67
       ms in every configuration, none over 50 ms.
     - Explosions with dust (`bench-destruction --routes batch,dust`, each with an A/A): singles, sustained (70
       explosions, 14,024 dust particles) and peak, frame start p95 16.67 ms and no frame over 50 ms in every run, the
       explosion frame's p50 within 0.8 ms of the batch route's, the worst settle 25.9 ms (24.7 without dust). One
       refresh of the occupancy grid per settle (one per piece first gave a 4 m explosion's settle 58 ms).
     - Layer A also: `gpu-particles/beside-particles` (with a stateless `Particles` node in the scene).
5b. **[ThreeRT] Density-grid volumes.** Besides procedural volume functions, a volume that stores a 3D density grid
   (simulated smoke and fire). A prerequisite of steps 6 and 10. It meets the rule: smoke and fire are in every kind of
   game, and a stored grid is a generic medium.
   - Status (branch `density-volumes`, 2026-11-10): done; the smoke does not collide with geometry.
     - `DensityGrid` (docs/MATERIAL_FUNCTIONS.md, "Density grids"): a grid of four numbers a cell across a box
       `VolumeMesh`, from the game (`data`, changed in place with `needsUpdate`) or simulated. The volume function reads
       it with `vgrid(i, lp)` and makes extinction, albedo and emission of it, so what the numbers mean is the game's.
     - On the GPU: every grid's field in one 3D texture of half floats (stacked along z), the kernel's binding 19 (a
       texture: the buffers are at Metal's limit). Kernel switch F_GRID (`kernel_source`: the binding, `vgrid`, 64 live
       steps through a medium with a grid instead of 8): scenes without grids keep main's kernel.
     - `GridProgram`: a WGSL body run per cell, two half steps a frame (`renderer/shaders/grid_sim.wgsl`, WGSL compute
       only). It reads both fields as they were anywhere (`ga`, `gb`), so advection, diffusion, forces and the game's
       sources (`grid.emit`) are the program's code: the contract is a cellular update, with no effect built in.
     - `SimulatedSmoke` (standard library): advection, a pressure relaxation (one Jacobi step a half step, kept),
       diffusion, buoyancy, turbulence, wind, cooling. Without the pressure, a puff in still air did not rise: it
       thinned where it was (the first version).
     - Lighting as every medium's: delta and ratio tracking offline; live, the camera rays' march and the volume
       voxels' in-scattering.
     - Demo: `games/voxel-level/level-destruction-smoke.game.ts` (bench-destruction route `smoke`): every explosion
       leaves a puff in a grid of 88 x 56 x 144 cells (0.25 m) over the streets, which rises and drifts with the wind.
     - Layer A: `volume/grid-uniform` (against `HomogeneousVolume`: ratio 1.002 offline, 1.011 live),
       `volume/grid-transmittance` 1.212 of 1.2131 (live 1.211), `-dense` 0.4457 of 0.4463, `volume/grid-data-live`
       0.7355 of 0.7358, `volume/grid-program` 0.6075 of 0.6024, `volume/grid-advect` 2.726 of 2.718,
       `volume/grid-source` 0.602 of 0.6024 (a source reaches one half step: added once), `volume/grid-smoke` (the puff
       rises: a regression value).
     - GPU time on the level (`tools/bench-density-grid.ts`: a 12 m box of smoke fed every frame, 2 rounds, ms a frame;
       `THREERT_PROFILE` now times the grid pass, `gridsim`, and the media passes, `media`, apart from the trace): 64³
       the simulation 0.13–0.21 and the rendering 0.79–0.91; 128³ 0.89–0.99 and 0.61–0.87. Frame start p95 16.67 ms in
       every configuration, none over 50 ms.
     - Explosions with smoke (`bench-destruction --routes batch,smoke`, each with an A/A): singles, sustained and peak,
       frame start p95 16.67 ms and no frame over 50 ms in every run; the trace pass (which holds the media passes)
       2.6–2.7 ms against 1.8 without smoke, the grid 709,632 cells.
6. **[engine] Edits and destruction.** (Rewritten 2026-11-10 for the triangle route chosen in step 2c-4.)
   - **Connectivity decides what falls.** After an edit, what no chain of voxels sharing a face (six-connected; one
     voxel is enough; no structural stress) joins to an anchor is cut loose: a new object that falls. Anchors are the
     terrain and objects marked so. Connections cross objects (a building's parts, a building and the ground). The
     removed voxels themselves are debris (instances, dust particles), no longer pieces by 4³ cell.
   - **Deterministic.** Edits, the split and the world-to-grid transforms are integers or fixed point: the same
     commands give bit-identical voxels on two runs and natively against wasm32, checked in CI by a hash.
   - **Edits:** explosions by material hardness (soft, medium, hard, indestructible), cuts (box, line), paint (soot)
     without a change of shape, added voxels.
   - Loose objects fall ballistically and rest where they land, static again; rigid bodies are steps 7 and 8.
   - Remeshing and re-uploading only the changed chunks of a large object: split off (below).
   - Acceptance:
     - What loses its support falls: unit tests, a level test, connections across objects included.
     - The same commands give bit-identical voxels on two runs, and natively against wasm32.
     - On the level, single explosions of 0.5–4 m, the sustained and the peak tier: the explosion's frame (the split
       included) ≤ 50 ms, frame start p95 ≤ 16.7 ms, no fallback; with an A/A.
     - Within the level's 2 GB; catalog byte-identical; scenes not involved keep main's kernel.
   - Status (branch `voxel-connectivity`, 2026-11-10): done, but the chunked remesh.
     - **voxel-core's `Level`** (`renderer/voxel-core/src/level.rs`): every object stands in one world lattice of the
       voxel size (`Placement`: quarter turns, whole voxels). An edit is a shape in world fixed point (1 / 256 voxel;
       `Shape`: sphere, box, capsule), a voxel inside when its centre is.
     - **Connectivity** (`lattice.rs`): the destructible static voxels' occupancy in world bricks of 8³ cells, whatever
       object a cell belongs to. Each brick keeps its components (bit operations on 8 words) with their cells on the
       six faces and whether they touch an anchor. An edit recomputes the bricks it changed and searches the
       components from them: a group that reaches no anchored component is loose. On the level: 277,681 bricks, built
       in 0.62–0.68 s at load, 98 MB of CPU memory (the GPU's is unchanged: 259 MB of triangles, 323 MB of BLASes).
     - **Loose parts** take their voxels out of every object they span and become one new object, falling under
       gravity in fixed point, a tick 1 / 60 s (`voxelFrame`); it rests when a cell under it is static, and is static
       there. Fewer than 8 voxels: debris. Drawn by its own mesh, written again in place each tick it moves.
     - **Edits** (`packages/voxel`): `voxelExplode` (each material within its hardness's share of the radius: 1, 0.7,
       0.4, 0), `voxelCarve` (box, capsule, sphere), `voxelPaint`, `voxelAdd`, `voxelHash`.
     - **Determinism:** `tools/voxel-determinism.ts` (CI): voxel-core's script of edits natively and as a wasm32
       module, hash 733db9ab69f495e9 both. `tools/voxel-determinism-level.ts`: the whole level twice in lockstep
       (explosions, a saw cut that drops buildings, a 4 m crater): the same three hashes.
     - **The level changed for it (version 2, its own asset directory):** by six-connectivity the generator's pitched
       roofs were 59–83 strips each, touching by edges only, and each tree crown held loose single voxels (4,552
       floating groups, 247,430 voxels). Roof steps are three voxels deep now and props are one connected piece: no
       floating group. 515.3 M voxels (514.8 M), 13,231,484 triangles (13,251,212).
     - Layer A: `voxel/cut-falls` (a cut through the lower of two stacked posts: its upper part and the post on it,
       two objects, fall as one and close the cut: 5.002 of 5), `-top` (the top they left: 0), `-live` (the same live,
       in place: 5.002), `voxel/cut-holds` (half the section cut: nothing falls, ratio 1).
     - **Frames** (`bench-destruction --routes batch`, with an A/A; others at 12–17 % of the GPU; main's in brackets):

       | scenario | explosion frame p50 / max, ms | frame start p95 | over 50 ms | fallbacks |
       | --- | --- | --- | --- | --- |
       | singles 0.5–4 m | 22.6 / 22.8, 22.5 / 28.3 (27.0 / 34.5) | 16.67 | 0 | 0 |
       | sustained (70) | 20.7 / 28.9, 20.6 / 31.0 (19.1 / 26.3) | 16.67 | 0 | 0 |
       | peak (20 in 1 s) | 19.1 / 23.0, 20.1 / 22.9 (19.9 / 26.2) | 16.67 | 0 | 0 |
       | topple: 8 trees' trunks, each crown cut loose | 20.9 / 25.9, 19.6 / 25.0 | 16.67 | 0 | 0 |
       | crater: 4 m on open ground, 140,785 voxels | 29.5, 31.9 (48.3–53.4) | – | 0 | 0 |

       The crater on main cut its voxels into 2,695 pieces; now they are debris, and nothing there is loose.
     - **The limit** (`fell`: a saw cut of 30 x 30 m that frees 20 parts of 199,426 voxels, buildings among them):
       its frame 88 ms (the cut and split 23 ms, the parts' meshes 9 ms, uploading 397,120 triangles 35 ms), then
       26–28 ms a frame while 280,572 triangles fall (written again every tick). No fallback. A falling object should
       move as an instance, not be rewritten: step 8's bodies.
     - **Chunked remesh: split off, not done.** Measured first, as asked: for the 4 m crater's 8 terrain tiles,
       meshing them again 7.4–9.3 ms, the upload 1.0–1.2 ms, their BLASes 4.0–4.9 ms on the GPU; the frame is 29–32
       ms. (The "10 ms of re-uploading whole tiles" I reported before was the upload of main's 2,695 pieces.) To cut
       it there are two ways with different risks: a per-object cache of chunk meshes (helps only an object's second
       edit; quads no longer merge across chunks) or a BLAS per chunk (changes the trace cost of the whole level). It
       needs its own measurements and gate.
7. **[ThreeRT] Physics and scene queries.** Rigid bodies, contacts, joints; ray, overlap and sweep queries for game
   code. Native, a shape interface the engine can extend. On Rapier.
   - Acceptance: stacking and joint tests against expected results; zero cost to scenes without bodies; the CPU split
     in the frame log.
   - **Done** (branch `physics`). A contract of ThreeRT for every kind of game; the voxel shape and debris bodies are
     step 8.
     - **The world is the engine's.** Rapier 0.36 behind `renderer/physics-core` (its own crate: no GPU, file system,
       clock or thread; CI builds it for `wasm32-unknown-unknown` and runs its 34 tests): dynamic, fixed and kinematic
       bodies; box, sphere, capsule, convex hull and static triangle-mesh colliders; fixed, hinge, slider, ball and
       rope joints with limits and motors; contact events; ray (first hit or all), overlap and sweep queries; a shape
       trait for the engine's own colliders (`CustomShape`: step 8's voxel shape). It is stepped at a fixed 1 / 60 s,
       at most 5 ticks a frame, after the frame script; scripts cannot step it and never see Rapier.
     - **The bridge** (`physics-core/src/bridge.rs`, also wasm32): scene code's calls (JSON; ids chosen by the
       caller) and the scene objects that bodies drive, against a `Scene` trait the renderer's `World` implements. A
       body's move goes from the world to the object's transform to its TLAS instance without passing through the
       script: a mesh a body drives is an instance of its own BLAS (`move_instances`), one added while the game runs
       is placed in place and moves the same way (no rebuild), and so does a new material without textures.
     - **The script's surface is three.js's** (`packages/scene`). `mesh.physics = "dynamic"` (or `{ type, shape,
       density | mass, friction, restitution, damping, ccd, sensor, contacts }`); in the scene the object has its body,
       out of it none, whether the scene began with it or a script adds it. The collider is the geometry's: a box, a
       sphere, a capsule, else the convex hull, or the triangles for a fixed body. `position` / `quaternion` /
       `rotation` read the body's pose when read (one typed array for every body a frame, then each object its own
       seven numbers, on first access); writing them moves the body (a kinematic one is driven there). `applyImpulse`,
       `applyForce`, `setLinearVelocity`, ... are methods of the object. `HingeJoint`, `SliderJoint`, `BallJoint`,
       `RopeJoint`, `FixedJoint` are objects that hold while they and their two objects are in the scene.
       `Raycaster.intersectPhysics()` and `Shapecaster` (`overlap`, `sweep`) ask the world. Contacts come once a frame
       as a batch: `contactstart` / `contactend` on the objects, `contacts` on the scene.
     - **Expected numbers** (physics-core's tests): free fall by Rapier's own law (a tick in 4 substeps: g dt² (n² / 2
       + n / 8) after n ticks, within 1 mm); five 1 m boxes rest as a column within 2 mm a contact; a box on a 20°
       slope holds at friction 0.6 and slides at g (sin - mu cos) at 0.1; a weight on a hinge passes the bottom at
       sqrt(2 g L) within 2 % and stays 1 m from it within 5 mm; a hinge stops at its limit (0.5 rad within 0.03); a
       slider keeps its axis, reaches its motor's 1.5 m/s and stops at its limit; a rope holds at its length (2 m
       within 2 cm), ball and fixed joints keep their anchors; joints join bodies as they are turned (a bar turned a
       quarter is not wrenched into line); contact events; queries; kinematic bodies push. The
       bridge's: a body drives its object (under a parent too), poses by slot, a force lasts its frame's ticks,
       impulses at a point, what a call refuses, limits. Layer A: `physics/box-rest` (5 nits where the box rests; also
       live), `physics/stack` (the strip above a three-box column: 0), `physics/hinge` (5 nits at the limit),
       `physics/instance-live` (a body's box against a still one: 1.005).
     - **What a script can do to it** (2026-11-10, after a review of physics-core: game scripts are untrusted and call
       `__engine.physics` directly, so the checks are in the world, not the binding; a panic in a tick is a crashed
       renderer natively and a trap in wasm32).
       - Every number of a body, collider, joint, call and query has a bound (`physics_core::limits`; the table is in
         RUNTIME.md), and what is outside is refused with its reason. Rotations and axes of any finite size are
         directions (1e20 squared overflowed into a zero quaternion).
       - A trimesh is for fixed bodies and static colliders (an open one gave a dynamic body 60 mg with its centre of
         mass far away); a joint joins two different bodies; a capsule under 1 mm long is its sphere; a hull with no
         volume is refused.
       - After a tick, a body whose pose or velocity went non-finite (Rapier disables it, and a joint on it would
         panic a later tick), one whose collider did, and one past 1,000 km are taken out of the world with their
         colliders and joints. The call `faults` reports each once with its kind, `state` answers `{ removed: why }`,
         and the binding leaves the object in the scene without a body. A body that left the world is an ordinary
         thing in a game (it fell off the level: 42 minutes of free fall, Rapier capping a body at 400 m/s): its
         object gets an `outofworld` event and nothing is thrown. One
         that went non-finite is an error: the next sync throws once, naming it.
       - A frame script or sync that throws no longer costs the world a frame: `GameSession::frame` makes the
         frame's ticks and commit all the same and returns the error after (before, both were skipped and the
         frame's time was lost), and the node host syncs the scene as the script left it, so the binding knows the
         world stepped (the next frame reads fresh poses). A `serve` job with frames still fails with the error, as
         with any script error.
       - A call that is refused changes nothing: everything is checked before any of it acts.
       - Memory: contacts nobody takes are held to 8,192 (the oldest go, counted); a trimesh to 1 M triangles, the
         world's shapes to 4 M points and triangles; queries take no trimesh.
       - Also: objects are placed at the body's own pose (not the call's numbers); a frame owing exactly 5 ticks keeps
         its remainder (12.5 fps made 562 ticks in 10 s); a body asleep when its object is removed goes with it.
       - The review's fuzz (random calls, ordinary and absurd numbers; a small version is in the crate's tests):
         611 panics in 120,000 runs at its two mildest levels before, none in 210,000 runs at all five after.
     - **Determinism:** the same bits natively and in wasm32, in physics-core's own build and in the renderer's, on
       what is checked: the script (`113d50eb06a47636`), 1,200 random worlds (tumbling convex hulls, bodies of
       several colliders, hinges and sliders with limits and motors, a rope; and, added with the world handlers,
       continuous collision detection on a triangle mesh, a kinematic body, fixed and ball joints, with the answers
       of rays, sweeps and overlaps folded in; `check::case`, folded hash `6fd1eba4551b8d39`, `9e16a6771fa8f317`
       for the first 1,000), each compared (`tools/physics-determinism.ts`, CI), and 32,000 runs of the review's
       random calls. It takes three things:
       - Rapier's `enhanced-determinism` (on in physics-core): without it none of the 1,000 worlds agree (the
         script: `dfdc13fb0be5cd0b` natively). It costs 1–3 % on the script (0.029 ms a tick) and 3–5 % on a pile of
         1,000 boxes (0.35 → 0.36 ms a tick), natively.
       - **A patched glamx** (`renderer/third_party/glamx`, PATCH.md). Rapier 0.36, parry 0.31 and glamx 0.3 call
         sin, cos and acos with method syntax, which on an f32 is the type's own method, the platform's libm, in
         every crate graph with `std` in it (any dependency that turns on `num-traits/std`: the renderer has several),
         whatever `enhanced-determinism` or their own `std` feature say. The eigen decomposition behind the mass
         properties of convex hulls and of bodies of several colliders is the one that showed: 48 of the 1,000
         worlds ended in other bits natively (macOS arm64, whose acos differs from libm's on 8.8 % of inputs, its sin
         and cos on 0.012 %) than in wasm32. The first version of the check passed because its one hull happens to
         agree. Rapier is also built without its `std` (it needs `alloc` only), which costs nothing measurable natively (ms
         a tick, best / median of 30 runs, two rounds alternated: the script 0.0285–0.0287 / 0.0290–0.0292 without
         `std` against 0.0287–0.0288 / 0.0290–0.0294 with; a pile of 1,000 boxes 0.349–0.355 / 0.353–0.357 against
         0.351–0.356 / 0.354–0.358). It is enough for physics-core's own build and not for the renderer's: the check now runs
         both ways (`std-in-graph`), and the renderer has a test of its own. `[patch.crates-io]` is read from the
         manifest of the crate being built only, so every crate that depends on physics-core repeats it and has that
         test (PATCH.md): the web build's wasm crate will be the next.
       - **Hinge limits centred.** Rapier takes the sine and cosine of a quarter of (min + max) the same way; the
         world hands it limits about 0, [-h, h] (of 0 they are exact), and turns the centre into the joint's frame
         with libm.
       - The other such calls on a 3D world's path are tested to change nothing on the platform the tests run on:
         the broad phase's BVH optimizer takes a log2 that depends on the collider count alone (every count checked),
         a convex hull's feature lookup the sine and cosine of one constant.
       - Platforms measured: macOS arm64 natively (where the hashes were recorded), Linux x86-64 natively (CI:
         `tools/physics-determinism.ts` there compares Linux's native build, wasm32 and the hashes recorded on macOS,
         all three equal), and wasm32 under Node's V8 on both. Not measured: Windows, Linux arm64, a browser's own
         wasm engine (the same wasm32 module; wasm's float arithmetic is specified, so no difference is expected),
         and iOS / Android natively.
       - What this does not give: a proof for platforms the tests have not run on, or for another version of Rapier,
         parry or glamx (the calls were found by reading their sources: an update repeats that, and the recorded
         hashes show a change). The complete fix is theirs: those calls made through simba. Step 9's replay rests on
         this for one build of the engine.
     - **Hello world** (`games/physics/hello.game.ts`, catalog `physics-hello`; `threert-render play` to click in a
       window, `tools/physics-hello.ts` for the same game with scripted clicks): a click's ray is cast into the world
       (`Raycaster.intersectPhysics`); on something fixed (a triangle-mesh ground and rock, a wall, a ramp) a box with
       `physics = "dynamic"` is added off the surface and falls onto what is there; on a box, the box is kicked. The
       scripted run adds 4 boxes (three end as a column at 0.250, 0.748 and 1.248 m), kicks one at 3 m/s (it slides
       0.913 m; v² / 2 mu g = 0.917) and ends with every box asleep. A click's chain: the pick 0.001–0.004 ms, the
       script (handler, frame script, binding) 0.30–0.54 ms of which the world's calls 0.007–0.053 ms, the commit
       0.39–0.57 ms in place, the next frame submitted 3.8–4.1 ms after the click. The first box of a scene without
       instances rebuilds it once (15–22 ms here: the kernel variant with instances).
     - **What the boundary costs a frame** (`tools/bench-physics.ts`; every body moving every tick, the most it can
       cost; M5 Pro, means of 120 frames, one run):

       | bodies | step (Rapier) | sync to objects | commit (records, TLAS) | script, reads none | script, reads all |
       | --- | --- | --- | --- | --- | --- |
       | 0 | 0.007 ms | 0.0001 | 0.001 | 0.085 | 0.097 |
       | 100 | 0.035 | 0.011 | 0.116 | 0.090 | 0.141 |
       | 1,000 | 0.258 | 0.173 | 0.780 | 0.166 | 0.741 |
       | 10,000 | 2.126 | 1.696 | 4.518 | 0.217 | 5.445 |

       A script that reads nothing pays nothing for the bodies (the same scene without bodies: 0.079 / 0.090 / 0.339 ms
       at 100 / 1,000 / 10,000 meshes). Reading a body's position and quaternion costs 0.5 µs, of which 0.28 µs is
       reading the two properties of any object in QuickJS. Bodies asleep cost nothing natively (a pile of 10,000 at
       rest: step 0.03 ms, sync 0.0003 ms, commit 0.003 ms).
     - The frame log has `physics: { ticks, step, sync, bodies, moved }` and `commit`.
     - **Open:** the first instance of a scene rebuilds it; a sleeping body does not follow a parent of its object that
       moves; reading every body's pose every frame at
       10,000 bodies is 5 ms of script (the typed array itself is not handed to scripts yet); a body made anew
       (`physics` set again, the object re-parented or rescaled) starts at rest; where physics is authoritative, the
       node or the room, is step 9's.
     - **Open, before game code from outside runs here:** a tick has no time bound. The limits are on counts (20,000
       bodies), not on what a tick costs: N one-metre dynamic boxes made at one point take 62 ms for their first tick
       at 250, 0.46 s at 500, 2.8 s at 1,000 and 45 s at 2,000 (M5 Pro, one run). It belongs to the security review
       before the cloud opens.
   - **World handlers** (2026-11-10, branch `world-handlers`; the user's decision of that day, with the runtime's
     shape: three.js-style scripts, Rust as the main runtime, four channels across the boundary, layout C on the web.
     docs/RUNTIME.md has the decisions, the layouts considered and the probe's measurements).
     - **`on[type](world, ev)`** (`packages/gameplay/src/world.ts`): an event's handler on the node that owns the
       scene and the physics world. It runs as the event arrives, before the next tick, and what it changes goes to
       the engine through the scene binding when it returns: no commit, no frame script run for it. Before, a handler
       could only write the JSON state (it may run in a room, where there is no scene), so a click that adds a body was
       recorded by a handler and acted on by the frame script.
     - With `handlers[type]` too: the world handler first, on the state before the event; then the transaction; then
       what the world handler emitted (`world.emit`, a state transaction). `keys` map to either; a game of world
       handlers needs no `handlers`. One that throws does not stop the world (its changes are synced, the error is in
       the event's log). In a room they do not run: an event with a world handler only is an error there.
     - **Only local authority.** Who owns the physics when a room is the authority is a product question, not
       decided. What a world handler adds is not in the game state, so a restored state does not have it (step 9).
     - **Queries see what the engine was told, with or without a tick since.** Rapier's broad phase is the last
       step's: a collider added since was not found, and a body moved by a call was found where it had been. Two
       events with no tick between them (a frame rate above the tick rate) are where an author meets it. Colliders
       added, or moved by a call, since the last step are now tested one by one beside the broad phase's answer
       (`Physics::fresh`; nothing of Rapier's state is touched; with none, a query is what it was). What the running
       script itself changed reaches the engine when it returns, so a handler's own query does not see what it just
       added: the one rule left, in RUNTIME.md.
     - **Events without a frame script.** The node host's `frame` now runs every frame for a game with a frame
       script, with world handlers, or whose scene has a physics world, so contacts and `outofworld` reach their
       listeners every frame (before: only when a handler committed). A scene with none of the three pays nothing.
     - **Hello world** is rewritten: `on.click` casts the ray and adds the box; no state, no handler, no frame
       script. `tools/physics-hello.ts` is unchanged and passes. A click's script time, two runs each on one build:
       0.33–0.62 ms before (0.30–0.54 ms in step 7's table), 0.16–0.37 ms now; the commit and the first frame after a
       click as before (0.29–0.53 ms in place, 3.9–4.0 ms). The frame's script time with no frame script is 0.083 ms
       mean (0.094 ms with the old frame script): the host's call, the events and an empty sync.
   - **The engine's library for wasm32** (2026-11-11, branch `wasm32-engine`; the first half of layout C, after a
     probe of what a browser can run).
     - The probe (measured on an M5 Pro, Chrome 157, the real GPU): the kernel itself, with software traversal and
       the native renderer's scene data, costs 0.98–1.08 × its native time in Chrome (hello 4.19 ms against 3.9 ms
       a full-resolution sample pass, Kabukichō 70.3 against 66.8–68.0), and draws the same image. WebGPU's limit
       on storage buffers a stage (8 by default, 10 in that Chrome) is under what the kernel (20) and the live
       `temporal` / `post` / `dtemporal` passes (19 / 13 / 11) bind: the web build gets a merged layout generated
       from the one source. What stands in the way is software traversal itself: one flat BVH, so every added or
       moving object rebuilds the scene (hello: 9 ms a frame while a body moves; the voxel level: 7 s an explosion),
       and it holds 2.1 GB of triangle data for the voxel level.
     - This step: the library builds for `wasm32-unknown-unknown` with the native shell left out, the GPU from
       futures, `web_time`, and file reads behind one module (docs/RUNTIME.md "The engine's library in a browser").
       Natively the renders are byte-identical.
   - **The engine in a browser** (2026-11-11, branch `web-hello`; the second half of layout C: the physics hello
     world runs in Chrome to the native run's numbers).
     - `renderer/web`: the wasm module (the engine API's 35 exports, one for one with the native `__engine`, on the
       same code, `renderer/src/api.rs`; the session's), the page (`__engine` over the exports, the node host called
       as `GameSession` calls it, the canvas' clicks) and `tools/build-web.ts`. The game bundle is the native one.
     - The merged binding layout (`renderer/src/merged.rs`): the browser's shaders written from the one source with 8
       storage buffers a stage (the kernel as written binds 20, the live passes 28), guarded by a test that reads
       them with naga; `THREERT_MERGED=1` runs it natively.
     - **A GPU job that did not end took the machine's display down** (2026-11-11 01:42, WindowServer's watchdog)
       while the atomic tests ran under software traversal: at `geometry/instanced-procedural` in the layout as
       written, then at `environment/background` in the merged one. Neither scene has a triangle. The likely cause,
       by reading: the tree over no triangle was one node with `count: 0`, an interior node to the kernel,
       whose children were itself and the bytes after its 32-byte buffer; zeros end the walk, an earlier scene's
       bytes need not. Fixed in `bvh.rs` (a leaf of the empty vertex buffer's zero triangle), with a test that walks
       the kernel's loop on the CPU. **Not confirmed as what hung**: the same sequence on a build without the fix,
       with the guard, passed (both scenes alone, with the fix and the guard, give their tests' answers). And a **loop guard** (`merged::guard`): every loop of the kernel bounded and
       what it stops counted and read back; always on in a browser, `THREERT_LOOP_GUARD` natively (since step 7b
       a native kernel with units has it with nothing set).
     - `tools/web-engine.ts` (CI): the module in Node with the page's `__engine` and the native bundle runs the
       physics hello world's script to the native test's numbers.
     - **In Chrome 157 on the machine's GPU** (`tools/web-hello.ts`, Playwright; docs/RUNTIME.md "Measured in a
       browser"): the scripted test passes at 1280 × 720 and its report is the native run's number for number. A
       still frame is 6.6 ms of GPU and 0.3 ms of the page (natively under software traversal 6.9–8.0 ms of GPU).
       **Every frame in which a body moves rebuilds the scene: 16.5 ms in the page** (9.0 natively), and a click that
       adds a box 17–24 ms: what hello world ships with until software traversal edits in place. The view opens in
       1.13 s, 1.08 s of it the sky's bake on one thread; a kernel the browser has not compiled takes 1.4 s before
       the first frame, 26 ms after that. Engine calls cost 40 ns (a scalar) to about a microsecond (a descriptor),
       10,000 poses 0.3 ms. The module is 7.3 MiB (1.5 MiB with brotli).
     - Open, first: **a frame in which a body moves costs the page 16.7 ms, 16.4–16.6 of it the rebuild** (9.0 natively
       under software traversal, nothing under hardware traversal): in-place edits under software traversal, the
       two-level tree, the next step (step 7b: 0.84 ms now, no rebuild). Then: the browser's image is not bit-identical
       to the native one (the guard changes the kernel's last bits; natively it is off and the catalog is byte-identical
       to main); 1.4 s for a kernel's first compile and 1.09 s of sky bake on the page's thread before the first frame;
       a scene larger than hello world in a browser, and the merged layout's cost on one natively; what hung on
       2026-11-11 is not confirmed; other browsers.
7b. **[ThreeRT] In-place edits under software traversal (a two-level tree).** Decided by the user on 2026-11-11, before
   step 8: a browser has no ray queries, and under software traversal every added object and every frame in which
   a body moves rebuilds the scene (hello world in Chrome: 16.5 ms of the page's 16.7 a frame; the voxel level: 7 s an
   explosion; its triangle data 2.1 GiB because instances are written out). Design: docs/RUNTIME.md "Software
   traversal with units".
   - The still scene stays one flat tree; what can move or be added (an instance of a mesh: a body's box, a voxel
     chunk) is a unit with a tree of its own in model space, found through a top tree walked after the still one. A
     scene without units gets the kernel it has today, byte for byte.
   - Acceptance: the two walks transcribed to the CPU and run against brute force over random scenes and random
     add / move / remove sequences (no read outside a range, always an end, the same closest hit), before any GPU
     run; scenes without units unchanged (the sources handed to wgpu, the catalog's bytes); hello world's two checks
     that fail under software traversal pass, and in Chrome a frame in which a body moves no longer rebuilds; the
     voxel level under software traversal: triangle data, load, an explosion, a still frame; overlap measured on a
     pile of debris.
   - Every GPU run of the new walk has the loop guard, natively too (what hung on 2026-11-11 is not confirmed, so
     another path that does not end may exist); whether software traversal has it by default natively is decided
     with its measured cost.
   - **Decided 2026-11-11, after the first GPU runs** (docs/RUNTIME.md "Software traversal with units"):
     - **Units are a live session's and a browser's. An offline render under software traversal flattens every
       instance by default**, its kernel and bytes main's; `THREERT_UNITS=1` gives it units, `THREERT_UNITS=0`
       flattens a session. Why: the offline render under software traversal is the reference render and what the
       catalog's byte gate compares, with no variable set; memory, load and edits in place are what a session and a
       browser need; no catalog scene with models but Kabukichō has been rendered with units.
     - **A native kernel with units has the loop guard with nothing set** (65,536; `THREERT_LOOP_GUARD=0` takes it
       out). A native kernel without units has none by default, as before: no existing byte changes. What a stop
       does is unchanged (an offline render fails, a session logs it, `test-render` fails the run). Whether all of
       native software traversal has it by default is not part of this step.
   - **Status: runs natively and in Chrome on the machine's GPU** (branch `sw-two-level`, 2026-11-11; every run with
     the loop guard, which stopped nothing). The figures and their tables: docs/RUNTIME.md "Software traversal with
     units", "Measured".
     - Done: `renderer/src/units.rs` (mesh trees, the top tree, where they go in the node buffer, the checks made
       where a tree is written), `shaders/trace_units.wgsl` (spliced into software traversal by a source feature,
       `F_UNITS`; `pathtrace.wgsl` not edited), `prepare_live` keeping a session's instances as units (`prepare`,
       an offline render's, flattens unless `THREERT_UNITS=1`), and the edits in place through `gpu::Top` (the TLAS
       or the units), so `move_instances`, `update_instanced` and `edit_geometry` are one path under both
       traversals. `THREERT_UNITS=0` flattens, as before. The guard's default for a kernel with units
       (`gpu::loop_guard_of`).
     - The acceptance:
       - The CPU tests pass (the walks transcribed from the shaders and compared with their lines; random scenes
         under random edits against brute force; a prepared scene with units against the same scene flattened;
         broken and stale trees refused), before the first GPU run.
       - Scenes without units: the sources handed to wgpu are main's (`the_default_sources_are_untouched`, hardware
         traversal's among them); **offline with nothing said about units the catalog's 23 bundles are
         byte-identical to main under software traversal** (both builds with the guard at 65,536, as every GPU run
         here); under hardware traversal 18 are, and the other five (Bistro's three, Zero Day's two) differ from
         main as two runs of main differ from each other ("Known issues": hardware traversal's ties).
       - Hello world: `tools/physics-hello.ts` passes under software traversal with nothing else set (units and
         the guard by default; it stopped nothing), the two checks that failed among them. In Chrome a frame in which a body moves costs the page **0.84 ms (was 16.7)** and none rebuilds the
         scene (295 did); a click that adds a box commits in 0.6–0.8 ms (18–24) after the first.
       - The voxel level under software traversal: triangle data **310.9 MiB (2,098)**, first frame after 5.2 s
         (11.6), an explosion **2.5–4.5 ms in place (6.9–7.9 s)**; a still frame's trace **7.85 ms (4.58): 71 %
         longer**.
       - Kabukichō (352 units): triangle data 55 MiB (634), first frame 1.55 s (3.3); the trace 13.9 ms (12.0):
         **16 % longer**, and with it the view misses 60 Hz on this machine (17.2 ms a frame).
       - The guard on Kabukichō, main's kernel: +1.23 ms of trace (11 %); the merged layout +0.13 ms.
       - The atomic tests under software traversal, as written and merged: 147 of 170 pass; the 23 that fail fail
         on main's build too, with the same values: 13 `particles/*`, the 5 `gpu-particles/*`, `translucent/depth`,
         `translucent/order`, `volume/lighting`, `volume/lighting-offline` (all particles, which software traversal
         does not draw) and `surface/windows` (not explained). The names: docs/RUNTIME.md; "Known issues".
     - Two faults the GPU runs found, fixed, each with a CPU test since: a scene of units alone drew a mesh's first
       triangle at the origin (the still tree's leaf named triangle 0); a mesh written again in place kept its
       units' old boxes (`voxel/cut-falls-live`: a falling part cut off).
     - **Not run**, one by one (none was among the GPU runs agreed for this step; each is asked for before it is
       made):
       - a kernel with units without the loop guard (every run of one had it);
       - a catalog scene with units, offline (`THREERT_UNITS=1`) or live, but Kabukichō;
       - explosions on the voxel level until the edit room is full (V4): how many fit before a rebuild;
       - a pile of debris (V5): overlap where it is worst;
       - anything but hello world in a browser.
     - Open:
       - **First, what the next item is for: a still frame costs more with units** (software traversal, live,
         1280 × 720 traced at half, the guard on in both columns):

         | scene | the trace, units / flat | frame GPU, units / flat | 60 Hz |
         | --- | --- | --- | --- |
         | hello world (natively) | 1.40–1.42 / 1.01–1.04 ms, **+37 %** | Chrome: 6.59 / 6.60 ms | held by both |
         | the voxel level | 7.85 / 4.58 ms, **+71 %** | 11.2 / 7.9 ms | held by both |
         | Kabukichō | 13.88 / 12.01 ms, **+16 %** | 17.18 / 15.35 ms | **missed with units** (a frame every 17.2–17.5 ms; flat 16.65) |

         Against main's session on Kabukichō under software traversal (flat, no guard: 14.11 ms of GPU a frame) the
         default now is 3.1 ms more, 1.2 of it the guard. Nothing was tuned: the top tree is built by median splits,
         every unit's ray is transformed, and every ray enters the top tree. An SAH top tree and a path for units
         whose transform is the identity are the first things to try; until then a session that only stands still
         is cheaper flattened (`THREERT_UNITS=0`), at its memory and load time.
       - The edit room and the tree ranges do not grow under software traversal: an edit that finds them full
         rebuilds the scene (seen: 1,100 instances added to a room of 1,024 records). How many explosions that is
         on the level is not measured.
       - Procedural edits, posed models and rigid groups' poses still rebuild under software traversal; a scene
         with more than 2^18 instances is flattened, as before.
       - Whether all of native software traversal has the loop guard by default (not this step's; it costs 11 % of
         Kabukichō's trace, and what hung on 2026-11-11 is still not confirmed).
       - The 23 atomic tests that fail under software traversal on main ("Known issues"): the other line's.
7c. **[ThreeRT] What a still frame costs with units.** Before step 8 (which adds units): step 7b's units made the
   trace 37 % / 71 % / 16 % longer on hello world, the voxel level and Kabukichō, and Kabukichō missed 60 Hz under
   software traversal, a browser's only path. Design and every figure: docs/RUNTIME.md "Software traversal with
   units" ("The stack", "How the top tree is made", "Measured: the still frame").
   - **Status: done on this machine** (branch `units-still-cost`, 2026-11-11; every GPU run with the loop guard, which
     stopped nothing). The trace, units as in 7b → now / flat, live, 1280 × 720 traced at half: hello world
     1.42 → **0.73** / 0.99 ms; the voxel level 7.83 → **4.17** / 4.54; Kabukichō 13.98 → **11.8** / 12.2, its frame
     15.2 ms of GPU: **60 Hz again**. No picture changed for it: the atomic tests with units pass, the catalog's
     models with units are their flattened pictures (ΔE 0.01–0.03 on the small ones), and the offline catalog with
     nothing set is main's bytes (24 of 24).
   - What it was, in the order of what it gave: arrays set to zero for every ray (a walk's own stack: now one for an
     invocation, the kernel module's); a unit's walk as a loop inside the top walk's, where a ray waited for its
     group (now one loop); a top tree by median splits (now by surface area, and kept: the settled units' tree
     with its boxes brought up to date, a small tree built for the units that come or travel).
   - Measured with it, as step 7b left open: the catalog's scenes with models live with units; explosions until the
     room is full (0.5–0.6 s for the rebuild, right after it); debris as units (8,192 in flight: +2–4 % of trace,
     2.3 ms of CPU a frame), and from it and a CPU count of piles: **step 8 has room for a few thousand debris
     bodies; the CPU's instance update bounds it before the trace does.**
   - Open:
     - **A flattened scene still sets its walk's 48 words to zero for every ray**: with units hello world's trace is
       0.73 ms and flattened 0.99, the voxel level 4.17 and 4.54. A kernel without units is `pathtrace.wgsl` as it
       is (its `trace` declares the stack; the file is shared and was not edited here). The same change there (the
       stack a private variable of the module; `trace_procs` has one too) should give every scene without
       units under software traversal what it gave these (0.36 / 1.1 / 1.5 ms on the three scenes), in a browser
       too. Not measured on a kernel without units; it changes that kernel's source, so the default sources'
       hashes and possibly the catalog's software-traversal bytes: its own step.
     - `zero-day` (+13 %) and `bistro` (+4 %) with units: one unit of millions of triangles whose transform is not a
       translation. Such a scene loses nothing flattened (it has one copy of the triangles either way); units for
       it only when it is edited.
     - The loop guard's 1.2 ms on Kabukichō (11 %): two other forms of the count gave nothing; not explained.
     - Bodies at rest on one another have no GPU number (no game makes a pile yet): step 8's first measurement.
     - Under software traversal a rigid group's pose still rebuilds the scene every frame (`zero-day-animated`:
       2.6 s a frame), as before step 7b.
8. **[engine] Voxel bodies.** The level collides, and what an edit leaves hanging on nothing is a rigid body (it
   replaces step 6's fall through the lattice). Native first: the acceptance is under hardware traversal; software
   traversal has to work (bodies are units, no rebuild) and gets one set of figures; a browser is not part of this
   step. Design: docs/VOXEL.md "Voxel bodies"; docs/RUNTIME.md "Physics" (the voxel shape, the engine's own bodies).
   - Scope:
     - **The collision shape**, on step 7's `Shape`: parry's `Voxels`, for the level's objects (fixed) and for the
       loose parts (moving), chosen against a compound of boxes, a triangle mesh and a convex hull by measurement.
     - **The level collides by default**: every object a fixed collider of its voxels, edited by the cells an edit
       cleared; rays and shape casts of the physics world meet it. `physics: false` on the level is step 6 as it
       was; `collide: false` on an object leaves it out. Ordinary meshes still declare `physics`.
     - **Loose parts are rigid bodies**: mass from their voxels and each palette entry's density; they tumble, land
       on the level and on one another, and sleep; they move as instances (no vertex is written again).
     - **Debris**: parts under 8 voxels stay the game's particles; at most 512 voxel bodies (`maxBodies`), past it
       the oldest sleeping one goes, its voxels debris.
     - **A body stays a body** when it has settled (asleep it costs no tick; turned, it does not fit the lattice,
       and writing it back would lose voxels). The cap bounds them.
     - **Edits reach bodies**: a carve cuts a body where it is, as it is turned; what is cut off is a body of its own.
     - **Determinism and robustness** as in step 7: wasm32 builds in CI; a class of voxel worlds in `check::cases`
       and the adapter's own script, native against wasm32; a fuzz of calls on voxel shapes.
   - Acceptance (hardware traversal): a 4 m explosion's frame ≤ 50 ms and 60 fps after it; a building cut loose
     falls, lands and sleeps within 16.7 ms a frame; about a hundred bodies with room to spare, and the number at
     which 60 fps is lost; the CPU split (physics step, placing, meshes, colliders) recorded; scenes without voxel
     bodies unchanged (the catalog's bytes, `renderer/shaders`, step 7's physics tests and hello world).
   - **Status: the CPU side is done (branch `voxel-bodies`, 2026-11-11); no GPU run of it has been made.**
     - Done: `Shape::Voxels`, `Physics::edit_voxels` and the world's own ray at voxel shapes (physics-core); the
       bridge's engine bodies and colliders (ids from 2^30 up: scene code names them and cannot make or remove
       them); `voxel_core::Edit::cleared` and `Level::take`; in renderer/src/voxel.rs the level's colliders, the
       bodies, their sync, the carve of bodies, the cap; `voxelBodies()` in `packages/voxel`.
     - Measured on the CPU (this Mac; docs/VOXEL.md has the tables): voxels against the other shapes; on the
       benchmark level, colliders of 61.6 M cells in 5.4 s and 303 MiB (objects of one file share a shape until one
       is edited); an explosion of 4 m 20 ms (of it the colliders 8.5); the 30 x 30 m cut 124 ms, and the building
       part it frees (95,694 voxels) then costs the physics under 0.5 ms a frame while it falls (step 6: 26–28 ms a
       frame of vertices written again) and nothing once asleep.
     - Three faults of Rapier 0.36 / parry 0.31 with voxel shapes, each with a test and a way round in physics-core
       (none patches the crates): the solver panics on a ball against voxels (parry's function for that pair keeps
       no workspace; the pair now goes through its general voxel function); parry's ray misses voxels along a
       lattice plane (the world walks the cells itself); a shape edited where it sits keeps stale contacts (the
       collider is taken out and put back).
     - Determinism: 300 voxel worlds and the adapter's script end in the same bits natively and in wasm32 (the
       physics job in CI); step 7's worlds end in the bits they did.
   - Open:
     - **Every GPU run**: the acceptance's frame times, the falling parts on the real level, the atomic tests (the
       three `voxel/cut-falls*` keep the fall through the lattice with `physics: false`; `voxel/cut-body*` are the
       same cut with the part as a body), software traversal with units.
     - **The level's colliders cost 5.4 s of load and 303 MiB** on the benchmark level: natively this is a fixed
       cost of every level of that size. They are a shell of the cells within 4 of the open (every solid cell was
       26 s and 1.3 GiB), and objects of one file share one shape until one is edited (before that: 6.9 s and
       455 MiB). What is left is the terrain's 1,600 tiles, a shape each. Decided against (2026-11-11): a shell of
       2 cells (a body would get through it at 12 m/s), and building them after the first frame (a level whose
       first seconds have no collision behaves by its load time).
     - The script surface is `voxelBodies()`, `voxelImpulse` and the ids physics hits report; the rest is step 10's
       (voxel bodies as objects with `.physics`, paint and added voxels on bodies, mass by cell).
     - A tick's time is still not bounded: the first tick after an edit that frees many parts steps them all (400
       jagged pieces in one heap: 7 ms a tick, at worst 19; 1,000: 23 ms, at worst 52).
     - A browser: a small voxel scene with bodies in Chrome is not run in this step.
     - The two faults above are to be reported upstream.
   - Then the decision: keep Rapier or write our own. So far Rapier does what the step needs on both ends (the
     same bits natively and in wasm32, voxel against voxel resting and sleeping), with the three faults above.
9. **[ThreeRT] World data and command replay.** Binary world blocks on the node outside the JSON game state, changed by
   deterministic commands the room logs and replays to late joiners.
   - Acceptance: a room replayed from its log gives bit-identical world data.
   - **The log carries world-hash checkpoints** (required by the supervisor of this line, 2026-11-10): at a fixed
     period the hash of the world data and of the physics world (`Physics::hash`) is logged, and a node replaying
     compares. The ends are not assumed to agree bit for bit: a difference is found at the first checkpoint after it
     and placed between two checkpoints.
   - **The voxel world now depends on the physics' determinism** (step 8): where a loose part comes to rest, and so
     which voxels a later edit carves from it, is the physics world's doing. A replay needs the physics' bits to
     agree wherever the voxels' must; the checkpoints above cover both hashes, and step 8's checks hold native
     against wasm32 for voxel shapes.
   - **World handlers' changes are commands.** What `on` handlers do to the scene (step 7) is not in the game state;
     it all leaves the script through the scene binding's sync, and that is where it is logged and replayed from. No
     other way out of a world handler may be added.
   - From step 7, for the physics' part of the log:
     - It rests on step 7's determinism, which holds for one build of the engine on the platforms checked; a new
       platform or a new Rapier runs `tools/physics-determinism.ts` first.
     - **Precondition: the platform math calls upstream knows of, against what our worlds reach** (dimforge/rapier
       issue #1054 and its reproductions, for Rapier 0.36.1 / parry 0.31.1 / glamx 0.3.1; read 2026-11-10). We build
       `rapier3d` with `dim3`, `f32`, `alloc` and `enhanced-determinism`: no `parallel`, no soft bodies, no multibody
       joints, and a game's calls make bodies, the five impulse joints (limits on hinges and sliders, velocity motors),
       box / sphere / capsule / hull / static trimesh colliders, CCD, contact events and ray / overlap / sweep queries.

       | upstream item | call | on our path | what holds it |
       | --- | --- | --- | --- |
       | inertia diagonalization ("suspicious", no repro upstream) | glamx `eigen3.rs` acos, cos | **yes**: every convex hull and body of several colliders | patched (third_party/glamx); 48 of 1,000 worlds differed before; cases 0 and 1 |
       | angular limits (2D repro) | `joint_constraint_helper.rs` sin_cos of the limits' centre, also the 3D line | **yes**: a hinge with limits | limits handed over centred, so the centre is 0 and exact (`add_joint`); case 2, `revolute_limits_off_centre` |
       | BVH optimizer ("suspicious") | parry `bvh_optimize.rs` log2 | **yes**: the broad phase, every world | tested: the same subtree count with the platform's log2 and libm's for every collider count up to 80,000, on each platform the tests run on |
       | arc geometry | parry `vector_ext.rs` `angle()` acos | no: its callers are a trimesh's pseudo normals (only with the `ORIENTED` / `FIX_INTERNAL_EDGES` flags) and arc generation | tested: a trimesh is built with no flags (`a_trimesh_has_no_pseudo_normals`); case 3 has a trimesh |
       | revolute angle | `RevoluteJoint::angle()` asin | no: neither Rapier's solver nor we call it | |
       | angular motor (2D repro) | `generic_joint_constraint_builder.rs` sin of a motor's target angle | no: the multibody path, and a position motor; ours are impulse joints with velocity motors (the impulse path's position motor uses simba's asin) | a position motor is not offered to games; adding one re-opens this |
       | contact coefficients | `CoefficientCombineRule::Max` on ±0 | no: we never set a combine rule (the default averages, which IEEE defines) | |
       | singular vectors | glamx `svd2.rs` | no: 2D | |
       | deformation damping, elasticity, cloth bending, tear smoothing, plasticity, elastic cap, disk / tube geometry, rest angle (9 repros) | soft bodies | no: no soft body is made | |
       | inverse kinematics | multibody | no | |
       | missed tear | `parallel` and soft bodies | no | |
       | Delaunay refinement, equal-key sorts ("suspicious") | soft bodies, spade | no (spade is not in our build) | |

       Also read in the sources and not in upstream's list: a convex hull's feature lookup takes the sine and cosine of
       one constant (tested equal to libm's); `Aabb::intersects_spiral` and the interval arithmetic under it have no
       caller in our build; the remaining method-syntax calls are in 2D or test code. Case 3 of
       `physics_core::check` (200 worlds: CCD on a trimesh, a kinematic body, fixed and ball joints, the answers of
       queries) is the evidence for the rest of the path: 200 of 200 equal natively and in wasm32, both ways. A new
       feature of the physics offered to games (soft bodies, position motors, multibody joints, another combine rule,
       trimesh flags) is checked against this table first.
     - The log needs the tick each call was made at, and the syncs: a frame makes 0 to 5 ticks from the wall clock's
       `dt`, and a force lasts "the ticks of its frame", so the calls alone do not replay.
     - A body that drives an object starts at `Scene::world_pose`, which is the renderer's own glam arithmetic (not
       under `enhanced-determinism`): the log should carry the pose the body was made at, as it should carry a voxel
       command's fixed-point shape (step 6).
10. **[engine] The game API and effects.**
    - `packages/voxel`: voxel edits, bodies, joints, lights, scene queries and effects.
    - **Voxel bodies as objects** (left by step 8, decided 2026-11-11): a loose part is today a body id
      (`voxelBodies()`, a physics hit's `body`, `voxelImpulse`), drawn by objects scene code did not make. How it
      appears to a game in three's style (an object with `.physics`: `applyImpulse`, `getLinearVelocity`, contact
      events, joints between voxel objects) is designed here. With it: paint and added voxels reaching bodies, and
      a body's mass spread by its cells' own densities (it is their mean today).
    - Fire and smoke on the existing volume and particle paths.
    - Vehicles and ropes on step 7's joints.
    - Acceptance: the benchmark level played through as a game, every line of `docs/VOXEL.md`'s benchmark measured.

## Known issues

- Under software traversal 23 of the 170 atomic tests fail (found 2026-11-11 in § 6 step 7b, the first whole run of
  the suite under software traversal; main `a707e1c`'s build and the step's, as written and in the merged layout,
  the same ids and values; not fixed: the rendering line's). Hardware traversal passes all 170.
  - 20 are particles, which software traversal does not draw (they stay collapsed at the pivot,
    docs/MATERIAL_FUNCTIONS.md): 13 `particles/*`, the 5 `gpu-particles/*`, `translucent/depth`, `translucent/order`.
  - `volume/lighting` and `volume/lighting-offline` read 6 for 1. Basis: both measure one translucent particle in a
    closed cube with one 6-nit wall behind it (their features are `particles`, `translucency`); without the particle
    the region is the wall. The same missing particles, in a family whose name does not say so.
  - `surface/windows` reads 0.3934 for 0.4234 (−7 %, tolerance 3 %); `surface/windows-mirror` beside it passes.
    Basis: the scene is one plane with the library's facade windows and no instance, so nothing of step 7b is in
    its kernel (the source is main's byte for byte), and main's build reads the same 0.3934 in both layouts. The
    cause was not looked for.
- Offline renders of `bistro`, `bistro-slice`, `bistro-district`, `zero-day` and `zero-day-animated` are not the same
  bytes twice under hardware traversal (measured 2026-11-11 in § 6 step 7b's merge gate; the last entry here has
  Bistro's ties between coincident surfaces at 2–4 %). Main `a707e1c` against itself, 320 × 180, 4 spp: 1,782 /
  1,095 / 125 / 7,988 / 10,426 of 57,600 pixels differ; the catalog's other 18 bundles repeat exactly, and software
  traversal repeats all 23. So hardware traversal's byte identity on these five says nothing; software traversal's
  is the gate.
- Emissive quads under masked decals of the same shape went black with hardware ray tracing (found 2026-11-08 by the
  unbiasedness check, `tools/unbiased-check.ts`; fixed the same day, branch `coplanar-flatten`).
  - Zero Day against the software BVH (16384 spp, 320 × 180, raw HDR): one 4 × 4 region 0.966, another 0.978, at every
    sample count and seed; two pixels the software BVH read at 30–40 nits were 0.05.
  - Cause: Zero Day has an emissive quad and an alpha-masked decal on exactly the same vertices. Instancing put the
    decal in the model's BLAS and flattened the emitter into world space; both met the ray at the same t, the decal
    won, its alpha let the ray through, and the next query started past both.
  - Fix (scene build): a pool triangle on exactly the same vertices as a flattened one (emissive, glass, material
    function) is flattened with it. Zero Day: 2 triangles move; Bistro: none. Regions now 1.007 / 1.005 at 4096 spp,
    no runtime cost; the catalog's offline renders are byte-identical (software traversal), Bistro's too (hardware).
  - Not taken: one more query per stepped cutout over the gap, opaque geometry only (`RAY_FLAG_CULL_NO_OPAQUE`). It
    fixed Zero Day, but Bistro's offline render took 28 % longer, and on Metal the flag changed the queries after it.
    The same query discarded unused still darkened Bistro's left third by 0.5 % and changed 18 % of its pixels;
    with `RAY_FLAG_FORCE_OPAQUE` instead it changed nothing.
- Live renders with emitters do not repeat exactly (found 2026-11-10 in § 6 step 3, looked into in step 4; not fixed).
  Two runs of the same live render differ from the second frame on: `geometry/emitter-added-live` in 98 % of its
  pixels, a static emissive panel the same; scenes lit by the sun and sky alone (`light/sun-point-live`,
  `geometry/edit-live`), volume emission and procedural objects repeat exactly; `water/scatter-live` differs in 1 %.
  - Where: the radiance cache's training tiles (pathtrace.wgsl `cache_train`, inserting). With the cache off
    (`THREERT_CACHE=0`) or only the training tiles off (`THREERT_CACHE_FLAGS=4`) the renders repeat exactly; without the
    per-frame cap on an entry's samples (`CACHE_FRAME_CAP`) they still differ. The tiles insert voxels into the hash
    table concurrently (`cache_find`: a sample whose free slot another key took first is dropped, and lookups in the
    same pass find this frame's insertions or not), so which samples an entry gets in a frame depends on the order the
    GPU runs the threads in. A light whose every sample is the same (a sun) hides it; emitter sampling draws a
    different point each time, so the entries' means differ from run to run, and through the next frames' reads, every
    pixel.
  - What it costs: noise, not bias (step 3's layer A tests read the same on both paths and runs). On the benchmark level
    at night (step 4) two live runs of the same shot differ by ΔE 0.5–1.1 (64 frames after a reset, 0.5 floor), and
    each reads the same against the offline render to ±0.01; step 4 reports both runs.
  - Fixing it would mean deterministic insertion (a pass that inserts the frame's new keys in a fixed order before the
    training) or per-tile accumulation merged in order; not taken while it is only run-to-run noise.
- An object added in place was often not drawn (found 2026-11-10 in § 6 step 2g; fixed in step 5, branch
  `incremental-structure`). A scene of one `dynamicGeometry` mesh, a second added by the frame script: the edit was in
  place, its data as the build's, but the camera saw the sky where it was (5 of 6 runs; with the queue drained around
  the builds, every run). The narrowing that found it: an object added with a mesh it already had (no new BLAS) always
  showed. Cause: the edit built the new mesh's BLAS and then the TLAS in two builds of one encoder; on Metal the TLAS
  reaches its BLASes through their resource ids (indirect instance descriptors), so nothing ordered its build after
  the BLAS's, and the BLAS barrier between them is a no-op there: the TLAS could see the BLAS unbuilt and keep its
  instance empty. Fix: the new BLASes (meshes' and procedural) and the TLAS in one build, as the upload does
  (`Gpu::build_edit`). 18 of 18 runs draw it; layer A `geometry/added-live`, `procedural/added-live` check the added
  object (`geometry/edit-live` checks a swapped geometry: it never showed). The shared-pose path still builds its
  meshes' BLASes and the TLAS in separate submissions (§ 0's crowd; not changed here).
- Renders did not repeat exactly with hardware ray queries (found 2026-11-07; fixed 2026-11-08).
  - Two runs of the same offline render (Kabukichō, 64 spp, 320 × 180, `THREERT_FREEZE_TIME=1`) differed in 1.4–1.6 % of
    pixels; with software traversal they were bit-identical.
  - Cause: the ray query's candidate loop for alpha-cutout geometry (non-opaque traversal) lost hits on Metal. It came
    up with potted ferns on sunlit paving (§ 5 item 6.5): about 0.1 % of rays near cutout foliage missed even the
    opaque ground behind it, or found it unlit, leaving black pixels; confirming every candidate regardless of alpha
    lost the same hits.
  - Fix (trace_hw.wgsl): every query forces opacity, and a hit on a cutout triangle that fails its alpha test is stepped
    through (the next query starts just past it, up to 16). Shadow rays still stop at the first hit unless it is a
    cutout one its alpha lets through.
  - After: 3 of 57,600 pixels differ between runs (0.01 %); the fern's black pixels are gone (offline 1 spp: 74 → 0 in a
    68,800-pixel patch). Kabukichō's mean is unchanged (0.6565, every 4 × 4 tile within 0.05 %), and it is faster:
    perf smoke 15.28 / 16.25 → 15.13 / 15.96 ms still / moving.
  - Back for shadow rays on 2026-11-08 (§ 0 item 3 part 2b, a candidate loop for shadow rays past a leaf), found the
    same day: two runs of one offline render (64 spp, 320 × 180) differed in 75 % of Kabukichō's pixels and 98 % of
    Bistro's (one differing shadow ray changes a pixel's random sequence from there on). Trusting only the loop's
    confirmed hits did not help: Bistro still differed in 98 %. Fixed (branch `shadow-determinism`): offline shadow
    rays step through cutouts in order, up to 64 (blocked past that); the live viewport keeps the candidate loop.
    Now 0 % on Kabukichō, 2–4 % on Bistro, whose ties between coincident surfaces made the renders before differ in
    2–10 % (hardware traversal only; software traversal repeats exactly).

## Known parity gaps (small)

- Live volume scattering lost energy (found 2026-11-08, fixed the same day, branch `live-volume-energy`).
  `volume/furnace-live` (a non-absorbing sphere of medium in a uniform sky, which reads 1) read 0.963, and 0.931
  without the temporal pass's clamp of the directly seen emission (the sky through the medium, 0 or 1 per sample,
  which the clamp biases up).
  - Not convergence: 120 / 480 / 2000 frames read 0.854 / 0.963 / 0.963. Not the training's firefly clamp (off, the
    same 0.963).
  - Cause: the media march looks the volume voxels up at a point jittered within a voxel (the accumulation then filters
    them instead of showing blocks). Near the medium's boundary the jitter left the medium for voxels no training
    point reached, read them as dark, and the training's feedback carried that loss into every scattering order.
  - Fix: a jittered lookup that finds a voxel short of CACHE_WARM samples takes the voxel of the point itself.
    furnace-live 0.9965, 0.993 without the clamp, so background pixels no longer keep the emission clamp (§ 0 live
    energy); its tolerance 0.05 -> 0.02. box-sun-scatter-live 13.44 -> 13.68 (expected 13.72), box-single-scatter-live
    6.255 -> 6.415 (6.349). The C slice's smoke segments read the same against the offline render (46 s ΔE 4.36,
    55 s 4.34 -> 4.33; its smoke is a small part of the frame); rain + smoke on Bistro 14.84 / 14.69 ms still / moving
    either way (alternated); perf smoke ±0.03, ghosting ok, catalog offline byte-identical (live only).
  - Left: `volume/lamp-inside-live` 7.61 against 6.607 (+15 %). The jitter's average is trilinear interpolation, which
    overestimates the convex 1 / r² field near the lamp (7.08 unjittered, but blocky). The Bistro C slice's smoke is
    denser and brighter live than offline (tools/slice-eval.ts).

- Sparks light what is near them (2026-11-07): `SparkBurst` particles are emitters, sampled as lights. UE 5.8's burst
  templates have no Niagara Light Renderer, so their sparks light nothing (`tools/bench-burst.ts`: floor +0.0028 vs 0).
  A UE effect that should light its surroundings adds one.

- Translucent particles (2026-11-07) are drawn over the image by the translucency pass and are in none of the path
  tracer's acceleration structures. They cast no shadow and show in no reflection, refraction or GI: UE's default for
  translucent materials (Lumen leaves them out of its scene, and screen traces see only opaque depth). A wet street
  reflects the signs but not the rain in front of them, in both. Their lighting comes from the radiance cache's volume
  voxels, the counterpart of UE's translucency lighting volume (docs/MATERIAL_FUNCTIONS.md).

- Auto exposure on a night scene (measured 2026-11-07, not changed). On the hello-world night scene
  (`compare/hello/threert/neon-reflections.ts`, every setting at its default), UE 5.8's default auto exposure lands
  at EV100 −4.7 and ours at −1.1. UE lifts the night 3.5 stops more than we do; both turn it into a lit room.
  - UE side: the scene built natively (`bun ue5/neon-ae.ts`; the knot as a gold sphere, no sky), Lumen with the
    project's default exposure; its EV100 is read off manual renders of the same frame (`node tools/neon-ae.ts`).
  - Aligning to UE would brighten it further, so nothing changes. A night scene states its exposure
    (`setManualExposure`), as a UE level would.
  - Why the two auto exposures differ by 3.5 stops on the same lighting (calibration, metering, UE's default +1 bias)
    is still open.

- Emissive light through objects that cast no shadow (fixed 2026-11-06). Shadow rays toward emissive surfaces skipped
  `castShadow = false` geometry (in the street scenes: most building parts and the ground), while the BSDF rays that
  find the same emitters, their MIS partner, are blocked by it. Light leaked through by the emitters' sampling share:
  - Kabukichō −0.45 % overall after the fix, −3 to −6 % on the wet crosswalk (grazing reflections of the signs).
  - District −1.7 to −4.3 % on the facades and ground near the camera.
  - It surfaced in the light LOD prototype, which raises near emitters' share and so made the leak +16 %.
  `castShadow` now applies to the analytic lights only, as in UE's path tracer, which reaches emissive surfaces only
  by rays that every mesh blocks. Glass now also blocks shadow rays toward emitters: lamps behind glass are lit through
  the glass's BSDF path alone, where before they were counted on both paths. Atomic test `shadow/emitter-no-cast`
  (main: ratio 8.09, fixed: 0). Cost at the 0.5 floor: Kabukichō +0.09 ms, district drive +0.11 ms p50.

- Auto exposure (fixed 2026-11-06): the presets' histogram range was UE's legacy -8 .. 4 (log2 luminance), which
  daylight saturates (a 3000-nit sky pinned the exposure at EV 6.5, the image washed out). UE 5 projects enable
  r.DefaultFeature.AutoExposure.ExtendDefaultLuminanceRange (every template), giving EV100 -10 .. 20, the same log2
  luminance at the default lens attenuation (Scene.cpp, PostProcessEyeAdaptation.cpp EV100ToLog2). The presets use
  it now; Kabukichō keeps the legacy range explicitly, as its UE project does. Atomic tests `exposure/auto-daylight`
  (offline and live; 14.00 vs 14.03 expected) and `exposure/auto-night`. Still different: UE's default exposure
  compensation is +1 EV (r.DefaultFeature.AutoExposure.Bias = 1.0); our camera's default is 0.

- Sky atmosphere: ours is 5–14 % brighter and bluer than UE's (mannequin scene, sRGB ratios UE/ours 0.95 / 0.92 /
  0.86 at the top, 0.95 / 0.93 / 0.88 at the horizon), which also tints sky-lit shadows; UE's horizon band is hazier.

- Below the horizon (done 2026-11-06). UE's references draw no lit planet ground there, as was assumed:
  - The path tracer shows its sky light, which holds the atmosphere as a cubemap: SkyLight CubemapResolution 128,
    lower hemisphere black, sampled bilinearly. So the sky fades to black across one texel row of the side faces:
    0.9° at a face's centre, 0.63° at its edges, about 15 rows in the Khronos studio's 35° view.
  - Lumen draws a hard dark line instead.
  - The atmosphere bake now writes exactly that fade, rows below the horizon included. The renderers are unchanged, so
    it costs nothing per frame. A first version computed it in the kernel's environment lookup and slowed Kabukichō's
    trace by 0.47 ms through code size alone, without even taking the branch.
  - The luminance profile across the Khronos horizon now has UE's shape (ramp to 48 vs UE's 50 over the same rows;
    the sky above stays ~15 % brighter, the item above).
  - Atomic tests `sky/horizon-black` and `sky/horizon-fade`.

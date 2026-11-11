# ThreeRT

A real-time GPU path tracer for games and scenes written in a Three-like TypeScript DSL.

- **Scenes** are written with `packages/scene`, a Three-like DSL. **Gameplay** uses `packages/gameplay`: Verse-inspired transactional handlers, per-frame scripts, and world handlers that change the scene as an event arrives.
- That code runs inside the **native renderer**, `renderer/`: Rust and wgpu, a GPU path tracer with UE5-matched shading and post, and an embedded QuickJS runtime. Scene code drives the engine API directly; there is no scene interchange format.
- **UE5 renders are the ground truth** (`ue5/`). **three.js** is the baseline it is compared against (`tools/three`, `compare/`).
- **The voxel engine** around it is described in [`docs/VOXEL.md`](docs/VOXEL.md).

**Where this came from.** This repository was extracted on 2026-11-10 from `taichi-dev/serverless-agent-test` (main at `42a51d6`), which keeps the history before the split.
- The PR numbers in [`docs/ROADMAP.md`](docs/ROADMAP.md) and the other docs (#1–#139) are that repository's.
- So is the serverless gameplay platform built around the engine: the Cloudflare gateway, the rooms and the render pool (`cloud/`), and the GPU render nodes (`render-node/`, `render-container/`). The docs that describe it ([`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`docs/REPORT.md`](docs/REPORT.md), [`docs/RUNTIME.md`](docs/RUNTIME.md)) refer to those directories there.

Mirror: [`doodlewind/ThreeRT`](https://github.com/doodlewind/ThreeRT).

## Layout

| Dir | What |
|---|---|
| `renderer/` | The engine: Rust + wgpu path tracer, live viewport, denoiser, post, embedded QuickJS (`threert-render`); `renderer/voxel-core` and `renderer/physics-core` (Rapier; both also build for wasm32); `renderer/third_party/glamx` (a patched dependency of Rapier's, see its PATCH.md) |
| `packages/scene` | The Three-like scene DSL (`@threert/scene`) |
| `packages/gameplay` | Games: state, transactional handlers, world handlers (`on`), frame scripts (`@threert/gameplay`) |
| `packages/voxel` | The voxel engine's TypeScript side (`@threert/voxel`) |
| `scenes/` | The reference catalog (Cornell box, material grid, Bistro, Zero Day, …) |
| `games/` | Playable content: Kabukichō Night, Bistro slice / district / crowd, Zero Day, Tokyo, the voxel level, Light Room, … |
| `tests/` | The render test suite (`tests/render`, analytic and temporal checks), three.js compatibility tests |
| `tools/` | Bundling, benches and gates (`perf-smoke`, `bench-ghosting`, `bench-district`, `test-render`, `test-layer-b`), three.js comparison, the Tokyo pipeline (`tools/city`, `tools/tokyo`) |
| `ue5/` | UE5 reference captures (Lumen and the Path Tracer) |
| `playground/` | ThreeRT Playground: a macOS native editor around the live viewport |
| `compare/` | Hello-world scenes side by side in three.js and ThreeRT |
| `docs/` | Roadmap, workflow and merge gate, parity, material functions, voxel engine |

## Getting started

```bash
npm install && bun scripts/setup.ts      # assets/ -> the machine's shared asset store, then fetch what is missing
node tools/build-scenes.ts                # scenes/*.ts + games -> out/bundles/<id>.js
bun renderer/build.ts && renderer/target/release/threert-render render out/bundles/cornell-box.js -o out/cornell.png --spp 32
renderer/target/release/threert-render compare out/ue/lumen/cornell-box.png out/cornell.png   # vs the UE5 reference
node tools/three/open.ts scenes/bistro.ts # the same scene in three.js, in your browser (orbit, fps, what it drops)
npm test && npm run typecheck && (cd renderer && cargo test --release)  # DSL, gameplay, runtime and engine tests
npm run test:render                       # the render test suite on the GPU
```

**Local real-time demo (native window, 60 fps):** `renderer/target/release/threert-render play out/bundles/light-room.js` runs the "Light Room" game (`games/light-room/light-room.game.ts`: Three-like DSL, transactional handlers and a frame script) in the native renderer.
- Click coins, the floor, the balls, the crate, the knot or the walls.
- Drag to orbit, right-drag to pan, wheel to zoom, WASD/QE to fly, `R` to reset, `P` for a snapshot.
- Any scene works the same way: `threert-render play out/bundles/sun-room.js`.

**Games as ES modules:** `node tools/build-runtime.ts` bundles `@threert/scene` + `@threert/gameplay` into `out/threert-runtime.js`. A game module that imports it renders with `threert-render render <game.js> --runtime out/threert-runtime.js -o out.png`.

**ThreeRT Playground (macOS native editor, 60 fps):** `bun playground/build.ts && bun playground/run.ts` opens an Unreal-Editor-style client around the native real-time viewport. See [`playground/README.md`](playground/README.md).
- A PocketJS UI: toolbar, outliner, details with live scrub editing and undo, content browser, output log, acceptance and a game panel.
- Pixel-aligned A/B against the UE5 Lumen and Path Tracer references.
- Play-in-Editor for games.

**Kabukichō Night** (`games/kabukicho/`): an interactive Shinjuku night street, generated procedurally with the Three-like DSL and lit almost entirely by emissive signage.
- The Ichiban-gai gate, ~120 signs, wet asphalt, shops, lamps, pedestrians and a taxi.
- Sign typography and storefronts come from one atlas drawn by `tools/kabukicho-atlas.ts`.
- v2 adds Poly Haven CC0 ground, facades and props (`node tools/fetch-polyhaven.ts` first), procedural windows, rain, volumetric fog and a night sky ([`docs/PARITY.md`](docs/PARITY.md)).
- Open it in the Playground and press Play. Click signs, the gate, vending machines, lanterns, lamps or the taxi; `R` rain, `L` closing time, `V` viewpoint.

**Scene code runs in the renderer** ([`docs/RUNTIME.md`](docs/RUNTIME.md)): `threert-render play out/bundles/kabukicho.js` runs the game inside the embedded QuickJS.
- Setup builds the scene through the engine API.
- Clicks and keys run handlers that commit game state, and world handlers (`on`) that change the scene at once.
- The game's `frame` script (UE's Director Tick: neon flicker, wet / dry ramps, moods, power cut) derives the scene from that state every frame. Parameter changes rewrite GPU tables in place.

**Tokyo** (`games/tokyo/`, `tools/tokyo/`): the open city, Shinjuku and Shiba, compiled by `tools/city` from public data. Live time of day uses the dynamic sky, windows switched by the clock, moonlight and the night sky.

**Scene authoring skill:** [`.claude/skills/threert-scene-authoring/`](.claude/skills/threert-scene-authoring/SKILL.md) covers photo-referenced scenes: the workflow, material / lighting / atmosphere values, the asset pipeline and the review / performance loop.

**How work lands:** [`docs/WORKFLOW.md`](docs/WORKFLOW.md) covers one PR per item from main, the merge gate by tier and who merges. [`docs/ROADMAP.md`](docs/ROADMAP.md) has the plan and every step's measurements.

## CI

`.github/workflows/ci.yml` runs on every push to main and every PR, as parallel jobs:
- `ts`: `npm run typecheck`, `npm test`;
- `renderer`: the renderer's release build;
- `voxel`: the voxel core's wasm32 build and tests, and native / wasm32 edit determinism;
- `physics`: the physics core's wasm32 build and tests, and native / wasm32 determinism (a script and 1,200 random worlds);
- `wasm`: the engine library's wasm32 build;
- `web`: the browser module's build (`renderer/web`), and the physics hello world on that module in Node (`tools/web-engine.ts`: no browser, no GPU).

A PR runs a job only when it changes that job's inputs (the paths in the workflow's `changes` job; a change to the workflows runs them all); a push to main runs them all.
`threert` sums them up and is the check to read before merging: it fails when a job failed or was cancelled, and passes when each one passed or was skipped by its paths.

The GPU checks (`test:render`, the perf smoke, the benches, the browser test `tools/web-hello.ts`) run on a Mac, per [`docs/WORKFLOW.md`](docs/WORKFLOW.md).

## Assets and third-party content

The assets are not committed: Poly Haven, Kabukichō's generated textures, Khronos and Quaternius models, the ORCA scenes and the compiled Tokyo tiles. That is ~1.8 GB, plus the 2 GB of ORCA archives.
- They live once per machine in `<main clone>/../threert-assets`.
- Every checkout's `assets/` is a relative link to it, made by `bun scripts/setup.ts`. Run it in each new clone or worktree; it never deletes anything.
- Scripts run with Bun.
- The ORCA scenes (Bistro, Zero Day) are FBX + DDS, converted to glTF by `scripts/orca.ts` (Blender headless; skipped without Blender, `brew install --cask blender`).

Third-party scenes are under Creative Commons Attribution 4.0 (https://creativecommons.org/licenses/by/4.0/).
- They were converted from FBX + DDS to glTF with PNG / JPEG textures by `scripts/orca.ts` and are otherwise unchanged.
- Each archive's LICENSE.txt and README.txt are kept next to the glTF in `assets/orca/<scene>/`.

The scenes:
- **Amazon Lumberyard Bistro**: Amazon Lumberyard (2017), Open Research Content Archive (ORCA), http://developer.nvidia.com/orca/amazon-lumberyard-bistro
- **Zero-Day**: Mike Winkelmann (Beeple) and Kate Anderson (2019), Open Research Content Archive (ORCA), https://developer.nvidia.com/orca/beeple-zero-day

Tokyo:
- The compiler in `tools/city` is imported from [jeantimex/tokyo](https://github.com/jeantimex/tokyo) (MIT, © 2026 Yong Su; [`tools/city/LICENSE`](tools/city/LICENSE)).
- The data: the 3D city model is Project PLATEAU (MLIT Japan); elevation and aerial photos are GSI; roads and places are © OpenStreetMap contributors (ODbL).
- Compiled tiles are a derived database under the ODbL and are not to be published.

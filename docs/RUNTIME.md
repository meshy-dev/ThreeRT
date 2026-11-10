# Scene runtime on the render node

> Split note (2026-11-10): this engine was extracted from `taichi-dev/serverless-agent-test` (main `42a51d6`). The serverless parts this document describes — `cloud/` (gateway, rooms, render pool), `render-node/`, `render-container/` and the deploy scripts — stay in that repository.

ThreeRT's goal is that an agent writes script-like code and gets a UE-quality world it can play at 60 fps. The
measure is how good the final work is for a given creative intent, so the platform does not need a cross-engine
interchange format. Round 2 (docs/PARITY.md) showed where expressiveness comes from: code. The UE session's look came
from Custom-HLSL materials, a C++ Director ticking every frame and gameplay switching material parameters. A field in
a scene schema is a ceiling.

So there is no scene document. The scene code runs where the scene is rendered, in the renderer's embedded JS
runtime, and drives the engine directly, the way a UE game drives its actors:

```
 Room (authority) ── committed game state ──┐
 handlers, OCC, log (state only)            ▼
                          ┌────────────── render node (threert-render) ───────────────┐
 game code ─────────────▶ │ ScriptHost (QuickJS): setup() builds the Three-like scene  │
 (bundle, by hash)        │   frame(f) derives it from the state, every frame          │
                          │   SceneBinding ──▶ engine API (__engine natives)           │
                          │ World (engine objects) ── changes ──▶ LiveCore             │──▶ pixels
                          │   parameters: GPU table entries in place (µs)              │
                          │   structure:  rebuild from the world (in memory)           │
                          └────────────────────────────────────────────────────────────┘
```

* **The unit of distribution is the game code.** Locally a bundle (`tools/bundle-game.ts`: game + `@threert/scene`
  + `@threert/gameplay` + the node host in one script); in the cloud the agent's ES module plus the shared runtime
  module. A node builds the scene from it (deterministic per room seed).
* **The world is the game state.** Handlers change `tx.state` only (OCC, log, replay). They never touch the scene.
* **The scene is a function of the state.** `frame(f)` (an actor's Tick on a UE client) reads the committed state and
  sets the scene: transforms, visibility, `power`, light and material parameters, the environment, the camera, objects
  added or removed, plus animation (flicker, ramps). It runs every frame in Play, and with `dt = 0` after setup and
  after every commit. After setup or a replaced state `f.snap` is true: the scene shows the state settled, so a render
  of any room state is right.
* **No interchange format.** Nothing is exported. The UE ground truth for a level comes from that level in UE
  (docs/PARITY.md § Round 2); the old IR → UE adapter is gone.

## Pieces

| Piece | Where | What |
|---|---|---|
| Engine API | `packages/scene/src/engine.ts` (`Engine`), `renderer/src/script.rs` (`__engine` natives) | Handles (u32) for geometries (typed arrays), textures (asset path, encoded image or RGBA8 pixels), material-function code, materials, objects (group / mesh / model / light / camera) and their transform, visibility, power, light / camera parameters; the environment and render settings. Descriptors are JSON-shaped (`renderer/src/desc.rs`). |
| Scene binding | `packages/scene/src/engine.ts` `SceneBinding` | Keeps the engine in step with a DSL scene. Objects report their own changes (core.ts change tracking: transforms, visibility, power, shadow flag, mesh geometry / material, add / remove), so a sync visits only what changed. Lights, cameras and materials are compared against what was last sent. |
| World | `renderer/src/world.rs` | The engine's objects, resources, environment and settings, and what changed since the last commit: parameter changes (power, light / material parameters, environment, camera) or structural ones. Asset paths must stay inside the asset root (game code is untrusted on nodes). |
| Live commit | `renderer/src/live.rs` `LiveCore::commit`, `tables.rs` | Parameter changes rewrite GPU table entries in place (materials, lights, object power, emitter CDF, wetness / rain / fog, camera). Anything structural rebuilds the GPU scene from the world, in memory; history survives, validated per pixel. |
| Scene runtime | `renderer/src/script.rs` `ScriptHost` | QuickJS (rquickjs) with no I/O (no fs, network or timers). Loads a bundle (one script) or ES modules (`game.js` + `threert-runtime.js`, the cloud layout). Memory capped at 1 GiB. Time budgets: setup 120 s, event 1 s, frame 50 ms; the interrupt handler aborts overruns. A prelude adds `TextEncoder`, `structuredClone`, `atob` and `console`. |
| Node host | `packages/gameplay/src/node-host.ts` | `globalThis.__threert`: `setup(room, seed)`, `event(type, payload, ts)` (local authority: execute + commit), `commit(writes, seq)` / `setState(state, seq)` (room authority), `state()`, `frame(dt, now)`, and the editor's `outline()`, `inspect(name)`, `edit(name, path, value)`. |
| Frame scripts | `packages/gameplay/src/frame.ts` | `FrameContext`: `time`, `dt`, `snap`, `now`, read-only `state`, `scene`, `camera`, `object(id)`, `random()`. |
| Game session | `renderer/src/script.rs` `GameSession` | A runtime + its live view: events, clicks (CPU pick → `click {target, point}`, with `instanceId` on an `InstancedMesh`), frame script, editor edits, each followed by a commit. Shared by `play`, `live` and the playground. |

## Entry points

```
threert-render play <bundle.js> [--assets DIR]          native window; keys / clicks go to the game
threert-render live <bundle.js> [--assets DIR]          headless 60 fps; {"cmd":"event"} / {"cmd":"click"} on stdin
threert-render render <bundle.js> -o out.png [--state s.json] [--camera {position,target,fov}] [--time t] [--preset p]
threert-render serve                                    JSON-lines jobs {game, runtime?, assets, room, seed, state, ...}
threert-render setup <bundle.js>                        setup cost in the runtime
threert-render edit-check <bundle.js> --events [...]    in-place edits == a fresh build
node tools/bundle-game.ts <game.ts | scene.ts>          a scene module (default export () => {scene, camera, options})
node tools/build-scenes.ts                              bundles every catalog entry to out/bundles/<id>.js
```

`serve` keeps one runtime per (code version, room, seed): the first job of a room runs setup, later ones only apply
the job's state (`setState`, settled by the frame script). A job whose scene reads files that are not under the asset
root yet fails with `missing: [...]`; the node fetches them and retries.

The playground runs the opened scene the same way: its host holds the `GameSession`; the Node backend keeps the
catalog, the selection, the undo stack (Details edits as `{node, path, before, after}`) and the Play flow. Stop
re-runs setup and replays the editor's edits (UE PIE: Play changes are discarded).

## Correctness

* `threert-render edit-check` builds the GPU tables both ways for a list of events: edited in place, and built fresh
  from the edited world. For Kabukichō (three moods, closing time, the gate, rain) materials, lights, object power, the
  emitter distribution and wetness / rain are bit-identical, and none of the edits is structural.
* Every reference scene and Kabukichō render from their bundles like they did from the old IR packages (same
  triangle, light, emitter and texture counts; at 64 spp mean ΔE 0.0 (material-grid, bit-identical) to 0.99, and
  1.43 for Kabukichō, where the error map is sampling noise only). The six UE cameras (`tools/kabukicho/views.py`,
  now `render --camera`) frame identically.
* `tools/kabukicho/verify-port.py` (against `node tools/kabukicho/dump-street.ts`) still matches every UE actor
  (2,158 shapes, 0.0000 m).
* Playground UI test (`threert-playground --uitest`): 14/14, including Play / Stop in Light Room.
* Cloud: `node cloud/test/e2e-local.ts` and `play-smoke.ts` pass (rooms hold state only; nodes run the game code
  against the room's state; a cold job fetched code, runtime and the missing asset, then rendered; a warm event →
  PNG takes 124 ms with 0.5 ms of script).
* Live temporal bench (`tools/bench-temporal.ts --scale 1`, same converged references as PR #9): camera motion
  unchanged (ΔE 5.29–6.25 vs 5.21–6.38); 4 s after closing time ΔE 4.00 vs 3.90. The earlier closing shots differ
  by design: each sign group now stays lit until its staggered delay and then flickers off, like the UE Director,
  where before the handler switched every group off at once.

## Kabukichō: the UE Director as a frame script

The UE session's `AKabukichoDirector` (C++) ticks every frame. Rain eases at 0.6 / s. Streets get wet at 0.35 / s
and dry at 0.06 / s. Fog eases toward the mood. Power groups that change state flicker like failing neon for
0.35–1.1 s (on) or 0.08–0.3 s (off), and switch with staggered delays when many change at once: moods, the 3 AM
closing, the power cut. `games/kabukicho/kabukicho.game.ts` ports it line for line:

- **Handlers** commit the Director's targets as game state: `rain` (R), `mood` (M), `blackout` (B), `lights` (L),
  `click` (a sign's power group, the gate), `viewpoint` (V).
- **`frame(f)`** makes the street follow them: group powers, gate emission, wetness / rain / fog, the camera marker;
  settled at once when `f.snap`.

![Kabukichō in the node runtime: start · power cut 0.15 s · 1.3 s · restoring 4.8 s](images/runtime-blackout.jpg)

## Measurements (Apple M5, 1280×720, Kabukichō: 4.0 M triangles, 2,665 objects, 147 lights)

| | IR patches (PR #8) | no IR |
|---|---:|---:|
| Scene setup in QuickJS | 345–381 ms (IR + generated GLBs / PNGs) | 255–266 ms (engine calls, no encoding) |
| Gameplay event → scene updated | patch 0.01–4.8 ms | handler + frame(0) + sync 6–13 ms; commit in place 0.00–0.45 ms |
| Frame script (Director port) + sync + commit, per frame | 0.12–1.1 ms (presentation writes) | 0.4–0.8 ms average, 2.1 ms max (7–8 ms before change tracking) |
| Generated-texture scene setup (glb-asset) | — | 43 ms (732 ms when DataTexture still encoded PNG in JS) |
| One-shot render of another room state (`serve`, warm, 640×360, 16 spp) | 0.88 s | 1.23 s (3.6 ms script) |
| Live open (playground) | 5.2 s | 5.8 s (setup 0.26 s, build 3.1 s, texture pages 1.3 s, upload 1.7 s) |

QuickJS is about 8× slower than V8 for this kind of code. It matters for setup-heavy content (large procedural
geometry in JS), where the runtime could move to V8 (deno_core) behind the same `ScriptHost` interface.

## Next

1. **Movable objects without rebuilds.** Transforms and visibility become TLAS instance updates for objects that
   scripts move (UE Mobility = Movable), e.g. the UE session's crowd (46 walkers, 14 loiterers) and vehicles. They
   are structural today: fine for small scenes or occasional changes, not every frame on a big street.
2. **Warm room sessions on nodes.** `serve` keeps runtimes per room already; a node could also keep the GPU scene of a
   room between jobs and apply state changes in place, as the live view does.
3. **Kernel variant warm-up.** An edit that enables a kernel feature (e.g. a dry street getting wet) compiles that
   variant on first use; pre-compile likely variants in the background.
4. **UE references for the simple scenes** came from the removed IR adapter (cached in out/ue). New ones need either
   the scenes rebuilt in UE by hand or an engine-API binding on the UE side (the scene code driving UE actors).

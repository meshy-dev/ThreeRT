# ThreeRT — serverless 3D gameplay & rendering runtime for AI agents

> Split note (2026-11-10): this engine was extracted from `taichi-dev/serverless-agent-test` (main `42a51d6`). The serverless parts this document describes — `cloud/` (gateway, rooms, render pool), `render-node/`, `render-container/` and the deploy scripts — stay in that repository.

```
 Agent (LLM)                                    Cloudflare                                         GPU render pool
 ───────────                                    ──────────                                         ───────────────
 game.ts  ──POST /v1/code──▶ agent3d-gateway (Worker) ── sucrase strip ──▶ R2 code/<hash>.js
 (Three-like DSL +          │  ├─ Room DO (per room, SQLite): authoritative game state, versions,
  Verse-style handlers +    │  │    tx log, OCC commit / retry, replay & determinism verify
  frame script)             │  │       │ snapshot + event                    ▲ reads / writes (state only)
                            │  │       ▼                                      │
                            │  │   Dynamic Worker (LOADER, globalOutbound: null)  ◀── handlers run here
                            │  ├─ RenderPool DO: jobs {code, room, seed, state} ──WS──▶ render-node ── threert-render serve
                            │  └─ R2: code/, assets/, renders/            (scene code runs here: QuickJS → engine API;
 screenshot ◀──GET /v1/jobs/:id─────────────────────────────────────  Metal / Vulkan, HW RT, OIDN) PUT result ◀──┘
```

## Layers

| Layer | Where | What |
|---|---|---|
| Scene DSL | `packages/scene` | Three.js-compatible API (`Scene`, `Mesh`, `*Geometry`, `MeshStandardMaterial`/`MeshPhysicalMaterial`, `DirectionalLight`/`PointLight`/`SpotLight`/`RectAreaLight`, `PerspectiveCamera`, `GLTFModel`, `SkyAtmosphere`). Physical units (lux / cd / nits). Pure TS, no DOM: runs in Node, Workers and Dynamic Workers. |
| Engine API | `packages/scene/src/engine.ts`, `renderer/src/{script,world,desc}.rs` | What scene code drives: handles for geometries (typed arrays), textures, materials (PBR + material functions), objects (group / mesh / model / light / camera) and their transform / visibility / power / parameters, the environment and render settings. `SceneBinding` keeps the engine in step with a DSL scene (objects report their own changes). No scene document or interchange format (see `docs/RUNTIME.md`). |
| Scene runtime (node) | `renderer/src/script.rs`, `packages/gameplay/src/node-host.ts`, `frame.ts` | Game code (`tools/bundle-game.ts` bundles, or the cloud's ES modules) runs in QuickJS inside the renderer: setup builds the scene, `frame(f)` derives it from the committed game state every frame, and the world's changes rewrite GPU tables in place or rebuild in memory. Time of day is in place too: a `dynamic` `SkyAtmosphere`'s sun moves without a rebuild (its light rewritten, the sky re-baked on a worker thread), and a manual exposure that changes value is applied in place. `threert-render play`, `live`, `render`, `serve`. |
| Presets | `presets/*.json` | `cinematic` / `preview`: renderer-neutral intent + the UE5 post chain constants (filmic tonemapper, SOG bloom, vignette, exposure) + per-backend knobs (`native`, `ue`). |
| UE5 ground truth | `ue5/` | `bun ue5/capture.ts <project> <job.json>` renders stills of an existing UE level at given cameras (headless UnrealEditor); `ue_dump_level.py` dumps a level's actors for port checks. The reference images of the simple scenes (out/ue) came from an earlier IR → UE adapter, now removed. |
| Native renderer | `renderer/` | Rust + wgpu compute path tracer (Metal / Vulkan / lavapipe). UE DefaultLit BRDF, Hillaire sky identical to UE SkyAtmosphere defaults, HW ray queries when available, OIDN denoising, exact port of UE's bloom/exposure/filmic tonemap/vignette. `render`, `serve` (warm JSON-lines worker), `compare`. |
| Gameplay | `packages/gameplay` | Verse-inspired transactions over the game state: `tx.decide` (failable), `tx.attempt` (nested speculative scope), read/write sets with path versions, OCC commit, deterministic `tx.random`/`tx.now`. Frame scripts (`frame(f)`) present the state on the render node. |
| Cloud | `cloud/` | Gateway Worker, Room DO, RenderPool DO, Dynamic Worker sandbox, R2. |
| Render node | `render-node/`, `render-container/` | WS client that feeds a warm `threert-render serve`; Linux image (Vulkan / lavapipe + OIDN) for GPU VMs or CPU fallback in Cloudflare Containers. |

## Conventions shared with UE

* Coordinates: scenes are three.js/glTF (right-handed, Y-up, meters). UE mapping `UE = 100·(x, z, y)`, the Interchange glTF importer's.
* UV: three-style DSL UVs (v up) are flipped to the GPU convention (`1 − v`) when geometry reaches the engine.
* Light units are physical (lux / cd / nits). UE quirk: rect-light radiance is `brightness / (0.5·W·H)` (RectLightSceneProxy.cpp); ports convert UE candela to nits over the area.
* Exposure: manual EV100 matches UE `AEM_Manual` with the physical camera off and bias = −EV100 (LuminanceMax = 1 with UE5 defaults); native: `2^-EV100`.
* Energy conservation: native GGX has a tunable multiple-scattering compensation; `0` matches UE 5.8 DefaultLit measured against UE's own path tracer, `1` passes the white furnace test.

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
| Presets | `presets/*.json` | `cinematic` / `preview`: renderer-neutral intent + the post chain's settings (`post`: exposure metering, bloom, colour grading, vignette, grain, fringe, local exposure, motion blur; named after Unreal's Post Process Volume settings and meaning what its documentation says, so a look can be compared with the same settings there; `post.tonemapper` holds Unreal's Film values for the comparison scripts in `ue5/` only) + per-backend knobs (`native`, `ue`). |
| UE5 ground truth | `ue5/` | `bun ue5/capture.ts <project> <job.json>` renders stills of an existing UE level at given cameras (headless UnrealEditor); `ue_dump_level.py` dumps a level's actors for port checks. The reference images of the simple scenes (out/ue) came from an earlier IR → UE adapter, now removed. |
| Native renderer | `renderer/` | Rust + wgpu compute path tracer (Metal / Vulkan / lavapipe). A metallic-roughness BSDF (GGX microfacets, [below](#the-surface-bsdf)), a physical clear-sky atmosphere (the Earth medium of Bruneton 2017 / Hillaire 2020, scattering orders computed one by one; `renderer/src/sky.rs`), HW ray queries when available, OIDN denoising, a post chain of its own (`renderer/src/post.rs`; `docs/RUNTIME.md` "The post chain": histogram auto exposure, sum-of-Gaussians bloom, local exposure, cos^4 vignette, the Academy's reference rendering around a fitted filmic tone curve). `render`, `serve` (warm JSON-lines worker), `compare`. |
| Gameplay | `packages/gameplay` | Verse-inspired transactions over the game state: `tx.decide` (failable), `tx.attempt` (nested speculative scope), read/write sets with path versions, OCC commit, deterministic `tx.random`/`tx.now`. Frame scripts (`frame(f)`) present the state on the render node. |
| Cloud | `cloud/` | Gateway Worker, Room DO, RenderPool DO, Dynamic Worker sandbox, R2. |
| Render node | `render-node/`, `render-container/` | WS client that feeds a warm `threert-render serve`; Linux image (Vulkan / lavapipe + OIDN) for GPU VMs or CPU fallback in Cloudflare Containers. |

## Conventions shared with UE

* Coordinates: scenes are three.js/glTF (right-handed, Y-up, meters). UE mapping `UE = 100·(x, z, y)`, the Interchange glTF importer's.
* UV: three-style DSL UVs (v up) are flipped to the GPU convention (`1 − v`) when geometry reaches the engine.
* Light units are physical (lux / cd / nits). UE quirk: rect-light radiance is `brightness / (0.5·W·H)` (RectLightSceneProxy.cpp); ports convert UE candela to nits over the area.
* Exposure: manual EV100 matches UE `AEM_Manual` with the physical camera off and bias = −EV100 (LuminanceMax = 1 with UE5 defaults); native: `2^-EV100`.
* Energy conservation: the native specular lobe has a tunable multiple-scattering compensation (`native.energyCompensation`, [below](#the-surface-bsdf)). `0` is the single-scattering lobe, which is what real-time engines render by default (the previous release's pictures at `0` were measured against UE 5.8's path tracer rendering DefaultLit materials and matched; this implementation renders the same pictures as that release). `1` passes the white furnace test.

## The surface BSDF

The kernel's surface model (`renderer/shaders/pathtrace.wgsl`, "BSDF") is the metallic-roughness model of glTF 2.0 and of the real-time literature, written from these public sources (each is cited again where the code uses it):

| term | what it is | source |
|---|---|---|
| roughness | `alpha = roughness²` | Burley, "Physically-Based Shading at Disney", SIGGRAPH 2012 course notes, § 5.4 |
| `d_ggx` | the GGX (Trowbridge-Reitz) distribution of microfacet normals | Walter et al., "Microfacet Models for Refraction through Rough Surfaces", EGSR 2007, eq. 33 |
| `vis_smith_joint_approx` | the visibility `G2 / (4 n·v n·l)` of the height-correlated Smith masking-shadowing, each of its two square roots `sqrt(a² + (1 − a²) c²)` replaced by `a + (1 − a) c` | Heitz, "Understanding the Masking-Shadowing Function in Microfacet-Based BRDFs", JCGT 2014; the approximation: Hammon, "PBR Diffuse Lighting for GGX+Smith Microsurfaces", GDC 2017, and Filament's documentation (`V_SmithGGXCorrelatedFast`) |
| `f_schlick` | `f0 + (f90 − f0) (1 − v·h)⁵` | Schlick, "An Inexpensive BRDF Model for Physically-based Rendering", 1994 |
| `f0` | dielectrics: `0.08 × specular` (0.5, the default, is 4 %: ior 1.5); metals: the base colour; glass: from its index of refraction | Burley 2012, § 5.4; glTF 2.0, appendix B |
| `f90` | 1, fading to 0 as `f0` falls under 2 % (a material switches its specular off with `specular` 0) | Lagarde & de Rousiers, "Moving Frostbite to Physically Based Rendering", SIGGRAPH 2014 course notes, § 4.10.3, Listing 27 (after Schüler, ShaderX7) |
| `energy_lookup` | the lobe's directional albedo as the split sum's two terms: it reflects `f0 × A + f90 × B` of a uniform surround; a 32 × 32 table over `sqrt(n·v)` and the roughness, integrated at start-up over visible normals (`lib.rs` `specular_albedo`, 10 ms) | Karis, "Real Shading in Unreal Engine 4", SIGGRAPH 2013 course notes, "Environment BRDF"; Lagarde & de Rousiers 2014, eq. 58; Heitz, "Sampling the GGX Distribution of Visible Normals", JCGT 2018; the table's shape: Kulla & Conty 2017, Turquin 2019 |
| multiple scattering | the lobe scaled by `1 + f0 (1 − E) / E`, `E = A + B`, times `energyCompensation` | Turquin, "Practical multiple scattering compensation for microfacet models", 2019, eq. 16; Kulla & Conty, "Revisiting Physically Based Shading at Imageworks", SIGGRAPH 2017 course |
| diffuse under the specular | Lambert × `1 −` the specular's directional albedo toward the view | Kulla & Conty 2017, "Microfacet Dielectrics – Diffuse Term" (theirs uses both directions) |
| clearcoat | a second GGX lobe, `f0` 0.04, over the base, which gets `1 −` the coat's directional albedo (same table) | glTF `KHR_materials_clearcoat`; Burley 2012, § 5.4 |
| glass | a rough dielectric interface with the exact Fresnel reflectance, its own energy table | Walter et al. 2007; Turquin 2019, § 3.2 |
| spot cone | linear in the cosine between the inner and the outer cone, squared; the inner cone is `angle × (1 − penumbra)` | glTF 2.0 `KHR_lights_punctual` (reference attenuation; Lagarde & de Rousiers 2014, Listing 4); `penumbra` as three.js's `SpotLight` |

The live renderer's reflection resolve (`live.wgsl`) weights neighbours' rays with the same `d_ggx` and visibility.

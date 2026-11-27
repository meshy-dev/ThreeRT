# Rendering methods: Raster and Path Traced

> Status: plan (2026-11-27). Nothing below is in this repository yet; the Raster runtime runs today in
> `pocket-nexus/pocket-agents-island` (`apps/desktop`, #28), where it ships the Pocket Island desktop
> app, on PocketJS's JavaScriptCore guest and native text path (pocket-nexus/pocketjs#773, #774).
> This document fixes the names, the code layout and the rules the merge will follow.

ThreeRT is one engine with **two rendering methods**, chosen per project and per build:

| | **Path Traced** (today) | **Raster** (planned) |
|---|---|---|
| What draws | ThreeRT's wgpu compute path tracer: hardware ray queries, OIDN, DLSS / MetalFX, the physical sky and post chain | three.js r187's own `WebGPURenderer`, running natively: `navigator.gpu` implemented over wgpu inside the engine's JavaScript guest |
| Scene code | `@threert/scene` (physical units) | plain three.js + TSL node materials |
| GPU | ray-tracing hardware (Apple M-series, RTX, RDNA2+) | any GPU wgpu supports (Metal, Vulkan, D3D12) |
| Interface | the game HUD (`docs/RUNTIME.md` "HUD") | PocketJS (Solid JSX + Tailwind) drawn by PocketJS's wgpu renderer over the 3D frame |
| Download | the ThreeRT clients | an app of 10 MB installed, 5 MB to download (the Pocket Island desktop app, below) |
| For | the look: production scenes lit by traced light at 60 fps | apps and games that ship small and run everywhere, with three.js's look |

The pairing follows Godot (Forward+ / Mobile / Compatibility) and Unreal (Lumen / Path Tracer): one engine,
one runtime, one scene API family, and a rendering method that is a build choice rather than a different
product. Both methods keep the name ThreeRT; neither is "lite".

## Why a Raster method

ThreeRT's runtime already does what a small native 3D app needs besides rendering: a JavaScript guest that
owns no threads, input, sound, networking, assets, a window. The Raster method adds the other half of the
three.js promise — the scene as written, on any GPU, in a native app a fraction of a browser's size:

- Pocket Island's desktop app moved from Electron (Chromium + Node) to this runtime
  (pocket-nexus/pocket-agents-island#28; measurements in its `apps/desktop/REPORT.md`, Apple M5):
  35.7 % of a core in the foreground against Electron's 64.7 %, 479 MB RSS against 1016 MB, 10.2 MB
  installed against 446 MB, 5.0 MB to download against 170 MB. The native WebGPU binding renders
  three.js pages identically to Chrome's WebGPU (mean difference 0.000 / 255 on its test scenes).
- A three.js developer gets a native build of their scene without rewriting it for `@threert/scene`, and
  can later move the parts that should be traced to the Path Traced method.

## Code layout when it lands

```text
renderer/                      threert-path   (today's crate; unchanged API)
runtime/                       threert-runtime: guest (JavaScriptCore on Apple platforms), window and
                                               frame pacing, input, audio, net, assets, PocketJS UI host
raster/                        threert-raster: WebGPU (`navigator.gpu`) over wgpu for the guest,
                                               the window canvas, Canvas 2D / OffscreenCanvas for textures
```

**Dependencies point one way: `threert-path → threert-runtime ← threert-raster`.** Raster never links the
path tracer: no ray-query kernels, OIDN, DLSS, MetalFX, sky baker or voxel traversal. Path Traced never
links the WebGPU binding. A build selects one method:

```bash
threert build --render raster    # a three.js app: runtime + raster
threert build --render path      # today's default: runtime + path tracer
```

**A size gate holds the Raster build small.** CI builds the Raster sample app (release, LTO, stripped) and
fails when it grows past its budget; the budget is the Pocket Island desktop app's size when the code
moves, plus a margin stated in the gate. Path Traced builds have no such gate.

**Path Traced users see no change.** `threert-render`, `@threert/scene`, `@threert/gameplay`, the clients,
the `dlss` / `oidn` / `net` / `audio` features and the default build stay as they are.

## What moves from pocket-agents-island

| `apps/desktop` today | ThreeRT |
|---|---|
| `crates/webgpu`, `runtime/webgpu.ts` (`navigator.gpu` over wgpu) | `raster/` |
| `crates/canvas2d` (Canvas 2D, images, fonts on CoreGraphics / CoreText) | `raster/` (textures) and `runtime/` (UI text) |
| `crates/pocket-host` (PocketJS runtime + JavaScriptCore guest + wgpu display + native text) | `runtime/` |
| `crates/net`, `crates/rt`, `runtime/pocket.ts` (timers, fetch, WebSocket, storage, audio) | `runtime/` |
| `crates/app` window, frame pacing, shell | `runtime/` (window, pacing); the shell stays with the app |
| `crates/agent-host`, Pocket Island's game and UI | stay in pocket-agents-island |

The JavaScriptCore guest and the native text path are PocketJS pieces (`pocket-guest-jsc`,
`pocket-wgpu-render`'s `TextPainter`); ThreeRT depends on them rather than copying them.

## README, when Raster lands

The headline stays about three.js and native: **"three.js, native."** — the subtitle names both methods:
*"From a few-MB desktop app to hardware ray tracing: write the scene you would write in three.js, ship it
rasterized on any GPU, or trace every light on ray-tracing hardware."* The current *Three + RT* section
becomes the Path Traced section; a Raster section shows the same scene file as a native app with its size
and frame cost.

## Open questions

- **The guest on other platforms.** three.js's renderer does per-frame work in JavaScript; QuickJS runs it
  several times slower than a JIT. Windows and Linux need a JIT engine (V8 or JavaScriptCore from WebKitGTK)
  before Raster ships there.
- **TSL coverage.** Raster runs three.js as written; materials that use `onBeforeCompile` or GLSL
  `ShaderMaterial` need TSL equivalents (Pocket Island's port is the worked example).
- **One scene, both methods.** Whether `@threert/scene` scenes should also render through Raster (its
  three.js fallback already exists for browsers) or Raster stays three.js-only.

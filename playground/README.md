# ThreeRT Playground

Native macOS editor client for ThreeRT scenes and games, styled after Unreal Editor. Everything runs locally: no
browser, no streaming.

```bash
bun playground/build.ts                     # PocketJS UI bundle + native host + dist/ThreeRT Playground.app
bun playground/run.ts                       # or: open "playground/dist/ThreeRT Playground.app"
bun playground/run.ts --scene sun-room      # open a specific catalog entry
bun playground/run.ts --uitest              # UI end-to-end test (real pointer input), then quits
bun playground/run.ts --tour                # opens every scene, edits / plays / A-Bs it, screenshots to out/playground/tour
```

Scenes and games are bundled from their sources when opened (`tools/bundle-game.ts`; `node tools/build-scenes.ts` builds
them all for the content browser) and run in the host's embedded JS runtime, as on a render node. A/B needs the UE5
references in `out/ue/{lumen,pathtracer}`.
`build.ts` clones PocketJS into `vendor/pocketjs` (or links `POCKETJS_DIR`) and builds its UI with bun.

## What is where

| | |
|---|---|
| `icon/` | app icon: the Pocket3D mark on the macOS icon grid |
| `host/` | `threert-playground` (Rust): winit window, one wgpu 30 device, input routing, frame pacing (60 fps content on 120 Hz panels) |
| `host/src/ui.rs` | PocketJS `UiProgram` + `pocket-web-render` recorded into the frame's encoder; compositor surfaces |
| `host/src/viewport.rs` | viewport compositor: letterbox, UE5 reference / split A/B, selection outline (silhouette mask) |
| `pocket-web-render30/` | PocketJS's wgpu renderer (`devices/web/pocket-web-render`) ported to wgpu 30 |
| `ui/` | Solid TSX UI: toolbar, viewport overlays, outliner, details, content browser, output log, acceptance, game |
| `../tools/playground-backend.ts` | Node backend: catalog, editor model (selection, details, undo / redo of Details edits) and the Play flow, on the live scene the host runs (`GameSession`: outline / inspect / edit / events) |
| `../renderer/` | `threert-render` as a library: LiveCore (HW-RT path tracing, temporal, denoise, UE post), CPU picking |

## Controls

Viewport: LMB drag orbit · RMB drag look (+ WASD / QE fly, Shift faster) · MMB or Shift-drag pan · wheel or trackpad
pinch dolly · click select · F focus selection · Home scene camera · G outline / game view · F11 fullscreen.
Editor: Alt+P Play / Stop · Esc stop / deselect · Cmd+Z / Cmd+Shift+Z undo / redo · Alt+1..6 view modes.
A/B switch (editor and Play; the Output Log shows the state): N motion blur on / off (whatever the scene's post
settings say).
Details fields: drag horizontally across a field to scrub its value (intensities scrub multiplicatively).

Compare (toolbar): ThreeRT Live · UE5 Lumen · UE5 Path Tracer — the UE5 modes split the viewport (drag to move the split).
Split views snap the camera to the scene camera, the pose the UE5 references were rendered from, so the A/B is pixel
aligned. Acceptance (bottom dock): measured ΔE / SSIM vs UE5, Accept / Reject per scene (`out/playground/acceptance.json`).

---
name: threert-scene-authoring
description: Author high-fidelity ThreeRT scenes and games (Three-like DSL in packages/scene, gameplay in packages/gameplay) that match photo references or an existing Unreal level — street/city/night/interior environments, PBR materials and WGSL material functions, wet surfaces, signage, lighting, fog, props, post grade — and verify them with the native renderer, the live viewport and UE5 ground truth. Use when building or upgrading a scene's visual quality, porting a level or look from Unreal, or reviewing why a scene looks flat/plain.
---

# ThreeRT scene authoring

Detail comes from **systems, not hand-placed meshes**: a material system that weathers every surface the same way,
procedural detail where geometry would be too expensive (windows, puddles, grime, LED pixels), many small coloured
lights, atmosphere, and clutter at human scale. The UE 5.8 Kabukichō session (docs/PARITY.md) got its richness this
way; every one of those techniques exists in our DSL and renders natively in real time.

## Workflow (follow in order; do not skip the review loop)

1. **References first.** Collect 3–6 photos of the target (keep CC-BY-SA / unlicensed ones out of git). Write down,
   per photo: palette (sign colours, light temperatures), materials (ground, facades, shutters), light sources and
   their colour spill, atmosphere (haze, wetness), clutter at eye level, and the camera (height, focal length).
2. **Blockout + camera.** Lay out the space at real scale (metres; street widths, 3.0–3.5 m floors, 2.2 m doors).
   Fix 2–4 hero viewpoints at eye height (1.6–1.7 m, 35–55° vertical fov) before adding detail.
3. **Material system.** Start from the standard material library (`SurfaceMaterial` for weathering on anything:
   world mapping, wet, puddles, grime, lit windows; `WetGroundMaterial`, `FacadeMaterial` with lit windows,
   `TriplanarMaterial`, `PaintMaterial`, `NeonSignMaterial`, `LedScreenMaterial`, `LightPanelMaterial`,
   `ShopWindowMaterial`, `RoadMarkingMaterial`): scanned PBR maps plus parameters, world-mapped where it matters,
   tints to vary one texture across buildings ([docs/MATERIAL_FUNCTIONS.md § Standard library](../../../docs/MATERIAL_FUNCTIONS.md)).
   See [materials](references/materials.md). A look the library doesn't cover is a `MaterialFunction` of its own,
   a WGSL body with named params and texture slots, like a UE material graph or Custom node; the library's functions
   are worked examples. `MeshStandardMaterial` itself is plain three.js: weathering, windows and animated emission
   are library materials, not material fields.
4. **Lighting.** Emissive surfaces are the main light at night (signs 300–700 nits, light boxes, shopfronts), plus a
   coloured point/spot light per shop/sign cluster for the spill pools (dozens–hundreds are fine: many-light sampling).
   Manual exposure. See [lighting & atmosphere](references/lighting-atmosphere.md).
5. **Atmosphere.** Height fog with volumetric scattering (halos, depth), a sky that reads behind silhouettes
   (`SkyGradient` with light-pollution glow and clouds at night), rain (`scene.wetness`, `scene.rain`).
6. **Life.** Emissive animation on a few signs (flicker / blink / marquee pan / LED screens), people, vehicles. Anything
   UE would do in an actor's Tick (neon groups flickering as they switch, rain / wetness ramps, fog moods, pulsing
   lights) goes in the game's `frame(f)` script. It runs on the render node every frame; handlers only commit the
   targets. See [frame scripts](references/frame-scripts.md).
7. **Clutter pass.** Poly Haven CC0 props at street level: AC units on facades, crates, bags, bins, chairs, barriers,
   hydrants, manholes; cables and poles overhead. See [assets](references/assets.md).
8. **Post grade.** Bloom threshold/size, slight fringe and grain, white balance, saturation/contrast via
   `render.overrides.post` (UE-equivalent fields).
9. **Review loop** — render every hero view, look at them next to the references, list the 3 biggest differences,
   fix, repeat (≥ 3 rounds). Check close-ups (window, ground, sign) not just the wide shot.
   See [review & performance](references/review-and-performance.md).
10. **Performance pass.** Live viewport ≥ 60 fps target (bench-live). The viewer and playground hold 60 fps with
    dynamic resolution (render scale 50–100 %, TSR-style upsampling), so measure the cost at `--scale 1` and trade
    features per the cost table before relying on the scale.

**Porting a look from an existing Unreal level?** Use the ground-truth loop in [ue-port](references/ue-port.md):
dump the level, capture it from fixed cameras with MRQ, port the content generator 1:1 (`PyRandom`, material
functions), and converge per view with `compare` / `compare-hdr`. Aim at UE's render of the level, never at a UE
render of your own approximation.

## Non-negotiables

- Physical units: metres, nits (emissive, sky, fog colour × intensity), candela (point/spot), lux (directional).
- No stretched textures: world mapping for architecture/ground, mesh UVs only for authored assets/signs.
- Every large flat surface gets at least two of: texture detail, grime, wetness, windows, decals/props.
- Light colour variety: warm 2700–3500 K interiors, cool 5000–6500 K fluorescents/LED, saturated neon accents.
- Wet night street = dark diffuse + glossy damp ground + mirror puddles that reflect the signs.
- Deterministic content from a seed (gameplay rooms replay setup); no Math.random in setup.
- Keep games self-contained TS files; external assets only via repo paths under `assets/` (fetched, not committed).

## Commands

```bash
node tools/fetch-polyhaven.ts --set kabukicho            # CC0 textures + models -> assets/polyhaven/
node tools/build-scenes.ts <id>                          # scenes/*.ts + games -> out/bundles/<id>.js
renderer/target/release/threert-render render out/bundles/x.js -o /tmp/x.png --spp 128 [--camera '{"position":[..],"target":[..],"fov":55}']
renderer/target/release/threert-render compare ref.png /tmp/x.png   # ΔE / PSNR / SSIM
node tools/bench-live.ts --scenes x --frames 1,16,256 [--scale 0.6]   # real-time cost + convergence
node tools/bundle-game.ts games/x/x.game.ts && renderer/target/release/threert-render play out/bundles/x.js   # game in the node runtime
bun ue5/capture.ts <scratch>.uproject job.json           # UE ground truth of an existing level (MRQ stills)
renderer/target/release/threert-render compare-hdr ue.exr ours.raw <ev> --heat heat.png   # radiometric parity
```

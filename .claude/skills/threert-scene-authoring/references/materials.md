# Materials

Start with the standard material library: Three-like constructors for the materials streets and interiors keep
needing, so a scene fills in parameters instead of writing shading (table and example in
[docs/MATERIAL_FUNCTIONS.md § Standard library](../../../../docs/MATERIAL_FUNCTIONS.md)).

```ts
import { FacadeMaterial, NeonSignMaterial, PaintMaterial, SurfaceMaterial, WetGroundMaterial, facadeWindows } from "@threert/scene";
const facade = new FacadeMaterial({ map, roughnessMap, normalMap, grimeMap, textureScale: 3, floorHeight: 3.3, bayWidth: 2.6, litChance: 0.55, seed: 3 });
const ground = new WetGroundMaterial({ map, roughnessMap, normalMap, puddleMap, textureScale: 2.5, puddles: 1 });
const sign = new NeonSignMaterial({ map: signArt, intensity: 9, flicker: true, glow: [1, 0.3, 0.6] });
const pole = new PaintMaterial({ color: [0.06, 0.06, 0.065], roughness: 0.45, metalness: 0.3, grimeMap });
```

The sections below cover the library materials scenes use most and the plain `MeshStandardMaterial` properties the
scene binding sends to the engine (packages/scene/src/engine.ts `MaterialDesc`), rendered by the native path tracer.
Plain materials pay nothing for unused features (the kernel is specialised per scene and per material function).

## Scanned PBR, world mapped

```ts
const tex = (id: string, map: "diff" | "nor_gl" | "rough") =>
  new Texture(`assets/polyhaven/tex/${id}/${id}_${map}_1k.jpg`, map === "diff" ? "srgb" : "linear");
const m = new SurfaceMaterial({ map: tex(id, "diff"), normalMap: tex(id, "nor_gl"), roughnessMap: tex(id, "rough"),
  roughness: 1, color: tint, worldScale: 2.5 });   // metres per repeat; dominant-axis planar, tangent frame included
```

- Share `Texture` objects (cache by uri); the renderer packs any number/size of textures into mip-mapped pages.
- Vary one set across buildings with `color` tints (0.55–0.85 grey, slightly warm/cool) and different `worldScale`.
- `roughnessMap` is read from G (glTF convention), metalness from B; normal maps are OpenGL (+Y up, `nor_gl`).
- `texture.repeat` / `texture.offset` / `texture.wrap = "clamp"` work on mesh UVs (signs, atlases); without
  `worldScale` a `SurfaceMaterial` uses mesh UVs too.
- `alphaTest` (0..1) cuts holes from map alpha × opacity on a plain `MeshStandardMaterial`: foliage, fences, cables
  on cards, decals.

| Surface | Poly Haven set | scale (m) |
|---|---|---|
| road asphalt | asphalt_02, clean_asphalt | 3–5 |
| Japanese street pavers | concrete_pavers (bone), square_tiles | 1.8–2.5 |
| curbs | concrete_pavers / precast_concrete_wall | 1–1.5 |
| tiled facades | rectangular_facade_tiles(_02), square_tiled_wall, grey_tiles, long_white_tiles | 1.4–2.4 |
| concrete facades | concrete_tile_facade, precast_concrete_wall, painted_concrete, beige_wall_001 | 2.5–3.5 |
| cladding / shutters | exterior_wall_cladding, painted_metal_shutter, rusty_metal_shutter | 1.5–2.6 |

## Weathering

```ts
const street = new SurfaceMaterial({ ...maps, worldScale: 3,
  wet: 1,                                    // 0 sheltered (under awnings) .. 1 exposed; scaled by scene.wetness
  puddles: { scale: 3, amount: 0.5 },        // up-facing faces; amount 0.25 ≈ 10 %, 0.5 ≈ 25 %, 1 ≈ 65 % flooded
  grime: { amount: 0.5, scale: 3 } });       // blotches + vertical streaks on walls (m)
scene.wetness = 1; scene.rain = 0.7;         // global; rain adds animated ripples on wet ground
```

Wet = darker diffuse (×0.5), roughness → ≤ 0.14, flattened normal; puddles = mirror water (roughness 0.02) with
rain ripples. Use `wet: 0.3–0.4` on facades (damp sheen), 0 under awnings/indoors. Darker texels flood first.
`WetGroundMaterial` is the UE port for streets with a painted puddle mask (`puddleMap`), `PaintMaterial` /
`TriplanarMaterial` take `wet` and a grime mask.

## Procedural windows (facades)

```ts
const wall = new SurfaceMaterial({ ...maps, worldScale: 3, wet: 0.35,
  windows: facadeWindows({ floorHeight: 3.2, bayWidth: 2.4, windowWidth: 1.5, windowHeight: 1.6,
    groundHeight: 3.5, sill: 0.9, litChance: 0.45, brightness: 30, top: height - 0.9, seed }) });
```

Applies to the material's vertical faces between `groundHeight` and `top`: frames + mullions/transoms, glass that
reflects the street, lit rooms with fake interiors (back wall + furniture, ceiling light panel, side walls, floor),
blinds, warm / fluorescent / rare pink palettes, whole floors on or off together. Vary every parameter per
building (bay 1.7–3.2 m, lit 0.3–0.65, brightness 15–45 nits). Shopfronts below `groundHeight` are separate meshes.
`FacadeMaterial` is the UE port (windows laid out from the mesh's bounds, room gradient instead of interior mapping).

## Emissive surfaces and animation

- Signs: `NeonSignMaterial({ map, intensity: 300–700, glow })`: the artwork emits; `glow` (its average colour) is what
  emitter sampling lights the street with. A plain `MeshStandardMaterial` with `emissiveMap` = the sign face, `emissive`
  white works too: NEE then samples the emissive map's texels.
- Bake glow/halo into sign textures only for neon tubes on dark backgrounds; light boxes stay flat.
- `NeonSignMaterial({ flicker: true })` on ~8 % of signs (failing tubes), `{ blink: 0.8–2 }` (Hz) on ~4 %,
  `{ scroll: [0.2, 0] }` for marquees/tickers; `LedScreenMaterial({ map, resolution: [cols, rows] })` for screens
  (pixel grid, alternating with a plasma); `LightPanelMaterial({ blink: true, rate })` for warning lights.
- Lit shopfronts: `ShopWindowMaterial({ map: interior, intensity: 60–120 })` (parallax), or an interior image as
  emissive at 60–120 nits.

## Common values

| Material | color (linear) | roughness | metalness |
|---|---|---|---|
| wet asphalt (`wet: 1`) | tex × 0.55 | tex | 0 |
| painted steel / poles | 0.03–0.1 | 0.35–0.5 | 0.6–0.9 |
| glass (shop) | 0.01–0.03 | 0.03–0.08 | 0 |
| fabric awning | 0.05–0.3 saturated | 0.6–0.8 | 0 |
| skin / hair | 0.25 / 0.03 | 0.5 / 0.6 | 0 |

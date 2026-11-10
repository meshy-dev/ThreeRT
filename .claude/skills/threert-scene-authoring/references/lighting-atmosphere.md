# Lighting, atmosphere, post

## Night street recipe (Kabukichō-like)

| Element | Setting |
|---|---|
| Exposure | `camera.setManualExposure(6)` (EV100 5.5–7 for neon streets; 13–15 daylight) |
| Signs (light boxes) | emissive 300–700 nits; gate/feature signs 700+ |
| Shopfronts | emissive 60–120 nits + one `PointLight` per shop: 250–600 cd, range 9–12 m, radius 0.2–0.3 m, warm 2700–3500 K or fluorescent |
| Bulbs / lanterns | 150–2000 nits small emitters (castShadow false on dense bulb meshes) |
| Street lamps | emissive globes ~650 nits; optional spot 1500–3000 cd |
| Windows | `windows.brightness` 15–45 nits |
| Sky | `SkyGradient` zenith ≈ 0.1–0.4 nits, horizon ≈ 1–2 nits, `setGlow(orange, 0.3–0.4, 0.12–0.15)`, `setClouds(0.4–0.6, warm grey)` |
| Fog | `ExponentialHeightFog(0.008–0.015, 0.04–0.08, dark blue-grey)` + `setVolumetric(0.6–0.8, 0.5–0.6)`, `maxOpacity 0.9` |
| Rain | `scene.wetness = 1`, `scene.rain = 0.5–0.8` |

Daylight: `DirectionalLight` (lux, ~100 000 sun) with the default `SkyAtmosphere`; fog density ≤ 0.002.

## Rules

- Many lights are cheap: local lights are resampled (RIS, one shadow ray), so add a coloured light wherever a real
  street would have a pool of colour (each shopfront, under awnings, big signs). Keep ranges finite.
- Emissive geometry lights the scene by itself (importance-sampled emitters); a point light per sign is only needed
  for strong, distinct spill (or for UE parity where Lumen needs help).
- Colour temperature variety reads as realism; one global white light reads as CG.
- Directional lights at night: none, or a weak moon (0.1–0.3 lux) — the sky gradient provides ambient.
- Fog colour × intensity is the ambient haze radiance; keep it ≤ 0.1 nits at night or the scene washes out.
  Volumetric scattering produces the halos around signs and lamps — the main depth cue in a long street.

## Post (render.overrides.post — UE PostProcessVolume equivalents)

```ts
options: { overrides: { post: {
  bloom: { intensity: 0.85, threshold: 1.2, sizeScale: 5 },
  vignette: 0.5, chromaticAberration: 0.25, filmGrain: 0.12,
  colorGrading: { whiteTemp: 6200, saturation: 1.12, contrast: 1.08, shadowsGain: 0.95, highlightsGain: 1.04 },
} } }
```

Grading is applied in the tonemap LUT like UE's CombineLUTs (white balance → saturation/contrast → gains → filmic).
Keep fringe ≤ 0.4 and grain ≤ 0.15; push saturation, not exposure, for "neon".

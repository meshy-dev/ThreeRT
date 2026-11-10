# Review loop and performance

## Review loop (every round)

1. Build the bundle, render each hero view offline (`--spp 128`, OIDN) and grab a live snapshot
   (`threert-render live out/bundles/x.js` + `{"cmd":"snapshot","path":...,"after":64}` on stdin, or the playground).
2. Put renders next to the reference photos; also render close-ups (`render --camera
   '{"position":[..],"target":[..],"fov":..}'`) of a facade, the ground, a sign cluster.
3. Write the three largest differences, ranked by how much of the frame they cover; fix those first. Typical order:
   ground (wetness / reflections) → facades (texture + windows + grime) → light colour spill → atmosphere → clutter.
4. Stop only when another round would not change the top-3 list.

Failure patterns and the fix:

| Looks like | Cause | Fix |
|---|---|---|
| plastic / CG walls | single colour, mesh UVs | scanned PBR, world mapping, tint variation, grime |
| black "decal" puddles | puddles as separate meshes | `material.puddles` on the ground + `scene.wetness` |
| concentric rings everywhere | rain too strong for the view | `scene.rain` 0.3–0.7; ripples are subtle by design |
| uniform grid of pale windows | high lit chance / brightness | litChance 0.3–0.6, brightness 15–45, vary per building |
| flat dark street | only emissive signs | point light per shopfront (warm), fog volumetric |
| no depth down the street | no atmosphere | height fog + volumetric (halos), sky glow behind silhouettes |
| empty pavement | no clutter | Poly Haven props at shopfronts, people, umbrellas when raining |
| diamond tread "metal" ground | wrong texture set | check the texture thumbnail before using it |

## Performance (Apple M5, 1280x720 live viewport, 1 spp/frame + temporal + denoise)

Kabukichō v2 (1.16 M triangles, 97 textures, 23 lights, 22 k emissive triangles): 23.8 ms/frame (~42 fps).
Costs measured by masking kernel features (`THREERT_FEATURES`, gpu::F_* bits):

| Feature | ms | Notes |
|---|---:|---|
| base (geometry, GI, emitters, RIS lights) | 15.6 | sky NEE skipped automatically at night |
| volumetric fog | 3.9 | half the pixel tiles per frame + firefly clamp |
| wetness / puddles / ripples | 2.6 | puddles + ripples on camera / mirror paths only |
| emissive animation (flicker, blink, LED) | 2.6 | |
| textures (mip pages, ray-cone LOD) | 2.5 | 1k maps; skip AO maps |
| procedural windows | 1.3 | interiors only on camera / mirror paths |
| grime | 0.6 | |

Kabukichō v3 (the UE session's level ported 1:1: 4.0 M triangles, 147 lights, 78 k emissive triangles, nine material
functions): 34 ms/frame at 100 %, **15.8–16.7 ms at 60 % render scale** (TSR-style upsampling, closer to UE's TSR
capture than 100 %: SSIM 0.689 vs 0.649). Material functions cost ~20 % of its frame. Use cheaper bodies at
`i.depth > 0` where the detail is invisible. The à-trous denoiser is 2.6 ms at 1280×720, bloom 0.3 ms.

Scenes that do not use a feature pay nothing for it (the kernel is compiled per feature mask). Levers, cheapest
first: render scale (`--scale 0.6`; the viewer's dynamic resolution does this automatically), fewer volumetric scenes (or `setVolumetric` off), puddles on fewer materials, 1k textures, fewer props
(triangles are flattened per instance), lights: many are fine, but keep `distance` finite. Offline renders are
unaffected (use 128–256 spp + OIDN for stills).

```bash
node tools/bench-live.ts --scenes x --frames 1,16,256     # GPU ms + convergence vs UE refs
THREERT_FEATURES=383 node tools/bench-live.ts --scenes x   # e.g. without volumetric fog (511 - 128)
```

## Motion and lighting-change quality (live viewport)

Static snapshots hide the two artefacts players notice: smearing while the camera moves and noise after lights
switch. `node tools/bench-temporal.ts --tag x [--scale 1|auto]` measures both against converged references. It runs a
dolly + turn, a stop, and a closing-time event with flicker on Kabukichō's bundle. Compare a change at `--scale 1`
first, so resolution does not confound it, then at `auto`. Look at the crops (`out/bench-temporal/<tag>/*.jpg`),
not only ΔE: ΔE prefers blur to noise. `THREERT_PROFILE=1 threert-render live . --game <bundle>` prints GPU ms per pass.

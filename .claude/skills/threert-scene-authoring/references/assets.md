# Assets

## Poly Haven (CC0)

```bash
node tools/fetch-polyhaven.ts --set kabukicho                 # curated street set (18 texture sets, 19 models)
node tools/fetch-polyhaven.ts --textures a,b --models c,d --res 1k
```

- Textures → `assets/polyhaven/tex/<id>/<id>_{diff,nor_gl,rough,ao}_1k.jpg`; models → `assets/polyhaven/models/<id>_1k.glb`
  (glTF + bin + images packed into one GLB). `assets/` is gitignored; `assets/polyhaven/manifest.json` records URLs.
- Browse ids at https://polyhaven.com (API: `https://api.polyhaven.com/assets?t=textures|models`, `/files/<id>`).
- Use 1k for everything except hero ground (2k): texture memory adds up (a 4096² page holds sixteen 1k maps).
- Reference them by repo-relative path: the renderer reads them from its asset root (the repo, `--assets`); render
  nodes fetch the ones they do not have yet. Paths must stay inside the asset root.

```ts
const ac = new GLTFModel("assets/polyhaven/models/exterior_aircon_unit_1k.glb");
ac.position.set(x, y, z); ac.rotation.y = Math.PI / 2;   // Poly Haven models are metres, Y-up, front +Z
```

Street clutter set (all metres, origin at the base): exterior_aircon_unit (0.8 × 0.9 m, bolt to facades every few
floors), trashbag, cardboard_box_01, plastic_crate_01/02, propane_tank, metal_stool_01, bar_chair_round_01,
plastic_monobloc_chair_01, utility_box_01/02, metal_trash_can, WetFloorSign_01, fire_hydrant, water_manhole_cover,
concrete_road_barrier, street_lamp_01, security_camera_01, mounted_fluorescent_lights. Each instance is flattened into
the scene's triangles (no instancing): budget ~1–2 M triangles for 60 fps live.

## Generated textures

- Sign faces / typography: render with headless Chrome (DOM text, system Japanese fonts) like
  `tools/kabukicho-atlas.ts`; one atlas or many separate PNGs both work (no size limits).
- `DataTexture` / `checkerTexture` for code-generated maps; generated textures are embedded as content-addressed PNGs.

## Licences

Only commit generated or CC0 content. Photo references under CC BY-SA or unknown licence: keep in `out/` (ignored).

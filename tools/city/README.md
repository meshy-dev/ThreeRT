# tools/city: the open city's compiler (from jeantimex/tokyo)

The offline pipeline that turns public data about a Japanese city into 256 m binary tiles, and the mesher that turns
tiles into geometry. Imported from [jeantimex/tokyo](https://github.com/jeantimex/tokyo), commit `1984760`
(MIT, © 2026 Yong Su: [`LICENSE`](LICENSE), kept as it was). The code is the upstream's, unchanged except:

- the import paths, for the new layout:
  - `pipeline/` was `tools/pipeline/`;
  - `shared/` was `src/shared/`;
  - `mesh/` was `src/world/` (only the mesher and what it needs: `meshing.js`, `drape.js`, `decals.js`,
    `constants.js`, `tower.js`);
  - `tests/` and `assets/` were `tools/tests/` and `tools/assets/`;
- where data goes: `assets/city/` (`ROOT` in `pipeline/config.mjs`) instead of `data/` and `public/`. `assets/` is not in
  git and holds the downloaded assets of every worktree on the machine, so an area is fetched and compiled once.

ThreeRT's own code that reads the tiles lives elsewhere (docs/ROADMAP.md § 0); this directory tracks the upstream.

## Use

```
node tools/city/pipeline/fetch.mjs --area=shinjuku                                  # raw data -> assets/city/raw/shinjuku, aerial photos -> assets/city/ortho/shinjuku
node --max-old-space-size=4096 tools/city/pipeline/compile.mjs --area=shinjuku     # -> assets/city/tiles/shinjuku
node tools/city/assets/fetch_textures.mjs                                           # Poly Haven CC0 textures -> assets/city/textures
node tools/city/tests/tileformat.test.mjs                                           # the tile format's round trip
```

Areas (`pipeline/config.mjs`): tokyo, shiba, shibuya, shinjuku, chiyoda, chuo, fujinomiya. Each is about 3.4 × 2.8 km.

Network access, all from `pipeline/fetch.mjs` and `pipeline/terrain.mjs`:
- the PLATEAU data catalogue (`api.plateau.reearth.io`) and the CityGML files it lists;
- Overpass (`overpass-api.de`, `overpass.kumi.systems`, `overpass.private.coffee`);
- GSI tiles (`cyberjapandata.gsi.go.jp`).

## Data, attribution and licences

The licence above covers the code only. The data keeps its sources' terms. Credit them wherever the city is shown:

- 3D city model, roof photos, bridges and street furniture: Project PLATEAU, MLIT Japan (CC BY 4.0, multi-licensed
  with ODbL).
- Elevation and aerial photos: Geospatial Information Authority of Japan (GSI).
- Road network, railways and places: © OpenStreetMap contributors (ODbL).
- Textures: Poly Haven (CC0).
- Upstream's client also uses ez-tree (MIT) and takram three-geospatial (MIT).

A compiled area is a derived database under the ODbL. Do not publish compiled tiles outside the team without its
terms (share-alike, attribution).

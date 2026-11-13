# Assets and third-party content

The assets are not committed: Poly Haven, Kabukichō's generated textures, Khronos and Quaternius models, the ORCA scenes and the compiled Tokyo tiles. That is ~1.8 GB, plus the 2 GB of ORCA archives.
- They live once per machine in `<main clone>/../threert-assets`.
- Every checkout's `assets/` is a relative link to it, made by `bun scripts/setup.ts`. Run it in each new clone or worktree; it never deletes anything.
- Scripts run with Bun.
- The ORCA scenes (Bistro, Zero Day) are FBX + DDS, converted to glTF by `scripts/orca.ts` (Blender headless; skipped without Blender, `brew install --cask blender`).

Third-party scenes are under Creative Commons Attribution 4.0 (https://creativecommons.org/licenses/by/4.0/).
- They were converted from FBX + DDS to glTF with PNG / JPEG textures by `scripts/orca.ts` and are otherwise unchanged.
- Each archive's LICENSE.txt and README.txt are kept next to the glTF in `assets/orca/<scene>/`.

The scenes:
- **Amazon Lumberyard Bistro**: Amazon Lumberyard (2017), Open Research Content Archive (ORCA), http://developer.nvidia.com/orca/amazon-lumberyard-bistro
- **Zero-Day**: Mike Winkelmann (Beeple) and Kate Anderson (2019), Open Research Content Archive (ORCA), https://developer.nvidia.com/orca/beeple-zero-day

Tokyo:
- The compiler in `tools/city` is imported from [jeantimex/tokyo](https://github.com/jeantimex/tokyo) (MIT, © 2026 Yong Su; [`tools/city/LICENSE`](../tools/city/LICENSE)).
- The data: the 3D city model is Project PLATEAU (MLIT Japan); elevation and aerial photos are GSI; roads and places are © OpenStreetMap contributors (ODbL).
- Compiled tiles are a derived database under the ODbL and are not to be published.

Other content:
- **Damaged Helmet** and the other glTF sample models: Khronos glTF sample models, under each model's own licence.
- **Poly Haven** textures, HDRIs and props: CC0.
- **Quaternius** models: CC0.

The engine's own code is under the MIT licence ([`LICENSE`](../LICENSE)); `renderer/third_party/` holds patched
dependencies under their own licences (each has a PATCH.md).

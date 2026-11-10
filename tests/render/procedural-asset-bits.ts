// An asset file's data words reach the geometry function exactly (the renderer decodes the little-endian file):
// words 2..7 of tests/render/data/proc-two-boxes.tpg are procedural/data-bits' NaN, infinity, -0 and denormal patterns.
// One box of the file emits 2 nits only when they all read back.
import { GeometryFunction, MeshStandardMaterial, ProceduralGeometry, ProceduralMesh } from "@threert/scene";
import { BOX_IF } from "./procedural-data-bounds.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/asset-bits", features: ["procedural", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.01 },
  requires: ["tests/render/data/proc-two-boxes.tpg"],
};

const WORDS = [0xffffffff, 0x7fc00001, 0x7f800001, 0xff800000, 0x80000000, 0x00000001];
const BITS = new GeometryFunction("proc_asset_bits", {
  code: BOX_IF(`i.count == 8u && ` + WORDS.map((w, k) => `gdata(i.data + ${k + 2}u) == 0x${w.toString(16)}u`).join(" && ")),
});

export default function () {
  const scene = sceneWithSky(0);
  const mats = [new MeshStandardMaterial({ name: "black", color: 0x000000 }), new MeshStandardMaterial({ name: "ok", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 })];
  const box = new ProceduralMesh(ProceduralGeometry.fromAsset(BITS, "tests/render/data/proc-two-boxes.tpg"), mats);
  box.position.x = 1;
  box.scale.set(1.2, 1.2, 1);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [box]);
}

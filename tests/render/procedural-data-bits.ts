// The game's data keeps its exact bits on the way to the geometry function (it rides through an f32 binding): NaN and
// infinity patterns, negative zero and a denormal read back as written through `gdata`, and through `gdataf` and back.
// As procedural/data-bounds, the box emits 2 nits only when every word matches.
import { GeometryFunction, MeshStandardMaterial, ProceduralGeometry, ProceduralMesh } from "@threert/scene";
import { BOX_IF } from "./procedural-data-bounds.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/data-bits", features: ["procedural", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.01 },
  note: "words 0xFFFFFFFF, 0x7FC00001, 0x7F800001, 0xFF800000, 0x80000000, 0x00000001 read back exactly",
};

const WORDS = [0xffffffff, 0x7fc00001, 0x7f800001, 0xff800000, 0x80000000, 0x00000001];
const hex = (w: number) => `0x${w.toString(16)}u`;
const BITS = new GeometryFunction("proc_data_bits", {
  code: BOX_IF(WORDS.map((w, k) => `gdata(i.data + ${k}u) == ${hex(w)} && bitcast<u32>(gdataf(i.data + ${k}u)) == ${hex(w)}`).join("\n        && ")),
});

export default function () {
  const scene = sceneWithSky(0);
  const mats = [new MeshStandardMaterial({ name: "black", color: 0x000000 }), new MeshStandardMaterial({ name: "ok", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 })];
  const box = new ProceduralMesh(new ProceduralGeometry(BITS, [-1, -1, -0.1, 1, 1, 0], WORDS), mats);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [box]);
}

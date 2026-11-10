// Procedural geometry from an asset file (`ProceduralGeometry.fromAsset`; tests/render/data/proc-two-boxes.tpg, written
// by tools/lib/proc-asset.ts): the boxes and material indices of procedural/materials, read by the renderer from the
// file instead of passed by the scene code. The right box (index 7, the last material: 5 nits) over the left (index 1,
// 2 nits) reads 2.5.
import { ANALYTIC_BOX, MeshStandardMaterial, ProceduralGeometry, ProceduralMesh } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/asset", features: ["procedural", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 2.5, tolerance: 0.02 },
  requires: ["tests/render/data/proc-two-boxes.tpg"],
  note: "procedural/materials with the boxes and data read from an asset file",
};

const emit = (nits: number, name: string) => new MeshStandardMaterial({ name, color: 0x000000, emissive: 0xffffff, emissiveIntensity: nits });

export default function () {
  const scene = sceneWithSky(0);
  const boxes = new ProceduralMesh(ProceduralGeometry.fromAsset(ANALYTIC_BOX, "tests/render/data/proc-two-boxes.tpg"), [emit(0, "none"), emit(2, "two"), emit(5, "five")]);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [boxes]);
}

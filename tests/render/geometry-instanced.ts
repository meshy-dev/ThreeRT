// InstancedMesh (ROADMAP § 6 step 2e): two copies of one plane, one shared mesh, each its own matrix and colour
// (`setColorAt` multiplies the material's base colour). White Lambert material, colours 0.2 and 0.5, under the same
// sky and seen straight on: right / left = 2.5.
import { Color, InstancedMesh, Matrix4, PlaneGeometry } from "@threert/scene";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/instanced", features: ["instancing", "instanced-mesh"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 2.5, tolerance: 0.02 },
  note: "one InstancedMesh, left copy colour 0.2, right 0.5, white Lambert material: right / left = 2.5",
};

export default function () {
  const scene = sceneWithSky(1);
  const copies = new InstancedMesh(new PlaneGeometry(1.8, 1.8), lambert(1, "white"), 2);
  copies.name = "copies";
  const m = new Matrix4();
  copies.setMatrixAt(0, m.makeRotationX(-Math.PI / 2).setPosition(-1, 0, 0));
  copies.setMatrixAt(1, m.makeRotationX(-Math.PI / 2).setPosition(1, 0, 0));
  copies.setColorAt(0, new Color().setRGB(0.2, 0.2, 0.2));
  copies.setColorAt(1, new Color().setRGB(0.5, 0.5, 0.5));
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [copies]);
}

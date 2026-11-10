// Hard shadow: under a black sky and an overhead sun, the ground under a black occluder gets no light (seen from the side,
// below the occluder).
import { BoxGeometry, DirectionalLight, Mesh } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "shadow/directional", features: ["directional-light", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.47, 0.47, 0.53, 0.53], b: [0.02, 0.85, 0.12, 0.95], expected: 0, tolerance: 0.005 },
};

export default function () {
  const scene = sceneWithSky(0);
  const sun = new DirectionalLight(0xffffff, 10000);
  sun.position.set(0, 1, 0);
  const box = new Mesh(new BoxGeometry(2, 0.2, 2), lambert(0, "occluder")); // black: no bounce light into the shadow
  box.position.set(0, 1.5, 0);
  return result(scene, camera(scene, [4, 2.2, 0.001], [0, 0, 0], 30), [sun, box, plane(40, lambert(0.5))]);
}

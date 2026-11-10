// Spot light cone: outside a hard 20-degree cone nothing is lit.
import { SpotLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/spot-cone", features: ["spot-light"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.05, 0.6, 0.15, 0.7], b: [0.47, 0.47, 0.53, 0.53], expected: 0, tolerance: 0.005 },
};

export default function () {
  const scene = sceneWithSky(0);
  const s = new SpotLight(0xffffff, 2000, 0, (20 * Math.PI) / 180, 0);
  s.radius = 0.001;
  s.position.set(0, 3, 0);
  s.target!.position.set(0, 0, 0);
  return result(scene, camera(scene, [0, 4, 0.001], [0, 0, 0], 60), [s, s.target!, plane(20, lambert(0.5))]);
}

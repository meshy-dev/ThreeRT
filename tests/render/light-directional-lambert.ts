// Directional light: E = 10000 lux at 60 degrees elevation on an albedo-0.5 Lambert plane: L = 0.5 E sin60 / pi.
import { DirectionalLight } from "@threert/scene";
import { CENTER, camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/directional-lambert", features: ["directional-light", "lambert", "light-units"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: (0.5 * 10000 * Math.sin(Math.PI / 3)) / Math.PI, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const sun = new DirectionalLight(0xffffff, 10000);
  sun.position.set(0, Math.sin(Math.PI / 3), Math.cos(Math.PI / 3));
  return result(scene, camera(scene, [2, 2, 0], [0, 0, 0]), [sun, plane(20, lambert(0.5))]);
}

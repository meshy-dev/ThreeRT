// Point light: I = 1000 cd, 2 m above an albedo-0.5 Lambert plane: L = 0.5 I / 2^2 / pi directly below it.
import { PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/point-inverse-square", features: ["point-light", "lambert", "light-units"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: (0.5 * 1000) / 4 / Math.PI, tolerance: 0.02 },
  note: "a 4 % patch around the nadir: cos / distance^2 stay within 0.5 % of the centre value",
};

export default function () {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 2, 0);
  return result(scene, camera(scene, [0, 1.5, 3], [0, 0, 0]), [l, plane(20, lambert(0.5))]);
}

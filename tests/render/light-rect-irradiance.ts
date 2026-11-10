// Rect light illumination: a 0.2 m square of 10000 nits facing down 2 m above an albedo-0.5 Lambert plane. On axis, the
// irradiance of a uniform rectangle (view factor of a parallel rectangle, half-sides X = Y = s / 2h in units of h) is
// E = 2 L [X / sqrt(1 + X^2) atan(Y / sqrt(1 + X^2)) + Y / sqrt(1 + Y^2) atan(X / sqrt(1 + Y^2))]; L_out = 0.5 E / pi.
import { RectAreaLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const s = 0.2, h = 2, L = 10000;
const X = s / 2 / h;
const E = 4 * L * (X / Math.sqrt(1 + X * X)) * Math.atan(X / Math.sqrt(1 + X * X));

export const test: TestMeta = {
  id: "light/rect-irradiance", features: ["rect-light", "light-units", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: (0.5 * E) / Math.PI, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const r = new RectAreaLight(0xffffff, L, s, s);
  r.position.set(0, h, 0);
  r.lookAt(0, 0, 0);
  r.card = false;
  return result(scene, camera(scene, [0, 1.5, 3], [0, 0, 0]), [r, plane(20, lambert(0.5))]);
}

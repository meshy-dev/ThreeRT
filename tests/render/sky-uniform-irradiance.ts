// Sky light: an unoccluded albedo-0.5 Lambert plane under a uniform 2-nit sky shows 0.5 x 2 = 1 nit.
import { CENTER, camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "sky/uniform-irradiance", features: ["sky-light", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.02 },
  note: "the plane is 2 km wide, so the lower hemisphere it hides is 0 for both the sky and the plane's own view",
};

export default function () {
  const scene = sceneWithSky(2);
  return result(scene, camera(scene, [0, 2, 2], [0, 0, 0]), [plane(2000, lambert(0.5))]);
}

// White furnace, diffuse: an albedo-1 Lambert sphere in a uniform 1-nit sky reflects exactly 1 nit everywhere.
import { Mesh, SphereGeometry } from "@threert/scene";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "furnace/diffuse", features: ["lambert", "sky-light", "energy-conservation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(1);
  const s = new Mesh(new SphereGeometry(0.5, 64, 48), lambert(1));
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [s]);
}

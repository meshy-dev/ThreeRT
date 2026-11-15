// A shadow ray through a medium that is not homogeneous to the renderer (a volume function): a point light (1000 cd)
// 3 m above an albedo-0.5 Lambert plane, a 1 m thick absorber (extinction 1 / m) between them. The plane under it
// reads 0.5 I / 9 / pi x e^-1. The camera looks at the patch from under the box.
import { PointLight, VolumeMesh } from "@threert/scene";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const CLEAR = (0.5 * 1000) / 9 / Math.PI;

export const test: TestMeta = {
  id: "shadow/medium", features: ["point-light", "volume", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: CLEAR * Math.exp(-1), tolerance: 0.02 },
  spp: 256,
  note: "box 2 x 1 x 2 m, y from 1 to 2, under a light at y = 3; the camera at y = 0.6",
};

export default function () {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 3, 0);
  const box = new VolumeMesh({ volume: UNIFORM.with({ Density: 1 }), majorant: 1 });
  box.scale.set(2, 1, 2);
  box.position.set(0, 1.5, 0);
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0]), [l, box, plane(20, lambert(0.5))]);
}

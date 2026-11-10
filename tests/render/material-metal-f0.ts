// Smooth metal at normal incidence reflects its base colour (F0) of a uniform 1-nit sky.
import { Color, MeshStandardMaterial } from "@threert/scene";
import { CENTER, camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/metal-f0", features: ["metal", "ggx", "fresnel"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: [0.9, 0.5, 0.2], tolerance: 0.03 },
};

export default function () {
  const scene = sceneWithSky(1);
  const p = plane(50, new MeshStandardMaterial({ color: new Color().setRGB(0.9, 0.5, 0.2), metalness: 1, roughness: 0 }));
  return result(scene, camera(scene, [0, 3, 0.001], [0, 0, 0], 20), [p]);
}

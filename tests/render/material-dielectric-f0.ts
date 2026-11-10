// Smooth black dielectric at normal incidence reflects F0 = 0.04 of a uniform 1-nit sky (specular 0.5 = UE default).
import { MeshStandardMaterial } from "@threert/scene";
import { CENTER, camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/dielectric-f0", features: ["dielectric", "fresnel"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.04, tolerance: 0.05 },
};

export default function () {
  const scene = sceneWithSky(1);
  const p = plane(50, new MeshStandardMaterial({ color: 0x000000, metalness: 0, roughness: 0 }));
  return result(scene, camera(scene, [0, 3, 0.001], [0, 0, 0], 20), [p]);
}

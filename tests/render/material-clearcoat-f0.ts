// Clearcoat (UE ClearCoat / car paint): a smooth coat (F0 0.04) over a base that reflects nothing (black, no specular)
// reflects 0.04 of a uniform 1-nit sky at normal incidence, like a bare dielectric.
import { MeshPhysicalMaterial } from "@threert/scene";
import { CENTER, camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/clearcoat-f0", features: ["clearcoat", "fresnel"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.04, tolerance: 0.05 },
};

export default function () {
  const scene = sceneWithSky(1);
  const p = plane(50, new MeshPhysicalMaterial({ color: 0x000000, metalness: 0, roughness: 1, specularIntensity: 0, clearcoat: 1, clearcoatRoughness: 0 }));
  return result(scene, camera(scene, [0, 3, 0.001], [0, 0, 0], 20), [p]);
}

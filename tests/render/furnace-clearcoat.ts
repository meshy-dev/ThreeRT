// White furnace, clearcoat over Lambert: the base gets what the coat does not reflect, so an albedo-1 Lambert sphere
// under a rough coat still reflects exactly 1 nit of a uniform 1-nit sky.
import { Mesh, MeshPhysicalMaterial, SphereGeometry } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "furnace/clearcoat", features: ["clearcoat", "lambert", "sky-light", "energy-conservation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(1);
  const m = new MeshPhysicalMaterial({ color: 0xffffff, metalness: 0, roughness: 1, specularIntensity: 0, clearcoat: 1, clearcoatRoughness: 0.3 });
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [new Mesh(new SphereGeometry(0.5, 64, 48), m)]);
}

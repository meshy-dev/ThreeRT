// Glass: a clear slab (ior 1.5, two interfaces at normal incidence) in front of a 100-nit wall transmits
// (1 - 0.04)^2 / (1 - 0.04^2) = 92.3 % of it.
import { BoxGeometry, Color, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const T = ((1 - 0.04) * (1 - 0.04)) / (1 - 0.04 * 0.04);

export const test: TestMeta = {
  id: "material/glass-transmission", features: ["transmission", "refraction"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 100 * T, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new PlaneGeometry(10, 10), new MeshStandardMaterial({ color: 0x000000, emissive: new Color().setRGB(1, 1, 1), emissiveIntensity: 100 }));
  wall.position.set(0, 0, -2);
  const glass = new Mesh(new BoxGeometry(1, 1, 0.05), new MeshPhysicalMaterial({ color: new Color().setRGB(1, 1, 1), transmission: 1, ior: 1.5, roughness: 0, metalness: 0 }));
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0], 10), [wall, glass]);
}

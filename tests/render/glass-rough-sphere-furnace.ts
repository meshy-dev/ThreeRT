// White furnace, frosted glass: a closed sphere (ior 1.5, roughness 0.5) in a uniform sky of radiance 1, with energy
// compensation. Each interface returns all it receives, entering and leaving, and the light samples blocked by the
// sphere's far wall are the refracted paths' to find: 1. (Single scattering alone loses a quarter at each leaving.)
import { Color, Mesh, MeshPhysicalMaterial, SphereGeometry } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/rough-sphere-furnace", features: ["transmission", "refraction", "ggx", "energy-conservation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.02 },
  spp: 256,
};

export default function () {
  const scene = sceneWithSky(1);
  const s = new Mesh(new SphereGeometry(0.5, 64, 48), new MeshPhysicalMaterial({ color: new Color().setRGB(1, 1, 1), transmission: 1, ior: 1.5, roughness: 0.5, metalness: 0 }));
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [s], { maxBounces: 64, energyCompensation: 1 });
}

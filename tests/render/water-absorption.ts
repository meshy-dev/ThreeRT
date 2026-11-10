// Through 1 m of water: a 2-nit emissive floor at the bottom of a calm WaterBody (extinction 0.5 / m, pure absorber),
// seen straight down through its surface (IOR 1.33). Beer-Lambert times the surface's transmission at normal
// incidence: 2 e^-0.5 (1 - 0.0201) = 1.1887.
import { BoxGeometry, Color, Mesh, MeshStandardMaterial, WaterBody } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const F0 = ((1.33 - 1) / 2.33) ** 2;

export const test: TestMeta = {
  id: "water/absorption", features: ["transmission", "refraction", "volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-0.5) * (1 - F0), tolerance: 0.02 },
  spp: 256,
};

export default function () {
  const scene = sceneWithSky(0);
  const water = new WaterBody({ width: 20, length: 20, depth: 1, extinction: 0.5, albedo: [0, 0, 0], roughness: 0, waves: { steepness: 0 } });
  const floor = new Mesh(new BoxGeometry(20, 0.1, 20), new MeshStandardMaterial({ color: 0x000000, emissive: new Color().setRGB(1, 1, 1), emissiveIntensity: 2 }));
  floor.position.set(0, -1.05, 0);
  return result(scene, camera(scene, [0, 3, 0], [0, 0, 0.001]), [water, floor]);
}

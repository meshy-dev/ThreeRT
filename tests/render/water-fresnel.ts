// Water's reflectance at normal incidence: a calm WaterBody (IOR 1.33, no waves) under a uniform sky of radiance 1,
// seen straight down; the water absorbs everything that enters it. The middle reads F0 = ((1.33 - 1) / 2.33)^2 = 0.0201
// (within 13 degrees of normal incidence the exact Fresnel term stays under 0.02010). A path picks reflection with about
// that 2 % chance, so the metric spans the middle half of the frame: a small region left 2 % sampling noise.
import { WaterBody } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "water/fresnel", features: ["transmission", "refraction", "volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.25, 0.25, 0.75, 0.75], expected: ((1.33 - 1) / 2.33) ** 2, tolerance: 0.03 },
  spp: 512,
};

export default function () {
  const scene = sceneWithSky(1);
  const water = new WaterBody({ width: 20, length: 20, depth: 2, extinction: 50, albedo: [0, 0, 0], roughness: 0, waves: { steepness: 0 } });
  return result(scene, camera(scene, [0, 3, 0], [0, 0, 0.001]), [water]);
}

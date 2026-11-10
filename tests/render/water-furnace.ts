// White furnace through water: a WaterBody whose water scatters everything it stops (albedo 1, extinction 2 / m, 3 x 3 m,
// 1.5 m deep, surface waves on) in a uniform sky of radiance 1. Nothing absorbs light, so every view of it reads 1: the
// refraction, the total internal reflection under the surface and the in-scattering together lose or add nothing.
import { WaterBody } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "water/furnace", features: ["transmission", "refraction", "volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.4, 0.4, 0.6, 0.6], expected: 1, tolerance: 0.03 },
  spp: 256,
};

export default function () {
  const scene = sceneWithSky(1);
  const water = new WaterBody({ width: 3, length: 3, depth: 1.5, extinction: 2, albedo: [1, 1, 1] });
  return result(scene, camera(scene, [0, 2, 3], [0, -0.3, 0]), [water], { maxBounces: 64 });
}

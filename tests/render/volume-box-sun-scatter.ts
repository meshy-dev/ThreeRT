// In-scattering from a directional light: the isotropic box of volume/box-single-scatter (extinction 0.1/m, albedo 1,
// 4 x 1 x 2 m) lit straight from above by 1000 lux. With one scattering event the centre pixel reads
// L = E / 4pi x exp(-s / 2) x (1 - exp(-2 s)): the light's leg is half the box's height at every point of the ray.
import { DirectionalLight, VolumeMesh } from "@threert/scene";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const s = 0.1, E = 1000;

export const test: TestMeta = {
  id: "volume/box-sun-scatter", features: ["volume", "directional-light"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (E / (4 * Math.PI)) * Math.exp(-s / 2) * (1 - Math.exp(-2 * s)), tolerance: 0.03 },
  spp: 1024,
  note: "single scattering only (maxBounces 0), analytic",
};

export default function () {
  const scene = sceneWithSky(0);
  const box = new VolumeMesh({ volume: UNIFORM.with({ Density: s, Albedo: [1, 1, 1, 0] }), majorant: s });
  box.scale.set(4, 1, 2);
  const sun = new DirectionalLight(0xffffff, E);
  sun.position.set(0, 10, 0);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [box, sun], { maxBounces: 0 });
}

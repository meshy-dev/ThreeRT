// White furnace for media: a non-absorbing sphere of medium (extinction 2 / m, albedo 1, radius 1 m) in a uniform sky
// of radiance 1 reads 1 from every direction (energy conservation): light lost to attenuation must come back as
// scattering, at every order.
import { VolumeMesh } from "@threert/scene";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/furnace", features: ["volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.03 },
  spp: 1024,
  note: "radiance 1 everywhere (energy conservation)",
};

export default function () {
  const scene = sceneWithSky(1);
  const s = new VolumeMesh({ shape: "sphere", volume: UNIFORM.with({ Density: 2, Albedo: [1, 1, 1, 0] }), majorant: 2 });
  s.scale.set(2, 2, 2);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [s], { maxBounces: 64 });
}

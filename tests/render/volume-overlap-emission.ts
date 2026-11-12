// Media that overlap are one medium where they do: an emitter (2 nits a metre, no extinction, 1 m thick) inside an
// absorber (extinction 1/m, 2 m thick) over a black sky. The emitter's light crosses the absorber in front of each of
// its points: 2 (e^-0.5 - e^-1.5) = 0.7668 nits. Taken whole in the order the ray enters them (until 2026-11-13), the
// emitter was drawn behind all of the absorber: 2 e^-2 = 0.2707.
import { VolumeMesh } from "@threert/scene";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/overlap-emission", features: ["volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2 * (Math.exp(-0.5) - Math.exp(-1.5)), tolerance: 0.02 },
  spp: 256,
  note: "absorber 2 x 2 x 2 m and emitter 2 x 2 x 1 m, both at the origin, camera at z = 5, nothing behind",
};

export default function () {
  const scene = sceneWithSky(0);
  const absorber = new VolumeMesh({ volume: UNIFORM.with({ Density: 1 }), majorant: 1 });
  absorber.scale.set(2, 2, 2);
  const emitter = new VolumeMesh({ volume: UNIFORM.with({ Density: 0, Emission: [2, 2, 2, 0] }), majorant: 1 });
  emitter.scale.set(2, 2, 1);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [absorber, emitter]);
}

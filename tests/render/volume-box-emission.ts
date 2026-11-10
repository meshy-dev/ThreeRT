// An emitting absorber: extinction 1/m, albedo 0, emission 1/m x 2 nits, 1 m thick over a black sky reads
// 2 (1 - e^-1) = 1.2642 nits (each metre emits, the medium in front absorbs it).
import { VolumeMesh } from "@threert/scene";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/box-emission", features: ["volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2 * (1 - Math.exp(-1)), tolerance: 0.02 },
  spp: 256,
  note: "box 2 x 2 x 1 m at the origin, camera at z = 5, nothing behind",
};

export default function () {
  const scene = sceneWithSky(0);
  const box = new VolumeMesh({ volume: UNIFORM.with({ Density: 1, Emission: [2, 2, 2, 0] }), majorant: 1 });
  box.scale.set(2, 2, 1);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [box]);
}

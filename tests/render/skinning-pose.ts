// Skinning: a mannequin posed from a clip is not drawn in its rest (T) pose. Posed Idle, the region where the rest
// pose's left hand would be shows the 1-nit sky behind it.
import { GLTFModel } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "animation/skinned-pose", features: ["skinning", "animation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.08, 0.42, 0.16, 0.5], expected: 1, tolerance: 0.02 },
  requires: ["assets/quaternius/ual/UAL1_Standard.glb"],
};

export default function () {
  const scene = sceneWithSky(1);
  const m = new GLTFModel("assets/quaternius/ual/UAL1_Standard.glb", { animation: { clip: "Idle_Loop", time: 0.5 } });
  return result(scene, camera(scene, [0, 1.35, 4], [0, 1.35, 0], 35), [m]);
}

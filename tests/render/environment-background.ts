// three.js scene.background (ROADMAP § 6 step 2f; the kernel's background switch): a grey background (0.5 linear) and
// no light: camera rays that miss see it at the scene's light level, here the floor of 1 / pi, so 0.5 / pi.
import { Color } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "environment/background", features: ["three-compat", "background"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.5 / Math.PI, tolerance: 0.02 },
  note: "scene.background 0.5, no light: radiance 0.5 / pi where nothing is",
};

export default function () {
  const scene = sceneWithSky(0);
  scene.background = new Color().setRGB(0.5, 0.5, 0.5);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]));
}

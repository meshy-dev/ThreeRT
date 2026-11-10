// three.js scene.background lights nothing (the kernel's background switch): a white Lambert floor under a white
// background and a black sky, no lights. The floor stays black: floor / background = 0.
import { Color } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "environment/background-lights-nothing", features: ["three-compat", "background"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.4, 0.8, 0.6, 0.9], b: [0.4, 0.1, 0.6, 0.2], expected: 0, tolerance: 0.02 },
  note: "a white floor under a white background, black sky: the floor stays black",
};

export default function () {
  const scene = sceneWithSky(0);
  scene.background = new Color(1, 1, 1);
  const floor = plane(6, lambert(1, "floor"), "floor");
  return result(scene, camera(scene, [0, 1.2, 5], [0, 1.2, 0], 50), [floor]);
}

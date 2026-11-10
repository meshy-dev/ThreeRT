// Rect light seen head-on: its visible card shows its luminance, 1000 nits.
import { RectAreaLight } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/rect-card-luminance", features: ["rect-light", "light-units"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1000, tolerance: 0.01 },
};

export default function () {
  const scene = sceneWithSky(0);
  const r = new RectAreaLight(0xffffff, 1000, 1, 1);
  r.position.set(0, 0, 0);
  r.lookAt(0, 0, 3);
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [r]);
}

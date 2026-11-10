// Auto exposure at night: a uniform 0.5-nit sky converges to EV100 log2(0.5 / 0.18) = 1.47 (the low end of the range).
import { PerspectiveCamera } from "@threert/scene";
import { result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "exposure/auto-night", features: ["auto-exposure"], reference: "ue", temporal: false,
  metric: { kind: "exposure", expected: Math.log2(0.5 / 0.18), tolerance: 0.1 },
};

export default function () {
  const scene = sceneWithSky(0.5);
  const c = new PerspectiveCamera(30, 16 / 9, 0.01, 1000);
  c.position.set(0, 2, 0);
  c.lookAt(0, 3, -5);
  c.setAutoExposure();
  scene.add(c);
  return result(scene, c);
}

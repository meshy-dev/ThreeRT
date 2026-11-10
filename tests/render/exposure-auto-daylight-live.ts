// Auto exposure in daylight, real-time renderer (its GPU histogram; the first frame snaps to the target): a uniform
// 3000-nit sky converges to EV100 log2(3000 / 0.18) = 14.03 (see exposure-auto-daylight).
import { PerspectiveCamera } from "@threert/scene";
import { result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "exposure/auto-daylight-live", features: ["auto-exposure"], reference: "ue", temporal: false, live: 4,
  metric: { kind: "exposure", expected: Math.log2(3000 / 0.18), tolerance: 0.15 },
};

export default function () {
  const scene = sceneWithSky(3000);
  const c = new PerspectiveCamera(30, 16 / 9, 0.01, 1000);
  c.position.set(0, 2, 0);
  c.lookAt(0, 3, -5);
  c.setAutoExposure();
  scene.add(c);
  return result(scene, c);
}

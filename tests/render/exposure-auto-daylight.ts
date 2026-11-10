// Auto exposure in daylight: a uniform 3000-nit sky (a bright overcast day) converges to EV100 = log2(3000 / 0.18) =
// 14.03, as UE's histogram method with the extended luminance range of every UE 5 project. The legacy range (log2
// luminance -8 .. 4) would clip the histogram and pin it at EV 6.5 (the image washed out).
import { PerspectiveCamera } from "@threert/scene";
import { result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "exposure/auto-daylight", features: ["auto-exposure"], reference: "ue", temporal: false,
  metric: { kind: "exposure", expected: Math.log2(3000 / 0.18), tolerance: 0.1 },
  note: "UE Scene.cpp: extended range EV100 -10 .. 20; EV100ToLog2 is the identity at lens attenuation 0.78",
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

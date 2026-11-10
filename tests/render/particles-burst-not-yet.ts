// A repeating burst shows nothing before its first shot: SparkBurst with a 1 s period whose customData[0] (the first
// burst) is 5 s away, rendered at scene time 0.5 s. Nothing else emits, so the region over the burst reads exactly 0
// (sparks of an earlier cycle would light it).
import { SparkBurst } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/burst-not-yet", features: ["particles"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.3, 0.2, 0.7, 0.8], expected: 0, tolerance: 0 },
  time: 0.5,
  note: "camera at (0, 1, 6) looking at (0, 1, 0); the burst rises from the origin",
};

export default function () {
  const scene = sceneWithSky(0);
  const sparks = new SparkBurst({ period: 1 });
  sparks.customData = [5, 0, 0, 0];
  return result(scene, camera(scene, [0, 1, 6], [0, 1, 0], 50), [sparks]);
}

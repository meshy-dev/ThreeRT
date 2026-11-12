// A light shown in place: light/hidden's two lights, the 5000 cd one hidden when the scene is built (it has its entry
// in the light table, black) and shown by the frame script after a few frames, without a rebuild. The plane reads both.
import { defineGame } from "@threert/gameplay";
import type { PointLight } from "@threert/scene";
import { ONE, lights } from "./light-hidden.ts";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/shown-in-place-live", features: ["point-light", "lambert", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: 6 * ONE, tolerance: 0.02 },
  live: 120,
  note: "the second light's own visible set true at frame 4: in place",
};

let elapsed = 0;
let second: PointLight;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    const scene = sceneWithSky(0);
    const [a, b] = lights();
    second = b;
    b.visible = false;
    return result(scene, camera(scene, [0, 1.5, 3], [0, 0, 0]), [a, b, plane(20, lambert(0.5))]);
  },
  frame(f) {
    elapsed += f.dt;
    if (elapsed > 3 / 60) second.visible = true;
  },
  handlers: {},
});

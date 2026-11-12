// A light hidden in place (world.rs `set_visible`: its table entry goes black, no rebuild): two 1000 cd lights as
// light/hidden's, both shown at first; the frame script hides one after a few frames. The plane reads the other's
// alone. (The light is in the plane's accumulated irradiance, which follows a change over frames: with a 5000 cd
// light hidden the plane still read 6 % over the one left 2 s later, the temporal filter's tail, not this test's
// matter.)
import { defineGame } from "@threert/gameplay";
import type { PointLight } from "@threert/scene";
import { ONE, lights } from "./light-hidden.ts";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/hidden-in-place-live", features: ["point-light", "lambert", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: ONE, tolerance: 0.02 },
  live: 240,
  note: "the second light's own visible set false at frame 4: in place",
};

let elapsed = 0;
let second: PointLight;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    const scene = sceneWithSky(0);
    const [a, b] = lights(1000);
    second = b;
    return result(scene, camera(scene, [0, 1.5, 3], [0, 0, 0]), [a, b, plane(20, lambert(0.5))]);
  },
  frame(f) {
    elapsed += f.dt;
    if (elapsed > 3 / 60) second.visible = false;
  },
  handlers: {},
});

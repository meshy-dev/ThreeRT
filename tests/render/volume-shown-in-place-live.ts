// A volume shown in place: volume/box-transmittance's box hidden when the scene is built (the build keeps a medium
// for it, with no shape; it is the scene's only one, so no media pass runs until then) and shown by the frame script
// after a few frames, without a rebuild. The wall behind it reads 2 e^-1 = 0.7358.
import { defineGame } from "@threert/gameplay";
import type { VolumeMesh } from "@threert/scene";
import { wallAndBox } from "./volume-hidden-in-place-live.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/shown-in-place-live", features: ["volume", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1), tolerance: 0.02 },
  live: 60,
  note: "the box's own visible set true at frame 4: in place",
};

let elapsed = 0;
let box: VolumeMesh;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    const scene = sceneWithSky(0);
    const [wall, b] = wallAndBox();
    box = b;
    b.visible = false;
    return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, b]);
  },
  frame(f) {
    elapsed += f.dt;
    if (elapsed > 3 / 60) box.visible = true;
  },
  handlers: {},
});

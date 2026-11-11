// voxel/cut-body in the real-time renderer: the cut is made by the frame script at its third frame; the loose part is
// added as a body's driven objects in place, falls as an instance (no vertex is written again, no rebuild) and rests
// on the stump: the cut's band reads 5 nits.
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { test as base } from "./voxel-cut-body.ts";
import { cut, level } from "./voxel-cut-falls.ts";

export const test: TestMeta = { ...base, id: "voxel/cut-body-live", temporal: true, time: undefined, live: 200 };

let frames = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    frames = 0;
    const scene = sceneWithSky(0);
    level(true);
    return result(scene, camera(scene, [0, 2, 8], [0, 2, 0]));
  },
  frame(f) {
    if (f.dt <= 0) return;
    if (++frames === 3) cut();
  },
  handlers: {},
});

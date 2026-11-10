// voxel/cut-falls in the real-time renderer: the cut is made by the frame script at its third frame, the loose part
// falls over the next frames (its mesh written again in place every tick, no rebuild) and rests on the stump: the cut's
// band reads 5 nits.
import { defineGame } from "@threert/gameplay";
import { voxelFrame } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { cut, level, test as base } from "./voxel-cut-falls.ts";

export const test: TestMeta = { ...base, id: "voxel/cut-falls-live", temporal: true, live: 200 };

let frames = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    frames = 0;
    const scene = sceneWithSky(0);
    level();
    return result(scene, camera(scene, [0, 2, 8], [0, 2, 0]));
  },
  frame(f) {
    if (f.dt <= 0) return;
    if (++frames === 3) cut();
    voxelFrame(f.dt);
  },
  handlers: {},
});

// The control of voxel/cut-falls: the same cut through half of the post's section only (x < 0). The other half still
// joins what is above to the slab, so nothing falls: the top 0.2 m of the upper post still glows, as the post below it
// (ratio 1; fallen: 0).
import { defineGame } from "@threert/gameplay";
import { voxelCarve, voxelFrame } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { BELOW, TOP, level, test as base } from "./voxel-cut-falls.ts";

export const test: TestMeta = { ...base, id: "voxel/cut-holds", metric: { kind: "ratio", a: TOP, b: BELOW, expected: 1, tolerance: 0.02 } };

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    level();
    voxelCarve({ box: [-0.5, 0.8, -0.5, 0, 1.0, 0.5] });
    for (let k = 0; k < 60; k++) voxelFrame(1 / 60);
    return result(scene, camera(scene, [0, 2, 8], [0, 2, 0]));
  },
  handlers: {},
});

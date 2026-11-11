// What a cut leaves hanging on nothing is a rigid body (ROADMAP § 6 step 8). The level of voxel/cut-falls with its
// physics on (the default): the slab and the posts are fixed colliders, and the cut frees the lower post's upper part
// with the post on it, 416 voxels, as one body of the physics world. It comes down the 0.2 m of the cut onto the
// stump, stands and sleeps (renderer/src/voxel.rs `the_level_collides_and_loose_parts_are_bodies` has the numbers:
// 0.2 m within 5 mm, no drift). Seen from the side after 2 s (120 ticks), black sky: the cut has closed, so its band
// reads 5 nits (still in the air, toppled or fallen through the slab: 0).
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { cut, level, test as base } from "./voxel-cut-falls.ts";

export const test: TestMeta = { ...base, id: "voxel/cut-body", features: ["voxel", "edits", "physics"], time: 2, note: "as voxel/cut-falls, the level's physics on: the loose part is a body" };

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    level(true);
    cut();
    return result(scene, camera(scene, [0, 2, 8], [0, 2, 0]));
  },
  handlers: {},
});

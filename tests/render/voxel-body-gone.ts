// A voxel body held by scene code and carved away by an edit of the level (ROADMAP § 6 step 10 a; the review of #25):
// on the real engine, through `@threert/voxel`. voxel/cut-body-live's cut; once the loose part stands on the stump the
// frame script takes its object, carves it away whole with a level edit (`voxelCarve`), and then calls everything an
// object has on it: none may throw (a gone body's calls do nothing), and it must be `gone` with reason "carved". Only
// if all of that holds does the script add a glowing block (palette entry 2, 5 nits) left of the posts, standing on the
// slab; the test reads that block. Anything that throws or is not as said leaves the place dark: 0 for 5.
import { defineGame } from "@threert/gameplay";
import { Vector3 } from "@threert/scene";
import { voxelAdd, voxelBodies, voxelBody, voxelCarve } from "@threert/voxel";
import { camera, result, sceneWithSky, type Region, type TestMeta } from "./lib.ts";
import { test as base } from "./voxel-cut-body.ts";
import { cut, level } from "./voxel-cut-falls.ts";

/** The block's place in the image (x -0.7 .. -0.5 m, y 0.4 .. 1.0 m; the camera of voxel/cut-falls): at 160 x 90 its
 *  three columns of pixels. */
const BLOCK: Region = [0.4135, 0.75, 0.428, 0.85];

export const test: TestMeta = {
  ...base, id: "voxel/body-gone", temporal: true, time: undefined, live: 200,
  metric: { kind: "radiance", region: BLOCK, expected: 5, tolerance: 0.05 },
  note: "as voxel/cut-body-live; at frame 90 the part's object is carved away by a level edit, every call on it must do nothing, then a glowing block is added",
};

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
    frames++;
    if (frames === 3) cut();
    if (frames !== 90) return;
    const part = voxelBodies()[0];
    if (!part || part.gone) return;
    const id = part.body;
    // The part rests on the stump (from 0.8 m up); the stump's top cell's centre is at 0.75 m, outside this box.
    voxelCarve({ box: [-0.6, 0.81, -0.6, 0.6, 4.0, 0.6] });
    try {
      part.applyImpulse(new Vector3(0, 10, 0));
      part.setLinearVelocity(new Vector3(1, 0, 0));
      void part.position.x;
      void part.getLinearVelocity().y;
      void part.sleeping;
      part.addEventListener("contactstart", () => {});
      part.paint({ sphere: [0, 1, 0, 1] }, 1);
      part.addVoxels({ box: [0, 1, 0, 0.2, 1.2, 0.2] }, 1);
      part.carve({ box: [-1, 0, -1, 1, 4, 1] });
    } catch {
      return;
    }
    if (!part.gone || part.goneReason !== "carved" || part.physics !== null || voxelBody(id) !== undefined) return;
    voxelAdd({ box: [-0.7, 0.4, -0.1, -0.5, 1.0, 0.1] }, 2);
  },
  handlers: {},
});

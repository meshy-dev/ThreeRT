// What a ray gives scene code is enough to act on a voxel body (ROADMAP § 6 step 8): voxel/cut-body-live's cut, and
// once the loose part stands on the stump the frame script casts two rays at the physics world. One straight down
// at the slab: a hit on the voxel level, whose `object` is `engineColliders` (no scene object: its `physics` is
// null) and which has no `body` (the level is fixed). One from the camera's side at the part: the same `object`,
// and a `body`, the id `voxelBodies()` lists. That id goes into `voxelImpulse`, sideways: the part is thrown off
// the stump. So where the part stood (y 2.9 .. 3.2) reads 0 against the stump, which is the level's and stays
// (y 0.45 .. 0.75, 5 nits). A ray that does not report the level or the body as said, or an impulse that does not
// move the part, leaves the part standing: the ratio is 1.
import { engineColliders, Raycaster, Vector3 } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { voxelBodies, voxelImpulse } from "@threert/voxel";
import { camera, result, sceneWithSky, type Region, type TestMeta } from "./lib.ts";
import { test as base } from "./voxel-cut-body.ts";
import { BELOW, cut, level } from "./voxel-cut-falls.ts";

const STUMP: Region = [0.485, 0.795, 0.515, 0.855];

export const test: TestMeta = {
  ...base, id: "voxel/cut-body-push", temporal: true, time: undefined, live: 240,
  metric: { kind: "ratio", a: BELOW, b: STUMP, expected: 0, tolerance: 0.02 },
  note: "as voxel/cut-body-live; at frame 90 a ray's hit names the body and voxelImpulse throws it off the stump",
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
    // The level (the slab is 1.6 m square about the origin): fixed, no scene object, no body.
    const ground = new Raycaster(new Vector3(0.55, 5, 0.55), new Vector3(0, -1, 0)).intersectPhysics()[0];
    if (!ground || ground.object !== engineColliders || ground.object.physics !== null || ground.body !== undefined || ground.collider === undefined) return;
    // The part: a body, the one the level lists.
    const hit = new Raycaster(new Vector3(0.05, 2.05, 5), new Vector3(0, 0, -1)).intersectPhysics()[0];
    if (!hit || hit.object !== engineColliders || hit.body === undefined || !voxelBodies().some((b) => b.body === hit.body)) return;
    // 416 voxels of 900 kg/m³: 374 kg; 1,500 N s is 4 m/s sideways.
    voxelImpulse(hit.body, [1500, 300, 0]);
  },
  handlers: {},
});

// A joint on a voxel body that the engine moved onto a part cut off stays on that part when the body it was made on
// goes (step 10 c1). voxel/cut-body's level and cut: the upper part comes down onto the stump (y 0.8 .. 3.4). At frame
// 60 a glowing 0.2 m box (the lamp) is held to it by a `FixedJoint` whose anchor is in its lower half (world
// (0.15, 1.4, 0)), the lamp beside it at x = 0.45. At frame 90 a cut through the part at y 1.9 .. 2.1: the upper piece
// (1.3 m) keeps the body, the lower (1.1 m, with the anchor) is a part of its own, and the engine moves the joint onto
// it. At frame 150 the upper piece is carved away whole (its body goes); at frame 160 a body is added to the scene,
// which makes the binding look at its joints. The joint names the lower piece and holds: the lamp is where it was
// (y 1.3 .. 1.5) and the region there reads its 5 nits. A binding that let go of the joint when the first body went
// drops the lamp to the slab (dark there); a failed check (the joint not on a `VoxelBody`, or broken) takes the lamp
// out (dark).
import { BoxGeometry, FixedJoint, Mesh, type Scene, Vector3 } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { voxelBodies, voxelCarve, VoxelBody } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { glow } from "./physics-box-rest.ts";
import { test as base } from "./voxel-cut-body.ts";
import { cut, level } from "./voxel-cut-falls.ts";

export const test: TestMeta = {
  ...base, id: "voxel/joint-moved-body", temporal: true, time: undefined, live: 240,
  // Camera 8 m away at y = 2, 30 deg vertical fov, 16:9: x -3.81 .. 3.81, y 4.14 .. -0.14. The lamp's middle: x 0.40
  // .. 0.50, y 1.35 .. 1.45.
  metric: { kind: "radiance", region: [0.553, 0.627, 0.565, 0.650], expected: 5, tolerance: 0.03 },
  note: "as voxel/cut-body-live; a lamp held to the part by a fixed joint, the part cut in two, the piece without the anchor carved away",
};

let frames = 0;
let scene: Scene;
let lamp: Mesh | null = null;
let joint: FixedJoint | null = null;
let first: VoxelBody | null = null;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    frames = 0;
    lamp = joint = first = null;
    scene = sceneWithSky(0);
    level(true);
    return result(scene, camera(scene, [0, 2, 8], [0, 2, 0]));
  },
  frame(f) {
    if (f.dt <= 0) return;
    frames++;
    if (frames === 3) cut();
    if (frames === 60) {
      first = voxelBodies()[0] ?? null;
      if (!first) return;
      // The anchor in the part's own frame.
      const local = new Vector3(0.15, 1.4, 0).sub(first.position).applyQuaternion(first.quaternion.clone().invert());
      lamp = new Mesh(new BoxGeometry(0.2, 0.2, 0.2), glow());
      lamp.position.set(0.45, 1.4, 0);
      lamp.physics = "dynamic";
      joint = new FixedJoint(first, lamp, { anchorA: local, anchorB: new Vector3(-0.3, 0, 0) });
      scene.add(lamp, joint);
    }
    if (frames === 90) voxelCarve({ box: [-0.5, 1.9, -0.5, 0.5, 2.1, 0.5] });
    if (frames === 150) voxelCarve({ box: [-0.5, 1.92, -0.5, 0.5, 3.7, 0.5] });
    if (frames === 160) {
      const crate = new Mesh(new BoxGeometry(0.2, 0.2, 0.2), glow());
      crate.position.set(-2.5, 0.6, 0);
      crate.physics = "dynamic";
      scene.add(crate);
    }
    if (frames === 200 && lamp && joint && (!(joint.bodyA instanceof VoxelBody) || joint.bodyA === first || joint.broken)) scene.remove(lamp);
  },
  handlers: {},
});

// A joint to the voxel level goes with the material at its anchor (step 10 c1), and the binding knows where it went.
// voxel/cut-body's level and cut: a glowing 0.2 m box (the lamp) is held by a `FixedJoint` to the level, its anchor
// in the upper post's side (0.15, 3.0, 0), the lamp beside it at x = 0.45. The cut lets the upper part down 0.2 m onto
// the stump: the engine moves the joint onto that part's body, and the lamp comes down with it. At frame 120 the frame
// script checks that the joint names the part (`bodyA` its `VoxelBody`, not broken) and makes the lamp's body anew
// (its density changed), which makes the joint anew: on the part, at the anchor the engine reported. So the lamp stays
// 0.2 m lower than it was, beside the part: the region there (y 2.76 .. 2.84) reads its 5 nits. A binding that remade
// the joint on the level at the old anchor would pull the lamp back up (the region dark, at the lamp's bottom edge
// at most); a failed check takes the lamp out (dark).
import { BoxGeometry, engineColliders, FixedJoint, Mesh, type Scene, Vector3 } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { VoxelBody } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { glow } from "./physics-box-rest.ts";
import { test as base } from "./voxel-cut-body.ts";
import { cut, level } from "./voxel-cut-falls.ts";

export const test: TestMeta = {
  ...base, id: "voxel/joint-moved", temporal: true, time: undefined, live: 240,
  // Camera 8 m away at y = 2, 30 deg vertical fov, 16:9: x -3.81 .. 3.81, y 4.14 .. -0.14. The lamp's middle after the
  // fall: x 0.40 .. 0.50, y 2.76 .. 2.84.
  metric: { kind: "radiance", region: [0.553, 0.303, 0.565, 0.322], expected: 5, tolerance: 0.03 },
  note: "as voxel/cut-body-live, with a lamp held to the level by a fixed joint in the upper post; its body made anew at frame 120",
};

let frames = 0;
let scene: Scene;
let lamp: Mesh;
let joint: FixedJoint;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    frames = 0;
    scene = sceneWithSky(0);
    level(true);
    lamp = new Mesh(new BoxGeometry(0.2, 0.2, 0.2), glow());
    lamp.position.set(0.45, 3, 0);
    lamp.physics = "dynamic";
    joint = new FixedJoint(engineColliders, lamp, { anchorA: new Vector3(0.15, 3, 0), anchorB: new Vector3(-0.3, 0, 0) });
    return result(scene, camera(scene, [0, 2, 8], [0, 2, 0]), [lamp, joint]);
  },
  frame(f) {
    if (f.dt <= 0) return;
    frames++;
    if (frames === 3) cut();
    if (frames !== 120) return;
    if (!(joint.bodyA instanceof VoxelBody) || joint.broken || joint.bodyB !== lamp) {
      scene.remove(lamp);
      return;
    }
    lamp.physics = { type: "dynamic", density: 300 };
  },
  handlers: {},
});

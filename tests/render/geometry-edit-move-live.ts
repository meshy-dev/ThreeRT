// An object added in place, then moved (live.rs `commit`: objects added in place are not in the move tables, so the
// move rebuilds the scene). After a few frames the frame script adds a procedural box beside the view, a few frames
// later it moves the box across the centre: Lambert 0.5 facing the camera under a uniform sky of 1 nit, nothing else in
// its hemisphere, so 0.5. Left where it was added, the centre would show the sky (1).
import { ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/edit-move-live", features: ["procedural", "edits", "movable"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "a box added in place, then moved across the centre: Lambert 0.5 under a 1 nit sky",
};

let elapsed = 0;
let box: ProceduralMesh | null = null;
let stage = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    box = null;
    stage = 0;
    const scene = sceneWithSky(1);
    // The box's material and geometry function, already in the scene (a new material, or the scene's first procedural
    // object, would rebuild rather than add in place): a procedural speck far behind the camera.
    const speck = new ProceduralMesh(proceduralBoxes([{ min: [0, 0, 0], max: [0.01, 0.01, 0.01] }]), lambert(0.5, "box"));
    speck.name = "speck";
    speck.position.set(0, 0, 50);
    return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [speck]);
  },
  frame(f) {
    elapsed += f.dt;
    if (stage === 0 && elapsed > 3 / 60) {
      stage = 1;
      box = new ProceduralMesh(proceduralBoxes([{ min: [-1, -1, -0.1], max: [1, 1, 0.1] }]), (f.scene.getObjectByName("speck") as ProceduralMesh).materials);
      box.position.set(8, 0, 0);
      f.scene.add(box);
    } else if (stage === 1 && elapsed > 10 / 60) {
      stage = 2;
      box!.position.set(0, 0, 0);
    }
  },
  handlers: {},
});

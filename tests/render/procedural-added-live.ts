// geometry/added-live with a procedural box (its AABB BLAS built with the edit's TLAS): a scene of one procedural speck
// gets a 2 m box across the centre at frame 3, its face toward the camera Lambert 0.5 under a uniform 1-nit sky (a
// convex box: nothing else in that face's hemisphere), so 0.5.
import { ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/added-live", features: ["procedural", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "a procedural box added in place to a scene of one instance: drawn, Lambert 0.5 under a 1 nit sky",
};

let elapsed = 0;
let added = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    added = false;
    const scene = sceneWithSky(1);
    const speck = new ProceduralMesh(proceduralBoxes([{ min: [5, 0, -10], max: [5.05, 0.05, -9.95] }]), lambert(0.5, "speck"));
    speck.name = "speck";
    return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [speck]);
  },
  frame(f) {
    elapsed += f.dt;
    if (added || elapsed <= 3 / 60) return;
    added = true;
    const box = new ProceduralMesh(proceduralBoxes([{ min: [-1, -1, -2], max: [1, 1, 0] }]), (f.scene.getObjectByName("speck") as ProceduralMesh).materials);
    f.scene.add(box);
  },
  handlers: {},
});

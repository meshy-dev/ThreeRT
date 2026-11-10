// An InstancedMesh added in place (live.rs `edit_geometry`; ROADMAP § 6 step 5): a scene of one `dynamicGeometry`
// speck gets an InstancedMesh of two planes at frame 3 (its records, TLAS instances and object entries from the room,
// contiguous, as the build places one), one across the centre: Lambert 0.5 under a uniform 1-nit sky, so 0.5. A
// later move of an instance (setMatrixAt) applies in place too.
import { InstancedMesh, Matrix4, Mesh, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/instanced-added-live", features: ["instancing", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "an InstancedMesh added in place, then an instance moved in place: the plane across the centre reads 0.5",
};

let frames = 0;
let copies: InstancedMesh | null = null;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    frames = 0;
    copies = null;
    const scene = sceneWithSky(1);
    const speck = new Mesh(new PlaneGeometry(0.05, 0.05), lambert(0.5, "speck"));
    speck.name = "speck";
    speck.dynamicGeometry = true;
    speck.position.set(5, 0, -10);
    return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [speck]);
  },
  frame(f) {
    frames++;
    if (frames === 4) {
      copies = new InstancedMesh(new PlaneGeometry(4, 4), (f.scene.getObjectByName("speck") as Mesh).material, 2);
      copies.setMatrixAt(0, new Matrix4().makeTranslation(0, 0, -20));
      copies.setMatrixAt(1, new Matrix4().makeTranslation(30, 0, 0));
      f.scene.add(copies);
    } else if (frames === 8 && copies) {
      copies.setMatrixAt(0, new Matrix4());
      copies.instanceMatrix.needsUpdate = true;
    }
  },
  handlers: {},
});

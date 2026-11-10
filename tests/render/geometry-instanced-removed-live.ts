// An InstancedMesh removed in place (live.rs `edit_geometry`; ROADMAP § 6 step 5): its two planes (Lambert 0.2) hide a
// `dynamicGeometry` plane (Lambert 0.5) behind them; at frame 3 the InstancedMesh is removed, its TLAS instances taken
// out without a rebuild, and the centre shows the plane behind: 0.5 under a uniform 1-nit sky.
import { InstancedMesh, Matrix4, Mesh, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/instanced-removed-live", features: ["instancing", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "an InstancedMesh in front removed in place: the plane behind it reads 0.5",
};

let frames = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    frames = 0;
    const scene = sceneWithSky(1);
    const back = new Mesh(new PlaneGeometry(4, 4), lambert(0.5, "back"));
    back.dynamicGeometry = true;
    back.position.z = -1;
    const front = new InstancedMesh(new PlaneGeometry(4, 4), lambert(0.2, "front"), 2);
    front.name = "front";
    front.setMatrixAt(0, new Matrix4());
    front.setMatrixAt(1, new Matrix4().makeTranslation(0, 0, -0.5));
    return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [back, front]);
  },
  frame(f) {
    if (++frames !== 4) return;
    const front = f.scene.getObjectByName("front");
    front?.parent?.remove(front);
  },
  handlers: {},
});

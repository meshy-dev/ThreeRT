// InstancedMesh instances changed in place (live/edits.rs `update_instanced`; ROADMAP § 6 step 2e): two copies of a dark
// cover hide the centre of a plane, a third waits behind it. After a few frames the frame script moves the first
// cover behind the plane (`setMatrixAt`, `needsUpdate`), recolours it, and lowers `count` to hide the last one; the
// second is hidden by a matrix scaled to 0. In the same frame a `dynamicGeometry` mesh off to the side gets a new
// geometry: the instances' TLAS changes then ride on that edit's TLAS build (live/edits.rs `commit`). The centre then shows
// the plane: Lambert, albedo 0.5, facing a uniform sky of 1 nit with nothing in its hemisphere, so 0.5. The same frames
// with THREERT_EDIT_REBUILD=1 (every change rebuilds the scene) are byte-identical (the PR's A/B).
import { Color, InstancedMesh, Matrix4, Mesh, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/instanced-live", features: ["instancing", "instanced-mesh", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "instances moved, hidden by count and by a zero scale, recoloured, all in place: a Lambert 0.5 plane under a 1 nit sky",
};

let elapsed = 0;
let edited = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    edited = false;
    const scene = sceneWithSky(1);
    const back = new Mesh(new PlaneGeometry(4, 4), lambert(0.5, "plane"));
    const covers = new InstancedMesh(new PlaneGeometry(1.5, 1.5), lambert(0.2, "cover"), 3);
    covers.name = "covers";
    const m = new Matrix4();
    covers.setMatrixAt(0, m.makeTranslation(0, 0, 1));
    covers.setMatrixAt(1, m.makeTranslation(0, 0, 1.5));
    covers.setMatrixAt(2, m.makeTranslation(0, 0, 2));
    for (let i = 0; i < 3; i++) covers.setColorAt(i, new Color(0xffffff));
    const side = new Mesh(new PlaneGeometry(0.2, 0.2), lambert(0.5, "plane"));
    side.name = "side";
    side.dynamicGeometry = true;
    side.position.set(-5, 0, -1);
    return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [back, covers, side]);
  },
  frame(f) {
    elapsed += f.dt;
    if (edited || elapsed <= 3 / 60) return;
    edited = true;
    const covers = f.scene.getObjectByName("covers") as InstancedMesh;
    covers.setMatrixAt(0, new Matrix4().makeTranslation(0, 0, -3));
    covers.setMatrixAt(1, new Matrix4().makeScale(0, 0, 0));
    covers.instanceMatrix.needsUpdate = true;
    covers.setColorAt(0, new Color(0xff0000));
    covers.instanceColor!.needsUpdate = true;
    covers.count = 2;
    (f.scene.getObjectByName("side") as Mesh).geometry = new PlaneGeometry(0.3, 0.3);
  },
  handlers: {},
});

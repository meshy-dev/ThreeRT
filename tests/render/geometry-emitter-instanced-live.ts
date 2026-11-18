// An emitting instanced mesh's copies changed in place (live/edits.rs `update_instanced`; ROADMAP § 6 step 3): the
// panel of geometry/emitter-instanced starts with one quad in place, one far away, one scaled to 0 and one past
// `count`. After a few frames the frame script puts all four in place: their emitters follow in the light list and
// are weighed again, no rebuild, and the floor below the centre reads the same analytic value.
import { InstancedMesh, Matrix4, Mesh, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { E, glow, tile } from "./geometry-emitter-instanced.ts";

export const test: TestMeta = {
  id: "geometry/emitter-instanced-live", features: ["emitters", "instancing", "instanced-mesh", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  live: 90,
  note: "four emitting copies moved, scaled up and shown in place tile an 8 x 8 m panel of 1 nit, 1 m above a Lambert 0.5 floor",
};

let elapsed = 0;
let edited = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    edited = false;
    const scene = sceneWithSky(0);
    const material = glow();
    const speck = new Mesh(new PlaneGeometry(0.01, 0.01), material);
    speck.name = "speck";
    speck.dynamicGeometry = true;
    speck.position.set(900, -50, 0);
    const panel = new InstancedMesh(new PlaneGeometry(4, 4), material, 4);
    panel.name = "panel";
    panel.setMatrixAt(0, tile(0));
    panel.setMatrixAt(1, tile(1).setPosition(300, 1, 0));
    panel.setMatrixAt(2, new Matrix4().makeScale(0, 0, 0));
    panel.setMatrixAt(3, tile(3));
    panel.count = 3;
    return result(scene, camera(scene, [3, 0.8, 0], [0, 0, 0]), [plane(40, lambert(0.5), "floor"), speck, panel]);
  },
  frame(f) {
    elapsed += f.dt;
    if (edited || elapsed <= 3 / 60) return;
    edited = true;
    const panel = f.scene.getObjectByName("panel") as InstancedMesh;
    for (let i = 0; i < 4; i++) panel.setMatrixAt(i, tile(i));
    panel.instanceMatrix.needsUpdate = true;
    panel.count = 4;
  },
  handlers: {},
});

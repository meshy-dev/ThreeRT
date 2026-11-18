// An emitting instanced mesh added in place (live/edits.rs `edit_geometry`; ROADMAP § 6 step 3): after a few frames
// the frame script adds the panel of geometry/emitter-instanced (four quads of 1 nit tiling 8 x 8 m, a fifth scaled
// to 0, a sixth past `count`), 1 m above a Lambert 0.5 floor. Each copy's emitters come into the light list in
// place, no rebuild, and the floor below the centre reads the same analytic value. The material is in the scene
// from the start (a speck far away): a new material rebuilds.
import { InstancedMesh, Matrix4, Mesh, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { E, glow, tile } from "./geometry-emitter-instanced.ts";

export const test: TestMeta = {
  id: "geometry/emitter-instanced-added-live", features: ["emitters", "instancing", "instanced-mesh", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  live: 90,
  note: "an InstancedMesh of four emitting quads (1 nit) added in place, 1 m above a Lambert 0.5 floor",
};

let elapsed = 0;
let edited = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    edited = false;
    const scene = sceneWithSky(0);
    const speck = new Mesh(new PlaneGeometry(0.01, 0.01), glow());
    speck.name = "speck";
    speck.dynamicGeometry = true;
    speck.position.set(900, -50, 0);
    return result(scene, camera(scene, [3, 0.8, 0], [0, 0, 0]), [plane(40, lambert(0.5), "floor"), speck]);
  },
  frame(f) {
    elapsed += f.dt;
    if (edited || elapsed <= 3 / 60) return;
    edited = true;
    const panel = new InstancedMesh(new PlaneGeometry(4, 4), (f.scene.getObjectByName("speck") as Mesh).material, 6);
    panel.name = "panel";
    for (let i = 0; i < 4; i++) panel.setMatrixAt(i, tile(i));
    panel.setMatrixAt(4, new Matrix4().makeScale(0, 0, 0));
    panel.setMatrixAt(5, new Matrix4().makeTranslation(0, 0.5, 0));
    panel.count = 5;
    f.scene.add(panel);
  },
  handlers: {},
});

// geometry/emitter-added-live with a light tree (ROADMAP § 6 step 4): 160 static emitter specks far below the floor
// (320 triangles, over the tree's 256) make the scene sample its emitters by a light tree, which, the scene's geometry
// being editable, keeps a slot for emitters added in place. The panel added at frame 3 gets that subtree (its entries
// right after the static ones, codes under the root's right child), and the floor reads 0.5 x 1 = 0.5 as without the
// tree; the specks add nothing visible (1 nit, 1 cm², 900 m away).
import { Mesh, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { glow } from "./geometry-emitter-added-live.ts";
import { CENTER, camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/emitter-added-tree-live", features: ["edits", "dynamic-emission", "light-tree"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.03 },
  live: 90,
  note: "an emissive panel (1 nit) added in place above a Lambert 0.5 floor, emitters sampled by a light tree: 0.5",
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
    const specks = Array.from({ length: 160 }, (_, k) => {
      const m = new Mesh(new PlaneGeometry(0.01, 0.01), material);
      m.name = `speck-${k}`;
      m.position.set(900 + (k % 16), -50, Math.floor(k / 16));
      return m;
    });
    return result(scene, camera(scene, [0, 0.5, 0.001], [0, 0, 0], 40), [plane(400, lambert(0.5, "floor"), "floor"), speck, ...specks]);
  },
  frame(f) {
    elapsed += f.dt;
    if (edited || elapsed <= 3 / 60) return;
    edited = true;
    const panel = new Mesh(new PlaneGeometry(400, 400), (f.scene.getObjectByName("speck") as Mesh).material);
    panel.name = "panel";
    panel.dynamicGeometry = true;
    panel.rotation.x = Math.PI / 2;
    panel.position.y = 1;
    f.scene.add(panel);
  },
  handlers: {},
});

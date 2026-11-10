// Emitting geometry added in place (live.rs `edit_geometry`; ROADMAP § 6 step 3): after a few frames the frame script
// adds a `dynamicGeometry` panel of an emissive material (1 nit, black) 1 m above a Lambert 0.5 floor, so large it
// fills the floor's hemisphere. Its emitter entries come in place (no rebuild), and the floor reads 0.5 x 1 = 0.5.
// The material is in the scene from the start (a speck far away): a new material rebuilds.
import { Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/emitter-added-live", features: ["edits", "dynamic-emission"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.03 },
  live: 90,
  note: "an emissive panel (1 nit) added in place above a Lambert 0.5 floor: 0.5",
};

export const glow = () => new MeshStandardMaterial({ name: "glow", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: 1, roughness: 1 });

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
    return result(scene, camera(scene, [0, 0.5, 0.001], [0, 0, 0], 40), [plane(400, lambert(0.5, "floor"), "floor"), speck]);
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

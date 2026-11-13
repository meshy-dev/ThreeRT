// A material that starts emitting in place (tables.rs `can_emit`, live/edits.rs `add_material_emitters`; ROADMAP § 6
// step 3): a panel 1 m above a Lambert 0.5 floor, filling its hemisphere, of a black material that emits nothing at
// first; after a few frames the frame script sets its emission to 1 nit. Its triangles get emitter entries in place
// (no rebuild), and the floor reads 0.5.
import { Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/emitter-started-live", features: ["dynamic-emission"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.03 },
  live: 90,
  note: "a panel's material starts emitting (1 nit) in place, above a Lambert 0.5 floor: 0.5",
};

let elapsed = 0;
let edited = false;
let lamp: MeshStandardMaterial;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    edited = false;
    const scene = sceneWithSky(0);
    lamp = new MeshStandardMaterial({ name: "lamp", color: new Color(0, 0, 0), emissive: new Color(0, 0, 0), roughness: 1 });
    const panel = new Mesh(new PlaneGeometry(400, 400), lamp);
    panel.name = "panel";
    panel.rotation.x = Math.PI / 2;
    panel.position.y = 1;
    return result(scene, camera(scene, [0, 0.5, 0.001], [0, 0, 0], 40), [plane(400, lambert(0.5, "floor"), "floor"), panel]);
  },
  frame(f) {
    elapsed += f.dt;
    if (edited || elapsed <= 3 / 60) return;
    edited = true;
    lamp.emissive = new Color(1, 1, 1);
    lamp.emissiveIntensity = 1;
  },
  handlers: {},
});

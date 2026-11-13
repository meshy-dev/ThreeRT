// Emitting geometry removed in place (live/edits.rs `edit_geometry`; ROADMAP § 6 step 3): a `dynamicGeometry` panel of an
// emissive material lights a white Lambert floor; after a few frames the frame script removes it. Its emitter entries
// go in place (no rebuild): the floor turns black, against an unlit patch of colour 1 beside it (MeshBasicMaterial,
// lighting nothing): floor / patch = 0.
import { Color, Mesh, MeshBasicMaterial, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { glow } from "./geometry-emitter-added-live.ts";

export const test: TestMeta = {
  id: "geometry/emitter-removed-live", features: ["edits", "dynamic-emission"], reference: "analytic", temporal: true,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 0, tolerance: 0.02 },
  live: 300,
  note: "an emissive panel removed in place: the floor it lit turns black (against an unlit patch of 1)",
};

let elapsed = 0;
let edited = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    edited = false;
    const scene = sceneWithSky(0);
    const panel = new Mesh(new PlaneGeometry(400, 400), glow());
    panel.name = "panel";
    panel.dynamicGeometry = true;
    panel.rotation.x = Math.PI / 2;
    panel.position.y = 1;
    const patch = new Mesh(new PlaneGeometry(0.15, 0.15), new MeshBasicMaterial({ name: "patch", color: new Color(1, 1, 1) }));
    patch.rotation.x = -Math.PI / 2;
    patch.position.set(-0.08, 0.01, 0);
    return result(scene, camera(scene, [0, 0.5, 0.001], [0, 0, 0], 40), [plane(400, lambert(1, "floor"), "floor"), panel, patch]);
  },
  frame(f) {
    elapsed += f.dt;
    if (edited || elapsed <= 3 / 60) return;
    edited = true;
    const panel = f.scene.getObjectByName("panel")!;
    panel.parent!.remove(panel);
  },
  handlers: {},
});

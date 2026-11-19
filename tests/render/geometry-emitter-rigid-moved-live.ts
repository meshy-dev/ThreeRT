// A movable posed rigid model moved in place takes the nodes its clip leaves alone with it (live/edits.rs
// `repose_rigid`: their instance, and its emitters in the light list). geometry/emitter-rigid's panel is built 300 m
// away; from 0.08 s the frame script holds it 1 m above the floor point the camera looks at, posed on. The floor
// reads the same analytic value; with the panel left where it was built it would read nothing.
import { GLTFModel } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { E, URI, models } from "./geometry-emitter-rigid.ts";

export const test: TestMeta = {
  id: "geometry/emitter-rigid-moved-live", features: ["emitters", "instancing", "gltf", "animation", "movable", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  requires: [URI],
  live: 240,
  note: "the panel moved 300 m in place, its clip playing: the floor below its centre as geometry/emitter-rigid",
};

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const [far, panel] = models([300, 1, 0]);
    panel.id = "panel";
    panel.movable = true;
    return result(scene, camera(scene, [3, 0.8, 0], [0, 0, 0]), [plane(40, lambert(0.5), "floor"), far, panel]);
  },
  frame(f) {
    const panel = f.object("panel") as GLTFModel;
    if (f.time >= 0.08) panel.position.set(-5, 1, 0);
    panel.animation = { clip: "slide", time: 0.25 + (f.time % 0.5) };
  },
  handlers: {},
});

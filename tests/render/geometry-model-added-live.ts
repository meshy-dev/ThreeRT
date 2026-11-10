// A glTF model added in place (live.rs `edit_geometry`; ROADMAP § 6 step 5): a static model already in the scene (a
// grey quad, tests/render/data/quad.glb, on the left) gets a second copy on the right at frame 3, another instance of
// its mesh, without a rebuild. Both under a uniform 1-nit sky: the right reads as the left, ratio 1.
import { GLTFModel } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/model-added-live", features: ["instancing", "edits", "gltf"], reference: "analytic", temporal: true,
  metric: { kind: "ratio", a: [0.6, 0.45, 0.7, 0.55], b: [0.3, 0.45, 0.4, 0.55], expected: 1, tolerance: 0.02 },
  requires: ["tests/render/data/quad.glb"],
  live: 60,
  note: "a second copy of a static glTF model added in place reads as the first: ratio 1",
};

const URI = "tests/render/data/quad.glb";
let elapsed = 0;
let added = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    added = false;
    const scene = sceneWithSky(1);
    const left = new GLTFModel(URI);
    left.position.set(-0.6, 0, 0);
    return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [left]);
  },
  frame(f) {
    elapsed += f.dt;
    if (added || elapsed <= 3 / 60) return;
    added = true;
    const right = new GLTFModel(URI);
    right.position.set(0.6, 0, 0);
    f.scene.add(right);
  },
  handlers: {},
});

// A posed rigid model's nodes that its clip leaves alone are one mesh, an instance (scene.rs `RigidModel::still`),
// and its emitters are that instance's in the light list. tests/render/data/panel-slider.glb: panel.glb's panel (a
// small grey quad, then an 8 x 8 m quad of 1 nit) on a node 5 m along x that no clip moves, and a grey quad 6 m
// beside it that the clip "slide" moves. As geometry/emitter-model: the panel faces down 1 m above a Lambert 0.5
// floor, and the floor below its centre reads 0.5 / pi x 2.989 (below its node's origin, 5 m aside, next to
// nothing). A second placement far away holds the list's first entries.
import { GLTFModel } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const corner = (x: number, y: number) => (x / Math.sqrt(1 + x * x)) * Math.atan(y / Math.sqrt(1 + x * x)) + (y / Math.sqrt(1 + y * y)) * Math.atan(x / Math.sqrt(1 + y * y));
export const E = 4 * (1 / 2) * corner(4, 4);
export const URI = "tests/render/data/panel-slider.glb";
export const POSE = { clip: "slide", time: 0.25 };

export const test: TestMeta = {
  id: "geometry/emitter-rigid", features: ["emitters", "lambert", "instancing", "gltf", "animation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  requires: [URI],
  spp: 256,
  note: "camera under the panel at (3, 0.8, 0) looking at the floor point below its centre; the model posed by its clip",
};

/** The model with its panel facing down at `at` (its node is 5 m along x), and one far away. */
export const models = (at: [number, number, number]) => {
  const far = new GLTFModel(URI, { animation: POSE });
  far.position.set(900, -50, 0);
  far.scale.setScalar(0.01);
  const panel = new GLTFModel(URI, { animation: POSE });
  panel.name = "panel";
  panel.rotation.x = Math.PI / 2;
  panel.position.set(at[0] - 5, at[1], at[2]);
  return [far, panel];
};

export default () => {
  const scene = sceneWithSky(0);
  return result(scene, camera(scene, [3, 0.8, 0], [0, 0, 0]), [plane(40, lambert(0.5), "floor"), ...models([0, 1, 0])]);
};

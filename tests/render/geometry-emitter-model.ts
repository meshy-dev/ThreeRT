// A model's triangles that only emit stay in its mesh, and each placement's are in the light list (scene.rs
// `InstMesh::emitters`): light sampling finds them where the placement puts them, and a ray that hits one is weighed
// against that emitter. tests/render/data/panel.glb (a small grey quad, then an 8 x 8 m quad of 1 nit: the emitting
// triangles are not the mesh's first), placed twice: far away at a hundredth of its size (it holds the list's first
// entries), and facing down 1 m above a Lambert 0.5 floor. As geometry/emitter-instance: the floor below the centre
// reads 0.5 / pi x 2.989.
import { GLTFModel } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const corner = (x: number, y: number) => (x / Math.sqrt(1 + x * x)) * Math.atan(y / Math.sqrt(1 + x * x)) + (y / Math.sqrt(1 + y * y)) * Math.atan(x / Math.sqrt(1 + y * y));
const E = 4 * (1 / 2) * corner(4, 4);
const URI = "tests/render/data/panel.glb";

export const test: TestMeta = {
  id: "geometry/emitter-model", features: ["emitters", "lambert", "instancing", "gltf"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  requires: [URI],
  spp: 256,
  note: "camera under the panel at (3, 0.8, 0) looking at the floor point below its centre",
};

export default () => {
  const scene = sceneWithSky(0);
  const far = new GLTFModel(URI);
  far.position.set(900, -50, 0);
  far.scale.setScalar(0.01);
  const panel = new GLTFModel(URI);
  panel.rotation.x = Math.PI / 2;
  panel.position.y = 1;
  return result(scene, camera(scene, [3, 0.8, 0], [0, 0, 0]), [plane(40, lambert(0.5), "floor"), far, panel]);
};

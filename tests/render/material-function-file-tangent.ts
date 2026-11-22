// A tangent a file gives is the one a material's function sees. tests/render/data/tangent-quads.glb: two quads of one
// material, their UVs' u along +x; the right one has TANGENT +y at every vertex, the left one none (its frame is
// made from its UVs: +x). The function emits 5 nits where the tangent it is given points along y and 2 where it
// does not.
import { MaterialFunction, GLTFModel } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const URI = "tests/render/data/tangent-quads.glb";

export const test: TestMeta = {
  id: "material/function-file-tangent", features: ["material-function", "gltf", "instancing"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 2.5, tolerance: 0.02 },
  requires: [URI],
  note: "right quad (the file's tangent, +y) over left quad (no tangent in the file: +x from its UVs): 5 / 2",
};

const ALONG_Y = new MaterialFunction("tangent_along_y", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(select(2.0, 5.0, abs(i.t.y) > 0.7));`,
});

export default () => {
  const scene = sceneWithSky(0);
  return result(scene, camera(scene, [0, 0, 8], [0, 0, 0]), [new GLTFModel(URI, { functions: { side: ALONG_Y.with({}) } })]);
};

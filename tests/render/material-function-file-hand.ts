// The hand a file gives a tangent is the one a material's function sees. tests/render/data/tangent-hands.glb: two
// quads of one material with TANGENT +x at every vertex, the left one's hand 1 (its bitangent, cross(normal,
// tangent) x hand, points up), the right one's -1 (down). The function emits 5 nits where the bitangent it is given
// points up and 2 where it points down.
import { MaterialFunction, GLTFModel } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const URI = "tests/render/data/tangent-hands.glb";

export const test: TestMeta = {
  id: "material/function-file-hand", features: ["material-function", "gltf", "instancing"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 0.4, tolerance: 0.01 },
  requires: [URI],
  note: "right quad (hand -1: the bitangent down) over left quad (hand 1: up): 2 / 5",
};

const UP = new MaterialFunction("bitangent_up", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(select(2.0, 5.0, i.b.y > 0.0));`,
});

export default () => {
  const scene = sceneWithSky(0);
  return result(scene, camera(scene, [0, 0, 8], [0, 0, 0]), [new GLTFModel(URI, { functions: { side: UP.with({}) } })]);
};

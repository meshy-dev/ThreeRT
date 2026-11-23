// A hit's tangent is its triangle's three vertices' at the hit, not one of them. tests/render/data/tangent-blend.glb:
// a quad whose file has TANGENT +x on its two left vertices and +y on its two right ones, so across it the tangent
// turns from x to y: at a fraction s of its width it is (1 - s, s), made of unit length. The function emits
// 1 + 4 |t.y| nits: 1.6 near the left edge, 4.9 near the right.
import { MaterialFunction, GLTFModel } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const URI = "tests/render/data/tangent-blend.glb";

export const test: TestMeta = {
  id: "material/function-vertex-tangents", features: ["material-function", "gltf", "instancing"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.64, 0.4, 0.71, 0.6], b: [0.29, 0.4, 0.36, 0.6], expected: 3.097, tolerance: 0.04 },
  requires: [URI],
  note: "near the right edge (the tangent nearly +y) over near the left edge (nearly +x): one tangent a triangle would give 1",
};

const BY_Y = new MaterialFunction("tangent_by_y", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(1.0 + 4.0 * abs(i.t.y));`,
});

export default () => {
  const scene = sceneWithSky(0);
  return result(scene, camera(scene, [0, 0, 8], [0, 0, 0]), [new GLTFModel(URI, { functions: { side: BY_Y.with({}) } })]);
};

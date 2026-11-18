// A mesh placed mirrored (a negative scale) is an instance like any other; the tangent frame its material sees
// changes hand with it (pathtrace.wgsl `inst_sign`). Two planes of one geometry and one material, each a mesh of its
// own, the right one mirrored in x: the material's function emits 5 nits where the bitangent it is given points up
// and 2 where it points down. A plane's image-up direction is up however it is mirrored in x: both read the same.
import { Color, MaterialFunction, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/function-mirrored", features: ["material-function", "instancing", "mirrored"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 1, tolerance: 0.02 },
  note: "left plane as it is, right plane mirrored in x, emission by the bitangent's side: right / left = 1",
};

const UP = new MaterialFunction("bitangent_up", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(select(2.0, 5.0, i.b.y > 0.0));`,
});

export default () => {
  const scene = sceneWithSky(0);
  const material = new MeshStandardMaterial({ name: "side", color: new Color(0, 0, 0), roughness: 1 });
  material.shader = UP.with({});
  const geometry = new PlaneGeometry(1.8, 1.8);
  const planes = [-1, 1].map((x) => {
    const m = new Mesh(geometry, material);
    m.name = x < 0 ? "left" : "right";
    m.dynamicGeometry = true;
    m.position.x = x;
    m.scale.x = x < 0 ? 1 : -1;
    return m;
  });
  return result(scene, camera(scene, [0, 0, 8], [0, 0, 0]), planes);
};

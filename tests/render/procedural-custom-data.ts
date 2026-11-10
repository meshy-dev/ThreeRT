// Procedural geometry and material functions: two procedural spheres share one material whose function emits the
// object's customData (`i.custom`): the records carry their object, so the left (2 nits) and the right (5) differ.
import { MaterialFunction, MeshStandardMaterial, ProceduralMesh, proceduralSpheres } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/custom-data", features: ["procedural", "material-function", "custom-data"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.62, 0.42, 0.72, 0.58], b: [0.28, 0.42, 0.38, 0.58], expected: 2.5, tolerance: 0.02 },
  note: "left sphere customData (2, 2, 2), right (5, 5, 5), one material function: right / left = 2.5",
};

const EMIT = new MaterialFunction("proc_custom_emit", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = i.custom.rgb;`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "custom" });
  mat.shader = EMIT.with({});
  const [left, right] = [-1, 1].map((x) => {
    const o = new ProceduralMesh(proceduralSpheres([{ center: [0, 0, 0], radius: 0.8 }]), mat);
    o.position.x = x;
    return o;
  });
  left.customData = [2, 2, 2, 0];
  right.customData = [5, 5, 5, 0];
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [left, right]);
}

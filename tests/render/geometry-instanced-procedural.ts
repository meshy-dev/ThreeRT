// InstancedMesh of procedural geometry with per-instance data (ROADMAP § 6 step 2e): two copies of one box, one
// geometry, each with its own `setCustomDataAt`, which a material function emits as `i.custom.rgb` (nits): (2, 2, 2)
// and (5, 5, 5), black sky, seen from above: right / left = 2.5.
import { InstancedMesh, MaterialFunction, Matrix4, MeshStandardMaterial, proceduralBoxes } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/instanced-procedural", features: ["procedural", "instanced-mesh", "custom-data"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 2.5, tolerance: 0.02 },
  note: "procedural box copies with per-instance data (2, 2, 2) and (5, 5, 5) emitted by a material function: right / left = 2.5",
};

const EMIT = new MaterialFunction("instance_emit", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = i.custom.rgb;`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "custom" });
  mat.shader = EMIT.with({});
  const copies = new InstancedMesh(proceduralBoxes([{ min: [-0.9, -0.1, -0.9], max: [0.9, 0, 0.9] }]), mat, 2);
  copies.name = "copies";
  const m = new Matrix4();
  copies.setMatrixAt(0, m.makeTranslation(-1, 0, 0));
  copies.setMatrixAt(1, m.makeTranslation(1, 0, 0));
  copies.setCustomDataAt(0, [2, 2, 2, 0]);
  copies.setCustomDataAt(1, [5, 5, 5, 0]);
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [copies]);
}

// material/vertex-colors on an InstancedMesh (its own BLAS, the triangles in the instanced pool) whose instance colour
// is 0.5 grey: instance colour (F_TINT) and vertex colours (F_VCOLOR) both multiply the base colour: 0.5 x 0.5.
import { Color, InstancedMesh, Matrix4 } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { gradientQuad, white } from "./material-vertex-colors.ts";

export const test: TestMeta = {
  id: "material/vertex-colors-instanced", features: ["vertex-colors", "instancing", "instance-colors"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.25, tolerance: 0.01 },
  note: "vertex colours 0 .. 1 x an instance colour of 0.5: 0.25 at the centre",
};

export default function () {
  const scene = sceneWithSky(1);
  const m = new InstancedMesh(gradientQuad(), white(), 1);
  m.setMatrixAt(0, new Matrix4());
  m.setColorAt(0, new Color(0.5, 0.5, 0.5));
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [m]);
}

// `vertexColors` per material (three.js's flag): a 2 x 2 m plane of two squares, its vertex colours 0.25 everywhere, the
// left square's group drawn with a white Lambert material with vertexColors, the right one's with one without. The
// right square stays white: 1 in the uniform 1-nit sky (Bugbot on #128: every group was tinted once any material asked).
import { BufferAttribute, Mesh, PlaneGeometry } from "@threert/scene";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { white } from "./material-vertex-colors.ts";

export const test: TestMeta = {
  id: "material/vertex-colors-groups", features: ["vertex-colors", "material-groups"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.6, 0.45, 0.7, 0.55], expected: 1, tolerance: 0.02 },
  note: "vertex colours 0.25 on a plane; the right square's material has no vertexColors: 1",
};

export default function () {
  const scene = sceneWithSky(1);
  const g = new PlaneGeometry(2, 2, 2, 1);
  g.setAttribute("color", new BufferAttribute(new Float32Array(g.attributes.position.count * 3).fill(0.25), 3));
  g.addGroup(0, 6, 0);
  g.addGroup(6, 6, 1);
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [new Mesh(g, [white(), lambert(1, "plain")])]);
}

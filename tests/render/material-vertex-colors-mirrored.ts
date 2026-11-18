// material/vertex-colors on a mesh placed mirrored (a negative scale): an instance, its vertex colours on their
// vertices. The quad's colours run 0 .. 1 along its own x; mirrored in x, the world's left side is the bright one:
// half a metre left of the centre reads 0.75, half a metre right 0.25.
import { Mesh } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { gradientQuad, white } from "./material-vertex-colors.ts";

export const test: TestMeta = {
  id: "material/vertex-colors-mirrored", features: ["vertex-colors", "instancing", "mirrored"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.3, 0.4, 0.35, 0.6], b: [0.65, 0.4, 0.7, 0.6], expected: 3, tolerance: 0.03 },
  note: "a quad with vertex colours 0 .. 1 along x, mirrored in x: 0.75 at x = -0.5 over 0.25 at x = +0.5",
};

export default function () {
  const scene = sceneWithSky(1);
  const quad = new Mesh(gradientQuad(), white());
  quad.name = "quad";
  quad.dynamicGeometry = true;
  quad.scale.x = -1;
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [quad]);
}

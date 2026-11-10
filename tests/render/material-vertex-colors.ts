// three.js `vertexColors` (F_VCOLOR): a white Lambert quad facing the camera in a uniform 1-nit sky, its geometry's
// `color` attribute black on its left edge and white on its right. The colours are interpolated across the triangle in
// linear, so its centre reads 0.5 (each vertex's colour stored as sRGB 8-bit: 0 and 1 exactly).
import { BufferAttribute, Mesh, PlaneGeometry } from "@threert/scene";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/vertex-colors", features: ["vertex-colors", "lambert", "sky-light"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  note: "vertex colours 0 on the left, 1 on the right, interpolated: 0.5 at the centre",
};

/** A 2 x 2 m quad, vertex colours from its x: 0 at -1, 1 at +1. */
export function gradientQuad(): PlaneGeometry {
  const g = new PlaneGeometry(2, 2);
  const p = g.attributes.position;
  const c = new Float32Array(p.count * 3);
  for (let i = 0; i < p.count; i++) c.fill((p.getX(i) + 1) / 2, i * 3, i * 3 + 3);
  g.setAttribute("color", new BufferAttribute(c, 3));
  return g;
}

export const white = () => {
  const m = lambert(1, "white");
  m.vertexColors = true;
  return m;
};

export default function () {
  const scene = sceneWithSky(1);
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [new Mesh(gradientQuad(), white())]);
}

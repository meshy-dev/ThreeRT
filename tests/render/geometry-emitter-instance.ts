// A ray that hits an emitter of a mesh of its own is weighed against that emitter: the light list's entry at its
// instance's first plus the triangle's number (pathtrace.wgsl `emit_mis`). An 8 x 8 m panel of 1 nit, 1 m above a
// Lambert 0.5 floor: light sampling and the floor's BSDF rays both carry its light (MIS), so the floor below its
// centre reads the analytic value only with the right entry. A speck of the same material, far away, holds the
// list's first entries: the panel's instance does not start at 0. Irradiance below the centre of a parallel square
// (4 corner terms, X = Y = half side / height = 4): E = 4 (L / 2) [X / sqrt(1 + X^2) atan(Y / sqrt(1 + X^2)) + Y /
// sqrt(1 + Y^2) atan(X / sqrt(1 + Y^2))] = 2.989 L, and the floor reads 0.5 / pi x E.
import { Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const corner = (x: number, y: number) => (x / Math.sqrt(1 + x * x)) * Math.atan(y / Math.sqrt(1 + x * x)) + (y / Math.sqrt(1 + y * y)) * Math.atan(x / Math.sqrt(1 + y * y));
const E = 4 * (1 / 2) * corner(4, 4);

export const test: TestMeta = {
  id: "geometry/emitter-instance", features: ["emitters", "lambert", "dynamic-emission"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  spp: 256,
  note: "camera under the panel at (3, 0.8, 0) looking at the floor point below its centre",
};

export default () => {
  const scene = sceneWithSky(0);
  const glow = new MeshStandardMaterial({ name: "glow", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: 1, roughness: 1 });
  const speck = new Mesh(new PlaneGeometry(0.01, 0.01), glow);
  speck.name = "speck";
  speck.dynamicGeometry = true;
  speck.position.set(900, -50, 0);
  const panel = new Mesh(new PlaneGeometry(8, 8), glow);
  panel.name = "panel";
  panel.dynamicGeometry = true;
  panel.rotation.x = Math.PI / 2;
  panel.position.y = 1;
  return result(scene, camera(scene, [3, 0.8, 0], [0, 0, 0]), [plane(40, lambert(0.5), "floor"), speck, panel]);
};

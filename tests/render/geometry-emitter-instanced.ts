// An instanced mesh that emits: each copy is an instance of one mesh, its emitters a run of the light list
// (ROADMAP § 6 step 3). Four copies of a 4 x 4 m quad of 1 nit tile an 8 x 8 m panel 1 m above a Lambert 0.5 floor; a
// fifth is hidden by a matrix scaled to 0, a sixth by `count`: both stay in the list at no weight. As
// geometry/emitter-instance: the floor below the centre reads 0.5 / pi x 2.989 only when light sampling and the BSDF
// rays weigh the same entries. A speck of the same material, far away, holds the list's first entries.
import { Color, InstancedMesh, Matrix4, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const corner = (x: number, y: number) => (x / Math.sqrt(1 + x * x)) * Math.atan(y / Math.sqrt(1 + x * x)) + (y / Math.sqrt(1 + y * y)) * Math.atan(x / Math.sqrt(1 + y * y));
export const E = 4 * (1 / 2) * corner(4, 4);

export const test: TestMeta = {
  id: "geometry/emitter-instanced", features: ["emitters", "lambert", "instancing", "instanced-mesh"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  spp: 256,
  note: "camera under the panel at (3, 0.8, 0) looking at the floor point below its centre",
};

export const glow = () => new MeshStandardMaterial({ name: "glow", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: 1, roughness: 1 });
/** Copy `i` of the panel's four quads, facing down at 1 m. */
export const tile = (i: number) => new Matrix4().makeRotationX(Math.PI / 2).setPosition(i % 2 ? 2 : -2, 1, i < 2 ? -2 : 2);

export default () => {
  const scene = sceneWithSky(0);
  const material = glow();
  const speck = new Mesh(new PlaneGeometry(0.01, 0.01), material);
  speck.name = "speck";
  speck.dynamicGeometry = true;
  speck.position.set(900, -50, 0);
  const panel = new InstancedMesh(new PlaneGeometry(4, 4), material, 6);
  panel.name = "panel";
  for (let i = 0; i < 4; i++) panel.setMatrixAt(i, tile(i));
  panel.setMatrixAt(4, new Matrix4().makeScale(0, 0, 0));
  panel.setMatrixAt(5, new Matrix4().makeTranslation(0, 0.5, 0));
  panel.count = 5;
  return result(scene, camera(scene, [3, 0.8, 0], [0, 0, 0]), [plane(40, lambert(0.5), "floor"), speck, panel]);
};

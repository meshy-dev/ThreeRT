// A mesh of three materials (three.js's material array, a geometry group each): a glass square, a cutout square whose
// alpha is 1 all over and a dark opaque one, side by side, as shadow/glass-model's model has them. The geometry has the
// glass first and the opaque square last, the other way round from the mesh's BLAS (an opaque, a cutout and a glass
// geometry). A point light (1000 cd) 2 m above an albedo-0.5 Lambert plane, the glass square half way, under the
// light: the plane below reads 0.5 I / 4 / pi x 0.96 x 0.5. shadow/glass-groups-frame and shadow/glass-groups-cutout
// read the plane in the other two squares' shadows.
import { Color, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, PlaneGeometry, PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { CLEAR } from "./shadow-glass-tint.ts";

export const test: TestMeta = {
  id: "shadow/glass-groups", features: ["point-light", "transmission", "shadows", "material-groups", "cutout"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.485, 0.48, 0.515, 0.52], expected: CLEAR * 0.96 * 0.5, tolerance: 0.02 },
  spp: 256,
  note: "the glass square (0.5 x 0.5 m) at y = 1 under a light at y = 2; the camera's rays to the plane pass under the mesh",
};

export default function () {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 2, 0);
  // Three squares along x; turned so that the first (the glass) is the one under the light, x -0.25 .. 0.25, the
  // cutout left of it and the opaque one at x -1.25 .. -0.75.
  const g = new PlaneGeometry(1.5, 0.5, 3, 1);
  g.addGroup(0, 6, 0);
  g.addGroup(6, 6, 1);
  g.addGroup(12, 6, 2);
  const dark = new Color().setRGB(0.05, 0.05, 0.05);
  const pane = new MeshPhysicalMaterial({ name: "pane", color: new Color().setRGB(0.5, 0.5, 0.5), transmission: 1, ior: 1.5, roughness: 0, metalness: 0, side: "double" });
  const grille = new MeshStandardMaterial({ name: "grille", color: dark, roughness: 1, metalness: 0, alphaTest: 0.5, side: "double" });
  const frame = new MeshStandardMaterial({ name: "frame", color: dark, roughness: 1, metalness: 0, side: "double" });
  const m = new Mesh(g, [pane, grille, frame]);
  m.name = "three";
  m.rotation.set(-Math.PI / 2, 0, Math.PI);
  m.position.set(-0.5, 1, 0);
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0], 50), [l, m, plane(20, lambert(0.5))]);
}

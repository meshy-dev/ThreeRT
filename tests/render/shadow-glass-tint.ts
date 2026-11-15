// A shadow ray through tinted glass: a point light (1000 cd) 2 m above an albedo-0.5 Lambert plane, a grey glass pane
// (colour 0.5, ior 1.5, 5 cm) half way. Each of the pane's two surfaces keeps the transmission lobe's share
// (1 - 0.04 at normal incidence) times the colour: the plane under it reads 0.5 I / 4 / pi x (0.96 x 0.5)^2.
// The camera looks at the patch from beside the pane, not through it.
import { BoxGeometry, Color, Mesh, MeshPhysicalMaterial, PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const CLEAR = (0.5 * 1000) / 4 / Math.PI;
export const THROUGH = CLEAR * (0.96 * 0.5) ** 2;

export const test: TestMeta = {
  id: "shadow/glass-tint", features: ["point-light", "transmission", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: THROUGH, tolerance: 0.02 },
  spp: 256,
  note: "pane 1 x 1 m at y = 1 under a light at y = 2; the camera's ray to the patch passes the pane's edge at z = 1",
};

export default function () {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 2, 0);
  const pane = new Mesh(new BoxGeometry(1, 0.05, 1), new MeshPhysicalMaterial({ name: "pane", color: new Color().setRGB(0.5, 0.5, 0.5), transmission: 1, ior: 1.5, roughness: 0, metalness: 0 }));
  pane.name = "pane";
  pane.position.set(0, 1, 0);
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0]), [l, pane, plane(20, lambert(0.5))]);
}

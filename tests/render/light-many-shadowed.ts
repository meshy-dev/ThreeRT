// More shadow-casting local lights than the kernel sums exactly (8), so they are resampled: 24 point lights of 300 cd
// on a ring of radius 2 m, 2 m above an albedo-0.5 Lambert plane, and a black slab at 1 m over x >= 0.3 that hides
// the 10 of them with x >= 0.6 from the centre. The other 14 give L = 0.5 / pi x 14 x 300 x cos / d^2 there
// (d^2 = 8, cos = 2 / sqrt 8). Live, the lights are guided by the light tiles (pathtrace.wgsl "Light tiles").
import { BoxGeometry, Mesh, PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/many-shadowed", features: ["point-light", "lambert", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: (0.5 / Math.PI) * 14 * 300 * (2 / Math.sqrt(8)) / 8, tolerance: 0.03 },
  note: "24 shadow-casting point lights, 10 hidden by a slab: the 14 others' sum at the centre",
};

export default function () {
  const scene = sceneWithSky(0);
  const lights = Array.from({ length: 24 }, (_, k) => {
    const a = ((k + 0.5) * 15 * Math.PI) / 180;
    const l = new PointLight(0xffffff, 300, 0);
    l.name = `lamp-${k}`;
    l.radius = 0.001;
    l.position.set(2 * Math.cos(a), 2, 2 * Math.sin(a));
    return l;
  });
  const slab = new Mesh(new BoxGeometry(6, 0.05, 12), lambert(0, "slab"));
  slab.name = "slab";
  slab.position.set(3.3, 1, 0);
  return result(scene, camera(scene, [0, 1.5, 3], [0, 0, 0]), [...lights, slab, plane(20, lambert(0.5))]);
}

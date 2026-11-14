// More shadow-casting local lights than the kernel sums exactly (8), so they are resampled: 24 point lights of 300 cd
// 2 m above an albedo-0.5 Lambert plane, each 2 m from the axis: 14 on the -x side, 10 on the +x side behind a
// black slab at 1 m over x >= 0.3, which hides them from the centre. The 14 give L = 0.5 / pi x 14 x 300 x cos / d^2
// there (d^2 = 8, cos = 2 / sqrt 8; the patch's mean is 0.02 % above its centre). The slab's shadow edges are half a
// metre from the patch. 1024 samples: resampled lights leave ~2 % of noise in the patch at the tests' 64. Live, the
// light tiles guide the resampling (pathtrace.wgsl "Light tiles").
import { BoxGeometry, Mesh, PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/many-shadowed", features: ["point-light", "lambert", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: (0.5 / Math.PI) * 14 * 300 * (2 / Math.sqrt(8)) / 8, tolerance: 0.02 },
  spp: 1024,
  note: "24 shadow-casting point lights, 10 hidden by a slab: the 14 others' sum at the centre",
};

export default function () {
  const scene = sceneWithSky(0);
  // Degrees around the axis: 100 .. 260 (x <= -0.35: seen), -54 .. 54 (x >= 1.18: hidden).
  const angles = [...Array.from({ length: 14 }, (_, k) => 100 + (k * 160) / 13), ...Array.from({ length: 10 }, (_, k) => -54 + k * 12)];
  const lights = angles.map((deg, k) => {
    const a = (deg * Math.PI) / 180;
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

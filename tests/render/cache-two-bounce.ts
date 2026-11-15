// Light that reaches the camera by two bounces: a 100-nit sky, an albedo-0.5 Lambert floor, a 4 m albedo-0.5 slab
// 1 m over it, and under that a 0.1 m albedo-0.5 slab at 0.5 m seen from above. Its top sees the large slab's
// underside, which the floor lights, which the sky lights (and a rim of sky past the slab's edge). With paths of
// three bounces at most, as a real-time view's first frame traces them: 7.05 nits (the offline renderer's value).
import { BoxGeometry, Mesh } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "cache/two-bounce", features: ["sky", "lambert", "gi"], reference: "converged", temporal: false,
  metric: { kind: "radiance", region: [0.46, 0.44, 0.54, 0.56], expected: 7.05, tolerance: 0.03 },
  spp: 1024,
  note: "maxBounces 3: sky -> floor -> the large slab -> the small one",
};

export default function () {
  const scene = sceneWithSky(100);
  const roof = new Mesh(new BoxGeometry(4, 0.02, 4), lambert(0.5, "roof"));
  roof.name = "roof";
  roof.position.set(0, 1.01, 0);
  const slab = new Mesh(new BoxGeometry(0.1, 0.01, 0.1), lambert(0.5, "slab"));
  slab.name = "slab";
  slab.position.set(0, 0.495, 0);
  return result(scene, camera(scene, [0, 0.9, 0.001], [0, 0.5, 0], 40), [roof, slab, plane(400, lambert(0.5))], { maxBounces: 3 });
}

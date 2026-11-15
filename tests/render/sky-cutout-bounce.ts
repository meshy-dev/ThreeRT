// Light that a cutout lets through and a surface sends on: a 100-nit sky over an albedo-0.5 Lambert floor (radiance
// 50), a card cut out everywhere at 1 m, and under it a 0.1 m albedo-0.5 slab at 0.5 m, seen from below. Its
// underside sees only the floor: 0.5 x 50 = 25, a quarter of the sky beside it (its own shadow on the floor takes
// under 1 % of that).
import { BoxGeometry, Mesh } from "@threert/scene";
import { clearCard } from "./shadow-cutout-emitter.ts";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "sky/cutout-bounce", features: ["sky", "alpha-test", "lambert", "gi"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.47, 0.47, 0.53, 0.53], b: [0.05, 0.05, 0.2, 0.2], expected: 0.25, tolerance: 0.01 },
  spp: 256,
  note: "the slab's underside against the sky seen past the card",
};

export default function () {
  const scene = sceneWithSky(100);
  const card = clearCard(400, 400);
  card.rotation.x = -Math.PI / 2;
  card.position.set(0, 1, 0);
  const slab = new Mesh(new BoxGeometry(0.1, 0.01, 0.1), lambert(0.5, "slab"));
  slab.name = "slab";
  slab.position.set(0, 0.505, 0);
  return result(scene, camera(scene, [0, 0.1, 0.001], [0, 0.5, 0], 40), [card, slab, plane(400, lambert(0.5))]);
}

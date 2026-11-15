// The sky's light past an alpha-tested card: an albedo-0.5 Lambert plane under a uniform 100-nit sky reads 50, and a
// card over it that is cut out everywhere changes nothing. The sky's light comes by two kinds of ray that share it
// (MIS): the sky's own samples, and the rays the plane sends on.
import { clearCard } from "./shadow-cutout-emitter.ts";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "sky/cutout-clear", features: ["sky", "alpha-test", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: 50, tolerance: 0.02 },
  note: "the card 400 x 400 m at y = 1, the camera at y = 0.6 under it",
};

export default function () {
  const scene = sceneWithSky(100);
  const card = clearCard(400, 400);
  card.rotation.x = -Math.PI / 2;
  card.position.set(0, 1, 0);
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0]), [card, plane(400, lambert(0.5))]);
}

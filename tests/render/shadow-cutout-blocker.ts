// A shadow ray that passes a cutout and then meets an opaque caster: a point light 3 m above an albedo-0.5 Lambert
// plane, a black 0.3 m slab at 2 m, and under it a card that is cut out everywhere. The slab's shadow (0.9 m wide on
// the plane) stays black: its middle against the lit plane 1.1 m to the side reads 0.
import { BoxGeometry, Mesh, PointLight } from "@threert/scene";
import { clearCard } from "./shadow-cutout-emitter.ts";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "shadow/cutout-blocker", features: ["point-light", "alpha-test", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.48, 0.48, 0.52, 0.52], b: [0.86, 0.48, 0.9, 0.52], expected: 0, tolerance: 0.01 },
  note: "a shadow ray that a cutout lets through goes on in order (trace_hw.wgsl) and stops at the slab",
};

export default function () {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 3, 0);
  const slab = new Mesh(new BoxGeometry(0.3, 0.05, 0.3), lambert(0, "slab"));
  slab.name = "slab";
  slab.position.set(0, 2, 0);
  const card = clearCard(12, 12);
  card.rotation.x = -Math.PI / 2;
  card.position.set(0, 1, 0);
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0]), [l, slab, card, plane(20, lambert(0.5))]);
}

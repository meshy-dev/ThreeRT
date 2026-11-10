// Emissive light is blocked by every surface: a slab that casts no shadow (castShadow = false, which only the analytic
// lights honour) still stops the light of an emissive panel above it, as it stops every ray that could reach the panel.
// A 2 x 2 m panel (500 nits, facing down) 3 m above the ground and a black 2.4 x 2.4 m slab 1.5 m up between them:
// the ground under the slab's middle sees none of the panel (black sky, black slab: nothing bounces in), the ground
// 4 m beyond it sees all of it. Seen from the side, from under the slab.
import { BoxGeometry, Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "shadow/emitter-no-cast", features: ["emissive", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.47, 0.47, 0.53, 0.53], b: [0.45, 0.18, 0.55, 0.24], expected: 0, tolerance: 0.01 },
  note: "Before 2026-11: emitter shadow rays skipped castShadow = false geometry, so the ground under the slab got the panel's light by its NEE share.",
};

export default function () {
  const scene = sceneWithSky(0);
  const panel = new Mesh(new PlaneGeometry(2, 2), new MeshStandardMaterial({ color: 0x000000, emissive: new Color().setRGB(1, 1, 1), emissiveIntensity: 500 }));
  panel.rotation.x = Math.PI / 2;
  panel.position.set(0, 3, 0);
  const slab = new Mesh(new BoxGeometry(2.4, 0.1, 2.4), lambert(0, "slab"));
  slab.position.set(0, 1.5, 0);
  slab.castShadow = false;
  return result(scene, camera(scene, [3.5, 1, 0.001], [0, 0, 0]), [panel, slab, plane(40, lambert(0.5))]);
}

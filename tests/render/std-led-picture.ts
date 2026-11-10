// Material library: an LedScreenMaterial without plasma or LED grid shows its picture x intensity nits (white picture,
// 3 nits; the screen's centre, away from the scan sweep).
import { DataTexture, LedScreenMaterial } from "@threert/scene";
import { CENTER, camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/led-picture", features: ["material-library", "emissive"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 3, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const white = new DataTexture(1, 1, new Uint8Array([255, 255, 255, 255]), "srgb");
  return result(scene, camera(scene, [0, 2, 0.001], [0, 0, 0]), [plane(4, new LedScreenMaterial({ map: white, intensity: 3, plasma: 0, led: 0 }))]);
}

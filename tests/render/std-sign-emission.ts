// Material library: a NeonSignMaterial emits its artwork x intensity nits (white artwork, 7 nits, no unlit albedo,
// black sky, seen straight on).
import { DataTexture, NeonSignMaterial } from "@threert/scene";
import { CENTER, camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/sign-emission", features: ["material-library", "emissive"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 7, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const white = new DataTexture(1, 1, new Uint8Array([255, 255, 255, 255]), "srgb");
  return result(scene, camera(scene, [0, 2, 0.001], [0, 0, 0]), [plane(4, new NeonSignMaterial({ map: white, intensity: 7, dim: 0 }))]);
}

// Material library: a NeonSignMaterial scrolling half a repeat per second shows its artwork shifted by half at t = 1 s.
// The artwork (8 x 1: white left half, black right half) on a 2 x 1 m sign, black sky: at t = 0 the left half glows;
// at t = 1 it is the black half, so left over right = 0.
import { DataTexture, Mesh, NeonSignMaterial, PlaneGeometry } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/sign-scroll", features: ["material-library", "emissive"], reference: "analytic", temporal: false, time: 1,
  metric: { kind: "ratio", a: [0.3, 0.4, 0.42, 0.6], b: [0.58, 0.4, 0.7, 0.6], expected: 0, tolerance: 0.01 },
};

export default function () {
  const scene = sceneWithSky(0);
  const px = new Uint8Array(8 * 4);
  for (let i = 0; i < 8; i++) px.set(i < 4 ? [255, 255, 255, 255] : [0, 0, 0, 255], i * 4);
  const sign = new Mesh(new PlaneGeometry(2, 1), new NeonSignMaterial({ map: new DataTexture(8, 1, px, "srgb"), intensity: 5, scroll: [0.5, 0] }));
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [sign]);
}

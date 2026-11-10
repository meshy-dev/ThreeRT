// Material library: a blinking NeonSignMaterial is dark in the first half of each period. Two 1 Hz signs at t = 0.3 s,
// one shifted by half a period: the one in its off half over the one in its on half = 0.
import { DataTexture, Mesh, NeonSignMaterial, PlaneGeometry } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/sign-blink", features: ["material-library", "emissive"], reference: "analytic", temporal: false, time: 0.3,
  metric: { kind: "ratio", a: [0.15, 0.4, 0.35, 0.6], b: [0.65, 0.4, 0.85, 0.6], expected: 0, tolerance: 0.01 },
};

export default function () {
  const scene = sceneWithSky(0);
  const white = new DataTexture(1, 1, new Uint8Array([255, 255, 255, 255]), "srgb");
  const sign = (x: number, blinkPhase: number) => {
    const m = new Mesh(new PlaneGeometry(1.8, 1.8), new NeonSignMaterial({ map: white, intensity: 5, blink: 1, blinkPhase }));
    m.position.set(x, 0, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 0, 4], [0, 0, 0]), [sign(-1, 0), sign(1, 0.5)]);
}

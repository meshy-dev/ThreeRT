// Material library: a blinking LightPanelMaterial is dark in the off half of its period (time 0: phase 0.25 off,
// phase 0.75 on), so the off panel over the on panel is 0.
import { LightPanelMaterial, Mesh, PlaneGeometry } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/light-blink", features: ["material-library", "emissive"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.65, 0.4, 0.85, 0.6], b: [0.15, 0.4, 0.35, 0.6], expected: 0, tolerance: 0.01 },
};

export default function () {
  const scene = sceneWithSky(0);
  const panel = (x: number, phase: number) => {
    const m = new Mesh(new PlaneGeometry(1.8, 1.8), new LightPanelMaterial({ color: [1, 1, 1], intensity: 5, blink: true, rate: 1, phase }));
    m.rotation.x = -Math.PI / 2;
    m.position.set(x, 0, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 4, 0.001], [0, 0, 0]), [panel(-1, 0.75), panel(1, 0.25)]);
}

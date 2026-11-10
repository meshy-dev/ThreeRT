// Sky atmosphere at the horizon: half the sky, as UE's sky light cubemap gives it (bilinear between the first texel row
// above the horizon and the black one below).
import { DirectionalLight, Scene, SkyAtmosphere } from "@threert/scene";
import { camera, result, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "sky/horizon-fade", features: ["sky-atmosphere", "sky-light"], reference: "ue", temporal: false,
  // 4 deg lens looking level along -Z (a cube face's centre): rows 0.49..0.51 straddle the horizon (+-0.04 deg),
  // rows 0.33..0.37 are 0.5..0.7 deg above it, past the fade.
  metric: { kind: "ratio", a: [0.3, 0.49, 0.7, 0.51], b: [0.3, 0.33, 0.7, 0.37], expected: 0.5, tolerance: 0.06 },
  note: "SkyLight CubemapResolution 128: one texel row spans 0.9 deg at a side face's centre, the horizon sits between rows",
};

export default function () {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 80000);
  sun.setSunAngles(38, 145);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });
  return result(scene, camera(scene, [0, 1.7, 0], [0, 1.7, -100], 4));
}

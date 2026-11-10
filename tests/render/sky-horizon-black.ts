// Sky atmosphere below the horizon: black past the sky light's fade, as in UE (lower hemisphere black).
import { DirectionalLight, Scene, SkyAtmosphere } from "@threert/scene";
import { camera, result, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "sky/horizon-black", features: ["sky-atmosphere", "sky-light"], reference: "ue", temporal: false,
  // 4 deg lens looking level along -Z (a cube face's centre): rows 0.25..0.35 are 0.6..1.0 deg above the horizon,
  // 0.65..0.75 as far below it.
  metric: { kind: "ratio", a: [0.3, 0.65, 0.7, 0.75], b: [0.3, 0.25, 0.7, 0.35], expected: 0, tolerance: 0.01 },
  note: "UE's sky light keeps the lower hemisphere black; its 128-texel cubemap fades over half a row (0.45 deg here) only",
};

export default function () {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 80000);
  sun.setSunAngles(38, 145);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });
  return result(scene, camera(scene, [0, 1.7, 0], [0, 1.7, -100], 4));
}

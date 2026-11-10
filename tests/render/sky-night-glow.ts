// The night sky's glow on the atmosphere (SkyAtmosphere glow + horizonGlow): the sun 40 degrees down (the air alone
// is black), 1 nit of glow everywhere above the horizon and a light dome of 2 nits more at the horizon, falling to 1/e
// by 0.1 rad. A narrow lens 10 degrees up reads 1 + 2 exp(-0.1745 / 0.1) = 1.349.
import { DirectionalLight, Scene, SkyAtmosphere } from "@threert/scene";
import { CENTER, camera, result, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "sky/night-glow", features: ["sky-atmosphere"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1.349, tolerance: 0.02 },
  note: "glow 1 + horizon glow 2 exp(-elevation / 0.1 rad) at 10 degrees up, the sun far below the horizon",
};

export default function () {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 100000).setSunAngles(-40, 0);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun, glow: 0xffffff, glowIntensity: 1, horizonGlow: 0xffffff, horizonGlowIntensity: 2, horizonGlowHeight: 0.1 });
  const up = 10 * (Math.PI / 180);
  return result(scene, camera(scene, [0, 1.7, 0], [0, 1.7 + 100 * Math.sin(up), -100 * Math.cos(up)], 2));
}

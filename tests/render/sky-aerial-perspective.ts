// The sky's aerial perspective (SkyAtmosphere aerialPerspective): a black wall 5 km away, its top at the camera's
// height (the horizon). Black, it sends nothing of its own, so what the camera sees on it is the haze between: (1 - T)
// of the sky just above the horizon, which the camera sees right above the wall. T over 5 km of air at ground level,
// Mie at 20x (8.04e-5 / m, grey) and Rayleigh (5.8 / 13.6 / 33.1e-6 / m): 1 - T = 0.35 / 0.38 / 0.43, about 0.38 in
// luminance for this sky's colour at the horizon.
import { DirectionalLight, Mesh, MeshStandardMaterial, PlaneGeometry, Scene, SkyAtmosphere } from "@threert/scene";
import { camera, result, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "sky/aerial-perspective", features: ["sky-atmosphere"], reference: "analytic", temporal: false,
  // 4 deg lens looking level along -Z: rows 0.60..0.75 are the wall (0.4..1 deg below the horizon), rows 0.30..0.45 the
  // sky 0.2..0.8 deg above it (seen at the haze's own elevation, 2.3 deg).
  metric: { kind: "ratio", a: [0.3, 0.6, 0.7, 0.75], b: [0.3, 0.3, 0.7, 0.45], expected: 0.38, tolerance: 0.03 },
  note: "1 - exp(-sigma * 5 km), sigma = Rayleigh + 20x Mie at sea level; the wall's rays reach 87 m below the camera, +7 % Mie density",
};

export default function () {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 80000);
  sun.setSunAngles(35, 160);
  scene.add(sun);
  const sky = new SkyAtmosphere({ sun, aerialPerspective: 1 });
  sky.mieScale = 20;
  scene.environment = sky;
  const wall = new Mesh(new PlaneGeometry(40000, 2000), new MeshStandardMaterial({ color: 0x000000, roughness: 1 }));
  wall.position.set(0, 1.7 - 1000, -5000);
  scene.add(wall);
  return result(scene, camera(scene, [0, 1.7, 0], [0, 1.7, -100], 4));
}

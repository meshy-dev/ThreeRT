// The standard library's simulated smoke (`SimulatedSmoke`: advection, a pressure relaxation and buoyancy on a density
// grid; ROADMAP § 6 step 5b): a puff of hot black smoke (radius 0.5 m) low in a 4 x 4 x 2 m box of still air, in front
// of a 2-nit wall; no diffusion, cooling or thinning, so buoyancy alone moves it. It rises: after 1 s it has left
// where it started (y -1.2 m) and dims the wall 0.9 m higher. The wall's radiance where it started over that 0.9 m
// higher is 2.46 (this renderer's own run: a regression value); at the start it is 0.29, and smoke that does not rise
// stays there.
import { BoxGeometry, Mesh, MeshStandardMaterial, SimulatedSmoke } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/grid-smoke", features: ["volume", "density-grid"], reference: "converged", temporal: false,
  // Camera 7 m from the box's front, 30 deg vertical fov: 1 m is 0.2666 of the image's height. The start: y -1.2 m; above: y -0.3.
  metric: { kind: "ratio", a: [0.47, 0.77, 0.53, 0.87], b: [0.47, 0.53, 0.53, 0.63], expected: 2.46, tolerance: 0.5 },
  spp: 256,
  time: 1,
  note: "box 4 x 4 x 2 m at the origin, wall at z = -2, camera at z = 8",
};

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(12, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -2);
  const smoke = new SimulatedSmoke({ size: [32, 32, 16], density: 3, albedo: [0, 0, 0], turbulence: 0, diffusion: 0, viscosity: 0, cooling: 0, dissipation: 0, drag: 0 });
  smoke.scale.set(4, 4, 2);
  smoke.puff([0, -1.2, 0], 0.5);
  return result(scene, camera(scene, [0, 0, 8], [0, 0, 0]), [wall, smoke]);
}

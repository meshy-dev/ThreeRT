// A sun with no angular size (a point direction, angularDiameter 0, as three.js lights are): a white Lambertian wall
// (no specular) lit by 4π lux at 60 degrees from its normal reads 2 nits. The kernel samples a directional light's
// cone; with the cone's cosine at 1 in f32 its pdf was infinite and the light NaN (black).
import { BoxGeometry, DirectionalLight, Mesh, MeshPhysicalMaterial } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "light/sun-point", features: ["directional-light", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshPhysicalMaterial({ name: "wall", color: 0xffffff, roughness: 1, specularIntensity: 0 }));
  wall.position.set(0, 0, -2);
  const sun = new DirectionalLight(0xffffff, 4 * Math.PI);
  sun.angularDiameter = 0;
  sun.position.set(Math.sin(Math.PI / 3), 0, Math.cos(Math.PI / 3));
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, sun]);
}

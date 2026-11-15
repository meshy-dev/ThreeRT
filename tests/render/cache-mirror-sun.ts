// A sunlit surface seen in a mirror: an albedo-0.5 Lambert ground under a sun of 2 pi lux on it (45 degrees up,
// along the mirror, which neither shades nor lights the ground), no sky, and a 2 x 1 m mirror that shows it: 1 nit.
// In the real-time renderer the surface in the mirror takes the sun's light afresh and nothing of it from the
// radiance cache. (The ground measured is 10 m and more from the mirror: the cache, which holds no directions,
// takes a mirror for a diffuse reflector of what falls on it, and close by that lights the ground 5 % more.)
import { BoxGeometry, Color, DirectionalLight, Mesh, MeshStandardMaterial } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "cache/mirror-sun", features: ["directional-light", "lambert", "mirror"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.4, 0.36, 0.6, 0.41], expected: 1, tolerance: 0.02 },
  note: "the region is the ground's image in the mirror, 1.5 to 3 degrees under the horizon's",
};

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(2, 1, 0.1), new MeshStandardMaterial({ name: "mirror", color: new Color().setRGB(1, 1, 1), metalness: 1, roughness: 0 }));
  wall.name = "wall";
  wall.position.set(0, 0.5, -1.05);
  const sun = new DirectionalLight(0xffffff, 2 * Math.SQRT2 * Math.PI);
  sun.position.set(1, 1, 0);
  return result(scene, camera(scene, [0, 0.5, 1], [0, 0.3, -1]), [wall, sun, plane(400, lambert(0.5, "ground"), "ground")], { energyCompensation: 1 });
}

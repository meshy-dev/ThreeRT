// Light scattered in water: a 50 cd lamp 3 m above a pool of water (extinction 1.2 / m, albedo 0.5, 1.5 m deep, calm)
// over a dark floor (albedo 0.13), seen from above at 50 degrees. Nearly all the light from the middle of the view is
// what the water scatters (with an absorbing body it reads 0.0004): 0.1335 offline (64 bounces, 4096 spp; this test
// guards it).
import { Mesh, MeshStandardMaterial, PlaneGeometry, PointLight, WaterBody } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "water/scatter", features: ["transmission", "refraction", "volume", "point-light"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.3, 0.3, 0.7, 0.7], expected: 0.1335, tolerance: 0.03 },
  spp: 4096,
  note: "the offline multiple-scattering reference (no analytic value)",
};

export default function () {
  const scene = sceneWithSky(0);
  const water = new WaterBody({ width: 20, length: 20, depth: 1.5, extinction: 1.2, albedo: [0.5, 0.5, 0.5], waves: { steepness: 0 } });
  const floor = new Mesh(new PlaneGeometry(20, 20), new MeshStandardMaterial({ color: 0x202020, roughness: 1, metalness: 0 }));
  floor.rotation.x = -Math.PI / 2;
  floor.position.y = -1.5;
  const lamp = new PointLight(0xffffff, 50);
  lamp.position.set(0, 3, 0);
  lamp.radius = 0.05;
  scene.add(lamp);
  return result(scene, camera(scene, [0, 2.5, 3], [0, -1, 0]), [water, floor], { maxBounces: 64 });
}

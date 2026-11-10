// fog/transmittance with a lit wall instead of an emissive one: a white Lambertian wall (no specular) 50 m away, lit by
// 4π lux at 60 degrees from its normal (2 nits; height fog does not dim light on its way to surfaces), behind fog of
// density 0.02 / m: 2 e^(-0.02 x 50) = 0.7358 nits. Live, a lit surface's direct light is a signal of its own: it must
// carry the fog's transmittance as emission does.
import { DirectionalLight, ExponentialHeightFog, Mesh, MeshPhysicalMaterial, PlaneGeometry } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "fog/transmittance-lit", features: ["fog", "directional-light"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-0.02 * 50), tolerance: 0.03 },
};

export default function () {
  const scene = sceneWithSky(0);
  const fog = new ExponentialHeightFog(0.02, 0, 0x000000);
  fog.height = -1000;
  scene.fog = fog;
  const wall = new Mesh(new PlaneGeometry(100, 100), new MeshPhysicalMaterial({ color: 0xffffff, roughness: 1, specularIntensity: 0 }));
  wall.position.set(0, 0, -50);
  const sun = new DirectionalLight(0xffffff, 4 * Math.PI);
  sun.position.set(Math.sin(Math.PI / 3), 0, Math.cos(Math.PI / 3));
  return result(scene, camera(scene, [0, 0, 0], [0, 0, -1]), [wall, sun]);
}

// Height fog with no height falloff and no in-scattering: a 100-nit wall 50 m away behind fog of density 0.02 / m shows
// 100 e^(-0.02 x 50) = 36.8 nits.
import { Color, ExponentialHeightFog, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "fog/transmittance", features: ["fog"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 100 * Math.exp(-0.02 * 50), tolerance: 0.03 },
};

export default function () {
  const scene = sceneWithSky(0);
  const fog = new ExponentialHeightFog(0.02, 0, 0x000000);
  fog.height = -1000;
  scene.fog = fog;
  const wall = new Mesh(new PlaneGeometry(100, 100), new MeshStandardMaterial({ color: 0x000000, emissive: new Color().setRGB(1, 1, 1), emissiveIntensity: 100 }));
  wall.position.set(0, 0, -50);
  return result(scene, camera(scene, [0, 0, 0], [0, 0, -1]), [wall]);
}

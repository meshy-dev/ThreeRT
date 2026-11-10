// White furnace, rough metal: with energy compensation on, an F0 = 1 metal sphere (roughness 0.5) in a uniform 1-nit
// sky reflects ~1 nit (multiple-scattering GGX). With UE 5.8 DefaultLit parity (compensation 0) it would be darker.
import { Color, Mesh, MeshStandardMaterial, SphereGeometry } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "furnace/metal-energy-compensated", features: ["ggx", "metal", "energy-conservation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.03 },
};

export default function () {
  const scene = sceneWithSky(1);
  const s = new Mesh(new SphereGeometry(0.5, 64, 48), new MeshStandardMaterial({ color: new Color().setRGB(1, 1, 1), metalness: 1, roughness: 0.5 }));
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [s], { energyCompensation: 1 });
}

// A 60 cd lamp at the centre of a sphere of medium (extinction 2 / m, albedo 0.9, radius 1.2 m): light reaches the
// camera mostly through multiple scattering. The pixel whose ray passes 0.6 m above the lamp reads 6.72 offline (full
// multiple scattering, 64 bounces; this test guards it). Converged at 32768 spp, 2026-11-14: 6.70-6.74 (white noise
// and Sobol); the 6.607 it had came unconverged from the original repository (its first build here reads 6.697).
import { PointLight, VolumeMesh } from "@threert/scene";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const yv = 0.5 - 0.12 / Math.tan((15 * Math.PI) / 180) / 2;

export const test: TestMeta = {
  id: "volume/lamp-inside", features: ["volume", "point-light"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, yv - 0.02, 0.52, yv + 0.02], expected: 6.72, tolerance: 0.03 },
  spp: 4096,
  note: "the offline multiple-scattering reference (no analytic value)",
};

export default function () {
  const scene = sceneWithSky(0);
  const s = new VolumeMesh({ shape: "sphere", volume: UNIFORM.with({ Density: 2, Albedo: [0.9, 0.9, 0.9, 0] }), majorant: 2 });
  s.scale.set(2.4, 2.4, 2.4);
  const light = new PointLight(0xffffff, 60, 0);
  light.radius = 0.01;
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [s, light], { maxBounces: 64 });
}

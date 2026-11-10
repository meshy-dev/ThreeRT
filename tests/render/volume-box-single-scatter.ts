// In-scattering from a point light: an isotropic scattering box (extinction 0.1/m, albedo 1) 4 x 1 x 2 m at the
// origin, a 2000 cd point light 2 m above its centre, the camera looking through it along z. With one scattering event
// (maxBounces 0) the centre pixel reads the single-scattering integral along the ray, attenuated on both legs:
// L = int_{-1}^{1} s / 4pi x I / r^2 x exp(-s (1 - z)) x exp(-s r / 4) dz, r^2 = 4 + z^2 (a quarter of the light's
// leg is inside the box).
import { PointLight, VolumeMesh } from "@threert/scene";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const s = 0.1, I = 2000;
const f = (z: number) => { const r = Math.sqrt(4 + z * z); return (s / (4 * Math.PI)) * (I / (r * r)) * Math.exp(-s * (1 - z)) * Math.exp((-s * r) / 4); };
let L = 0;
const n = 2000;
for (let k = 0; k < n; k++) L += f(-1 + (2 * (k + 0.5)) / n) * (2 / n);

export const test: TestMeta = {
  id: "volume/box-single-scatter", features: ["volume", "point-light"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: L, tolerance: 0.03 },
  spp: 1024,
  note: "single scattering only (maxBounces 0), integral evaluated numerically",
};

export default function () {
  const scene = sceneWithSky(0);
  const box = new VolumeMesh({ volume: UNIFORM.with({ Density: s, Albedo: [1, 1, 1, 0] }), majorant: s });
  box.scale.set(4, 1, 2);
  const light = new PointLight(0xffffff, I, 0);
  light.radius = 0.001;
  light.position.set(0, 2, 0);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [box, light], { maxBounces: 0 });
}

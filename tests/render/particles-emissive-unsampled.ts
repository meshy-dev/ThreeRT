// An emissive particle without emitterArea is not sampled as a light: the floor's BSDF rays are the only way its light
// arrives, with full weight even when other emitters are light-sampled (an emissive panel under the floor here). A 40 x
// 40 cm quad of 100 nits 1 m above an albedo-0.5 floor: E = 4 x 50 x corner(0.2, 0.2) = 15.19 lux, 2.418 nits.
import { BoxGeometry, Mesh, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const corner = (x: number, y: number) => (x / Math.sqrt(1 + x * x)) * Math.atan(y / Math.sqrt(1 + x * x)) + (y / Math.sqrt(1 + y * y)) * Math.atan(x / Math.sqrt(1 + y * y));
const E = 4 * (100 / 2) * corner(0.2, 0.2);

export const test: TestMeta = {
  id: "particles/emissive-unsampled", features: ["particles", "emitters", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  spp: 4096,
  note: "as particles/emissive-spark, a larger dimmer quad without emitterArea (BSDF rays only: many samples)",
};

const PANEL = new ParticleFunction("test_panel", {
  code: `
    o.center = vec3<f32>(0.0, 1.0, 0.0);
    o.u = vec3<f32>(0.2, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, 0.2);`,
});

export default function () {
  const scene = sceneWithSky(0);
  const glow = new MeshStandardMaterial({ name: "panel", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 100 });
  const under = new Mesh(new BoxGeometry(0.3, 0.02, 0.3), new MeshStandardMaterial({ name: "under", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 50 }));
  under.position.set(0, -1, 0);
  return result(scene, camera(scene, [2, 1.5, 0], [0, 0, 0]), [plane(20, lambert(0.5)), new Particles({ count: 1, emitter: PANEL.with({}), material: glow }), under]);
}

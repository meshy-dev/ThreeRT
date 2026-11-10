// An emissive particle lights the floor by the analytic amount: a 10 x 10 cm quad of 1000 nits facing down, 1 m above an
// albedo-0.5 Lambert floor. Irradiance right below a parallel square (4 corner terms, E = L/2 [X/sqrt(1+X^2) atan(Y/
// sqrt(1+X^2)) + Y/sqrt(1+Y^2) atan(X/sqrt(1+Y^2))], X = Y = 0.05) is 9.9668 lux, so the floor reads 0.5/pi x 9.9668
// = 1.5863 nits. The particle is sampled as a light (emitterArea), and is also what the floor's BSDF rays find (MIS). An
// emissive panel under the floor takes part of the light sampling without reaching the region (the floor shadows it).
import { BoxGeometry, Mesh, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const corner = (x: number, y: number) => (x / Math.sqrt(1 + x * x)) * Math.atan(y / Math.sqrt(1 + x * x)) + (y / Math.sqrt(1 + y * y)) * Math.atan(x / Math.sqrt(1 + y * y));
const E = 4 * (1000 / 2) * corner(0.05, 0.05);

export const test: TestMeta = {
  id: "particles/emissive-spark", features: ["particles", "emitters", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  spp: 256,
  note: "camera at (2, 1.5, 0) looking at the floor point below the quad (the quad is out of view)",
};

export const SPARK = new ParticleFunction("test_spark", {
  code: `
    o.center = vec3<f32>(0.0, 1.0, 0.0);
    o.u = vec3<f32>(0.05, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, 0.05);`,
});

export function sparkScene(emitterArea: number) {
  const scene = sceneWithSky(0);
  const glow = new MeshStandardMaterial({ name: "spark", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 1000 });
  const p = new Particles({ count: 1, emitter: SPARK.with({}), material: glow, emitterArea });
  const under = new Mesh(new BoxGeometry(0.3, 0.02, 0.3), new MeshStandardMaterial({ name: "under", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 50 }));
  under.position.set(0, -1, 0);
  return result(scene, camera(scene, [2, 1.5, 0], [0, 0, 0]), [plane(20, lambert(0.5)), p, under]);
}

export default () => sparkScene(0.01);

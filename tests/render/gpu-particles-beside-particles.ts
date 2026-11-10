// Stateful and stateless particles in one scene (ROADMAP § 6 step 5a): `gpu-particles/plane`'s particle resting on the
// plane, beside a `Particles` node whose particle function places a quad elsewhere (x = -1.5 m). Both passes run, the
// stateless one rebuilding the dynamic BLAS for both: the GPU particle still reads 5 nits where it rests.
import { MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { type TestMeta } from "./lib.ts";
import { test as plane } from "./gpu-particles-plane.ts";
import scene from "./gpu-particles-plane.ts";

export const test: TestMeta = { ...plane, id: "gpu-particles/beside-particles", note: "as gpu-particles/plane, with a stateless particle beside it" };

const AT = new ParticleFunction("test_beside", {
  code: `
    o.center = vec3<f32>(-1.5, 0.5, 0.0);
    o.u = vec3<f32>(0.25, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.25, 0.0);`,
});

export default function () {
  const r = scene();
  const mat = new MeshStandardMaterial({ name: "glow2", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
  r.scene.add(new Particles({ count: 1, emitter: AT.with(), material: mat }));
  return r;
}

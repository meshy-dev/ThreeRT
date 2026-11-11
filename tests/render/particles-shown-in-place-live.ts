// Particles shown in place (world.rs `set_visible` on a particle node that is not light-sampled, gpu.rs `set_particles_hidden`):
// two systems of one 1 x 1 m quad emitting 5 nits over a black sky, the left one always shown; the right one starts
// hidden (in the build, idle) and is shown by the frame script after a few frames, without a rebuild. Right / left 1.
import { MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/shown-in-place-live", features: ["particles", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "ratio", a: [0.66, 0.42, 0.8, 0.58], b: [0.2, 0.42, 0.34, 0.58], expected: 1, tolerance: 0.02 },
  live: 60,
  note: "a particle system hidden at build, shown in place: it draws as the other, right / left 1",
};

const AT = new ParticleFunction("test_place_vis", {
  params: { X: 0 },
  code: `
    o.center = vec3<f32>($X, 0.0, 0.0);
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, -0.5);`,
});

let elapsed = 0;
let flipped = false;
let right: Particles;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    flipped = false;
    const scene = sceneWithSky(0);
    const mat = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
    const left = new Particles({ count: 1, emitter: AT.with({ X: -1 }), material: mat });
    right = new Particles({ count: 1, emitter: AT.with({ X: 1 }), material: mat });
    right.visible = false;
    return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [left, right]);
  },
  frame(f) {
    elapsed += f.dt;
    if (flipped || elapsed <= 3 / 60) return;
    flipped = true;
    right.visible = true;
  },
  handlers: {},
});

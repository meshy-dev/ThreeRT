// Particle motion vectors survive a repose: two movable boxes that the frame script moves every frame sit before and
// after a particle node in the dynamic triangles, so each repose re-uploads a span covering the particles. The quad
// (5 nits, 1 m from its node) holds still: with motion blur on it must stay sharp (5 nits). Motion vectors taken from
// the re-uploaded placeholders (all at the node) would blur it by 1 m.
import { BoxGeometry, Mesh, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/motion-repose", features: ["particles", "movable", "motion-blur"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.66, 0.42, 0.8, 0.58], expected: 5, tolerance: 0.02 },
  live: 30,
  note: "boxes at x = -1.6, z = -1 and z = +1 bob every frame; the particle node at the origin draws its quad at x = +1",
};

const OFFSET = new ParticleFunction("test_offset", {
  code: `
    o.center = i.pivot + vec3<f32>(1.0, 0.0, 0.0);
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, -0.5);`,
});

let t = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const box = (name: string, z: number) => {
      const m = new Mesh(new BoxGeometry(0.3, 0.3, 0.3), lambert(0.5, name));
      m.name = name;
      m.movable = true;
      m.castShadow = false;
      m.position.set(-1.6, 0, z);
      return m;
    };
    const glow = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
    // Build order (the dynamic triangles' order within their group): box, particles, box.
    const r = result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [box("a", -1), new Particles({ count: 1, emitter: OFFSET.with({}), material: glow }), box("b", 1)]);
    return { ...r, options: { ...r.options, overrides: { ...r.options.overrides, post: { motionBlur: { amount: 1, max: 10, targetFps: 60 } } } } };
  },
  frame(f) {
    t += 1;
    f.object("a")!.position.y = 0.2 * Math.sin(t);
    f.object("b")!.position.y = 0.2 * Math.cos(t);
  },
  handlers: {},
});

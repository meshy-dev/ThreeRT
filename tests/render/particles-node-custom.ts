// A particle node's customData reaches its particle function as `i.custom`, live: the quad shows only while
// i.custom.x > 0.5, and the frame script sets the node's customData to [1, 0, 0, 0] after a few rendered frames (a game
// firing a burst). The quad's region reads 5 nits; 0 if the value never reached the particle pass.
import { MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/node-custom", features: ["particles", "custom-data"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.66, 0.42, 0.8, 0.58], expected: 5, tolerance: 0.02 },
  live: 30,
  note: "as particles/quad-placement, the quad hidden until the frame script sets customData",
};

const GATED = new ParticleFunction("test_gated", {
  code: `
    o.center = vec3<f32>(1.0, 0.0, 0.0);
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, -0.5);
    o.visible = i.custom.x > 0.5;`,
});

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const glow = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
    const p = new Particles({ count: 1, emitter: GATED.with({}), material: glow });
    p.name = "gated";
    return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [p]);
  },
  frame(f) {
    f.object("gated")!.customData = f.time > 3 / 60 ? [1, 0, 0, 0] : [0, 0, 0, 0];
  },
  handlers: {},
});

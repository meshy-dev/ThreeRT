// Particles follow their emitter: the particle function places its quad at `i.pivot`, the node's world position. The
// node hangs under a movable group that the frame script moves from x = -1 to x = +1 m after a few rendered frames, so
// the quad must show at +1 (5 nits there; 0 if the pivot stayed where the scene was built).
import { Group, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/follow-emitter", features: ["particles", "movable"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.66, 0.42, 0.8, 0.58], expected: 5, tolerance: 0.02 },
  live: 30,
  note: "as particles/quad-placement, the quad's node moved through its parent group in the frame script",
};

const AT = new ParticleFunction("test_follow", {
  code: `
    o.center = i.pivot;
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, -0.5);`,
});

// Frame time so far: the game's state is applied (a frame at dt 0) before the scene is built, so the move waits for
// rendered frames.
let elapsed = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const rig = new Group();
    rig.name = "rig";
    rig.movable = true;
    rig.position.set(-1, 0, 0);
    const glow = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
    rig.add(new Particles({ count: 1, emitter: AT.with({}), material: glow }));
    return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [rig]);
  },
  frame(f) {
    elapsed += f.dt;
    f.object("rig")!.position.x = elapsed > 3 / 60 ? 1 : -1;
  },
  handlers: {},
});

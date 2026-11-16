// Shared poses follow the pose: two mannequins built in one pose (A_TPose) share one skinned mesh (scene::PoseGroup).
// From 0.08 s the frame script poses both Idle_Loop at 0.5 s, so the shared mesh is re-skinned live: the region where
// the T pose's arm was (screen left, the forearm) shows the 1-nit sky behind it, and the chest is still there. The
// ratio of the two: 2.13 in the offline render of this pose (skinning-pose.ts: the chest 0.47 nit under the sky); a
// mesh left in its build pose keeps the arm there (about 1.1), a model that is no longer drawn shows sky in both
// (1.0: its mesh built apart from the TLAS lost its instance, 2026-11-16). The second mannequin stands behind the
// camera.
import { GLTFModel } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "animation/shared-pose-live", features: ["skinning", "animation", "movable"], reference: "converged", temporal: true,
  metric: { kind: "ratio", a: [0.33, 0.45, 0.39, 0.48], b: [0.47, 0.5, 0.53, 0.58], expected: 2.13, tolerance: 0.3 },
  requires: ["assets/quaternius/ual/UAL1_Standard.glb"],
  live: 30,
  note: "both built in A_TPose (one group), both posed Idle_Loop at 0.5 s by the frame script; the other at z = 8",
};

const URI = "assets/quaternius/ual/UAL1_Standard.glb";

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(1);
    const model = (id: string, z: number) => {
      const m = new GLTFModel(URI, { animation: { clip: "A_TPose", time: 0 } });
      m.id = id;
      m.movable = true;
      m.position.set(0, 0, z);
      return m;
    };
    return result(scene, camera(scene, [0, 1.35, 4], [0, 1.35, 0], 35), [model("a", 0), model("b", 8)]);
  },
  frame(f) {
    // The scene is built at time 0 (the job seeks the frame script there first): A_TPose. From 0.08 s, Idle.
    const pose = f.time < 0.08 ? { clip: "A_TPose", time: 0 } : { clip: "Idle_Loop", time: 0.5 };
    for (const id of ["a", "b"]) (f.object(id) as GLTFModel).animation = pose;
  },
  handlers: {},
});

// Procedural geometry moves without a rebuild: an emissive procedural box (2 nits) that the frame script moves after a
// few frames from x = -4 (out of view) to the origin, in front of a black wall. The centre reads 2; 0 if it stayed where
// the scene was built.
import { MeshStandardMaterial, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/follow-node-live", features: ["procedural", "movable"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.02 },
  live: 60,
  note: "a 2-nit procedural box moved into view by the frame script (its record and TLAS instance, no rebuild)",
};

let elapsed = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const box = new ProceduralMesh(proceduralBoxes([{ min: [-1, -1, -0.1], max: [1, 1, 0.1] }]), new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
    box.name = "box";
    box.position.set(-4, 0, 0);
    return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [box]);
  },
  frame(f) {
    elapsed += f.dt;
    f.object("box")!.position.x = elapsed > 3 / 60 ? 0 : -4;
  },
  handlers: {},
});

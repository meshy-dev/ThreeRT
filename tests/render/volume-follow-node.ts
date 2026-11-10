// A medium follows its node: an absorbing box (1 m, extinction 1/m) that the frame script moves after a few frames
// from x = -3 (off the view axis) to the origin, in front of a 2-nit wall. The centre reads 2 e^-1 = 0.7358; 2 if the
// medium stayed where the scene was built.
import { BoxGeometry, Mesh, MeshStandardMaterial, VolumeMesh } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/follow-node", features: ["volume", "movable"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1), tolerance: 0.02 },
  live: 60,
  note: "as volume/box-transmittance, the box moved into place by the frame script (movable)",
};

let elapsed = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
    wall.position.set(0, 0, -2);
    const box = new VolumeMesh({ volume: UNIFORM.with({ Density: 1 }), majorant: 1 });
    box.name = "box";
    box.movable = true;
    box.scale.set(2, 2, 1);
    box.position.set(-3, 0, 0);
    return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box]);
  },
  frame(f) {
    elapsed += f.dt;
    f.object("box")!.position.x = elapsed > 3 / 60 ? 0 : -3;
  },
  handlers: {},
});

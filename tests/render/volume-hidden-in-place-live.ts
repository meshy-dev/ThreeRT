// A volume hidden in place (world.rs `set_visible`: its medium keeps its place in the table with no shape, no
// rebuild): volume/box-transmittance's absorbing box before the 2-nit wall, hidden by the frame script after a few
// frames. The wall reads its 2 nits.
import { BoxGeometry, Mesh, MeshStandardMaterial, VolumeMesh } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/hidden-in-place-live", features: ["volume", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.02 },
  live: 60,
  note: "the box's own visible set false at frame 4: in place",
};

/** The wall and the box of volume/box-transmittance. */
export function wallAndBox(): [Mesh, VolumeMesh] {
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -2);
  const box = new VolumeMesh({ volume: UNIFORM.with({ Density: 1 }), majorant: 1 });
  box.scale.set(2, 2, 1);
  return [wall, box];
}

let elapsed = 0;
let box: VolumeMesh;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    const scene = sceneWithSky(0);
    const [wall, b] = wallAndBox();
    box = b;
    return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, b]);
  },
  frame(f) {
    elapsed += f.dt;
    if (elapsed > 3 / 60) box.visible = false;
  },
  handlers: {},
});

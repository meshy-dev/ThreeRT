// A density grid's data changed at run time (ROADMAP § 6 step 5b): volume/box-transmittance's scene with a grid of
// zeros, which the frame script fills with ones at its third frame (`needsUpdate`: uploaded in place, no rebuild). The
// box then reads as the uniform absorber does: 2 e^-1 = 0.7358 nits. (Not uploaded: 2.)
import { BoxGeometry, DensityGrid, Mesh, MeshStandardMaterial, VolumeMesh } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { GRID_MEDIUM } from "./volume-grid-uniform.ts";

export const test: TestMeta = {
  id: "volume/grid-data-live", features: ["volume", "density-grid", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1), tolerance: 0.02 },
  live: 240,
  note: "box 2 x 2 x 1 m at the origin, wall at z = -2, camera at z = 5",
};

let grid: DensityGrid;
let frames = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    frames = 0;
    const scene = sceneWithSky(0);
    const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
    wall.position.set(0, 0, -2);
    grid = new DensityGrid({ size: [8, 8, 8], data: new Float32Array(512) });
    const box = new VolumeMesh({ volume: GRID_MEDIUM.with({ Density: 1 }), majorant: 1, grid });
    box.scale.set(2, 2, 1);
    return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box]);
  },
  frame(f) {
    if (f.dt > 0 && ++frames === 3) {
      grid.data!.fill(1);
      grid.needsUpdate = true;
    }
  },
  handlers: {},
});

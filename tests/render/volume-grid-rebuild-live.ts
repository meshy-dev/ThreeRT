// What a grid program has simulated survives a rebuild of the live scene (gpu.rs `carry_grids`): volume/grid-program's
// grid, 1.5 a second into every cell, in the real-time renderer for 24 frames; after 12 the frame script hides a mesh
// out of view, a structural change (the scene is built again). The grid holds 0.6 as if nothing had happened, and the
// wall behind the absorber reads 2 e^-1.2 = 0.6024. (Started again at the rebuild it held 0.3: 1.098.)
import { BoxGeometry, DensityGrid, GridProgram, Mesh, MeshStandardMaterial, VolumeMesh } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { GRID_MEDIUM } from "./volume-grid-uniform.ts";

export const test: TestMeta = {
  id: "volume/grid-rebuild-live", features: ["volume", "density-grid", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1.2), tolerance: 0.03 },
  live: 24,
  note: "as volume/grid-program; a mesh behind the camera hidden at frame 12 rebuilds the scene",
};

const FILL = new GridProgram("test_fill_rebuild", {
  params: { Rate: 1 },
  code: `
    s.a.x += $Rate * i.dt;`,
});

let frames = 0;
let extra: Mesh;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    frames = 0;
    const scene = sceneWithSky(0);
    const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
    wall.position.set(0, 0, -2);
    const box = new VolumeMesh({ volume: GRID_MEDIUM.with({ Density: 2 }), majorant: 2, grid: new DensityGrid({ size: [16, 16, 8], program: FILL.with({ Rate: 1.5 }) }) });
    box.scale.set(2, 2, 1);
    extra = new Mesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ name: "extra", color: 0x808080 }));
    extra.position.set(0, 0, 20);
    return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box, extra]);
  },
  frame() {
    if (++frames === 12) extra.visible = false;
  },
  handlers: {},
});

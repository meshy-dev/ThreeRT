// A grid program run on the GPU (ROADMAP § 6 step 5b; renderer/shaders/grid_sim.wgsl): a grid of zeros whose program
// adds 1.5 a second to every cell. An offline render at 0.4 s runs it for 24 frames (two half steps each): the grid
// holds 0.6, and a pure absorber of extinction 2/m x the grid, 1 m thick, in front of a 2-nit wall reads
// 2 e^-1.2 = 0.6024 nits. (Not run, it would read 2; run twice as long, 0.18.)
import { BoxGeometry, DensityGrid, GridProgram, Mesh, MeshStandardMaterial, VolumeMesh } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { GRID_MEDIUM } from "./volume-grid-uniform.ts";

export const test: TestMeta = {
  id: "volume/grid-program", features: ["volume", "density-grid"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1.2), tolerance: 0.02 },
  spp: 256,
  time: 0.4,
  note: "box 2 x 2 x 1 m at the origin, wall at z = -2, camera at z = 5",
};

const FILL = new GridProgram("test_fill", {
  params: { Rate: 1 },
  code: `
    s.a.x += $Rate * i.dt;`,
});

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -2);
  const box = new VolumeMesh({ volume: GRID_MEDIUM.with({ Density: 2 }), majorant: 2, grid: new DensityGrid({ size: [16, 16, 8], program: FILL.with({ Rate: 1.5 }) }) });
  box.scale.set(2, 2, 1);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box]);
}

// Advection by a grid program (ROADMAP § 6 step 5b): a box 4 m wide, its grid 64 x 4 x 4 cells (0.0625 m), holding a
// block of density 1 in cells 8..19 along x (x -1.5 .. -0.75 m). The program moves the field along +x at 7.5 m/s by
// semi-Lagrangian advection (each cell takes what was upstream of it: `ga(to_uvw(i.p - v dt))`), which is one cell a
// half step exactly, so the block is not blurred. After 0.25 s (15 frames, 30 half steps) it lies in cells 38..49
// (x 0.375 .. 1.125 m): seen from the front against a 2-nit wall through 1 m of extinction 1/m, where it was reads 2
// nits and where it is 2 e^-1; their ratio is e = 2.718. (Not moved: 1 / e = 0.37.)
import { BoxGeometry, DensityGrid, GridProgram, Mesh, MeshStandardMaterial, VolumeMesh } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { GRID_MEDIUM } from "./volume-grid-uniform.ts";

export const test: TestMeta = {
  id: "volume/grid-advect", features: ["volume", "density-grid"], reference: "analytic", temporal: false,
  // Camera 8 m from the box's front, 30 deg vertical fov: 1 m there is 0.1312 of the image's width. Was: x -1.4 .. -0.85;
  // is: x 0.5 .. 0.9 at the front (rays there stay inside the block to the back: they lean outward by up to 0.11 m).
  metric: { kind: "ratio", a: [0.3163, 0.45, 0.3885, 0.55], b: [0.5656, 0.45, 0.6181, 0.55], expected: Math.E, tolerance: 0.05 },
  spp: 256,
  time: 0.25,
  note: "box 4 x 1 x 1 m at the origin, wall at z = -2, camera at z = 8.5 (8 m from the box's front)",
};

const MOVE = new GridProgram("test_move", {
  params: { Speed: 1 },
  code: `
    s.a = ga(to_uvw(i.p - vec3<f32>($Speed, 0.0, 0.0) * i.dt));`,
});

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(12, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -2);
  const [nx, ny, nz] = [64, 4, 4];
  const data = new Float32Array(nx * ny * nz);
  for (let z = 0; z < nz; z++) for (let y = 0; y < ny; y++) for (let x = 8; x < 20; x++) data[(z * ny + y) * nx + x] = 1;
  const box = new VolumeMesh({ volume: GRID_MEDIUM.with({ Density: 1 }), majorant: 1, grid: new DensityGrid({ size: [nx, ny, nz], data, program: MOVE.with({ Speed: 7.5 }) }) });
  box.scale.set(4, 1, 1);
  return result(scene, camera(scene, [0, 0, 8.5], [0, 0, 0]), [wall, box]);
}

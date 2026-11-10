// Transmittance through a density grid that varies (ROADMAP § 6 step 5b): a pure absorber 1 m thick in front of a 2-nit
// wall, its grid (8 x 4 x 16 cells) a ramp along the view (0 at the front to 1 at the back: the cell centres' values
// are (k + 0.5) / 16, trilinear between them, so the density's integral along a ray is 0.5 m exactly), three times
// denser in the box's right half. With extinction 1/m x the grid: the left half reads 2 e^-0.5 = 1.2131 nits (this
// test), the right 2 e^-1.5 = 0.4463 (volume/grid-transmittance-dense).
import { BoxGeometry, DensityGrid, Mesh, MeshStandardMaterial, VolumeMesh } from "@threert/scene";
import { camera, result, sceneWithSky, type Region, type TestMeta } from "./lib.ts";
import { GRID_MEDIUM } from "./volume-grid-uniform.ts";

/** Inside the box's left and right halves (the box spans u 0.335-0.665; the halves meet at 0.5). */
export const LEFT: Region = [0.37, 0.44, 0.45, 0.56], RIGHT: Region = [0.55, 0.44, 0.63, 0.56];

export const test: TestMeta = {
  id: "volume/grid-transmittance", features: ["volume", "density-grid"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: LEFT, expected: 2 * Math.exp(-0.5), tolerance: 0.02 },
  spp: 256,
  note: "box 2 x 2 x 1 m at the origin, wall at z = -2, camera at z = 5",
};

export function rampGrid(): DensityGrid {
  const [nx, ny, nz] = [8, 4, 16];
  const data = new Float32Array(nx * ny * nz);
  // z runs from the box's back (-0.5) to its front: the ramp is 1 at the back.
  for (let z = 0; z < nz; z++) for (let y = 0; y < ny; y++) for (let x = 0; x < nx; x++) data[(z * ny + y) * nx + x] = (1 - (z + 0.5) / nz) * (x < nx / 2 ? 1 : 3);
  return new DensityGrid({ size: [nx, ny, nz], data });
}

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -2);
  const box = new VolumeMesh({ volume: GRID_MEDIUM.with({ Density: 1 }), majorant: 3, grid: rampGrid() });
  box.scale.set(2, 2, 1);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box]);
}

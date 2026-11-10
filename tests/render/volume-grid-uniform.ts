// A density grid of one value against the analytic medium of the same coefficients (ROADMAP § 6 step 5b): on the left a
// box whose volume function reads an 8³ grid of ones (extinction 1/m x the grid, albedo 0.8), on the right a
// `HomogeneousVolume` of extinction 1/m and albedo 0.8, mirrored about the image's centre line, both 1 m thick in
// front of a 2-nit wall under a uniform 1-nit sky (so both attenuate the wall and scatter the sky): they read alike.
import { BoxGeometry, DensityGrid, HomogeneousVolume, Mesh, MeshStandardMaterial, VolumeFunction, VolumeMesh } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/grid-uniform", features: ["volume", "density-grid"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.2, 0.4, 0.3, 0.6], b: [0.7, 0.4, 0.8, 0.6], expected: 1, tolerance: 0.02 },
  spp: 1024,
  note: "boxes 2 x 2 x 1 m at x = -1.5 and 1.5, wall at z = -2, camera at z = 6",
};

/** Extinction = $Density x the grid's first number; albedo and emission constant. */
export const GRID_MEDIUM = new VolumeFunction("test_grid_medium", {
  params: { Density: 1, Albedo: [0, 0, 0, 0], Emission: [0, 0, 0, 0] },
  code: `
    o.extinction = $Density * vgrid(i, i.lp).x;
    o.albedo = $Albedo.rgb;
    o.emission = $Emission.rgb;`,
});

export default function () {
  const scene = sceneWithSky(1);
  const wall = new Mesh(new BoxGeometry(12, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -2);
  const grid = new VolumeMesh({ volume: GRID_MEDIUM.with({ Density: 1, Albedo: [0.8, 0.8, 0.8, 0] }), majorant: 1, grid: new DensityGrid({ size: [8, 8, 8], data: new Float32Array(512).fill(1) }) });
  grid.position.set(-1.5, 0, 0);
  grid.scale.set(2, 2, 1);
  const analytic = new HomogeneousVolume({ extinction: 1, albedo: [0.8, 0.8, 0.8] });
  analytic.position.set(1.5, 0, 0);
  analytic.scale.set(2, 2, 1);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [wall, grid, analytic]);
}

// A MagicaVoxel file through the voxel package in the scene runtime (encodeVox writes it, parseVox reads it back,
// voxGrid turns z-up into y-up, paletteMaterials makes its colours materials): a 6 x 1 x 6 plate (file axes) standing
// up after the turn, its left half palette entry 1 glowing 2 nits, its right half entry 2 glowing 5. Right / left 2.5.
import { encodeVox, paletteMaterials, parseVox, voxGrid, voxelMesh } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "voxel/vox-file", features: ["voxel", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.55, 0.4, 0.65, 0.6], b: [0.35, 0.4, 0.45, 0.6], expected: 2.5, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const voxels: number[] = [];
  // File axes: x right, y depth (one voxel), z up.
  for (let z = 0; z < 6; z++) for (let x = 0; x < 6; x++) voxels.push(x, 0, z, x < 3 ? 1 : 2);
  const palette = new Uint32Array(256).fill(0xffffffff);
  const file = parseVox(encodeVox([{ size: [6, 1, 6], voxels: new Uint8Array(voxels) }], palette));
  const mats = paletteMaterials(file.palette, { emissive: { 1: 2, 2: 5 } });
  for (const m of mats) m.color.setRGB(0, 0, 0);
  const plate = voxelMesh(voxGrid(file.models[0], 0.25), mats);
  plate.position.set(-0.75, -0.75, 0);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [plate]);
}

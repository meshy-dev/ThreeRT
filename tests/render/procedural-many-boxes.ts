// Many boxes in one geometry (the software traversal walks its BVH over them, hardware RT its BLAS): a 64 x 64 checker
// of emissive 2-nit tiles, 2048 boxes 1 mm deep (no lit sides off axis), filling the view head on. Half the area emits:
// the mean over the wall is 1.
import { MeshStandardMaterial, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/many-boxes", features: ["procedural", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.3, 0.2, 0.7, 0.8], expected: 1, tolerance: 0.02 },
  note: "2048 boxes as a checker over a wall: mean 1 (half of 2 nits)",
};

export default function () {
  const scene = sceneWithSky(0);
  const s = 8 / 64;
  const tiles: { min: [number, number, number]; max: [number, number, number] }[] = [];
  for (let y = 0; y < 64; y++) for (let x = 0; x < 64; x++) if ((x + y) % 2 === 0) tiles.push({ min: [-4 + x * s, -4 + y * s, -0.001], max: [-4 + (x + 1) * s, -4 + (y + 1) * s, 0] });
  const wall = new ProceduralMesh(proceduralBoxes(tiles), new MeshStandardMaterial({ name: "tile", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  return result(scene, camera(scene, [0, 0, 10], [0, 0, 0]), [wall]);
}

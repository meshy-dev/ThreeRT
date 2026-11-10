// A procedural mesh's materials: two boxes of one geometry pick materials by their data (proceduralBoxes: box k reads
// its index). The left picks 1 (2 nits), the right 7, past the end: the last (5 nits). Black sky, seen straight on.
import { MeshStandardMaterial, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/materials", features: ["procedural", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 2.5, tolerance: 0.02 },
  note: "material index 1 (2 nits) left, 7 clamped to the last (5 nits) right: right / left = 2.5",
};

const emit = (nits: number, name: string) => new MeshStandardMaterial({ name, color: 0x000000, emissive: 0xffffff, emissiveIntensity: nits });

export default function () {
  const scene = sceneWithSky(0);
  const boxes = new ProceduralMesh(proceduralBoxes([
    { min: [-1.9, -0.9, -0.1], max: [-0.1, 0.9, 0], material: 1 },
    { min: [0.1, -0.9, -0.1], max: [1.9, 0.9, 0], material: 7 },
  ]), [emit(0, "none"), emit(2, "two"), emit(5, "five")]);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [boxes]);
}

// One procedural geometry, three meshes (one BLAS, three instances with hardware ray tracing; one blob): each mesh has
// its own transform and material, the left emitting 2 nits, the right 5 (the middle one out of view). Right / left 2.5.
import { MeshStandardMaterial, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/shared-geometry", features: ["procedural", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 2.5, tolerance: 0.02 },
  note: "three meshes of one geometry: left 2 nits, right 5",
};

const emit = (nits: number, name: string) => new MeshStandardMaterial({ name, color: 0x000000, emissive: 0xffffff, emissiveIntensity: nits });

export default function () {
  const scene = sceneWithSky(0);
  const geo = proceduralBoxes([{ min: [-0.9, -0.9, -0.1], max: [0.9, 0.9, 0] }]);
  const [left, right, away] = [emit(2, "two"), emit(5, "five"), emit(9, "nine")].map((m) => new ProceduralMesh(geo, m));
  left.position.x = -1;
  right.position.x = 1;
  away.position.set(0, 0, 20);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [left, right, away]);
}

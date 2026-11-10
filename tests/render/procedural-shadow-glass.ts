// Procedural geometry in a scene with glass (F_GLASS: offline shadow rays walk through glass, `glass_hit`): a black
// procedural slab under an overhead sun still shades the ground under it completely, and a glass pane beside it lets
// the sun through to the ground below it (seen from the side, as procedural/shadow).
import { DirectionalLight, Mesh, MeshPhysicalMaterial, PlaneGeometry, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/shadow-glass", features: ["procedural", "glass", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.47, 0.47, 0.53, 0.53], b: [0.02, 0.85, 0.12, 0.95], expected: 0, tolerance: 0.005 },
};

export default function () {
  const scene = sceneWithSky(0);
  const sun = new DirectionalLight(0xffffff, 10000);
  sun.position.set(0, 1, 0);
  const slab = new ProceduralMesh(proceduralBoxes([{ min: [-1, -0.1, -1], max: [1, 0.1, 1] }]), lambert(0, "occluder"));
  slab.position.set(0, 1.5, 0);
  const pane = new Mesh(new PlaneGeometry(1, 1), new MeshPhysicalMaterial({ name: "glass", color: 0xffffff, roughness: 0, transmission: 1, ior: 1.5 }));
  pane.rotation.x = -Math.PI / 2;
  pane.position.set(0, 1.2, 3);
  return result(scene, camera(scene, [4, 2.2, 0.001], [0, 0, 0], 30), [sun, slab, pane, plane(40, lambert(0.5))]);
}

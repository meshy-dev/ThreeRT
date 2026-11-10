// Procedural geometry under a transform: an analytic box (proceduralBoxes) and a BoxGeometry, both rotated and scaled
// unevenly by their nodes (the box's normals go through the instance's inverse transpose), mirrored about the image's
// centre line, on a floor under a sun in the plane between them and a sky: the halves agree.
import { BoxGeometry, DirectionalLight, Mesh, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/box-mesh", features: ["procedural", "transform", "shadows"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0, 0, 0.5, 1], b: [0.5, 0, 1, 1], expected: 1, tolerance: 0.01 },
  note: "procedural box (left) against BoxGeometry (right), mirror images under rotation and uneven scale",
};

export default function () {
  const scene = sceneWithSky(0.3);
  const sun = new DirectionalLight(0xffffff, 3);
  sun.position.set(0, 1, 0.6);
  const mat = lambert(0.6, "crate");
  const proc = new ProceduralMesh(proceduralBoxes([{ min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] }]), mat);
  const mesh = new Mesh(new BoxGeometry(1, 1, 1), mat);
  for (const [o, side] of [[proc, -1], [mesh, 1]] as const) {
    o.position.set(1.2 * side, 0.7, 0);
    o.rotation.set(0.4, 0.5 * side, 0.3 * side);
    o.scale.set(1.3, 0.9, 0.7);
  }
  return result(scene, camera(scene, [0, 2.5, 6], [0, 0.5, 0]), [sun, proc, mesh, plane(30, lambert(0.5))]);
}

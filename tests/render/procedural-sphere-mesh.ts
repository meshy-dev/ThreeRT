// Procedural geometry against triangles: an analytic sphere (packages/scene proceduralSpheres: a geometry function
// over one box) and its triangle version (SphereGeometry 256 x 128), one material, side by side on a floor under a sun
// in the plane between them and a sky. The left half of the image (the sphere, its shadow, the floor it lights) reads
// as the right half.
import { DirectionalLight, Mesh, ProceduralMesh, SphereGeometry, proceduralSpheres } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/sphere-mesh", features: ["procedural", "shadows"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0, 0, 0.5, 1], b: [0.5, 0, 1, 1], expected: 1, tolerance: 0.01 },
  note: "procedural sphere (left) against SphereGeometry 256 x 128 (right), mirrored: the halves agree within tessellation",
};

export default function () {
  const scene = sceneWithSky(0.3);
  const sun = new DirectionalLight(0xffffff, 3);
  sun.position.set(0, 1, 0.6);
  const mat = lambert(0.6, "ball");
  const proc = new ProceduralMesh(proceduralSpheres([{ center: [-1.1, 0.8, 0], radius: 0.8 }]), mat);
  const mesh = new Mesh(new SphereGeometry(0.8, 256, 128), mat);
  mesh.position.set(1.1, 0.8, 0);
  return result(scene, camera(scene, [0, 2.5, 6], [0, 0.5, 0]), [sun, proc, mesh, plane(30, lambert(0.5))]);
}

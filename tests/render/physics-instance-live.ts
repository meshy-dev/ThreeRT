// A body's mesh moves as an instance (ROADMAP § 6 step 7): a white box of 0.5 m with a dynamic body, let go 1.5 m up
// on the left, and the same box as a plain mesh resting on the ground on the right, mirrored about the image's centre
// line, under a uniform 1-nit sky. A lit, opaque mesh that a body drives is placed as an instance of its own BLAS: each
// tick only its TLAS transform changes, no triangle. Once it has landed the two boxes read alike: ratio 1 (still in the
// air, the left region shows the sky: 1.5).
import { BoxGeometry, Mesh } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { ground } from "./physics-box-rest.ts";

export const test: TestMeta = {
  id: "physics/instance-live", features: ["physics"], reference: "analytic", temporal: true,
  // Camera 6 m away at y 0.25: the boxes at x = -1.2 and 1.2 span u 0.244-0.336 and 0.664-0.756.
  metric: { kind: "ratio", a: [0.26, 0.45, 0.32, 0.55], b: [0.68, 0.45, 0.74, 0.55], expected: 1, tolerance: 0.03 },
  live: 240,
  note: "boxes 0.5 m at x = -1.2 (a body) and 1.2 (static), ground plane y = 0",
};

export default function () {
  const scene = sceneWithSky(1);
  const white = lambert(0.8, "box");
  const [left, right] = [new Mesh(new BoxGeometry(0.5, 0.5, 0.5), white), new Mesh(new BoxGeometry(0.5, 0.5, 0.5), white)];
  left.position.set(-1.2, 1.5, 0);
  right.position.set(1.2, 0.25, 0);
  left.physics = "dynamic";
  return result(scene, camera(scene, [0, 0.25, 6], [0, 0.25, 0]), [left, right, plane(30, lambert(0.3)), ground()]);
}

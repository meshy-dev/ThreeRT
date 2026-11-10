// A hinge with a limit (ROADMAP § 6 step 7): a bar of 1 x 0.3 x 0.4 m held at one end by a `HingeJoint` to a fixed
// post, its axis toward the camera, limited to ±0.5 rad. Let go level, it swings down and rests against the limit, 0.5
// rad below level. Three quarters along it is then at (0.658, -0.360) m from the hinge: the region there reads the
// bar's 5 nits (still level, or hanging straight down without the limit: 0). No gravity on anything else; black sky.
import { BoxGeometry, Group, HingeJoint, Mesh, Vector3 } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { glow } from "./physics-box-rest.ts";

export const test: TestMeta = {
  id: "physics/hinge", features: ["physics"], reference: "analytic", temporal: false,
  // Camera 6 m away, 30 deg vertical fov: the image is 3.215 m high, 5.716 m wide.
  metric: { kind: "radiance", region: [0.603, 0.592, 0.627, 0.632], expected: 5, tolerance: 0.02 },
  time: 4,
  note: "the hinge at the origin, the camera at z = 6 looking at it",
};

export default function () {
  const scene = sceneWithSky(0);
  const bar = new Mesh(new BoxGeometry(1, 0.3, 0.4), glow());
  bar.position.set(0.5, 0, 0);
  // (Damped, so that it has come to rest by the render.)
  bar.physics = { type: "dynamic", angularDamping: 2 };
  // The post: nothing drawn, a fixed body at the hinge that touches nothing.
  const post = new Group();
  post.physics = { type: "fixed", shape: { type: "sphere", radius: 0.05 }, sensor: true };
  const hinge = new HingeJoint(post, bar, { anchorB: new Vector3(-0.5, 0, 0), axis: new Vector3(0, 0, 1), limits: [-0.5, 0.5] });
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [bar, post, hinge]);
}

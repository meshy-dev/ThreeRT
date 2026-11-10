// Physics (ROADMAP § 6 step 7): a rigid body drives a scene object. A box of 0.5 m (a mesh emitting 5 nits) with
// `physics = "dynamic"` (its collider is the box its geometry is), let go 2 m up over a fixed ground (its top the plane
// y = 0), falls, lands and rests with its centre 0.25 m up. Seen from the side at that height after 2 s (120 ticks):
// the region inside the box reads 5 nits (still in the air, or fallen through: 0). Black sky.
import { BoxGeometry, Mesh, MeshStandardMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "physics/box-rest", features: ["physics"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.47, 0.445, 0.53, 0.555], expected: 5, tolerance: 0.02 },
  time: 2,
  note: "camera 6 m away at y 0.25, 30 deg vertical fov: the resting box spans u 0.454-0.546, v 0.419-0.581",
};

/** A fixed body whose top is the plane y = 0: a box of 40 x 1 x 40 m that is not drawn (a hidden object keeps its body). */
export function ground(): Mesh {
  const g = new Mesh(new BoxGeometry(40, 1, 40), new MeshStandardMaterial({ name: "ground", color: 0x000000 }));
  g.position.set(0, -0.5, 0);
  g.visible = false;
  g.physics = "fixed";
  return g;
}

export const glow = () => new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });

export default function () {
  const scene = sceneWithSky(0);
  const box = new Mesh(new BoxGeometry(0.5, 0.5, 0.5), glow());
  box.position.set(0, 2, 0);
  box.physics = "dynamic";
  return result(scene, camera(scene, [0, 0.25, 6], [0, 0.25, 0]), [box, ground()]);
}

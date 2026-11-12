// A vehicle's wheel drawn (step 10 c2): physics/vehicle-rests-live's car, its chassis black, a glowing 0.2 m box as its
// front right wheel's object (`VehicleWheel.object`), put 10 m under the ground. The binding places it where the
// wheel is every frame: at rest, the wheel's centre is its radius up, (0.9, 0.3, 1.2), the box (x 0.8 .. 1.0) clear of
// the chassis's side (x 0.8). Seen from the side after 150 frames, the box's middle reads its 5 nits; a wheel the binding
// does not place stays under the ground (dark there).
import { BoxGeometry, Mesh, MeshStandardMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { ground, glow } from "./physics-box-rest.ts";
import { car } from "./physics-vehicle-rests-live.ts";

export const test: TestMeta = {
  id: "physics/vehicle-wheel-live", features: ["physics", "vehicle"], reference: "analytic", temporal: true, live: 150,
  // The camera as physics/vehicle-rests-live's (6 m away on +x at y = 0.5, 30 deg vertical fov, 16:9). The box's face
  // toward it is 5 m from it (x = 1.0): there the view spans z ±2.38 and y 0.5 ± 1.34, so the box (z 1.1 .. 1.3, y 0.2
  // .. 0.4) is at u 0.227 .. 0.269, v 0.537 .. 0.612; its middle.
  metric: { kind: "radiance", region: [0.235, 0.552, 0.262, 0.595], expected: 5, tolerance: 0.03 },
  note: "the wheel's object placed at the wheel's centre, (0.9, 0.3, 1.2)",
};

export default function () {
  const scene = sceneWithSky(0);
  const { chassis, vehicle } = car(new MeshStandardMaterial({ name: "black", color: 0x000000 }));
  const tyre = new Mesh(new BoxGeometry(0.2, 0.2, 0.2), glow());
  tyre.position.set(0, -10, 0);
  vehicle.wheels[1].object = tyre;
  return result(scene, camera(scene, [6, 0.5, 0], [0, 0.5, 0]), [chassis, vehicle, tyre, ground()]);
}

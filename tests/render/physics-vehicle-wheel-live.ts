// A vehicle's wheel drawn (step 10 c2): physics/vehicle-rests-live's car, its chassis black, a glowing 0.2 m box as its
// front right wheel's object (`VehicleWheel.object`), put 10 m under the ground. The binding places it where the
// wheel is every frame: at rest, the wheel's centre is its radius up, (0.9, 0.3, 1.2), the box (x 0.8 .. 1.0) clear of
// the chassis's side (x 0.8). Seen from the side after 150 frames, the box's middle reads its 5 nits; a wheel the binding
// does not place stays under the ground (dark there). A game (the binding reads the vehicles back before each frame
// script: a scene without one is drawn, its chassis driven, but its wheels' objects are not placed).
import { BoxGeometry, Mesh, MeshStandardMaterial } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { ground, glow } from "./physics-box-rest.ts";
import { car } from "./physics-vehicle-rests-live.ts";

export const test: TestMeta = {
  id: "physics/vehicle-wheel-live", features: ["physics", "vehicle"], reference: "analytic", temporal: true, live: 150,
  // The camera as physics/vehicle-rests-live's: the box (z 1.1 .. 1.3, y 0.2 .. 0.4) at u 0.273 .. 0.308, v 0.531 ..
  // 0.593; its middle.
  metric: { kind: "radiance", region: [0.281, 0.545, 0.300, 0.580], expected: 5, tolerance: 0.03 },
  note: "the wheel's object placed at the wheel's centre, (0.9, 0.3, 1.2)",
};

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const { chassis, vehicle } = car(new MeshStandardMaterial({ name: "black", color: 0x000000 }));
    const tyre = new Mesh(new BoxGeometry(0.2, 0.2, 0.2), glow());
    tyre.position.set(0, -10, 0);
    vehicle.wheels[1].object = tyre;
    return result(scene, camera(scene, [6, 0.5, 0], [0, 0.5, 0]), [chassis, vehicle, tyre, ground()]);
  },
  frame() {},
  handlers: {},
});

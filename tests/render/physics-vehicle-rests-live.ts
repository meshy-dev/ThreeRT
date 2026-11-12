// A vehicle (step 10 c2) on the real engine: a box chassis of 1.6 x 0.4 x 3 m (300 kg/m³: 576 kg, emitting 5 nits) on
// four wheels (radius 0.3 m, the suspension 0.3 m unloaded, 0.2 m of travel, stiffness 25 per kg), let go 0.3 m above
// its ride height over a fixed ground (its top the plane y = 0). It settles on its springs: each carries a quarter of
// the weight, so they sag g / (4 x 25) = 0.098 m, the wheels' centres 0.3 m up, the anchors (the chassis's centre)
// 0.3 + 0.3 - 0.098 = 0.502 m up: the chassis from 0.302 to 0.702 m. Seen from the side after 150 frames, the band 0.55
// .. 0.65 m reads its 5 nits. Without the suspension the chassis lies on the ground (0 .. 0.4 m: dark there); on its
// stops (fully compressed) its top is at 0.575 m (half the band dark). Black sky.
import { BoxGeometry, Mesh, MeshStandardMaterial, Vector3, Vehicle } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { ground, glow } from "./physics-box-rest.ts";

export const test: TestMeta = {
  id: "physics/vehicle-rests-live", features: ["physics", "vehicle"], reference: "analytic", temporal: true, live: 150,
  // Camera 6 m away on +x at y = 0.5, 30 deg vertical fov, 16:9: z +2.86 .. -2.86 left to right, y 2.11 .. -1.11 top to
  // bottom. The band: z -1 .. 1 -> u 0.325 .. 0.675; y 0.65 .. 0.55 -> v 0.453 .. 0.484.
  metric: { kind: "radiance", region: [0.40, 0.457, 0.60, 0.481], expected: 5, tolerance: 0.02 },
  note: "the chassis settles at its ride height on its springs: centre 0.502 m up",
};

/** The test's car: the chassis (its material given) and its vehicle, the wheels at the corners of its centre plane. */
export function car(material: MeshStandardMaterial): { chassis: Mesh; vehicle: Vehicle } {
  const chassis = new Mesh(new BoxGeometry(1.6, 0.4, 3), material);
  chassis.position.set(0, 0.8, 0);
  chassis.physics = { type: "dynamic", density: 300 };
  const wheel = (x: number, z: number) => ({ anchor: new Vector3(x, 0, z), rest: 0.3, travel: 0.2, radius: 0.3, width: 0.2, stiffness: 25, compression: 2, damping: 3, frictionSlip: 1, bumpLength: 0.03, bumpStiffness: 500 });
  // (The wheels' centre lines 0.1 m outside the chassis's sides: a wheel's object is seen clear of the body.)
  const vehicle = new Vehicle(chassis, { wheels: [wheel(-0.9, 1.2), wheel(0.9, 1.2), wheel(-0.9, -1.2), wheel(0.9, -1.2)] });
  return { chassis, vehicle };
}

export default function () {
  const scene = sceneWithSky(0);
  const { chassis, vehicle } = car(glow());
  return result(scene, camera(scene, [6, 0.5, 0], [0, 0.5, 0]), [chassis, vehicle, ground()]);
}

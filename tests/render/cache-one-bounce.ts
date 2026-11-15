// One bounce off a lit floor: a 100-nit sky over an albedo-0.5 Lambert floor (radiance 50) and a 0.1 m albedo-0.5
// slab at 0.5 m, seen from below. Its underside sees only the floor: 0.5 x 50 = 25, a quarter of the sky beside it
// (its own shadow on the floor takes under 1 % of that). In the real-time renderer the slab's light is the radiance
// cache's alone: the path from its underside ends there.
import { BoxGeometry, Mesh, type MeshStandardMaterial } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "cache/one-bounce", features: ["sky", "lambert", "gi", "radiance-cache"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.47, 0.47, 0.53, 0.53], b: [0.05, 0.05, 0.2, 0.2], expected: 0.25, tolerance: 0.01 },
  spp: 256,
  note: "the slab's underside against the sky seen beside it",
};

/** `floor`: its material; `up`: the whole scene this much higher (m). */
export const under = (floor: MeshStandardMaterial = lambert(0.5), up = 0) => () => {
  const scene = sceneWithSky(100);
  const slab = new Mesh(new BoxGeometry(0.1, 0.01, 0.1), lambert(0.5, "slab"));
  slab.name = "slab";
  slab.position.set(0, 0.505 + up, 0);
  const ground = plane(400, floor);
  ground.position.y = up;
  return result(scene, camera(scene, [0, 0.1 + up, 0.001], [0, 0.5 + up, 0], 40), [slab, ground]);
};

export default under();

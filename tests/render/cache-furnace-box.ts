// White furnace inside a box, in the real-time renderer: albedo-1 Lambert walls in a uniform 1-nit sky reflect exactly
// 1 nit everywhere, however deep the concavity (every path either escapes to the sky or keeps bouncing). Paths end in
// the radiance cache after one bounce, so this holds only if the cache carries every further bounce (it feeds on
// itself) without losing or adding energy. Inside an open-top box, seen from above, most light has bounced many times.
import { BoxGeometry, Mesh } from "@threert/scene";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "cache/furnace-box", features: ["radiance-cache", "lambert", "sky-light", "energy-conservation"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.4, 0.55, 0.6, 0.7], expected: 1, tolerance: 0.04 },
  live: 600,
  note: "Box floor 2 x 2 m inside 1 m walls: the sky covers 8 % of the floor centre's cosine-weighted hemisphere, so most of its 1 nit is light that bounced several times.",
};

export default function () {
  const scene = sceneWithSky(1);
  const white = lambert(1);
  const box: Mesh[] = [];
  const slab = (w: number, h: number, d: number, x: number, y: number, z: number) => {
    const m = new Mesh(new BoxGeometry(w, h, d), white);
    m.position.set(x, y, z);
    box.push(m);
  };
  slab(2.2, 0.1, 2.2, 0, -0.05, 0); // floor
  slab(0.1, 1, 2.2, -1.05, 0.5, 0);
  slab(0.1, 1, 2.2, 1.05, 0.5, 0);
  slab(2.2, 1, 0.1, 0, 0.5, -1.05);
  slab(2.2, 1, 0.1, 0, 0.5, 1.05);
  return result(scene, camera(scene, [0, 2.6, 0.3], [0, 0, 0], 40), box);
}

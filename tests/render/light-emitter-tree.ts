// Emitters sampled by a light tree from the first frame: 144 emissive specks (5 cm, 20,000 nits; 288 triangles, over
// the tree's 256) in a 12 x 12 grid 2 m above an albedo-0.5 Lambert plane, facing it. Each gives the centre
// L A cos^2 / d^2 (cos = 2 / d), so the plane shows 0.5 / pi x their sum; a speck is 0.0006 sr from there, so paths
// that are not sent to the emitters find almost none of it.
import { Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { CENTER, camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const [N, STEP, H, SIDE, NITS] = [12, 0.5, 2, 0.05, 20000];
const at = (k: number) => (k - (N - 1) / 2) * STEP;
const cells = Array.from({ length: N * N }, (_, k) => [at(k % N), at(Math.floor(k / N))]);
const sum = cells.reduce((s, [x, z]) => s + (H * H) / (x * x + z * z + H * H) ** 2, 0);

export const test: TestMeta = {
  id: "light/emitter-tree", features: ["emissive", "light-tree", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: (0.5 / Math.PI) * NITS * SIDE * SIDE * sum, tolerance: 0.02 },
  note: "144 emissive specks over a Lambert plane, sampled by a light tree: their sum at the centre",
};

export default function () {
  const scene = sceneWithSky(0);
  const glow = new MeshStandardMaterial({ name: "glow", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: NITS, roughness: 1 });
  const geometry = new PlaneGeometry(SIDE, SIDE);
  const specks = cells.map(([x, z], k) => {
    const m = new Mesh(geometry, glow);
    m.name = `speck-${k}`;
    m.rotation.x = Math.PI / 2;
    m.position.set(x, H, z);
    return m;
  });
  return result(scene, camera(scene, [0, 0.5, 0.001], [0, 0, 0], 40), [plane(400, lambert(0.5, "floor"), "floor"), ...specks]);
}

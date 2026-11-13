// An object added in place is drawn (live/edits.rs `edit_geometry`; ROADMAP § 6 step 5): a scene of one `dynamicGeometry`
// speck far away (its TLAS one instance) gets a `dynamicGeometry` plane across the centre at frame 3: Lambert 0.5 facing
// a uniform 1-nit sky, so 0.5. Before, the plane's new BLAS and the TLAS were built apart in one encoder, and Metal,
// which the TLAS reaches its BLASes by resource id through, could build the TLAS first: the plane was missing in most
// runs (geometry/edit-live checks a swapped geometry, not an added object).
import { Mesh, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/added-live", features: ["instancing", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "a dynamicGeometry plane added in place to a scene of one instance: drawn, Lambert 0.5 under a 1 nit sky",
};

let elapsed = 0;
let added = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    added = false;
    const scene = sceneWithSky(1);
    const speck = new Mesh(new PlaneGeometry(0.05, 0.05), lambert(0.5, "speck"));
    speck.name = "speck";
    speck.dynamicGeometry = true;
    speck.position.set(5, 0, -10);
    return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [speck]);
  },
  frame(f) {
    elapsed += f.dt;
    if (added || elapsed <= 3 / 60) return;
    added = true;
    const plane = new Mesh(new PlaneGeometry(4, 4), (f.scene.getObjectByName("speck") as Mesh).material);
    plane.dynamicGeometry = true;
    f.scene.add(plane);
  },
  handlers: {},
});

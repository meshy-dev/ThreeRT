// In-place edits past the room reserved at upload (live/edits.rs `edit_geometry`; ROADMAP § 6 step 5): at frame 3 the frame
// script adds 1,100 `dynamicGeometry` specks far away (more records and object entries than the room's 1,024) and a
// 72,200-triangle plane across the centre (more triangles than its 65,536). The room grows (buffers and the TLAS
// doubled, copied on the GPU), nothing rebuilds, and the plane reads 0.5: Lambert 0.5 under a uniform 1-nit sky.
import { Mesh, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/grow-live", features: ["instancing", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "1,100 objects and a 72,200-triangle plane added in place, past the room: it grows; the plane reads 0.5",
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
    const material = (f.scene.getObjectByName("speck") as Mesh).material;
    const small = new PlaneGeometry(0.05, 0.05);
    for (let k = 0; k < 1100; k++) {
      const s = new Mesh(small, material);
      s.dynamicGeometry = true;
      s.position.set(20 + (k % 40) * 0.2, (Math.floor(k / 40) - 14) * 0.2, -30);
      f.scene.add(s);
    }
    const plane = new Mesh(new PlaneGeometry(4, 4, 190, 190), material);
    plane.dynamicGeometry = true;
    f.scene.add(plane);
  },
  handlers: {},
});

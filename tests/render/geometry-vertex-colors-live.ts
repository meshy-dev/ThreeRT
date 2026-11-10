// Vertex colours on geometry added in place (live.rs `edit_geometry`; F_VCOLOR): a scene with vertex colours from the
// start (a coloured speck far away, `dynamicGeometry`) gets material/vertex-colors' quad at frame 3, in place; its
// colours are written into the edit room with its triangles, and its centre reads 0.5 as when built. (A plain mesh far
// below too: in a scene whose TLAS held one instance, an object added in place is not drawn; ROADMAP Known issues.)
import { Mesh } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { gradientQuad, white } from "./material-vertex-colors.ts";

export const test: TestMeta = {
  id: "geometry/vertex-colors-live", features: ["vertex-colors", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "a vertex-coloured quad (0 .. 1 left to right) added in place: 0.5 at the centre",
};

let elapsed = 0;
let added = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    added = false;
    const scene = sceneWithSky(1);
    const speck = new Mesh(gradientQuad(), white());
    speck.name = "speck";
    speck.dynamicGeometry = true;
    speck.scale.setScalar(0.001);
    speck.position.set(900, -50, 0);
    const below = new Mesh(gradientQuad(), lambert(0.5, "below"));
    below.position.set(0, -500, 0);
    return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [speck, below]);
  },
  frame(f) {
    elapsed += f.dt;
    if (added || elapsed <= 3 / 60) return;
    added = true;
    const quad = new Mesh(gradientQuad(), (f.scene.getObjectByName("speck") as Mesh).material);
    quad.name = "quad";
    quad.dynamicGeometry = true;
    f.scene.add(quad);
  },
  handlers: {},
});

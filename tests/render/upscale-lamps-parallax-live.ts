// Lamps finer than a pixel moving against what is behind them (live.wgsl `Lock`, `lock_carry`): the field of
// upscale/lamps-still-live cut off at x = 0.41, before the black sky, the camera sliding along it 1.4 pixels a frame.
// The sky does not move with the camera's slide, so a lamp no sample hit this frame, if it went with what is behind
// it, would leave its light where it was: a trail behind the field's edge. The band right of the edge (1 to 15
// pixels from it at the end) is sky: none of the field's light may be there. Before the locks kept their lamps' depth
// it held 0.357 of the field's mean (a trail of some 20 pixels); 0.075 is left (2026-11-14): spots where the test for
// something gone from in front missed the frame a lamp left (it is barred while the lamp is among the nine samples
// around, and a frame later the lamp's last place is out of the three by three it reads).
import { defineGame } from "@threert/gameplay";
import { lampField } from "./upscale-lamps-still-live.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "upscale/lamps-parallax-live", features: ["dynamic-emission", "upscaler", "procedural"], reference: "analytic", temporal: true,
  // The field's edge ends 30.5 pixels left of the centre (u 0.309): b the field, a the sky beside it.
  metric: { kind: "ratio", a: [0.315, 0.3, 0.4, 0.7], b: [0.1, 0.3, 0.28, 0.7], expected: 0, tolerance: 0.1 },
  live: 60,
  note: "lamps a third of a pixel wide before the black sky, the camera sliding 1.4 pixels a frame: no light beside the field",
};

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => lampField(4),
  frame(f) {
    f.camera.position.x += 0.05;
  },
  handlers: {},
});

// Lamps finer than a pixel while the view moves (live.wgsl `Lock`, `judge`'s disocclusion): the field of
// upscale/lamps-still-live, the camera sliding along it 1.4 pixels a frame. Its mean radiance is the lamps' share of
// it, as at rest: 0.1. (In motion the upscaler clamps its history into the frame's nine samples, and a lamp no sample
// hit this frame is outside them; and the lamp the samples missed, in front of a background that parallax moves
// against it, was taken for something gone. The field kept a sixth to a quarter of its light.)
import { defineGame } from "@threert/gameplay";
import { lampField } from "./upscale-lamps-still-live.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "upscale/lamps-in-motion-live", features: ["dynamic-emission", "upscaler", "procedural"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.3, 0.3, 0.7, 0.7], expected: 0.1, tolerance: 0.15 },
  live: 60,
  note: "lamps a third of a pixel wide, 1.44 % of the field at 6.94 nits, the camera sliding 1.4 pixels a frame: 0.1",
};

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => lampField(),
  frame(f) {
    f.camera.position.x += 0.05;
  },
  handlers: {},
});

// wind/pivot-sway in the real-time renderer: 240 frames at 60 fps up to t = 0.5 s while the plant sways (movable: its
// triangles re-transformed in place every frame); the last frame has the marker where the wind function puts it.
import def, { test as base } from "./wind-pivot-sway.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "wind/pivot-sway-live", temporal: true, live: 240 };

export default def;

// sky/cutout-bounce in the real-time renderer as it ships: black. What is left of `APPROX.cutoutPaths`: the radiance
// cache's rays take the card as solid, their light samples too, so the floor it holds is unlit and sends nothing on.
import def, { test as base } from "./sky-cutout-bounce.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "sky/cutout-bounce-live", metric: { ...base.metric, expected: 0 }, temporal: true, live: 120 };

export default def;

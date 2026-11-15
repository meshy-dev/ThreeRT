// sky/cutout-bounce in the real-time renderer without `APPROX.cutoutPaths`: what the live view reads with no card
// at all, 0.21. (That is 16 % under the offline 0.25: the radiance cache's light off the floor, card or not.)
import def, { test as base } from "./sky-cutout-bounce.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "sky/cutout-bounce-live-exact", metric: { ...base.metric, expected: 0.21, tolerance: 0.01 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.cutoutPaths };

export default def;

// sky/cutout-bounce in the real-time renderer without `APPROX.cutoutPaths`: the offline value.
import def, { test as base } from "./sky-cutout-bounce.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "sky/cutout-bounce-live-exact", metric: { ...base.metric, tolerance: 0.015 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.cutoutPaths };

export default def;

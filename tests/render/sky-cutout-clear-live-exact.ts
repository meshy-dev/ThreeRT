// sky/cutout-clear in the real-time renderer without `APPROX.cutoutPaths` (as it ships, the rays a surface sends on
// take the card as opaque, and their share of the sky is lost): the offline value.
import def, { test as base } from "./sky-cutout-clear.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "sky/cutout-clear-live-exact", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.cutoutPaths };

export default def;

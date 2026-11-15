// sky/cutout-clear in the real-time renderer without `APPROX.cutoutPaths`: the rays the plane sends on pass the card
// too, and share the sky's light with its samples as they do offline.
import def, { test as base } from "./sky-cutout-clear.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "sky/cutout-clear-live-exact", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.cutoutPaths };

export default def;

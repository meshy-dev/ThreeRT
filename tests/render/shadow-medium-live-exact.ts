// shadow/medium in the real-time renderer without `APPROX.mediaShadows`: the offline value.
import def, { test as base } from "./shadow-medium.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "shadow/medium-live-exact", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.mediaShadows };

export default def;

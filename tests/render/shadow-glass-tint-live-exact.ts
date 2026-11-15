// shadow/glass-tint in the real-time renderer without `APPROX.glassShadows`: the offline value.
import def, { test as base } from "./shadow-glass-tint.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "shadow/glass-tint-live-exact", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.glassShadows };

export default def;

// shadow/glass-tint-instance in the real-time renderer without `APPROX.glassShadows`: the offline value.
import def, { test as base } from "./shadow-glass-tint-instance.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "shadow/glass-tint-instance-live-exact", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.glassShadows };

export default def;

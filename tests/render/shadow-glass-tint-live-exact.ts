// shadow/glass-tint in the real-time renderer without `APPROX.glassShadows`: the offline value. (5 %: it reads 3 %
// low. The glass passes a shadow ray or stops it by one random decision, 23 % here, and the live clamps darken a
// signal that noisy; carried as a weight it would not be.)
import def, { test as base } from "./shadow-glass-tint.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "shadow/glass-tint-live-exact", metric: { ...base.metric, tolerance: 0.05 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.glassShadows };

export default def;

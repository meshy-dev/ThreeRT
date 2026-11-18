// shadow/glass-model in the real-time renderer without `APPROX.glassShadows`: the offline value. (Under software
// traversal the model is a unit, and a shadow ray skips its glass triangles by their flag.)
import def, { test as base } from "./shadow-glass-model.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "shadow/glass-model-live-exact", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.glassShadows };

export default def;

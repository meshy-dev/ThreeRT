// light/emitter-far in the real-time renderer without `APPROX.farEmitters` (as it ships, the light tree proposes no
// emitter beyond 120 m, and what a path finds of them by chance the clamps cut): the offline value.
import def, { test as base } from "./light-emitter-far.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "light/emitter-far-live-exact", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120, approx: APPROX.all & ~APPROX.farEmitters };

export default def;

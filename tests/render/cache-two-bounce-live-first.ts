// cache/two-bounce in the real-time renderer's first frame, when the radiance cache holds nothing yet: every path
// traces its own light to the end, and where it stops its light samples stand alone (pathtrace.wgsl `nee_alone`).
// Weighed as if a BSDF ray followed them, the sky's samples kept 18 % of its light, and the slab read 4.0.
import def, { test as base } from "./cache-two-bounce.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/two-bounce-live-first", metric: { ...base.metric, tolerance: 0.15 }, temporal: true, live: 1 };

export default def;

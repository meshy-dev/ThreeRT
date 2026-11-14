// light/many-shadowed in the real-time renderer: the resampled lights guided by the light tiles. (3 %: its readings
// spread 0.8 % either way with the seed; without the guiding it read 2 % low, light lost to the samples' clamp.)
import def, { test as base } from "./light-many-shadowed.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "light/many-shadowed-live", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120 };

export default def;

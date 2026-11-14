// light/many-shadowed in the real-time renderer: the resampled lights guided by the light tiles.
import def, { test as base } from "./light-many-shadowed.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "light/many-shadowed-live", temporal: true, live: 120 };

export default def;

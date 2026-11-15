// cache/corner in the real-time renderer: a sky-lit corner is within the cache's voxels' reach (0.975 for 0.971).
import def, { test as base } from "./cache-corner.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/corner-live", temporal: true, live: 240 };

export default def;

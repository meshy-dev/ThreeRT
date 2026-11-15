// cache/mirror-sun in the real-time renderer.
import def, { test as base } from "./cache-mirror-sun.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/mirror-sun-live", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 240 };

export default def;

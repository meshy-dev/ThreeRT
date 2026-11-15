// cache/one-bounce-coat in the real-time renderer: the radiance cache's light off a coated surface is the coat's too.
import def, { test as base } from "./cache-one-bounce-coat.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/one-bounce-coat-live", metric: { ...base.metric, tolerance: 0.002 }, temporal: true, live: 120 };

export default def;

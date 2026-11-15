// cache/mirror-metal in the real-time renderer.
import def, { test as base } from "./cache-mirror-metal.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/mirror-metal-live", temporal: true, live: 240 };

export default def;

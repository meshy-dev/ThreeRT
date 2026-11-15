// shadow/cutout-blocker in the real-time renderer: the same ordered queries.
import def, { test as base } from "./shadow-cutout-blocker.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "shadow/cutout-blocker-live", temporal: true, live: 120 };

export default def;

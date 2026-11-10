// light/sun-point in the real-time renderer.
import def, { test as base } from "./light-sun-point.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "light/sun-point-live", temporal: true, live: 120 };

export default def;

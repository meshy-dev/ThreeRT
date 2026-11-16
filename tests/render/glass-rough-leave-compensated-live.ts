// glass/rough-leave-compensated in the real-time renderer.
import def, { test as base } from "./glass-rough-leave-compensated.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "glass/rough-leave-compensated-live", metric: { ...base.metric, tolerance: 0.03 } as TestMeta["metric"], temporal: true, live: 120 };

export default def;

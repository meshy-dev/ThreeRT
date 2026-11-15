// sky/cutout-clear in the real-time renderer as it ships: the offline value. The sky's samples pass the cutout and
// keep the whole of its light there, because the rays the plane sends on take the card as solid
// (`APPROX.cutoutPaths`) and bring none of it (pathtrace.wgsl `light_mis`). Until 2026-11-15 this read 8.8.
import def, { test as base } from "./sky-cutout-clear.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "sky/cutout-clear-live", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120 };

export default def;

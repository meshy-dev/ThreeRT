// volume/overlap-emission in the real-time renderer: the camera ray is marched a stretch at a time, the media it is in
// together (pathtrace.wgsl media_march). Same analytic value.
import def, { test as base } from "./volume-overlap-emission.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/overlap-emission-live", temporal: true, live: 240, metric: { ...base.metric, tolerance: 0.02 } as TestMeta["metric"] };

export default def;

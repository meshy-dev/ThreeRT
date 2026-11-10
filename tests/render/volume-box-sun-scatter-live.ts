// volume/box-sun-scatter in the real-time renderer: the camera ray's march, in-scattering from the volume voxels.
import def, { test as base } from "./volume-box-sun-scatter.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/box-sun-scatter-live", temporal: true, live: 240, metric: { ...base.metric, tolerance: 0.05 } as TestMeta["metric"] };

export default def;

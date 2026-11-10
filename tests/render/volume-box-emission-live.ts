// volume/box-emission in the real-time renderer: camera rays through media are marched (pathtrace.wgsl media_march),
// in-scattering lit by the radiance cache's volume voxels. Same analytic value.
import def, { test as base } from "./volume-box-emission.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/box-emission-live", temporal: true, live: 240, metric: { ...base.metric, tolerance: 0.02 } as TestMeta["metric"] };

export default def;

// volume/furnace in the real-time renderer: multiple scattering from the volume voxels. 0.9965 (0.993 without the
// temporal pass's emission clamp); 0.963 / 0.931 while the media lookups' jitter lost light at the boundary.
import def, { test as base } from "./volume-furnace.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/furnace-live", temporal: true, live: 480, metric: { ...base.metric, tolerance: 0.02 } as TestMeta["metric"] };

export default def;

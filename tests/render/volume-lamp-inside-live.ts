// volume/lamp-inside in the real-time renderer: the volume voxels' feedback carries multiple scattering. Live reads
// 7.61 (+15 %): the lookups' jitter within a voxel averages to trilinear interpolation, which overestimates the convex
// 1 / r^2 field near the lamp (7.08 unjittered, blocky); with single scattering alone it read 1.44.
import def, { test as base } from "./volume-lamp-inside.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/lamp-inside-live", temporal: true, live: 480, metric: { ...base.metric, tolerance: 0.2 } as TestMeta["metric"] };

export default def;

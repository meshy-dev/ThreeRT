// water/scatter in the real-time renderer: in-scattering along the refracted rays from the volume voxels, which learn
// the water below the surface (training rays pass the surface or reflect off it).
import def, { test as base } from "./water-scatter.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "water/scatter-live", temporal: true, live: 480, metric: { ...base.metric, tolerance: 0.1 } as TestMeta["metric"] };

export default def;

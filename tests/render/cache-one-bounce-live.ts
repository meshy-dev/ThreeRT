// cache/one-bounce in the real-time renderer, at this suite's 160 x 90, where few of the cache's training samples
// land on any one voxel of the floor: the slab's path ends on a voxel that has none, or none yet, and reads the
// coarser levels there. (Until 2026-11-16 such a path read black past a view's first 30 frames: 0.21.)
import def, { test as base } from "./cache-one-bounce.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/one-bounce-live", temporal: true, live: 120 };

export default def;

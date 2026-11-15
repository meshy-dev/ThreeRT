// cache/slit in the real-time renderer: a known limit, held here so that it is seen when it moves. The face reads
// 6.8 for 5.97: its path ends on the wall opposite, 2 cm away, in a voxel 6 cm across that holds the mean of light
// which falls off within it. Every other cache of this kind goes on tracing where the last segment is shorter than
// its voxel (SHaRC: 1.7 voxels), and so read 5.98 here; in this kernel that test cost Bistro's trace 15 to 22 %
// (the pixels that go on hold their SIMD groups), and it was not taken (2026-11-16).
import def, { test as base } from "./cache-slit.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/slit-live", metric: { ...base.metric, expected: 6.82, tolerance: 0.04 }, temporal: true, live: 240 };

export default def;

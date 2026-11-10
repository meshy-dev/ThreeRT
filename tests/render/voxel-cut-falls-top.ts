// voxel/cut-falls from the other end: the top 0.2 m of the upper post (another object than the one cut) is empty after
// the fall. Its radiance over the post's just below: 0 (not fallen: 1).
import def, { BELOW, TOP, test as base } from "./voxel-cut-falls.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "voxel/cut-falls-top", metric: { kind: "ratio", a: TOP, b: BELOW, expected: 0, tolerance: 0.02 } };

export default def;

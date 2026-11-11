// voxel/cut-body from the other end: the top 0.2 m of the upper post is empty once the body has come down. Its radiance
// over the post's just below: 0 (not fallen: 1).
import def, { test as base } from "./voxel-cut-body.ts";
import { BELOW, TOP } from "./voxel-cut-falls.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "voxel/cut-body-top", metric: { kind: "ratio", a: TOP, b: BELOW, expected: 0, tolerance: 0.02 } };

export default def;

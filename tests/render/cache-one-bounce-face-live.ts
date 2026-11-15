// cache/one-bounce-live with the scene 3 cm higher: the floor inside the cache's voxels, not on a face of them. The
// same reading: a surface's voxel does not depend on which side of a face its hits are rounded to.
import { test as base, under } from "./cache-one-bounce.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/one-bounce-face-live", temporal: true, live: 120 };

export default under(undefined, 0.03);

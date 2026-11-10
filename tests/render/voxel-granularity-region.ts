// voxel/granularity-object with boxes per 32³ region (left) against per brick (right).
import { pair } from "./voxel-granularity-object.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "voxel/granularity-region", features: ["voxel", "procedural"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0, 0, 0.5, 1], b: [0.5, 0, 1, 1], expected: 1, tolerance: 0.005 },
};

export default function () {
  return pair("region", "brick");
}

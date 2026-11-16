// glass/rough-leave at a cosine of 0.4 (66 degrees from the normal): R 0.531 + T 0.116 = 0.6474.
import { sheet } from "./glass-rough-enter.ts";
import { CENTER, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/rough-leave-low", features: ["transmission", "refraction", "ggx"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.6474, tolerance: 0.015 },
  spp: 256,
};

export default () => sheet({ roughness: 0.6, leaving: true, cos: 0.4 });

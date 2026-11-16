// glass/rough-leave with energy compensation: 0.739 by single scattering, 1 with it.
import { sheet } from "./glass-rough-enter.ts";
import { CENTER, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/rough-leave-compensated", features: ["transmission", "refraction", "ggx", "energy-conservation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.02 },
  spp: 256,
};

export default () => sheet({ roughness: 0.6, leaving: true, compensation: 1 });

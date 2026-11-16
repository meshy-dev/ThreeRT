// glass/rough-enter with energy compensation: the interface's single scattering is scaled by 1 / (R + T) from the
// glass energy table (Turquin 2019), so nothing is lost: 1.
import { sheet } from "./glass-rough-enter.ts";
import { CENTER, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/rough-enter-compensated", features: ["transmission", "refraction", "ggx", "energy-conservation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.02 },
  spp: 256,
};

export default () => sheet({ roughness: 0.6, compensation: 1 });

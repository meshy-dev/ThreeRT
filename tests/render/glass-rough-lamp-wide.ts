// glass/rough-lamp with a 1 m square: most of the lobe ends on it, 0.836 of its radiance, and the BSDF's own rays
// find it as often as its light samples do.
import { sheet } from "./glass-rough-enter.ts";
import { lamp } from "./glass-rough-lamp.ts";
import { CENTER, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/rough-lamp-wide", features: ["transmission", "refraction", "ggx", "emissive"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 8.3641, tolerance: 0.02 },
  spp: 256,
};

export default () => sheet({ roughness: 0.6, sky: 0 }, [lamp(1)]);

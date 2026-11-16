// Total internal reflection is not tinted: a smooth sheet of grey glass (base colour 0.5) seen from inside at 45
// degrees, past the critical angle (41.8), reflects the whole sky: 1. The tint is for light that crosses the surface.
// (Until 2026-11-16 the reflected ray carried the transmission lobe's weight: 0.5.)
import { sheet } from "./glass-rough-enter.ts";
import { CENTER, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/internal-reflection", features: ["transmission", "refraction"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 1, tolerance: 0.02 },
};

export default () => sheet({ roughness: 0, leaving: true, color: 0.5 });

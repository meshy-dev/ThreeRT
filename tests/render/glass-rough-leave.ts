// glass/rough-enter from inside the glass (the sheet's back toward the camera, relative index 1 / 1.5): at 45 degrees,
// past the critical angle of smooth glass, the rough interface reflects 0.367 and transmits 0.372 by single
// scattering: 0.739 (lib.rs `glass_energy_tests`). Total internal reflection is reflection at the microfacet, by
// its Fresnel reflectance.
import { sheet } from "./glass-rough-enter.ts";
import { CENTER, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/rough-leave", features: ["transmission", "refraction", "ggx"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.739, tolerance: 0.015 },
  spp: 256,
};

export default () => sheet({ roughness: 0.6, leaving: true });

// One rough dielectric interface in a white furnace: a single sheet of glass (ior 1.5, roughness 0.6; the far side
// stays glass) in a uniform sky of radiance 1, seen at 45 degrees from the air. Without energy compensation the pixel
// reads what one interface reflects and transmits by single scattering: R 0.037 + T 0.920 = 0.9573, the BSDF
// (Walter et al. 2007, GGX) integrated over both sides' directions and, apart, summed over visible microfacet
// normals (renderer/src/lib.rs `glass_energy_tests`). The sky is sampled on both sides of the sheet, so the BSDF's
// value, its density and its sampling are held to each other.
import { Color, Mesh, MeshPhysicalMaterial, PlaneGeometry, type Object3D } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/rough-enter", features: ["transmission", "refraction", "ggx"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.9573, tolerance: 0.015 },
  spp: 256,
};

/** The sheet (its normal up; `leaving`: down, so the camera sees it from inside the glass), seen from above at `cos`. */
export function sheet(o: { roughness: number; leaving?: boolean; cos?: number; sky?: number; color?: number; compensation?: number }, extra: Object3D[] = []) {
  const scene = sceneWithSky(o.sky ?? 1);
  const c = o.color ?? 1;
  const glass = new Mesh(new PlaneGeometry(200, 200), new MeshPhysicalMaterial({ color: new Color().setRGB(c, c, c), transmission: 1, ior: 1.5, roughness: o.roughness, metalness: 0 }));
  glass.rotation.x = o.leaving ? Math.PI / 2 : -Math.PI / 2;
  const cos = o.cos ?? Math.SQRT1_2;
  return result(scene, camera(scene, [0, 3 * cos, 3 * Math.sqrt(1 - cos * cos)], [0, 0, 0], 5), [glass, ...extra], { energyCompensation: o.compensation ?? 0 });
}

export default () => sheet({ roughness: 0.6 });

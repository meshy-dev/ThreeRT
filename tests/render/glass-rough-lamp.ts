// A lamp seen through frosted glass: under glass/rough-enter's sheet (roughness 0.6, no sky) a 0.3 m square of
// radiance 10 lies 1 m below, where the view's refracted ray arrives. The pixel reads the BTDF integrated over the
// square, 0.404 of the lamp's radiance (Walter et al. 2007 eq. 21 from the view side; lib.rs
// `the_lamp_through_frosted_glass`). A lamp this small is found by its light samples: their shadow rays start on
// the sheet's far side.
import { Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { sheet } from "./glass-rough-enter.ts";
import { CENTER, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "glass/rough-lamp", features: ["transmission", "refraction", "ggx", "emissive"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 4.0415, tolerance: 0.02 },
  spp: 256,
};

export function lamp(side: number): Mesh {
  const m = new Mesh(new PlaneGeometry(side, side), new MeshStandardMaterial({ color: 0x000000, emissive: new Color().setRGB(1, 1, 1), emissiveIntensity: 10 }));
  m.rotation.x = -Math.PI / 2;
  m.position.set(0, -1, -0.5345);
  return m;
}

export default () => sheet({ roughness: 0.6, sky: 0 }, [lamp(0.3)]);

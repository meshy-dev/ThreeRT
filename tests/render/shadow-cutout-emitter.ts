// An emitter's light past an alpha-tested card: a 10 x 10 m emissive panel (10 nits) 2 m above an albedo-0.5 Lambert
// plane, and between them a card whose texture is transparent everywhere (alpha 0, alphaTest 0.5): it blocks nothing.
// Under the panel's centre the plane reads 0.5 / pi x L x 4 s atan(s), s = a / sqrt(a^2 + h^2) (a = 5, h = 2).
import { Color, DataTexture, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const [A, H, NITS] = [5, 2, 10];
const S = A / Math.hypot(A, H);

/** A two-sided card that is cut out everywhere. */
export function clearCard(width: number, height: number): Mesh {
  const texture = new DataTexture(2, 2, new Uint8Array([255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0]));
  const m = new Mesh(new PlaneGeometry(width, height), new MeshStandardMaterial({ name: "card", color: 0xffffff, map: texture, alphaTest: 0.5, roughness: 1, side: "double" }));
  m.name = "card";
  return m;
}

export const test: TestMeta = {
  id: "shadow/cutout-emitter", features: ["emissive", "alpha-test", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: (0.5 / Math.PI) * NITS * 4 * S * Math.atan(S), tolerance: 0.02 },
  spp: 256,
  note: "the card 12 x 12 m at y = 1, the camera at y = 0.6 under it",
};

export default function () {
  const scene = sceneWithSky(0);
  const panel = new Mesh(new PlaneGeometry(2 * A, 2 * A), new MeshStandardMaterial({ name: "panel", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: NITS, roughness: 1 }));
  panel.name = "panel";
  panel.rotation.x = Math.PI / 2;
  panel.position.set(0, H, 0);
  const card = clearCard(12, 12);
  card.rotation.x = -Math.PI / 2;
  card.position.set(0, 1, 0);
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0]), [panel, card, plane(20, lambert(0.5))]);
}

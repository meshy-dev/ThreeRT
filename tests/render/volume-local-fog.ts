// FogVolume's density against its formula, written out and integrated here: an absorbing FogVolume of radius 1.5 m in
// front of a 2-nit emissive wall, seen from above through its centre, so that the ray crosses the heights where the
// height part goes from thinner than the radial part (above) to much denser (below). With u the point in the unit
// sphere (radius 1): radial = radialExtinction (1 - |u|^2), height = heightExtinction exp(-heightFalloff (u.y -
// heightOffset)), extinction = radial (1 - exp(-height / radial)). The pixel reads 2 exp(-optical depth).
import { BoxGeometry, FogVolume, Mesh, MeshStandardMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const R = 1.5, radial = 1, height = 0.6, falloff = 3, offset = 0.2;
const EYE: [number, number, number] = [0, 3, 6];

/** The optical depth along the ray from EYE through the sphere's centre (Simpson's rule over the chord, 2 R long). */
function opticalDepth(): number {
  const len = Math.hypot(...EYE), dy = -EYE[1] / len;
  const n = 2000;
  let sum = 0;
  for (let k = 0; k <= n; k++) {
    const s = -1 + (2 * k) / n; // along the chord, in the unit sphere: u = s x direction
    const rad = radial * Math.max(1 - s * s, 0);
    const hgt = height * Math.exp(-falloff * (s * dy - offset));
    const ext = rad > 0 ? rad * (1 - Math.exp(-hgt / rad)) : 0;
    sum += ext * (k === 0 || k === n ? 1 : k % 2 ? 4 : 2);
  }
  return (sum * (2 / n) * R) / 3;
}

export const test: TestMeta = {
  id: "volume/local-fog", features: ["volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.495, 0.49, 0.505, 0.51], expected: 2 * Math.exp(-opticalDepth()), tolerance: 0.03 },
  spp: 16384,
  note: "FogVolume's density (radial part, height part with falloff and offset, their combination), optical depth integrated numerically",
};

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(12, 12, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -3);
  const fog = new FogVolume({ radialExtinction: radial, heightExtinction: height, heightFalloff: falloff, heightOffset: offset, albedo: [0, 0, 0] });
  fog.scale.set(2 * R, 2 * R, 2 * R);
  return result(scene, camera(scene, EYE, [0, 0, 0]), [wall, fog]);
}

// FogVolume against UE's Local Fog Volume density (VolumetricFog.usf), integrated here: an absorbing FogVolume of
// radius 1.5 m (radial extinction 1.5 / m, height extinction 2 / m with falloff 1 and offset 0) centred in front of a
// 2-nit emissive wall; the centre pixel's ray crosses the sphere through its centre at u.y = 0, so the height part is
// 2 there and the radial part 1.5 (1 - |t|)^0.82 along t in [-1, 1] (unit-sphere units of 1.5 m).
import { BoxGeometry, FogVolume, Mesh, MeshStandardMaterial } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const R = 1.5, radial = 1.5, height = 2;
let od = 0;
const n = 4000;
for (let k = 0; k < n; k++) {
  const t = -1 + (2 * (k + 0.5)) / n;
  const a = Math.exp(-height), b = Math.exp(-radial * Math.pow(Math.max(1 - Math.abs(t), 0), 0.82));
  od += Math.max(-Math.log(a - a * b + b), 0) * ((2 * R) / n);
}

export const test: TestMeta = {
  id: "volume/local-fog", features: ["volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.495, 0.49, 0.505, 0.51], expected: 2 * Math.exp(-od), tolerance: 0.03 },
  spp: 16384,
  note: "UE 5.8 Local Fog Volume density (volumetric fog voxelization), optical depth integrated numerically",
};

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(12, 12, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -3);
  const fog = new FogVolume({ radialExtinction: radial, heightExtinction: height, heightFalloff: 1, heightOffset: 0, albedo: [0, 0, 0] });
  fog.scale.set(2 * R, 2 * R, 2 * R);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [wall, fog]);
}

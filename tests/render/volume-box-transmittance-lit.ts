// volume/box-transmittance with a lit wall instead of an emissive one: a white Lambertian wall (no specular) lit by
// 4π lux at 60 degrees from its normal reads 2 nits; the 1 m absorber in front of it leaves 2 e^-1 = 0.7358. The light
// comes from +x, so the box's shadow falls on the wall well left of the centre. Live, a lit surface's direct light is
// a signal of its own: it must carry the transmittance to the camera as emission and indirect light do.
import { BoxGeometry, DirectionalLight, Mesh, MeshPhysicalMaterial, VolumeMesh } from "@threert/scene";
import { UNIFORM } from "./volume-box-transmittance.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/box-transmittance-lit", features: ["volume", "directional-light"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1), tolerance: 0.02 },
  spp: 256,
  note: "box 2 x 2 x 1 m at the origin, wall at z = -2, camera at z = 5, light from (sin 60°, 0, cos 60°)",
};

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshPhysicalMaterial({ name: "wall", color: 0xffffff, roughness: 1, specularIntensity: 0 }));
  wall.position.set(0, 0, -2);
  const box = new VolumeMesh({ volume: UNIFORM.with({ Density: 1 }), majorant: 1 });
  box.scale.set(2, 2, 1);
  const sun = new DirectionalLight(0xffffff, 4 * Math.PI);
  sun.position.set(Math.sin(Math.PI / 3), 0, Math.cos(Math.PI / 3));
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box, sun]);
}

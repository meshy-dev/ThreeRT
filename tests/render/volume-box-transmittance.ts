// Beer-Lambert through a homogeneous box: a pure absorber (extinction 1/m, albedo 0) 1 m thick in front of a 2-nit
// emissive wall reads 2 e^-1 = 0.7358 nits.
import { BoxGeometry, Mesh, MeshStandardMaterial, VolumeFunction, VolumeMesh } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/box-transmittance", features: ["volume"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1), tolerance: 0.02 },
  spp: 256,
  note: "box 2 x 2 x 1 m at the origin, wall at z = -2, camera at z = 5",
};

export const UNIFORM = new VolumeFunction("test_uniform", {
  params: { Density: 1, Albedo: [0, 0, 0, 0], Emission: [0, 0, 0, 0] },
  code: `
    o.extinction = $Density;
    o.albedo = $Albedo.rgb;
    o.emission = $Emission.rgb;`,
});

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -2);
  const box = new VolumeMesh({ volume: UNIFORM.with({ Density: 1 }), majorant: 1 });
  box.scale.set(2, 2, 1);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box]);
}

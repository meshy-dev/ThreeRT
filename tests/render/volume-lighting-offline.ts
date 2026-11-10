// Translucency lighting offline (no radiance cache: a fresh estimate per sample of the mean incident radiance): the
// same cube as volume/lighting, a white particle at the centre of a closed 2 m cube with one 6-nit wall reads 1 nit.
import { BoxGeometry, Mesh, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/lighting-offline", features: ["particles", "translucency"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.47, 0.45, 0.53, 0.55], expected: 1, tolerance: 0.03 },
  spp: 256,
  note: "one face of a cube covers 1/6 of the sphere seen from its centre",
};

const CENTRE = new ParticleFunction("test_vol_centre", {
  code: `
    o.center = vec3<f32>(0.0, 1.0, 0.0);
    o.u = vec3<f32>(0.06, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.06, 0.0);`,
});

export function volumeRoom() {
  const scene = sceneWithSky(0);
  const black = lambert(0, "black");
  const glow = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 6 });
  const slab = (w: number, h: number, d: number, x: number, y: number, z: number, m: MeshStandardMaterial) => {
    const s = new Mesh(new BoxGeometry(w, h, d), m);
    s.position.set(x, y, z);
    return s;
  };
  const walls = [
    slab(2.2, 0.1, 2.2, 0, -0.05, 0, black), // floor
    slab(2.2, 0.1, 2.2, 0, 2.05, 0, black), // ceiling
    slab(0.1, 2.2, 2.2, -1.05, 1, 0, black),
    slab(0.1, 2.2, 2.2, 1.05, 1, 0, black),
    slab(2.2, 2.2, 0.1, 0, 1, -1.05, glow), // the lit wall, behind the particle
    slab(2.2, 2.2, 0.1, 0, 1, 1.05, black),
  ];
  const white = new MeshStandardMaterial({ name: "white-tl", transparent: true, color: 0xffffff });
  return result(scene, camera(scene, [0, 1, 0.8], [0, 1, 0], 30), [...walls, new Particles({ count: 1, emitter: CENTRE.with({}), material: white })]);
}

export default volumeRoom;

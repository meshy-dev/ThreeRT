// Translucency pass, ordered blending: two half-opaque translucent quads over a 2-nit emissive wall, the near one
// emitting 4 nits, the far one black. Front to back: 0.5 * 4 + 0.5 * (0.5 * 0 + 0.5 * 2) = 2.5. The reverse order would
// give 1.5; weighted blended OIT (order-independent) something in between.
import { BoxGeometry, MaterialFunction, Mesh, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "translucent/order", features: ["particles", "translucency"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2.5, tolerance: 0.02 },
  note: "quads at z = 1 (4 nits) and z = 0 (0 nits), both opacity 0.5, wall at z = -1 emitting 2 nits, camera at z = 5",
};

const PAIR = new ParticleFunction("test_tlpair", {
  code: `
    let k = f32(i.index);
    o.center = vec3<f32>(0.0, 0.0, 1.0 - k);
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.5, 0.0);
    o.custom = vec4<f32>(vec3<f32>(4.0 * (1.0 - k)), 0.0);`,
});

const EMIT = new MaterialFunction("test_tl_emit", {
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = i.custom.rgb;
    o.opacity = 0.5;`,
});

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -1);
  const mat = new MeshStandardMaterial({ name: "tl", transparent: true });
  mat.shader = EMIT.with({});
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, new Particles({ count: 2, emitter: PAIR.with({}), material: mat })]);
}

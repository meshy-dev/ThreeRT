// Translucency pass, several systems: two translucent particle systems of two quads each, A (emitting 2 nits, its
// node's power 2: 4 nits) and B (black), both half opaque, over a 2-nit wall. On the left A's quad is in front of
// B's, on the right B's in front of A's: front to back, left = 0.5 * 4 + 0.5 * (0.5 * 0 + 0.5 * 2) = 2.5 and
// right = 0.5 * 0 + 0.5 * (0.5 * 4 + 0.5 * 2) = 1.5. A hit taken for the other system's (its material, its node's
// power) reads the same on both sides.
import { BoxGeometry, Mesh, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "translucent/systems", features: ["particles", "translucency"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.25, 0.4, 0.35, 0.6], b: [0.65, 0.4, 0.75, 0.6], expected: 2.5 / 1.5, tolerance: 0.02 },
  note: "quads 1 x 1 m at z = 1 and z = 0 behind one another as the camera at z = 5 sees them (30 deg vertical fov), wall at z = -1",
};

// Particle 0 on the left, 1 on the right; $Swap 0: the left one near, the right one far; 1: the other way round.
const PAIRS = new ParticleFunction("test_tl_systems", {
  params: { Swap: 0 },
  code: `
    let left = i.index == 0u;
    let near = left != ($Swap > 0.5);
    o.center = vec3<f32>(select(1.0, -1.0, left) * select(0.9524, 0.762, near), 0.0, select(0.0, 1.0, near));
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.5, 0.0);`,
});

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -1);
  const glow = new MeshStandardMaterial({ name: "tl-a", transparent: true, opacity: 0.5, color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 });
  const black = new MeshStandardMaterial({ name: "tl-b", transparent: true, opacity: 0.5, color: 0x000000 });
  const b = new Particles({ count: 2, emitter: PAIRS.with({ Swap: 1 }), material: black });
  const a = new Particles({ count: 2, emitter: PAIRS.with({ Swap: 0 }), material: glow });
  a.power = 2;
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, b, a]);
}

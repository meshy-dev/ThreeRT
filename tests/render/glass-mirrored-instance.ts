// Glass on an instance that mirrors its mesh: which side of a surface a ray meets goes by the instance's sign
// (pathtrace.wgsl `surface`, `inst_sign`). Two glass slabs of one mesh, each a mesh of its own, turned 50 degrees to
// the camera's rays, mirror images of each other, the right one placed with a negative scale; behind them an
// emitting wall of 1 nit, no sky. Each shows the wall through it, alike. A slab entered as if from inside reflects
// the ray whole at that angle (past the critical one), and shows black.
import { BoxGeometry, Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { glass } from "./shadow-glass-tint-instance.ts";

export const test: TestMeta = {
  id: "glass/mirrored-instance", features: ["transmission", "instancing", "mirrored"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.61, 0.4, 0.66, 0.6], b: [0.34, 0.4, 0.39, 0.6], expected: 1, tolerance: 0.03 },
  spp: 256,
  note: "left slab as it is, right slab mirrored in x: the wall seen through each, right / left = 1",
};

export default () => {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new PlaneGeometry(40, 40), new MeshStandardMaterial({ name: "wall", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: 1, roughness: 1 }));
  wall.position.z = -3;
  const geometry = new BoxGeometry(1.6, 1.6, 0.1);
  const material = glass();
  const slabs = [-1, 1].map((x) => {
    const m = new Mesh(geometry, material);
    m.name = x < 0 ? "left" : "right";
    m.dynamicGeometry = true;
    m.position.x = x;
    m.rotation.y = (-x * 50 * Math.PI) / 180;
    m.scale.x = x < 0 ? 1 : -1;
    return m;
  });
  return result(scene, camera(scene, [0, 0, 8], [0, 0, 0]), [wall, ...slabs]);
};

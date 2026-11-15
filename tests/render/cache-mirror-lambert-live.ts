// cache/mirror-metal-live with a white Lambert ground: the same furnace.
import { BoxGeometry, Color, Mesh, MeshStandardMaterial } from "@threert/scene";
import { test as base } from "./cache-mirror-metal.ts";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "cache/mirror-lambert-live", features: ["lambert", "mirror", "energy-conservation"], temporal: true, live: 240 };

export default function () {
  const scene = sceneWithSky(1);
  const wall = new Mesh(new BoxGeometry(8, 4, 0.1), new MeshStandardMaterial({ name: "mirror", color: new Color().setRGB(1, 1, 1), metalness: 1, roughness: 0 }));
  wall.name = "wall";
  wall.position.set(0, 2, -1.05);
  return result(scene, camera(scene, [0, 0.5, 1], [0, 0.3, -1]), [wall, plane(400, lambert(1, "ground"), "ground")], { energyCompensation: 1 });
}

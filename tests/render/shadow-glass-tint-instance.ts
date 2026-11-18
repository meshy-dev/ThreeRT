// shadow/glass-tint with the pane a mesh of its own (`dynamicGeometry`): an instance whose mesh is glass, a BLAS
// geometry that is not flagged opaque. A shadow ray culls such geometry (trace_hw.wgsl) and `glass_tr` weighs what
// the pane lets through: the plane under it reads the same 0.5 I / 4 / pi x (0.96 x 0.5)^2. Taken as opaque, the
// pane would leave it black.
import { BoxGeometry, Color, Mesh, MeshPhysicalMaterial, PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { test as base } from "./shadow-glass-tint.ts";

export const test: TestMeta = { ...base, id: "shadow/glass-tint-instance", features: [...base.features, "instancing"] };

export const glass = () => new MeshPhysicalMaterial({ name: "pane", color: new Color().setRGB(0.5, 0.5, 0.5), transmission: 1, ior: 1.5, roughness: 0, metalness: 0 });

export default function () {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 2, 0);
  const pane = new Mesh(new BoxGeometry(1, 0.05, 1), glass());
  pane.name = "pane";
  pane.dynamicGeometry = true;
  pane.position.set(0, 1, 0);
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0]), [l, pane, plane(20, lambert(0.5))]);
}

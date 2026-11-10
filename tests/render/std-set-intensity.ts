// Material library: set() keeps the derived standard fields in step. Two signs light a grey floor in a black sky: the
// left one made at 10 nits, the right one made at 2 and set() to 10. The sign's emission (its function) and what it
// lights the street with (emitter sampling: emissive x emissiveIntensity) both follow, so the floor in front of each
// is equally lit: right over left = 1.
import { DataTexture, Mesh, NeonSignMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/set-intensity", features: ["material-library", "emissive"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.7, 0.8, 0.85], b: [0.2, 0.7, 0.4, 0.85], expected: 1, tolerance: 0.03 },
};

export default function () {
  const scene = sceneWithSky(0);
  const white = new DataTexture(1, 1, new Uint8Array([255, 255, 255, 255]), "srgb");
  const sign = (x: number, made: number) => {
    const material = new NeonSignMaterial({ map: white, intensity: made, dim: 0 });
    if (made !== 10) material.set({ intensity: 10 });
    const m = new Mesh(new PlaneGeometry(0.8, 0.8), material);
    m.position.set(x, 0.5, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 2.5, 3], [0, 0, 0.6]), [plane(20, lambert(0.8), "floor"), sign(-1.2, 10), sign(1.2, 2)]);
}

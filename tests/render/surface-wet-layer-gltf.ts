// A material function on a glTF file's own material (`GLTFModel.functions`, by material name): `wetLayer` on the
// grey quad of tests/render/data/quad.glb (albedo 0.5, roughness 1), puddles off, at full `scene.wetness`. As
// `surface/wet`: the diffuse halves and the roughness drops to 0.14 (Lagarde), under a uniform 1-nit sky wet over
// dry = (0.25 + s_wet) / (0.5 + s_dry), the sky's specular reflection s = 0.012 rough, 0.031 at roughness 0.14
// (`surface/wet`'s 0.531 at albedo 0.8). The other copy of the file, without the function, stays dry.
import { GLTFModel, wetLayer } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "surface/wet-layer-gltf", features: ["material-library", "wetness", "gltf"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.65, 0.4, 0.85, 0.6], b: [0.15, 0.4, 0.35, 0.6], expected: 0.549, tolerance: 0.015 },
  requires: ["tests/render/data/quad.glb"],
  note: "wetLayer on a glTF material, by name: wet over dry 0.549; the copy without it stays dry",
};

const URI = "tests/render/data/quad.glb";

export default function () {
  const scene = sceneWithSky(1);
  scene.wetness = 1;
  const quad = (x: number, wet: boolean) => {
    const m = new GLTFModel(URI, wet ? { functions: { gr: wetLayer({ puddles: 0 }) } } : {});
    m.rotation.x = -Math.PI / 2;
    m.scale.setScalar(1.8);
    m.position.set(x, 0, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 4, 0.001], [0, 0, 0]), [quad(-1, false), quad(1, true)]);
}

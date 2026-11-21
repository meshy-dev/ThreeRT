// A texture on an instance is as sharp as its triangles are large in the world, whatever the instance's scale: the
// texel density is taken from the triangle as it stands (pathtrace.wgsl `mesh_lod`). A quad of 6.25 cm scaled 32
// times (2 m), a checker of 8 x 8 squares (64 x 64 texels) on a Lambert surface under a uniform sky of 1 nit, seen
// straight on: a square is 14 pixels wide, the texture magnified. A black square reads 0 and a white one 1; from the
// quad's own size the level would be five too high and both would read 0.5.
import { Color, DataTexture, Mesh, MeshPhysicalMaterial, PlaneGeometry } from "@threert/scene";
import { camera, result, sceneWithSky, type Region, type TestMeta } from "./lib.ts";

/** The insides of the two squares above the centre: left of it a white one (a kept one), right of it a black one (a hole). */
export const LEFT: Region = [0.44, 0.39, 0.47, 0.455];
export const RIGHT: Region = [0.53, 0.39, 0.56, 0.455];

export const test: TestMeta = {
  id: "texture/level-instance", features: ["textures", "instancing"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: RIGHT, b: LEFT, expected: 0, tolerance: 0.03 },
};

/** 64 x 64 texels, 8 x 8 squares; `alpha`: the checker in the alpha (white all over), else in the colour. */
export function checker(alpha = false): DataTexture {
  const px = new Uint8Array(64 * 64 * 4);
  for (let y = 0; y < 64; y++) {
    for (let x = 0; x < 64; x++) {
      const on = ((x >> 3) + (y >> 3)) % 2 === 0 ? 255 : 0;
      px.set(alpha ? [255, 255, 255, on] : [on, on, on, 255], (y * 64 + x) * 4);
    }
  }
  return new DataTexture(64, 64, px, "srgb");
}

/** The quad: 2 m, its geometry 1 / `scale` of that. */
export function quad(material: MeshPhysicalMaterial, scale: number): Mesh {
  const m = new Mesh(new PlaneGeometry(2 / scale, 2 / scale), material);
  m.name = "quad";
  m.scale.setScalar(scale);
  return m;
}

export default function () {
  const scene = sceneWithSky(1);
  const m = new MeshPhysicalMaterial({ color: new Color().setRGB(1, 1, 1), map: checker(), roughness: 1, specularIntensity: 0 });
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [quad(m, 32)]);
}

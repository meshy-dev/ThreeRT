// A small glTF 2.0 binary builder for the city converter: meshes of triangles grouped by material, embedded images.
// Writes positions, normals and UVs (no vertex colours: the renderer takes colour from materials and textures).
// Triangles go to named nodes ("city" unless told otherwise), one mesh each, so a scene can load some of them only
// (`GLTFModel`'s `nodes`): games/tokyo loads "city" and "day" by day, "city" and "night" by night.
import { packGLB } from "@threert/scene";

/** A glTF material as the renderer reads it (renderer/src/scene.rs `gltf_material`). */
export interface Material {
  name: string;
  color: [number, number, number];
  roughness: number;
  metallic?: number;
  emissive?: [number, number, number];
  /** Emissive strength (KHR_materials_emissive_strength): nits per unit of `emissive` x the model's power. */
  emissiveStrength?: number;
  /** Index of an image added with `image()`, as the emissive texture (it scales `emissive`). */
  emissiveTexture?: number;
  /** KHR_materials_transmission factor (glass). */
  transmission?: number;
  ior?: number;
  doubleSided?: boolean;
  /** Index of an image added with `image()`, as the base colour texture. */
  texture?: number;
  /** Alpha cutout threshold (MASK) on the base colour texture's alpha. */
  alphaCutoff?: number;
}

export class GlbBuilder {
  private parts: Uint8Array[] = [];
  private len = 0;
  private views: { buffer: number; byteOffset: number; byteLength: number }[] = [];
  private accessors: object[] = [];
  private materials: object[] = [];
  private materialIndex = new Map<string, number>();
  private images: object[] = [];
  private textures: object[] = [];
  private primitives = new Map<string, object[]>();
  readonly stats = { triangles: 0, vertices: 0, images: 0, bytes: 0 };

  private view(bytes: Uint8Array): number {
    this.views.push({ buffer: 0, byteOffset: this.len, byteLength: bytes.length });
    this.parts.push(bytes);
    this.len += (bytes.length + 3) & ~3;
    return this.views.length - 1;
  }

  /**
   * An embedded image (PNG or JPEG bytes); returns its index for `Material.texture`. `clamp`: sampled with
   * CLAMP_TO_EDGE (an atlas of separate cells) rather than REPEAT.
   */
  image(bytes: Uint8Array, mimeType: "image/png" | "image/jpeg", clamp = false): number {
    this.images.push({ bufferView: this.view(bytes), mimeType });
    this.textures.push({ source: this.images.length - 1, sampler: clamp ? 1 : 0 });
    this.stats.images++;
    return this.textures.length - 1;
  }

  /** The material's index, added on first use (by name). */
  material(m: Material): number {
    const known = this.materialIndex.get(m.name);
    if (known !== undefined) return known;
    const ext: Record<string, object> = {};
    if (m.transmission) ext.KHR_materials_transmission = { transmissionFactor: m.transmission };
    if (m.ior) ext.KHR_materials_ior = { ior: m.ior };
    if (m.emissiveStrength) ext.KHR_materials_emissive_strength = { emissiveStrength: m.emissiveStrength };
    this.materials.push({
      name: m.name,
      pbrMetallicRoughness: {
        baseColorFactor: [...m.color, 1],
        metallicFactor: m.metallic ?? 0,
        roughnessFactor: m.roughness,
        ...(m.texture !== undefined ? { baseColorTexture: { index: m.texture } } : {}),
      },
      ...(m.emissive ? { emissiveFactor: m.emissive } : {}),
      ...(m.emissiveTexture !== undefined ? { emissiveTexture: { index: m.emissiveTexture } } : {}),
      ...(m.doubleSided ? { doubleSided: true } : {}),
      ...(m.alphaCutoff !== undefined ? { alphaMode: "MASK", alphaCutoff: m.alphaCutoff } : {}),
      ...(Object.keys(ext).length ? { extensions: ext } : {}),
    });
    this.materialIndex.set(m.name, this.materials.length - 1);
    return this.materials.length - 1;
  }

  /** Indexed triangles with one material, in the node `node`. `uv` may be omitted (zeros). */
  triangles(material: number, position: Float32Array, normal: Float32Array, uv: Float32Array | null, index: Uint32Array, node = "city"): void {
    const n = position.length / 3;
    if (!n || !index.length) return;
    let lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < position.length; i += 3) for (let k = 0; k < 3; k++) {
      lo[k] = Math.min(lo[k], position[i + k]);
      hi[k] = Math.max(hi[k], position[i + k]);
    }
    const acc = (view: number, count: number, type: string, componentType: number, extra = {}) => {
      this.accessors.push({ bufferView: view, componentType, count, type, ...extra });
      return this.accessors.length - 1;
    };
    const bytes = (a: ArrayBufferView) => new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
    const attributes: Record<string, number> = {
      POSITION: acc(this.view(bytes(position)), n, "VEC3", 5126, { min: lo, max: hi }),
      NORMAL: acc(this.view(bytes(normal)), n, "VEC3", 5126),
      TEXCOORD_0: acc(this.view(bytes(uv ?? new Float32Array(n * 2))), n, "VEC2", 5126),
    };
    const indices = acc(this.view(bytes(index)), index.length, "SCALAR", 5125);
    if (!this.primitives.has(node)) this.primitives.set(node, []);
    this.primitives.get(node)!.push({ attributes, indices, material });
    this.stats.triangles += index.length / 3;
    this.stats.vertices += n;
  }

  /** The GLB: a node and a mesh per node name (named `<name> <node>`), in the order they were first used. */
  build(name: string): Uint8Array {
    const bin = new Uint8Array(this.len);
    let o = 0;
    for (const p of this.parts) {
      bin.set(p, o);
      o += (p.length + 3) & ~3;
    }
    const used = new Set<string>();
    for (const m of this.materials as { extensions?: Record<string, unknown> }[]) for (const k of Object.keys(m.extensions ?? {})) used.add(k);
    const json = {
      asset: { version: "2.0", generator: "threert tools/tokyo" },
      ...(used.size ? { extensionsUsed: [...used] } : {}),
      scene: 0,
      scenes: [{ nodes: [...this.primitives.keys()].map((_, i) => i) }],
      nodes: [...this.primitives.keys()].map((node, i) => ({ name: node, mesh: i })),
      meshes: [...this.primitives].map(([node, primitives]) => ({ name: `${name} ${node}`, primitives })),
      materials: this.materials,
      ...(this.images.length ? { images: this.images, textures: this.textures, samplers: [{ wrapS: 10497, wrapT: 10497 }, { wrapS: 33071, wrapT: 33071 }] } : {}),
      accessors: this.accessors,
      bufferViews: this.views,
      buffers: [{ byteLength: this.len }],
    };
    const glb = packGLB(json, bin);
    this.stats.bytes = glb.length;
    return glb;
  }
}

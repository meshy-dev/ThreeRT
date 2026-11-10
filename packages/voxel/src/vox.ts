// MagicaVoxel `.vox` files (https://github.com/ephtracy/voxel-model, MagicaVoxel-file-format-vox.txt): the models
// (SIZE + XYZI chunk pairs) and the palette (RGBA). Other chunks (materials, the scene graph) are skipped. Bounds are
// checked everywhere: a file may come from an untrusted game.
import { VoxelGrid } from "./grid.ts";

export interface VoxModel {
  /** Size in voxels, MagicaVoxel axes (z up). */
  size: [number, number, number];
  /** x, y, z, palette index (1..255) per voxel. */
  voxels: Uint8Array;
}

export interface VoxFile {
  models: VoxModel[];
  /** 256 colours as 0xAABBGGRR (index k = palette entry k; entry 0 unused). */
  palette: Uint32Array;
}

/** A neutral palette when a file has none: entry k is grey k / 255. */
function greyPalette(): Uint32Array {
  const p = new Uint32Array(256);
  for (let k = 0; k < 256; k++) p[k] = (0xff << 24) | (k << 16) | (k << 8) | k;
  return p;
}

export function parseVox(buf: ArrayBuffer | Uint8Array): VoxFile {
  const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (o: number) => String.fromCharCode(bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]);
  const need = (o: number, n: number, what: string) => {
    if (o < 0 || n < 0 || o + n > bytes.length) throw new Error(`.vox: ${what} runs past the end of the file`);
  };
  need(0, 8, "header");
  if (tag(0) !== "VOX ") throw new Error(".vox: not a MagicaVoxel file");
  need(8, 12, "MAIN chunk");
  if (tag(8) !== "MAIN") throw new Error(".vox: no MAIN chunk");
  const mainContent = view.getUint32(12, true), mainChildren = view.getUint32(16, true);
  let o = 20 + mainContent;
  const end = o + mainChildren;
  need(20, mainContent + mainChildren, "MAIN's children");
  const models: VoxModel[] = [];
  let size: [number, number, number] | null = null;
  let palette: Uint32Array | null = null;
  while (o < end) {
    need(o, 12, "a chunk header");
    const id = tag(o), n = view.getUint32(o + 4, true), m = view.getUint32(o + 8, true);
    const c = o + 12;
    need(c, n + m, `chunk ${id}`);
    if (id === "SIZE") {
      if (n < 12) throw new Error(".vox: SIZE chunk too short");
      size = [view.getUint32(c, true), view.getUint32(c + 4, true), view.getUint32(c + 8, true)];
      if (size.some((v) => v < 1 || v > 2048)) throw new Error(`.vox: model size ${size.join(" x ")} out of range`);
    } else if (id === "XYZI") {
      if (!size) throw new Error(".vox: XYZI chunk before its SIZE");
      if (n < 4) throw new Error(".vox: XYZI chunk too short");
      const count = view.getUint32(c, true);
      if (4 + count * 4 > n) throw new Error(".vox: XYZI voxel count runs past its chunk");
      const voxels = bytes.slice(c + 4, c + 4 + count * 4);
      for (let k = 0; k < count; k++) {
        if (voxels[k * 4] >= size[0] || voxels[k * 4 + 1] >= size[1] || voxels[k * 4 + 2] >= size[2]) throw new Error(".vox: a voxel outside its model");
      }
      models.push({ size, voxels });
      size = null;
    } else if (id === "RGBA") {
      if (n < 1024) throw new Error(".vox: RGBA chunk too short");
      // File entry k is palette index k + 1 (index 0 is never a colour).
      palette = new Uint32Array(256);
      for (let k = 0; k < 255; k++) palette[k + 1] = view.getUint32(c + k * 4, true);
    }
    o = c + n + m;
  }
  if (models.length === 0) throw new Error(".vox: no model");
  return { models, palette: palette ?? greyPalette() };
}

/**
 * A `.vox` model as a voxel grid: MagicaVoxel's z-up axes turned to y-up, right-handed: voxel (x, y, z) of the file goes
 * to (x, z, size_y - 1 - y), so the model stays within voxels >= 0.
 */
export function voxGrid(model: VoxModel, size = 0.1): VoxelGrid {
  const g = new VoxelGrid(size);
  const v = model.voxels;
  for (let k = 0; k < v.length; k += 4) g.set(v[k], v[k + 2], model.size[1] - 1 - v[k + 1], v[k + 3]);
  return g;
}

/** A `.vox` file of `models` and `palette` (0xAABBGGRR, entry k = palette index k): tests and tools. */
export function encodeVox(models: VoxModel[], palette?: Uint32Array): Uint8Array {
  const chunk = (id: string, content: Uint8Array) => {
    const out = new Uint8Array(12 + content.length);
    const v = new DataView(out.buffer);
    for (let k = 0; k < 4; k++) out[k] = id.charCodeAt(k);
    v.setUint32(4, content.length, true);
    out.set(content, 12);
    return out;
  };
  const parts: Uint8Array[] = [];
  for (const m of models) {
    const size = new Uint8Array(12);
    m.size.forEach((s, k) => new DataView(size.buffer).setUint32(k * 4, s, true));
    parts.push(chunk("SIZE", size));
    const xyzi = new Uint8Array(4 + m.voxels.length);
    new DataView(xyzi.buffer).setUint32(0, m.voxels.length / 4, true);
    xyzi.set(m.voxels, 4);
    parts.push(chunk("XYZI", xyzi));
  }
  if (palette) {
    const rgba = new Uint8Array(1024);
    for (let k = 0; k < 255; k++) new DataView(rgba.buffer).setUint32(k * 4, palette[k + 1], true);
    parts.push(chunk("RGBA", rgba));
  }
  const children = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(20 + children);
  const v = new DataView(out.buffer);
  out.set([0x56, 0x4f, 0x58, 0x20], 0); // "VOX "
  v.setUint32(4, 150, true);
  out.set([0x4d, 0x41, 0x49, 0x4e], 8); // "MAIN"
  v.setUint32(12, 0, true);
  v.setUint32(16, children, true);
  let o = 20;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}

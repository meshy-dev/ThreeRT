// A voxel palette as materials: entry k of a VoxelGrid is material k of its ProceduralMesh (entry 0, empty, unused).
import { Color, MeshStandardMaterial, ProceduralGeometry, ProceduralMesh } from "@threert/scene";
import { type Granularity, VoxelGrid } from "./grid.ts";
import { VOXEL_BRICKS } from "./geometry.ts";

/** Palette colours (0xAABBGGRR, sRGB, as `.vox` stores them) as 256 materials; `emissive`: entries that glow (nits). */
export function paletteMaterials(palette: Uint32Array, o: { roughness?: number; metalness?: number; emissive?: Record<number, number> } = {}): MeshStandardMaterial[] {
  return [...palette].map((c, k) => {
    // 0xAABBGGRR -> 0xRRGGBB (a hex colour is sRGB).
    const color = new Color(((c & 0xff) << 16) | (c & 0xff00) | ((c >>> 16) & 0xff));
    const glow = o.emissive?.[k] ?? 0;
    return new MeshStandardMaterial({ name: `voxel_${k}`, color, roughness: o.roughness ?? 0.8, metalness: o.metalness ?? 0, ...(glow > 0 ? { emissive: color, emissiveIntensity: glow } : {}) });
  });
}

/** A voxel object as a ProceduralMesh (its boxes at `granularity`), shaded with `materials` by palette entry. */
export function voxelMesh(grid: VoxelGrid, materials: MeshStandardMaterial[], granularity: Granularity = "brick"): ProceduralMesh {
  const l = grid.layout(granularity);
  return new ProceduralMesh(new ProceduralGeometry(VOXEL_BRICKS, l.boxes, l.data), materials);
}

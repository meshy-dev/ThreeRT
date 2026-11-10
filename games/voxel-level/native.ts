// The benchmark level for the renderer's native voxel objects (`voxelLevel`, renderer/src/voxel.rs): its objects from
// the plan (plan.ts), each referring to its geometry file under the object-box assets (tools/voxel-level/generate.ts
// --granularity object), and the palette in linear colour, with or without the lamps' glow.
import { srgbToLinear } from "@threert/scene";
import type { NativeLevel } from "@threert/voxel";
import { BUILDING_PARTS, PALETTE, assetDir, buildingGeometry, levelPlan, propGeometry, tileGeometry, toScene } from "./plan.ts";

/** The level's objects: terrain tiles (with their tile, so neighbours hide each other's side faces), building parts and
 *  props. */
export function nativeObjects(seed: number): NativeLevel["objects"] {
  const plan = levelPlan(seed);
  const dir = assetDir(seed, "object");
  const objects: NativeLevel["objects"] = [];
  for (const t of plan.tiles) objects.push({ name: t.name, uri: `${dir}/${tileGeometry(seed, t)}.tpg`, position: toScene(t.tx * 64, 0, t.tz * 64), tile: [t.tx, t.tz] });
  for (const b of plan.buildings) for (const part of BUILDING_PARTS) objects.push({ name: `${b.name}-${part}`, uri: `${dir}/${buildingGeometry(b, part)}.tpg`, position: toScene(b.x, b.base, b.z) });
  plan.props.forEach((p, k) => objects.push({ name: `${p.kind}-${k}`, uri: `${dir}/${propGeometry(p, k, false)}.tpg`, position: toScene(p.x, p.y, p.z), turn: p.turn }));
  return objects;
}

/** The palette in linear colour; with `glow`, the emissive entries (the lamps) glow at their nits. */
export function nativePalette(glow: boolean): NativeLevel["palette"] {
  return PALETTE.map(([name, hex, roughness, metalness, nits]) => ({
    name,
    color: [16, 8, 0].map((s) => srgbToLinear(((hex >> s) & 255) / 255)) as [number, number, number],
    roughness,
    metalness,
    ...(glow && nits > 0 ? { emissive: nits } : {}),
  }));
}

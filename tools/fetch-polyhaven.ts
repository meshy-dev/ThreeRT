// Downloads CC0 Poly Haven PBR texture sets and models into assets/polyhaven/ (not committed; scenes reference them by
// repo-relative path and the packaging tools copy them into render packages).
//   node tools/fetch-polyhaven.ts [--res 1k|2k] [--textures a,b] [--models c,d] [--set kabukicho]
// Textures: assets/polyhaven/tex/<id>/<id>_<map>_<res>.jpg   (diff, nor_gl, rough, ao, metal when available)
// Models:   assets/polyhaven/models/<id>_<res>.glb           (glTF + bin + textures packed into one GLB)
// Licence: CC0 (https://polyhaven.com/license); assets/polyhaven/manifest.json records every source URL.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { packGLB } from "@threert/scene";
import { repoRoot } from "./bundle-game.ts";

/** Curated lists (the UE 5.8 Kabukichō session's choices: wet asphalt, pavers, tiled/clad facades, shutters + street clutter). */
export const SETS: Record<string, { textures: string[]; models: string[] }> = {
  // The UE 5.8 session's exact lists (kabukicho-ue5/Tools/scripts/fetch_polyhaven.py), fetched at 2k for the port.
  "kabukicho-ue": {
    textures: [
      "asphalt_02", "asphalt_04", "clean_asphalt", "worn_asphalt", "asphalt_track", "interlocking_concrete_pavers", "concrete_pavers",
      "square_tiles", "concrete_floor_02", "rectangular_facade_tiles", "rectangular_facade_tiles_02", "concrete_tile_facade",
      "long_white_tiles", "square_tiled_wall", "grey_tiles", "exterior_wall_cladding", "exterior_wall_cladding_02",
      "precast_concrete_wall", "concrete_wall_008", "painted_concrete", "beige_wall_001", "brushed_concrete", "painted_metal_shutter",
      "rusty_metal_shutter", "metal_plate", "corrugated_iron_02", "blue_metal_plate", "dirty_concrete", "white_plaster_02",
    ],
    models: [
      "exterior_aircon_unit", "modular_electricity_poles", "modular_electric_cables", "trashbag", "metal_trash_can", "fire_hydrant",
      "utility_box_01", "utility_box_02", "power_box_01", "plastic_crate_01", "plastic_crate_02", "plastic_crate_03", "security_camera_01",
      "street_lamp_01", "street_lamp_02", "water_manhole_cover", "modular_airduct_rectangular_01", "modular_pipes", "rollershutter_door",
      "rollershutter_window_01", "concrete_road_barrier", "WetFloorSign_01", "cardboard_box_01", "modular_metal_gutter",
      "plastic_monobloc_chair_01", "bar_chair_round_01", "industrial_wall_lamp", "mounted_fluorescent_lights", "modular_fire_escape",
      "metal_stool_01", "hand_truck", "propane_tank", "caged_hanging_light", "compost_bags",
    ],
  },
  kabukicho: {
    textures: [
      "asphalt_02", "clean_asphalt", "interlocking_concrete_pavers", "concrete_pavers", "rectangular_facade_tiles",
      "rectangular_facade_tiles_02", "concrete_tile_facade", "long_white_tiles", "square_tiled_wall", "grey_tiles",
      "exterior_wall_cladding", "precast_concrete_wall", "painted_concrete", "beige_wall_001", "painted_metal_shutter",
      "rusty_metal_shutter", "corrugated_iron_02", "dirty_concrete",
    ],
    models: [
      "exterior_aircon_unit", "trashbag", "metal_trash_can", "fire_hydrant", "utility_box_01", "utility_box_02",
      "plastic_crate_01", "plastic_crate_02", "security_camera_01", "street_lamp_01", "water_manhole_cover",
      "concrete_road_barrier", "cardboard_box_01", "plastic_monobloc_chair_01", "bar_chair_round_01", "metal_stool_01",
      "propane_tank", "WetFloorSign_01", "mounted_fluorescent_lights",
    ],
  },
};

const MAPS: Record<string, string> = { Diffuse: "diff", nor_gl: "nor_gl", Rough: "rough", AO: "ao", Metal: "metal" };
const UA = { "User-Agent": "threert-fetch-polyhaven" };
const out = join(repoRoot, "assets/polyhaven");

type FileInfo = Record<string, Record<string, Record<string, { url: string; size: number; include?: Record<string, { url: string }> }>>>;

async function json(url: string): Promise<FileInfo> {
  return JSON.parse(new TextDecoder().decode(await bytes(url))) as FileInfo;
}

async function bytes(url: string): Promise<Uint8Array> {
  for (let attempt = 0; ; attempt++) {
    try {
      const r = await fetch(url, { headers: UA });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      return new Uint8Array(await r.arrayBuffer());
    } catch (e) {
      if (attempt >= 3) throw new Error(`${url}: ${e}`);
      await new Promise((res) => setTimeout(res, 1000 * (attempt + 1)));
    }
  }
}

async function save(url: string, dst: string, manifest: Record<string, string>) {
  manifest[dst.slice(repoRoot.length + 1)] = url;
  if (existsSync(dst)) return;
  mkdirSync(dirname(dst), { recursive: true });
  writeFileSync(dst, await bytes(url));
}

async function fetchTexture(id: string, res: string, manifest: Record<string, string>): Promise<string[]> {
  const info = await json(`https://api.polyhaven.com/files/${id}`);
  const got: string[] = [];
  for (const [key, short] of Object.entries(MAPS)) {
    const r = info[key]?.[res] ?? info[key]?.["1k"];
    const f = r?.jpg ?? r?.png;
    if (!f) continue;
    await save(f.url, join(out, "tex", id, `${id}_${short}_${res}.${f.url.split(".").pop()}`), manifest);
    got.push(short);
  }
  return got;
}

/** glTF (+ external bin / images) -> single GLB with embedded images. */
function toGLB(gltf: Record<string, any>, files: Map<string, Uint8Array>): Uint8Array {
  const chunks: Uint8Array[] = [];
  let size = 0;
  const push = (b: Uint8Array) => {
    const off = size;
    chunks.push(b);
    size += b.length;
    const pad = (4 - (size % 4)) % 4;
    if (pad) {
      chunks.push(new Uint8Array(pad));
      size += pad;
    }
    return off;
  };
  const bufferOffsets = (gltf.buffers ?? []).map((b: { uri: string }) => push(files.get(decodeURIComponent(b.uri))!));
  for (const v of gltf.bufferViews ?? []) {
    v.byteOffset = (v.byteOffset ?? 0) + bufferOffsets[v.buffer];
    v.buffer = 0;
  }
  for (const img of gltf.images ?? []) {
    if (!img.uri) continue;
    const data = files.get(decodeURIComponent(img.uri))!;
    const off = push(data);
    img.bufferView = gltf.bufferViews.push({ buffer: 0, byteOffset: off, byteLength: data.length }) - 1;
    img.mimeType = /\.png$/i.test(img.uri) ? "image/png" : "image/jpeg";
    delete img.uri;
  }
  const bin = new Uint8Array(size);
  let o = 0;
  for (const c of chunks) {
    bin.set(c, o);
    o += c.length;
  }
  gltf.buffers = [{ byteLength: size }];
  return packGLB(gltf, bin);
}

async function fetchModel(id: string, res: string, manifest: Record<string, string>): Promise<string> {
  const dst = join(out, "models", `${id}_${res}.glb`);
  const info = await json(`https://api.polyhaven.com/files/${id}`);
  const g = (info.gltf?.[res] ?? info.gltf?.["1k"])?.gltf;
  if (!g) throw new Error(`${id}: no glTF download`);
  manifest[dst.slice(repoRoot.length + 1)] = g.url;
  if (existsSync(dst)) return dst;
  const gltf = JSON.parse(new TextDecoder().decode(await bytes(g.url)));
  const files = new Map<string, Uint8Array>();
  for (const [rel, inc] of Object.entries(g.include ?? {})) files.set(rel, await bytes(inc.url));
  mkdirSync(dirname(dst), { recursive: true });
  writeFileSync(dst, toGLB(gltf, files));
  return dst;
}

/** Runs `jobs` with at most `n` in flight. */
async function pool<T>(items: T[], n: number, fn: (t: T) => Promise<void>) {
  const queue = [...items];
  await Promise.all(Array.from({ length: n }, async () => {
    for (let t = queue.shift(); t !== undefined; t = queue.shift()) await fn(t);
  }));
}

if (import.meta.main) {
  const arg = (k: string) => {
    const i = process.argv.indexOf(k);
    return i > 0 ? process.argv[i + 1] : undefined;
  };
  const res = arg("--res") ?? "1k";
  const set = SETS[arg("--set") ?? "kabukicho"];
  const textures = arg("--textures")?.split(",") ?? set.textures;
  const models = arg("--models")?.split(",") ?? set.models;
  const mfile = join(out, "manifest.json");
  const manifest: Record<string, string> = existsSync(mfile) ? JSON.parse(readFileSync(mfile, "utf8")) : {};
  let failed = 0;
  await pool([...textures.map((t) => ["tex", t]), ...models.map((m) => ["model", m])], 6, async ([kind, id]) => {
    try {
      if (kind === "tex") console.log(`tex   ${id}: ${(await fetchTexture(id, res, manifest)).join(" ")}`);
      else console.log(`model ${id}: ${(await fetchModel(id, res, manifest)).slice(repoRoot.length + 1)}`);
    } catch (e) {
      failed++;
      console.error(`FAIL  ${id}: ${e}`);
    }
  });
  mkdirSync(out, { recursive: true });
  writeFileSync(mfile, JSON.stringify({ licence: "CC0 1.0 (https://polyhaven.com/license)", ...manifest }, null, 1));
  if (failed) process.exitCode = 1;
}

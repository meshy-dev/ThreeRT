// A MagicaVoxel file as procedural geometry asset files (packages/voxel, tools/lib/proc-asset.ts): one file per model,
// read by `ProceduralGeometry.fromAsset(VOXEL_BRICKS, uri)` with the materials `paletteMaterials(palette)`.
//   node tools/voxel-build.ts model.vox out/dir [--size 0.1] [--granularity brick|region|object]
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { type Granularity, parseVox, voxGrid } from "@threert/voxel";
import { encodeProcAsset } from "./lib/proc-asset.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const [src, out] = args.filter((a, k) => !a.startsWith("--") && !args[k - 1]?.startsWith("--"));
if (!src || !out) throw new Error("usage: node tools/voxel-build.ts model.vox out/dir [--size 0.1] [--granularity brick]");
const size = Number(opt("--size", "0.1"));
const granularity = opt("--granularity", "brick") as Granularity;
const file = parseVox(readFileSync(src));
mkdirSync(out, { recursive: true });
file.models.forEach((m, k) => {
  const l = voxGrid(m, size).layout(granularity);
  const path = join(out, `${basename(src, ".vox")}-${k}.tpg`);
  writeFileSync(path, encodeProcAsset(l.boxes, l.data));
  console.log(`${path}: ${l.stats.voxels} voxels, ${l.stats.bricks} bricks (${l.stats.uniform} uniform), ${l.boxes.length / 6} boxes, ${((l.data.length * 4) / 1024).toFixed(1)} KiB`);
});
writeFileSync(join(out, `${basename(src, ".vox")}.palette.json`), JSON.stringify([...file.palette]));

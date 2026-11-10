// Fetches Khronos glTF Sample Assets (KhronosGroup/glTF-Sample-Assets, glTF-Binary variants) for layer B of the render
// suite (docs/ROADMAP.md § 3): material test models whose look three.js, UE and ThreeRT are compared on.
//
//   node tools/fetch-khronos.ts [--out assets/khronos] [names...]   (default: the layer-B set)
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const outArg = args.includes("--out") ? args[args.indexOf("--out") + 1] : join(root, "assets/khronos");
const out = resolve(outArg);
const names = args.filter((a, i) => !a.startsWith("--") && args[i - 1] !== "--out");
export const LAYER_B = ["TransmissionTest", "ClearCoatTest", "MetalRoughSpheres"];
const BASE = "https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models";

mkdirSync(out, { recursive: true });
for (const name of names.length ? names : LAYER_B) {
  const file = join(out, `${name}.glb`);
  if (existsSync(file)) {
    console.log(`${file} exists`);
    continue;
  }
  const res = await fetch(`${BASE}/${name}/glTF-Binary/${name}.glb`);
  if (!res.ok) throw new Error(`${name}: HTTP ${res.status}`);
  const buf = Buffer.from(await res.arrayBuffer());
  if (buf.toString("ascii", 0, 4) !== "glTF") throw new Error(`${name}: not a GLB`);
  writeFileSync(file, buf);
  console.log(`${file}: ${(buf.length / 1024).toFixed(0)} KiB`);
}

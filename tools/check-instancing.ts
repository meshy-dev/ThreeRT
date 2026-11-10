// Instancing consistency check: the same scene of instanced cutout props (tests/checks/instanced-cutouts.ts) rendered
// three ways. The flattened render (THREERT_INSTANCING=0: world-space triangles, as before instancing) is the reference;
// hardware instancing and software traversal (which flattens the instances itself) must match it. Mean dE and p99 dE
// catch whole surfaces gone wrong: cutouts rendered solid (software traversal losing the cutout flag), holes thinned
// or filled by the alpha test sampling the wrong mip on scaled placements.
//
//   node tools/check-instancing.ts [--assets DIR] [--spp 64] [--bin path]   (exit 1 above the limits)
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { encodePNG, packGLB } from "@threert/scene";
import { rendererBin, repoRoot } from "./lib/bench.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const assets = resolve(flag("--assets", repoRoot));
const spp = flag("--spp", "64");
const bin = resolve(flag("--bin", rendererBin));
const out = join(repoRoot, "out/check-instancing");
mkdirSync(out, { recursive: true });
// The model: a 1 x 1 m quad (UVs 0..4) whose mask is 2-texel bars every 8 texels of a 64 x 64 texture (MASK, cutoff
// 0.5): at full resolution bars, from about 2 mips coarser an average alpha below the cutoff, no bars.
function fenceGLB(): Uint8Array {
  const n = 64;
  const px = new Uint8Array(n * n * 4).fill(255);
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) px[(y * n + x) * 4 + 3] = x % 8 < 2 || y % 8 < 2 ? 255 : 0;
  const png = encodePNG(n, n, px);
  const f32 = (a: number[]) => new Uint8Array(new Float32Array(a).buffer);
  const parts = [
    f32([-0.5, 0, 0, 0.5, 0, 0, 0.5, 1, 0, -0.5, 1, 0]), // positions
    f32([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), // normals
    f32([0, 4, 4, 4, 4, 0, 0, 0]), // uvs
    new Uint8Array(new Uint16Array([0, 1, 2, 0, 2, 3]).buffer),
    png,
  ];
  const offsets: number[] = [];
  let len = 0;
  for (const p of parts) {
    offsets.push(len);
    len += (p.length + 3) & ~3;
  }
  const bin = new Uint8Array(len);
  parts.forEach((p, i) => bin.set(p, offsets[i]));
  const view = (i: number) => ({ buffer: 0, byteOffset: offsets[i], byteLength: parts[i].length });
  return packGLB(
    {
      asset: { version: "2.0" },
      scene: 0,
      scenes: [{ nodes: [0] }],
      nodes: [{ name: "fence", mesh: 0 }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 }, indices: 3, material: 0 }] }],
      materials: [{ name: "bars", alphaMode: "MASK", alphaCutoff: 0.5, doubleSided: true, pbrMetallicRoughness: { baseColorFactor: [0.8, 0.8, 0.8, 1], baseColorTexture: { index: 0 }, metallicFactor: 0, roughnessFactor: 0.8 } }],
      textures: [{ source: 0, sampler: 0 }],
      samplers: [{ wrapS: 10497, wrapT: 10497 }],
      images: [{ bufferView: 4, mimeType: "image/png" }],
      accessors: [
        { bufferView: 0, componentType: 5126, count: 4, type: "VEC3", min: [-0.5, 0, 0], max: [0.5, 1, 0] },
        { bufferView: 1, componentType: 5126, count: 4, type: "VEC3" },
        { bufferView: 2, componentType: 5126, count: 4, type: "VEC2" },
        { bufferView: 3, componentType: 5123, count: 6, type: "SCALAR" },
      ],
      bufferViews: parts.map((_, i) => view(i)),
      buffers: [{ byteLength: len }],
    },
    bin,
  );
}
writeFileSync(join(out, "fence.glb"), fenceGLB());
const bundle = (await bundleGame(join(repoRoot, "tests/checks/instanced-cutouts.ts"), join(out, "instanced-cutouts.js"))).out;

/** Limits against the flattened render: hardware paths differ by sampling noise only; software traversal also by its
 *  own traversal order (paths decorrelate), not by surfaces. */
const runs = [
  { name: "flattened", env: { THREERT_INSTANCING: "0" } },
  { name: "instanced", env: {}, limit: { mean: 0.15, p99: 3 } },
  { name: "software", env: { THREERT_HWRT: "0" }, limit: { mean: 0.6, p99: 6 } },
];
let failed = 0;
for (const r of runs) {
  const png = join(out, `${r.name}.png`);
  const stats = JSON.parse(execFileSync(bin, ["render", bundle, "-o", png, "--assets", assets, "--spp", spp], { env: { ...process.env, ...r.env } }).toString().trim().split("\n").pop()!);
  if (!r.limit) {
    console.log(`${r.name}: reference (${stats.triangles} triangles)`);
    continue;
  }
  const m = JSON.parse(execFileSync(bin, ["compare", join(out, "flattened.png"), png]).toString().trim().split("\n").pop()!);
  const bad = m.mean_delta_e > r.limit.mean || m.p99_delta_e > r.limit.p99;
  if (bad) failed++;
  console.log(`${bad ? "FAIL" : "ok  "} ${r.name} vs flattened: dE ${m.mean_delta_e} (≤ ${r.limit.mean}), p99 ${m.p99_delta_e} (≤ ${r.limit.p99}), SSIM ${m.ssim}`);
}
console.log(`${out}/{flattened,instanced,software}.png`);
if (failed) process.exit(1);

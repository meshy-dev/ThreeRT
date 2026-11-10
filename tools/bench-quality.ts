// Quality / cost sweep of the native renderer against the UE5 references.
//   node tools/bench-quality.ts [--spp 16,32,64,128,256] [--scenes a,b] [--denoise true|false]
// Uses one warm `threert-render serve` process (as a render node would), compares every frame with
// UE5 Lumen (MRQ cinematic) and UE5 Path Tracer (1024 spp) and writes out/bench/quality.{json,md}.
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { mkdirSync, writeFileSync, existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const bin = join(root, "renderer/target/release/threert-render");
const arg = (k: string, d: string) => {
  const i = process.argv.indexOf(k);
  return i > 0 ? process.argv[i + 1] : d;
};
const sppList = arg("--spp", "16,32,64,128,256").split(",").map(Number);
const scenes = arg("--scenes", "cornell-box,material-grid,sun-room,neon-reflections,glb-asset").split(",");
const denoise = arg("--denoise", "true") === "true";
const outDir = join(root, "out/bench");
mkdirSync(join(outDir, "frames"), { recursive: true });

const proc = spawn(bin, ["serve"], { stdio: ["pipe", "pipe", "inherit"] });
const lines = createInterface({ input: proc.stdout })[Symbol.asyncIterator]();
const ready = JSON.parse((await lines.next()).value as string);
let id = 0;
async function render(scene: string, spp: number, out: string) {
  proc.stdin.write(JSON.stringify({ id: ++id, game: join(root, "out/bundles", `${scene}.js`), assets: root, out, spp, denoise }) + "\n");
  const msg = JSON.parse((await lines.next()).value as string);
  if (!msg.ok) throw new Error(msg.error);
  return msg.stats;
}
const compare = (ref: string, img: string) => (existsSync(ref) ? JSON.parse(execFileSync(bin, ["compare", ref, img]).toString()) : null);

const ueTimings = (m: string) => {
  const p = join(root, `out/ue/${m}/timings_${m}_all.json`);
  return existsSync(p) ? JSON.parse(readFileSync(p, "utf8")) : {};
};
const ue = { lumen: ueTimings("lumen"), pathtracer: ueTimings("pathtracer") };

const rows: Record<string, unknown>[] = [];
for (const scene of scenes) {
  // warm-up so first-use shader/denoiser compilation is not attributed to the sweep
  await render(scene, 4, join(outDir, "frames", "warmup.png"));
  for (const spp of sppList) {
    const out = join(outDir, "frames", `${scene}-${spp}.png`);
    const s = await render(scene, spp, out);
    const row = {
      scene, spp, denoiser: s.denoise, ms: s.timings_ms.total_ms, trace_ms: s.timings_ms.trace_ms, mpaths_s: +(s.samples_per_sec / 1e6).toFixed(1),
      triangles: s.triangles, vs_ue_pt: compare(join(root, "out/ue/pathtracer", `${scene}.png`), out), vs_ue_lumen: compare(join(root, "out/ue/lumen", `${scene}.png`), out),
    };
    rows.push(row);
    console.log(JSON.stringify(row));
  }
  rows.push({ scene, spp: "UE Lumen", ms: (ue.lumen[scene]?.render_s ?? NaN) * 1000, vs_ue_pt: compare(join(root, "out/ue/pathtracer", `${scene}.png`), join(root, "out/ue/lumen", `${scene}.png`)) });
  rows.push({ scene, spp: "UE PT 1024", ms: (ue.pathtracer[scene]?.render_s ?? NaN) * 1000 });
}
proc.stdin.end();
writeFileSync(join(outDir, "quality.json"), JSON.stringify({ adapter: ready.adapter, denoise, rows }, null, 1));
const fmt = (m: { [k: string]: number } | null | undefined) => (m ? `${m.mean_delta_e} / ${m.psnr_db} / ${m.ssim}` : "–");
const md = [
  `Adapter: ${ready.adapter}. Metrics: mean ΔE76 / PSNR dB / SSIM (1280x720).`,
  "",
  "| scene | renderer | time (s) | vs UE Path Tracer | vs UE Lumen |",
  "|---|---|---:|---|---|",
  ...rows.map((r) => `| ${r.scene} | ${typeof r.spp === "number" ? `native ${r.spp} spp (${r.denoiser})` : r.spp} | ${((r.ms as number) / 1000).toFixed(2)} | ${fmt(r.vs_ue_pt as never)} | ${fmt(r.vs_ue_lumen as never)} |`),
].join("\n");
writeFileSync(join(outDir, "quality.md"), md + "\n");
console.log(md);

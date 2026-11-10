// Real-time (60 fps) renderer vs UE5: for each reference scene, capture the live viewport k frames after a history
// reset (k = 1 ~ first frame after a disocclusion / fast motion, 256 ~ a few seconds of holding still) and compare
// with UE5 Lumen (MRQ cinematic) and the UE5 Path Tracer.
//   node tools/bench-live.ts [--scenes a,b] [--frames 1,4,16,64,256] [--scale 1|0.6|auto]  (render scale, upsampled like UE TSR)
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const bin = join(root, "renderer/target/release/threert-render");
const arg = (k: string, d: string) => (process.argv.includes(k) ? process.argv[process.argv.indexOf(k) + 1] : d);
const scenes = arg("--scenes", "cornell-box,material-grid,sun-room,neon-reflections,glb-asset,kabukicho").split(",");
const ks = arg("--frames", "1,4,16,64,256").split(",").map(Number);
const scale = arg("--scale", "1");
const out = join(root, "out/bench/live");
mkdirSync(out, { recursive: true });

const compare = (ref: string, img: string) => (existsSync(ref) ? JSON.parse(execFileSync(bin, ["compare", ref, img]).toString()) : null);
const rows: Record<string, unknown>[] = [];
for (const scene of scenes) {
  const p = spawn(bin, ["live", join(root, "out/bundles", `${scene}.js`), "--assets", root, "--width", "1280", "--height", "720", "--fps", "60", "--scale", scale], { stdio: ["pipe", "ignore", "pipe"] });
  const send = (c: object) => p.stdin.write(JSON.stringify(c) + "\n");
  const got = new Map<number, { spp: number; gpu: number }>();
  const done = new Promise<void>((res) => {
    createInterface({ input: p.stderr! }).on("line", (l) => {
      const m = /snapshot .*-(\d+)\.png at history (\d+) \((\d+) spp\/frame, GPU ([\d.]+) ms\)/.exec(l);
      if (m) {
        got.set(Number(m[1]), { spp: Number(m[3]), gpu: Number(m[4]) });
        if (got.size === ks.length) res();
      }
    });
  });
  await new Promise((r) => setTimeout(r, 1500)); // pipelines + adaptive spp settle
  send({ cmd: "reset" });
  for (const k of ks) send({ cmd: "snapshot", path: join(out, `${scene}${scale === "1" ? "" : `-s${scale}`}-${k}.png`), after: k });
  await done;
  send({ cmd: "quit" });
  for (const k of ks) {
    const img = join(out, `${scene}${scale === "1" ? "" : `-s${scale}`}-${k}.png`);
    const g = got.get(k)!;
    const row = { scene, frames: k, scale, spp_per_frame: g.spp, gpu_ms: g.gpu, vs_ue_lumen: compare(join(root, "out/ue/lumen", `${scene}.png`), img), vs_ue_pt: compare(join(root, "out/ue/pathtracer", `${scene}.png`), img) };
    rows.push(row);
    console.log(JSON.stringify(row));
  }
}
// Merge with earlier runs: rows of the scenes measured now replace their old rows.
const previous: Record<string, unknown>[] = existsSync(join(out, "live.json")) ? JSON.parse(readFileSync(join(out, "live.json"), "utf8")) : [];
const merged = [...previous.filter((r) => !scenes.includes(r.scene as string) || String(r.scale ?? "1") !== scale), ...rows];
writeFileSync(join(out, "live.json"), JSON.stringify(merged, null, 1));
const f = (m: any) => (m ? `${m.mean_delta_e} / ${m.psnr_db} / ${m.ssim}` : "–");
const md = ["| scene | frames of history | render scale | spp/frame | GPU ms/frame | vs UE Lumen (ΔE / PSNR / SSIM) | vs UE Path Tracer |", "|---|---:|---:|---:|---:|---|---|", ...merged.map((r: any) => `| ${r.scene} | ${r.frames} | ${r.scale ?? 1} | ${r.spp_per_frame} | ${r.gpu_ms} | ${f(r.vs_ue_lumen)} | ${f(r.vs_ue_pt)} |`)].join("\n");
writeFileSync(join(out, "live.md"), md + "\n");
console.log(md);

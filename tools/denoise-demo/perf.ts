// Real-time speed of each method on Kaleido's show: uncapped, 12 s, the first 3 s (load, warm-up) left out.
//   node tools/denoise-demo/perf.ts [--methods builtin,metalfx-denoised] [--repeat 2]   (best of the repeats)
import { spawn } from "node:child_process";
import { readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const bin = process.env.THREERT_BIN ?? join(root, "renderer/target/release/threert-render");
const bundle = resolve(flag("--bundle", join(root, "out/bundles/kaleido.js")));
const env = { ...process.env, THREERT_SEED: "24301", THREERT_CACHE: "0" };
const pct = (xs: number[], q: number) => [...xs].sort((a, b) => a - b)[Math.floor(q * (xs.length - 1))];
const results: Record<string, unknown> = {};
for (const method of flag("--methods", "builtin,metalfx-denoised").split(",")) {
  let best: { fps: number; gpuP50: number; gpuP95: number } | null = null;
  for (let r = 0; r < Number(flag("--repeat", "2")); r++) {
    const log = `/tmp/denoise-perf-${method}.jsonl`;
    rmSync(log, { force: true });
    const p = spawn(bin, ["live", bundle, "--assets", root, "--presets", join(root, "presets"), "--width", "1920", "--height", "1080",
      "--quality", flag("--quality", "performance"), "--reconstruction", method, "--frame-log", log, "--fps", "1000"], { cwd: root, stdio: ["pipe", "ignore", "ignore"], env });
    await new Promise((res) => setTimeout(res, 12_000));
    p.stdin!.write('{"cmd":"quit"}\n');
    await new Promise((res) => p.on("exit", res));
    const all = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l));
    const fs = all.filter((f) => f.t >= all[0].t + 3000);
    if (fs.some((f) => f.reconstruction !== method)) throw new Error(`${method} did not run on every frame`);
    const run = { fps: (1000 * (fs.length - 1)) / (fs[fs.length - 1].t - fs[0].t), gpuP50: pct(fs.map((f) => f.gpu), 0.5), gpuP95: pct(fs.map((f) => f.gpu), 0.95) };
    if (!best || run.fps > best.fps) best = run;
  }
  results[method] = best;
  console.log(method, JSON.stringify(best));
}
console.log(JSON.stringify(results));

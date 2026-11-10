// Hitches of the live viewport (docs/ROADMAP.md § 4): frame-time p50 / p95 / p99 and spikes (frames over 2x the median)
// while a scenario of game events plays, each spike attributed to the event before it (first compile of a kernel
// variant, a structural rebuild, a heavy handler).
//
//   node tools/bench-hitches.ts [--bundle out/bundles/kabukicho.js] [--assets DIR] [--scale auto]
//                               [--events '[[2,"lights"],[4,"mood"],[6,"rain"]]'] [--seconds 9] [--force]
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { frameStats, rendererBin, repoRoot, requireQuietGpu, summary } from "./lib/bench.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const bundle = resolve(flag("--bundle", join(repoRoot, "out/bundles/kabukicho.js")));
const events: [number, string][] = JSON.parse(flag("--events", '[[3,"lights"],[5,"mood"],[7,"rain"],[9,"viewpoint"]]'));
const seconds = Number(flag("--seconds", "12"));
const warmup = 2; // s after the scene is up: dropped
requireQuietGpu();
const log = join(repoRoot, "out/bench/hitches.jsonl");
const p = spawn(rendererBin, ["live", bundle, "--assets", resolve(flag("--assets", repoRoot)), "--scale", flag("--scale", "auto"), "--frame-log", log], { stdio: ["pipe", "ignore", "pipe"] });
let started = 0;
p.stderr!.on("data", (d: Buffer) => {
  if (!started && String(d).includes("live:")) started = performance.now();
});
while (!started) await new Promise((r) => setTimeout(r, 50));
for (const [t, type] of events) setTimeout(() => p.stdin!.write(JSON.stringify({ cmd: "event", type }) + "\n"), (warmup + t) * 1000);
await new Promise((r) => setTimeout(r, (warmup + seconds) * 1000));
p.stdin!.write('{"cmd":"quit"}\n');
await new Promise((r) => p.on("exit", r));

type F = { t: number; interval: number; cpu: number; script: number; gpu: number; scale: number };
const frames = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l) as F);
const t0 = frames[0].t + warmup * 1000;
const kept = frames.filter((f) => f.t >= t0);
const st = frameStats(kept.map((f) => f.interval));
const gpu = frameStats(kept.map((f) => f.gpu));
console.log(`${kept.length} frames: interval p50 ${st.p50.toFixed(1)} p95 ${st.p95.toFixed(1)} p99 ${st.p99.toFixed(1)} max ${st.max.toFixed(1)} ms, ${st.spikes} spikes; GPU p50 ${gpu.p50.toFixed(1)} p99 ${gpu.p99.toFixed(1)} ms`);
// Spikes after events (within 1 s).
for (const f of kept.filter((f) => f.interval > 2 * st.p50)) {
  const since = f.t - t0;
  const ev = events.filter(([t]) => t * 1000 <= since + 50).at(-1);
  console.log(`  spike ${f.interval.toFixed(1)} ms at ${(since / 1000).toFixed(2)} s (cpu ${f.cpu.toFixed(1)}, script ${f.script.toFixed(1)}, gpu ${f.gpu.toFixed(1)})${ev && since - ev[0] * 1000 < 1000 ? ` after "${ev[1]}"` : ""}`);
}
const name = bundle.split("/").pop()!.replace(/\.js$/, "");
summary(`hitches ${name}: frame p99`, st.p99, { unit: " ms", digits: 1 });
summary(`hitches ${name}: spikes`, Math.max(st.spikes, 0.5), { digits: 0 });

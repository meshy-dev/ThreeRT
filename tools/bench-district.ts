// LOD / streaming acceptance (docs/ROADMAP.md § 5, item 5): the district in the live viewport. Bistro tiled 3 x 3
// (games/bistro-district, 25.5 M triangles; ROADMAP § 0 step 10) along Bistro's own camera path through the middle copy;
// --scene kabukicho: the Kabukichō district of before (games/kabukicho/district.game.ts, 3 x 3 blocks of distinct
// streets; --row: its middle row only) driven down its middle street.
//
//   node tools/bench-district.ts [--scene bistro|kabukicho] [--row] [--scale 0.5|auto] [--set-baseline]
//
// Startup: time from launch to the first frame. Drive (camera commands at 60 Hz): Bistro: the FBX camera path's first
// 50 s (about 2 m/s); Kabukichō: down the middle street at 15 m/s (a car in town), eye 1.7 m, then back at 30 m/s
// (tools/lib/live.ts `Drive`). Per frame (live --frame-log): GPU time and the frame interval -> p50 / p95 / p99 / max
// and spikes (> 2 x the median). Memory: the process's physical footprint (`footprint`; on Apple silicon it includes
// GPU resources) at the end of the drive.
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { frameStats, requireQuietGpu, summary } from "./lib/bench.ts";
import { bistroDrive, KABUKICHO_DRIVE } from "./lib/live.ts";
import { BISTRO } from "../scenes/bistro.ts";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const row = args.includes("--row");
const bistro = flag("--scene", "bistro") === "bistro";
const name = bistro ? "bistro-district" : row ? "kabukicho-district-row" : "kabukicho-district";
const source = bistro ? "games/bistro-district/bistro-district.game.ts" : `games/kabukicho/${row ? "district-row" : "district"}.game.ts`;
const driveOf = bistro ? bistroDrive(join(root, BISTRO)) : KABUKICHO_DRIVE;
const bundle = join(root, "out/bundles", `${name}.js`);
const scale = flag("--scale", "0.5");
const bin = process.env.THREERT_BIN ?? join(root, "renderer/target/release/threert-render");
const log = join(root, "out/bench", `${name}-frames.jsonl`);
if (!existsSync(bundle)) {
  execFileSync("node", [join(root, "tools/bundle-game.ts"), join(root, source), "-o", bundle], { stdio: "inherit" });
}
requireQuietGpu();
mkdirSync(join(root, "out/bench"), { recursive: true });
rmSync(log, { force: true });


const t0 = performance.now();
const p = spawn(bin, ["live", bundle, "--assets", root, "--width", "1280", "--height", "720", "--scale", scale, "--frame-log", log], { stdio: ["pipe", "ignore", "pipe"] });
let failed = "";
const first = new Promise<number>((res) =>
  createInterface({ input: p.stderr! }).on("line", (l) => {
    if (l.includes("fps on")) res(performance.now() - t0);
    if (/panicked|error/i.test(l)) failed ||= l;
  }),
);
const exited = new Promise<number>((res) => p.on("exit", (c) => res(c ?? 1)));
const startup = await Promise.race([first, exited.then(() => -1)]);
if (startup < 0) {
  console.log(`${name}: the renderer exited before its first frame: ${failed}`);
  process.exit(1);
}
const send = (o: object) => p.stdin!.write(JSON.stringify(o) + "\n");
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
send({ cmd: "camera", ...driveOf.at(0) });
await sleep(3000);
const driveStart = performance.now();
for (let k = 1, n = Math.round(driveOf.seconds * 60); k <= n; k++) {
  send({ cmd: "camera", ...driveOf.at(k / 60) });
  await sleep(Math.max(0, driveStart + (k * 1000) / 60 - performance.now()));
}
const fp = execFileSync("footprint", ["-p", String(p.pid)]).toString();
const mem = /phys_footprint:\s*([\d.]+)\s*([KMG]B)/.exec(fp) ?? /Footprint:\s*([\d.]+)\s*([KMG]B)/.exec(fp);
const memMB = mem ? +mem[1] * ({ KB: 1 / 1024, MB: 1, GB: 1024 } as Record<string, number>)[mem[2]] : NaN;
send({ cmd: "quit" });
await exited;

type F = { t: number; interval: number; gpu: number; scale: number };
const frames = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l) as F);
const tDrive = frames[0].t + (driveStart - t0) - startup; // frame-log times start with the first frame
const kept = frames.filter((f) => f.t >= tDrive);
const iv = frameStats(kept.map((f) => f.interval));
const gpu = frameStats(kept.map((f) => f.gpu));
const scales = kept.map((f) => f.scale).sort((a, b) => a - b);
console.log(`${name} at scale ${scale}: first frame after ${(startup / 1000).toFixed(1)} s; ${kept.length} frames while driving`);
console.log(`  GPU p50 ${gpu.p50.toFixed(1)} p95 ${gpu.p95.toFixed(1)} p99 ${gpu.p99.toFixed(1)} max ${gpu.max.toFixed(1)} ms`);
console.log(`  interval p50 ${iv.p50.toFixed(1)} p95 ${iv.p95.toFixed(1)} p99 ${iv.p99.toFixed(1)} max ${iv.max.toFixed(1)} ms, ${iv.spikes} spikes, ${kept.filter((f) => f.interval > 50).length} frames over 50 ms`);
console.log(`  render scale ${scales[0]?.toFixed(2)}..${scales.at(-1)?.toFixed(2)}; memory (physical footprint) ${memMB.toFixed(0)} MB`);
summary(`${name} ${scale}: GPU p95`, gpu.p95, { unit: " ms", digits: 1 });
summary(`${name} ${scale}: frame p99`, iv.p99, { unit: " ms", digits: 1 });
summary(`${name} ${scale}: first frame`, startup / 1000, { unit: " s", digits: 1 });

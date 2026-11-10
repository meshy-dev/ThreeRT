// 10 k instances against 10 k separate meshes (ROADMAP § 6 step 2e): games/instancing/bench*.game.ts live (1280 x 720,
// 0.5 scale), alternated over rounds. Setup: the scene graph in the game (`graph`), setup with the engine calls in the
// node runtime (`setup`), the renderer's scene build and acceleration structures (`build`, `accel`, from its load
// timings) and the first frame from launch (`first`). Then 3 s still and 8 s with every copy moving each frame (the
// "move" event): frame start interval p50 / p95, the frame's CPU, the frame script in QuickJS (`js`), the in-place
// work in the renderer (instance updates `instanced`, re-transformed triangles `skin` + `upload`), GPU and trace ms.
// The GPU lock is held.
//
//   node tools/bench-instancing.ts [--rounds 2]
//
// Result: out/bench/instancing/<time>.json and a table on stdout.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { acquireGpuLock, frameStats, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";

const args = process.argv.slice(2);
const rounds = Number(args.includes("--rounds") ? args[args.indexOf("--rounds") + 1] : "2");
const outDir = join(repoRoot, "out/bench/instancing");
mkdirSync(outDir, { recursive: true });
const holder = acquireGpuLock("bench-instancing");
if (holder) { console.log(`not measured: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }

const MODES = { instanced: "bench.game.ts", meshes: "bench-meshes.game.ts" } as const;
type Mode = keyof typeof MODES;
const b = branchBuild(rendererBin);
const bundles = {} as Record<Mode, string>;
for (const [mode, file] of Object.entries(MODES) as [Mode, string][]) {
  bundles[mode] = join(repoRoot, "out/bundles", `instancing-bench-${mode}.js`);
  execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/instancing", file), "-o", bundles[mode]], { stdio: "ignore" });
}

type Frame = { interval: number; start?: number; cpu: number; js?: number; script?: number; instanced?: number; skin?: number; upload?: number; gpu: number; trace: number; rebuild: number; fallback: string | null };
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function once(mode: Mode) {
  const log = join(outDir, "frames.jsonl");
  const live = startLive(b, bundles[mode], { frameLog: log });
  const first = await live.firstFrame;
  // (The log's last line may be half written.)
  const frames = () => readFileSync(log, "utf8").split("\n").filter((l) => l.endsWith("}")).map((l) => JSON.parse(l) as Frame);
  await sleep(3000);
  const a = frames().length;
  await sleep(3000);
  const still = frames().slice(a);
  live.send({ cmd: "event", type: "move" });
  await sleep(1000);
  const c = frames().length;
  await sleep(8000);
  const moving = frames().slice(c);
  await live.quit();
  const setupLine = live.lines.find((l) => l.startsWith("game: setup")) ?? "";
  const graph = JSON.parse(live.lines.find((l) => l.includes("bench setup "))!.split("bench setup ")[1]) as { graph_ms: number };
  const load = JSON.parse(live.lines.find((l) => l.startsWith("live: load "))!.slice("live: load ".length)) as Record<string, number>;
  return {
    mode, first_ms: Math.round(first), graph_ms: graph.graph_ms, setup_ms: Number(/setup (\d+) ms/.exec(setupLine)?.[1]), objects: Number(/(\d+) objects/.exec(setupLine)?.[1]),
    build_ms: load.build_ms, load, still, moving,
  };
}

const runs: Awaited<ReturnType<typeof once>>[] = [];
for (let r = 0; r < rounds; r++) {
  for (const mode of (r % 2 ? ["meshes", "instanced"] : ["instanced", "meshes"]) as Mode[]) {
    const x = await once(mode);
    runs.push(x);
    console.error(`round ${r + 1} ${mode}: first frame ${x.first_ms} ms, setup ${x.setup_ms} ms, build ${x.build_ms?.toFixed(0)} ms`);
  }
}
writeFileSync(join(outDir, `${new Date().toISOString().replace(/[:.]/g, "-")}.json`), JSON.stringify(runs));

const med = (v: number[]) => [...v].sort((p, q) => p - q)[Math.floor(v.length / 2)];
const f1 = (v: number) => (Number.isFinite(v) ? v.toFixed(1) : "–");
const pick = (fs: Frame[], k: keyof Frame) => fs.map((f) => Number(f[k] ?? 0));
console.log("| | objects | graph ms | setup ms | build ms | first frame ms | still: start p50 / p95, CPU p50 | moving: start p50 / p95 | CPU p50 / p95 | js p50 | instanced / skin+upload p50 | GPU p50 | trace p50 | rebuilds |");
console.log("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
for (const mode of Object.keys(MODES) as Mode[]) {
  const rs = runs.filter((x) => x.mode === mode);
  const stat = (sel: (x: (typeof rs)[number]) => number) => med(rs.map(sel));
  const st = (fs: Frame[], k: keyof Frame) => frameStats(pick(fs, k));
  const startOf = (fs: Frame[]) => fs.map((f) => f.start ?? f.interval);
  console.log(`| ${mode} | ${rs[0].objects} | ${stat((x) => x.graph_ms)} | ${stat((x) => x.setup_ms)} | ${f1(stat((x) => x.build_ms))} | ${stat((x) => x.first_ms)} | ` +
    `${f1(stat((x) => frameStats(startOf(x.still)).p50))} / ${f1(stat((x) => frameStats(startOf(x.still)).p95))}, ${f1(stat((x) => st(x.still, "cpu").p50))} | ` +
    `${f1(stat((x) => frameStats(startOf(x.moving)).p50))} / ${f1(stat((x) => frameStats(startOf(x.moving)).p95))} | ${f1(stat((x) => st(x.moving, "cpu").p50))} / ${f1(stat((x) => st(x.moving, "cpu").p95))} | ` +
    `${f1(stat((x) => st(x.moving, "js").p50))} | ${f1(stat((x) => st(x.moving, "instanced").p50))} / ${f1(stat((x) => frameStats(x.moving.map((f) => (f.skin ?? 0) + (f.upload ?? 0))).p50))} | ` +
    `${f1(stat((x) => st(x.moving, "gpu").p50))} | ${f1(stat((x) => st(x.moving, "trace").p50))} | ${rs.reduce((s, x) => s + x.moving.filter((f) => f.rebuild > 0).length, 0)} |`);
}

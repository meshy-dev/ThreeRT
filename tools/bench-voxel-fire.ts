// An explosion's fire and smoke (ROADMAP § 6 step 10 b; docs/VOXEL.md, "Fire and smoke"): what the fire light must be,
// how the live frame compares with our offline render, and what it costs on the benchmark level.
//
//  calibrate  games/voxel-level/level-fire.game.ts `fireRoom`: a fireball of 2.2 m and of 8.8 m burning over a grey
//             floor before a grey wall, offline at ages through its life, against a 1,000 cd point light at its centre:
//             per channel the floor's and the wall's radiance near it, fire over light x 1,000 cd, per m² of the
//             fireball's size² and per nit of its intensity: the rows of `FIRE_LIGHT`.
//  quality    `fireShot`: the benchmark level by day, a 4 m explosion's fire 0.2, 0.5 and 1.0 s after it starts (at 4 s),
//             the live frame (lockstep at 60 fps from the start, render scale 0.75: never 1, where the live frame has
//             no edge anti-aliasing) against our offline render at that scene time (no clamps, no denoiser), which has
//             the fireball's own light and no fire light. Live with the fire light and without (what it buys). ΔE (mean,
//             p99) and SSIM over the image, the fireball, the street before it, and 3 x 3 regions.
//  smoke      the same with the smoke grid, the fire at 0 s (its puff at the grid's first step, live and offline): 1.0 s
//             and 3.0 s after; with the grid's turbulence (a random push, so live and offline draw different smoke) and
//             without (the same flow: what is left is the rendering).
//  cost       games/voxel-level/level-destruction{,-smoke,-fire}.game.ts at 0.75, lockstep (frames as fast as the GPU
//             goes): the frame's CPU and GPU ms for 2 s before and 4 s after the 4 m explosion (`crater`); with fire,
//             `sustained` (7 a second for 10 s) and `peak` (20 within a second): frame times, fireballs burning, lights.
//             `--software`: instead, the fire route's crater under software traversal (THREERT_HWRT=0, loop guard 8,192).
//
//   node tools/bench-voxel-fire.ts [--only calibrate,quality,smoke,cost] [--spp 2048] [--software]
//
// One renderer at a time. A step that gives no answer in its time ends this script with exit 3 and leaves the renderer
// alone (never killed). Result: out/bench/voxel-fire/<time>.json and tables on stdout.
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fireRoomView, fireShotView } from "../games/voxel-level/level-fire.game.ts";
import { FIRE_LIGHT, FIREBALL_SIZE } from "../packages/voxel/src/explosions.ts";
import { bundleGame } from "./bundle-game.ts";
import { frameStats, rendererBin, repoRoot } from "./lib/bench.ts";
import { generate } from "./voxel-level/generate.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const only = opt("--only", "calibrate,quality,smoke,cost").split(",");
const SPP = Number(opt("--spp", "2048"));
const SCALE = opt("--scale", "0.75");
const dir = join(repoRoot, "out/voxel-fire");
const truthDir = join(repoRoot, "out/cache/truth");
const outDir = join(repoRoot, "out/bench/voxel-fire");
for (const d of [dir, truthDir, outDir]) mkdirSync(d, { recursive: true });
const r2 = (v: number) => +v.toFixed(2);
const r3 = (v: number) => +v.toFixed(3);
const result: Record<string, unknown> = { at: new Date().toISOString(), spp: SPP, scale: SCALE };

/** A bundle of a game module's builder called with options. */
async function bundle(name: string, module: string, fn: string, o: object): Promise<string> {
  const entry = join(dir, `${name}.entry.ts`);
  writeFileSync(entry, `import { ${fn} } from ${JSON.stringify(join(repoRoot, module))};\nexport default ${fn}(${JSON.stringify(o)});\n`);
  return (await bundleGame(entry, join(dir, `${name}.js`))).out;
}

/** Our unbiased offline render of a bundle at a scene time (png and linear HDR), cached by content. */
function offline(name: string, b: string, time: number, spp = SPP, size?: [number, number]): { png: string; hdr: string } {
  const key = createHash("sha256").update(readFileSync(b)).update(readFileSync(rendererBin)).update(`fire ${spp} ${time} ${size}`).digest("hex").slice(0, 12);
  const png = join(truthDir, `${name}-${key}.png`), hdr = join(truthDir, `${name}-${key}.raw`);
  if (!existsSync(png) || !existsSync(hdr)) {
    const t0 = performance.now();
    execFileSync(rendererBin, ["render", b, "-o", png, "--hdr", hdr, "--assets", repoRoot, "--spp", String(spp), "--time", String(time), "--no-denoise", ...(size ? ["--width", String(size[0]), "--height", String(size[1])] : []), "--override", JSON.stringify({ native: { clampIndirect: 1e9 } })], { stdio: ["ignore", "ignore", "inherit"], timeout: 0 });
    console.log(`  offline ${name} at ${time} s (${spp} spp): ${((performance.now() - t0) / 1000).toFixed(1)} s`);
  }
  return { png, hdr };
}

type M = { mean_delta_e: number; p99_delta_e: number; ssim: number; luminance_ratio: number };
const compare = (ref: string, test: string, region?: number[]): M => JSON.parse(execFileSync(rendererBin, ["compare", ref, test, ...(region ? ["--region", region.map((v) => v.toFixed(4)).join(",")] : [])]).toString().trim().split("\n").pop()!);
const fmt = (m: M) => `${m.mean_delta_e.toFixed(2)} / ${m.p99_delta_e.toFixed(1)} / ${m.ssim.toFixed(3)} / ${m.luminance_ratio.toFixed(2)}`;
const meets = (m: M) => m.mean_delta_e <= 3 && m.ssim >= 0.9;

/** A point's place in the image (u, v from the top left) for a camera at `pos` looking at `target`. */
function projector(pos: readonly number[], target: readonly number[], fovDeg: number, aspect = 16 / 9) {
  const f = target.map((t, i) => t - pos[i]), fl = Math.hypot(...f), fw = f.map((v) => v / fl);
  const r = [-fw[2], 0, fw[0]], rl = Math.hypot(...r), rt = r.map((v) => v / rl);
  const up = [rt[1] * fw[2] - rt[2] * fw[1], rt[2] * fw[0] - rt[0] * fw[2], rt[0] * fw[1] - rt[1] * fw[0]];
  const t = Math.tan((fovDeg * Math.PI) / 360);
  return (p: readonly number[]): [number, number] => {
    const d = p.map((v, i) => v - pos[i]);
    const z = d[0] * fw[0] + d[1] * fw[1] + d[2] * fw[2], x = d[0] * rt[0] + d[1] * rt[1] + d[2] * rt[2], y = d[0] * up[0] + d[1] * up[1] + d[2] * up[2];
    return [0.5 + x / (z * t * aspect) / 2, 0.5 - y / (z * t) / 2];
  };
}
const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
/** The image region holding the points (their bounding box, clamped to the image). */
function box(points: [number, number][]): number[] {
  return [Math.min(...points.map((p) => p[0])), Math.min(...points.map((p) => p[1])), Math.max(...points.map((p) => p[0])), Math.max(...points.map((p) => p[1]))].map(clamp01);
}
const corners = (lo: number[], hi: number[]) => [0, 1].flatMap((i) => [0, 1].flatMap((j) => [0, 1].map((k) => [i ? hi[0] : lo[0], j ? hi[1] : lo[1], k ? hi[2] : lo[2]])));

/** Mean linear radiance (rgb) of an HDR render (RGB f32) over a region. */
function hdrMean(raw: string, [u0, v0, u1, v1]: number[], w: number, h: number): [number, number, number] {
  const px = new Float32Array(readFileSync(raw).buffer.slice(0));
  const s = [0, 0, 0];
  let n = 0;
  for (let y = Math.floor(v0 * h); y < Math.ceil(v1 * h); y++) for (let x = Math.floor(u0 * w); x < Math.ceil(u1 * w); x++) {
    for (let c = 0; c < 3; c++) s[c] += px[(y * w + x) * 3 + c];
    n++;
  }
  return s.map((v) => v / n) as [number, number, number];
}

/** One headless live session driven in lockstep; never killed (a step without an answer ends this script). */
function live(name: string, b: string, o: { env?: Record<string, string>; frameLog?: string } = {}) {
  const argv = ["live", b, "--assets", repoRoot, "--presets", join(repoRoot, "presets"), "--width", "1280", "--height", "720", "--scale", SCALE];
  if (o.frameLog) {
    rmSync(o.frameLog, { force: true });
    argv.push("--frame-log", o.frameLog);
  }
  const p = spawn(rendererBin, argv, { stdio: ["pipe", "ignore", "pipe"], env: { ...process.env, ...o.env } });
  const lines: string[] = [];
  let waiters: [(l: string) => boolean, (l: string) => void][] = [];
  createInterface({ input: p.stderr! }).on("line", (l) => {
    lines.push(l);
    waiters = waiters.filter(([pred, res]) => (pred(l) ? (res(l), false) : true));
  });
  let exited = false;
  const gone = new Promise<void>((r) => p.on("exit", () => { exited = true; r(); }));
  const send = (c: object) => p.stdin!.write(JSON.stringify(c) + "\n");
  const wait = (pred: (l: string) => boolean, what: string, ms = 180_000) => new Promise<string>((res) => {
    const hit = lines.find(pred);
    if (hit) return res(hit);
    const t = setTimeout(() => {
      console.log(`live ${name}: STUCK waiting for ${what} (${lines.slice(-3).join(" | ")}); the renderer (pid ${p.pid}) is left alone`);
      p.unref();
      process.exit(3);
    }, ms);
    waiters.push([pred, (l) => (clearTimeout(t), res(l))]);
    gone.then(() => {
      if (!lines.find(pred)) {
        console.log(`live ${name}: the renderer exited while waiting for ${what} (${lines.slice(-3).join(" | ")})`);
        process.exit(1);
      }
    });
  });
  // Lockstep from the first frame: the frame script, the grids and the shaders all step 1/60 s a frame.
  send({ cmd: "lockstep", dt: 1 / 60 });
  let frame = 0, seq = 0;
  return {
    lines,
    /** `n` frames on; with `snapshot`, the last one saved there. */
    async frames(n: number, snapshot?: string) {
      if (n <= 0 && !snapshot) return;
      const id = `s${seq++}`;
      send({ cmd: "frames", n, ...(snapshot ? { snapshot } : { sync: id }) });
      await wait((l) => (snapshot ? l.includes(snapshot) : l === `live: sync ${id}`), snapshot ? `the snapshot ${snapshot}` : `frames to ${frame + n}`);
      frame += n;
    },
    event(type: string) { send({ cmd: "event", type }); },
    get frame() { return frame; },
    async quit() {
      send({ cmd: "quit" });
      const t = setTimeout(() => { console.log(`live ${name}: did not quit in 60 s; left alone (pid ${p.pid})`); p.unref(); process.exit(3); }, 60_000);
      await gone;
      clearTimeout(t);
      if (exited && p.exitCode !== 0) console.log(`live ${name}: exit ${p.exitCode}`);
    },
  };
}

if (only.includes("calibrate")) {
  const ages = [0.05, 0.15, 0.3, 0.5, 0.7, 0.9], life = 1.6, intensity = 30000, W = 640, H = 360;
  const rows: Record<string, unknown>[] = [];
  console.log(`\ncalibrate: the fireball's equivalent point light (cd per m² of size² per nit; floor | wall), offline ${SPP} spp`);
  for (const d of [2.2, 8.8]) {
    const v = fireRoomView(d);
    const proj = projector(v.pos, v.target, v.fov);
    const floor = box(corners([1.2 * d, 0, -0.4 * d], [2.0 * d, 0, 0.4 * d]).map(proj));
    const wall = box(corners([1.0 * d, 0.2 * d, v.wall], [1.8 * d, 1.0 * d, v.wall]).map(proj));
    const lb = await bundle(`room-${d}-light`, "games/voxel-level/level-fire.game.ts", "fireRoom", { size: d, light: true });
    const lt = offline(`room-${d}-light`, lb, 0.5, SPP, [W, H]);
    const ref = { floor: hdrMean(lt.hdr, floor, W, H), wall: hdrMean(lt.hdr, wall, W, H) };
    const fb = await bundle(`room-${d}-fire`, "games/voxel-level/level-fire.game.ts", "fireRoom", { size: d });
    for (const u of ages) {
      const t = offline(`room-${d}-fire`, fb, u * life, SPP, [W, H]);
      const per = (region: number[], base: number[]) => hdrMean(t.hdr, region, W, H).map((x, c) => (x / Math.max(base[c], 1e-12)) * 1000 / (d * d * intensity));
      const f = per(floor, ref.floor), w = per(wall, ref.wall);
      rows.push({ size: d, u, floor: f.map((x) => +x.toPrecision(3)), wall: w.map((x) => +x.toPrecision(3)) });
      console.log(`  ${String(d).padStart(4)} m  u ${u.toFixed(2)}  ${f.map((x) => x.toFixed(3)).join(" ")} | ${w.map((x) => x.toFixed(3)).join(" ")}`);
    }
  }
  // The table: per age the mean over sizes and surfaces.
  const ages2 = [...new Set(rows.map((r) => r.u as number))];
  const table = ages2.map((u) => {
    const at = rows.filter((r) => r.u === u);
    const m = [0, 1, 2].map((c) => at.reduce((s, r) => s + (r.floor as number[])[c] + (r.wall as number[])[c], 0) / (2 * at.length));
    return [u, ...m.map((x) => +x.toPrecision(3))];
  });
  result.calibrate = { rows, table, current: FIRE_LIGHT };
  console.log(`  measured table (u, r, g, b): ${JSON.stringify(table)}`);
}

/** The fire shot's regions: the fireball at full size, the street before it, and 3 x 3. */
function shotRegions(r = 4) {
  const v = fireShotView(1);
  const proj = projector(v.pos, v.target, v.fov);
  const c = [v.spot[0], v.spot[1] + 0.3 * r, v.spot[2]], rad = (FIREBALL_SIZE * r) / 2;
  const fire = box(corners([c[0] - rad, c[1] - rad, c[2] - rad], [c[0] + rad, c[1] + rad, c[2] + rad]).map(proj));
  const regions: [string, number[]][] = [["all", [0, 0, 1, 1]], ["fireball", fire], ["street", [clamp01(fire[0] - 0.15), fire[3], clamp01(fire[2] + 0.15), 1]]];
  if (fire[3] >= 0.98) regions.pop();
  for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) regions.push([`r${i}${j}`, [j / 3, i / 3, (j + 1) / 3, (i + 1) / 3]]);
  return regions;
}

function judge(ref: string, test: string) {
  return Object.fromEntries(shotRegions().map(([k, reg]) => [k, compare(ref, test, k === "all" ? undefined : reg)]));
}
function report(tag: string, j: Record<string, M>) {
  const short = ["all", "fireball", "street"].filter((k) => j[k]).map((k) => `${k} ${fmt(j[k])}${meets(j[k]) ? "" : " ✗"}`).join("  ");
  const grid = Object.entries(j).filter(([k]) => k.startsWith("r")).map(([k, m]) => `${k} ${m.mean_delta_e.toFixed(1)}/${m.ssim.toFixed(2)}`).join(" ");
  console.log(`  ${tag.padEnd(26)} ${short}\n  ${"".padEnd(26)} ${grid}`);
}

if (only.includes("quality")) {
  generate(1, "object");
  const at = 4, ages = [0.2, 0.5, 1.0];
  const refB = await bundle("shot-ref", "games/voxel-level/level-fire.game.ts", "fireShot", { light: false, at });
  const litB = await bundle("shot-lit", "games/voxel-level/level-fire.game.ts", "fireShot", { light: true, at });
  console.log(`\nquality: the live frame (lockstep 60 fps, scale ${SCALE}) against our offline render (${SPP} spp), the 4 m explosion's fire; ΔE mean / p99 / SSIM / luminance (✗: ΔE > 3 or SSIM < 0.9)`);
  console.log(`  regions: ${shotRegions().slice(0, 3).map(([k, r]) => `${k} ${r.map((x) => x.toFixed(3)).join(",")}`).join("; ")}`);
  const truths = ages.map((a) => offline(`shot-${a}`, refB, at + a));
  const rows: Record<string, unknown>[] = [];
  for (const [tag, b] of [["with the fire light", litB], ["without", refB]] as const) {
    const s = live(`shot-${tag}`, b);
    const shots: string[] = [];
    for (const a of ages) {
      const png = join(dir, `shot-${tag.replace(/\W+/g, "-")}-${a}.png`);
      const target = Math.round((at + a) * 60);
      await s.frames(target - s.frame - 1);
      await s.frames(1, png);
      shots.push(png);
    }
    const fireLine = s.lines.find((l) => l.startsWith("fire "));
    await s.quit();
    for (let k = 0; k < ages.length; k++) {
      const j = judge(truths[k].png, shots[k]);
      rows.push({ tag, age: ages[k], j });
      report(`${tag}, ${ages[k]} s`, j);
    }
    console.log(`  (${fireLine ?? "no fire line"})`);
  }
  // The offline reference's own noise: the same frame at half the samples.
  const half = offline(`shot-0.5-half`, refB, at + 0.5, SPP / 2);
  const own = judge(truths[1].png, half.png);
  report(`offline ${SPP / 2} vs ${SPP} spp, 0.5 s`, own);
  result.quality = { rows, own };
}

if (only.includes("smoke")) {
  generate(1, "object");
  const times = [1.0, 3.0];
  const rows: Record<string, unknown>[] = [];
  console.log(`\nsmoke: the fire and its smoke from 0 s, live (scale ${SCALE}) against offline (${SPP} spp)`);
  for (const turbulence of [undefined, 0]) {
    const t = turbulence === undefined ? "turbulence 15" : "turbulence 0";
    const refB = await bundle(`smoke-ref-${t}`, "games/voxel-level/level-fire.game.ts", "fireShot", { light: false, smoke: true, at: 0, ...(turbulence === undefined ? {} : { turbulence }) });
    const litB = await bundle(`smoke-lit-${t}`, "games/voxel-level/level-fire.game.ts", "fireShot", { light: true, smoke: true, at: 0, ...(turbulence === undefined ? {} : { turbulence }) });
    const truths = times.map((x) => offline(`smoke-${t.replace(" ", "")}-${x}`, refB, x));
    const s = live(`smoke-${t}`, litB);
    const shots: string[] = [];
    for (const x of times) {
      const png = join(dir, `smoke-${t.replace(" ", "")}-${x}.png`);
      await s.frames(Math.round(x * 60) - s.frame - 1);
      await s.frames(1, png);
      shots.push(png);
    }
    await s.quit();
    for (let k = 0; k < times.length; k++) {
      const j = judge(truths[k].png, shots[k]);
      rows.push({ turbulence: t, time: times[k], j });
      report(`${t}, ${times[k]} s`, j);
    }
  }
  result.smoke = rows;
}

if (only.includes("cost")) {
  generate(1, "object");
  type Frame = { interval: number; cpu: number; gpu: number; voxel?: [number, number]; guard?: number | null };
  const routes: [string, string, Record<string, string>][] = [["mesh", "level-destruction.game.ts", {}], ["smoke", "level-destruction-smoke.game.ts", {}], ["fire", "level-destruction-fire.game.ts", {}]];
  if (args.includes("--software")) routes.splice(0, routes.length, ["fire, software", "level-destruction-fire.game.ts", { THREERT_HWRT: "0", THREERT_LOOP_GUARD: "8192" }]);
  const rows: Record<string, unknown>[] = [];
  console.log(`\ncost: lockstep at scale ${SCALE}, frames as fast as the GPU goes; ms p50 / p95 / max`);
  const st = (xs: number[]) => { const s = frameStats(xs); return `${s.p50.toFixed(1)} / ${s.p95.toFixed(1)} / ${Math.max(...xs).toFixed(1)}`; };
  for (const [tag, game, env] of routes) {
    const b = (await bundleGame(join(repoRoot, "games/voxel-level", game), join(dir, game.replace(".ts", ".js")))).out;
    for (const scenario of tag === "fire" && !args.includes("--crater") ? ["crater", "sustained", "peak"] : ["crater"]) {
      const log = join(dir, `cost-${tag.replace(/\W+/g, "-")}-${scenario}.jsonl`);
      const s = live(`cost-${tag}-${scenario}`, b, { env, frameLog: log });
      await s.frames(240); // warm: the level loaded, the radiance cache trained
      s.event(scenario);
      await s.frames(scenario === "sustained" ? 11 * 60 : scenario === "peak" ? 4 * 60 : 5 * 60);
      const explosions = s.lines.filter((l) => l.startsWith("explosion ")).map((l) => JSON.parse(l.slice(10)) as { burning?: number; lit?: number });
      await s.quit();
      const F = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l) as Frame);
      const ex = F.findIndex((f, k) => k > 200 && (f.voxel?.[0] ?? 0) > 0);
      if (ex < 0) {
        console.log(`  ${tag} ${scenario}: no explosion in the frame log`);
        continue;
      }
      const before = F.slice(Math.max(ex - 120, 0), ex), after = F.slice(ex, ex + (scenario === "crater" ? 240 : scenario === "sustained" ? 600 : 180));
      const row = {
        tag, scenario, explosion_frame: { cpu: r2(Math.max(F[ex].cpu, F[ex + 1]?.cpu ?? 0)), gpu: r2(Math.max(F[ex].gpu, F[ex + 1]?.gpu ?? 0)) },
        before: { cpu: frameStats(before.map((f) => f.cpu)), gpu: frameStats(before.map((f) => f.gpu)) }, after: { cpu: frameStats(after.map((f) => f.cpu)), gpu: frameStats(after.map((f) => f.gpu)), gpu_max: r2(Math.max(...after.map((f) => f.gpu))) },
        burning_max: Math.max(0, ...explosions.map((e) => e.burning ?? 0)), lit_max: Math.max(0, ...explosions.map((e) => e.lit ?? 0)), explosions: explosions.length,
        guard: Math.max(0, ...F.map((f) => f.guard ?? 0)),
        // The GPU ms per frame over the 4 s after, in 0.25 s bins (the fire burns 1.6 s; the smoke after).
        gpu_bins: Array.from({ length: Math.ceil(after.length / 15) }, (_, k) => r2(frameStats(after.slice(k * 15, k * 15 + 15).map((f) => f.gpu)).p50)),
      };
      rows.push(row);
      console.log(`  ${tag.padEnd(15)} ${scenario.padEnd(9)} before: CPU ${st(before.map((f) => f.cpu))}, GPU ${st(before.map((f) => f.gpu))};  explosion frame CPU ${row.explosion_frame.cpu}, GPU ${row.explosion_frame.gpu};  after: CPU ${st(after.map((f) => f.cpu))}, GPU ${st(after.map((f) => f.gpu))}${tag.startsWith("fire") ? `; burning at most ${row.burning_max}, lit ${row.lit_max}` : ""}${row.guard ? `; guard ${row.guard}` : ""}`);
      console.log(`  ${"".padEnd(25)} GPU p50 per 0.25 s after: ${row.gpu_bins.join(" ")}`);
    }
  }
  result.cost = rows;
}

const file = join(outDir, `${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
writeFileSync(file, JSON.stringify(result, null, 1) + "\n");
console.log(`-> ${file}`);

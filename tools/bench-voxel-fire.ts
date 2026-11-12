// An explosion's fire and smoke (ROADMAP § 6 step 10 b; docs/VOXEL.md, "Fire and smoke"): what the fire light must be,
// how the live frame compares with our offline render, and what it costs on the benchmark level.
//
//  calibrate  games/voxel-level/level-fire.game.ts `fireRoom`: fireballs of 1.1, 2.2, 4.4 and 8.8 m burning over a grey
//             floor before a grey wall, offline at ages through its life, against a 1,000 cd point light at its centre:
//             per channel the floor's and the wall's radiance near it, fire over light x 1,000 cd, per m² of the
//             fireball's size² and per nit of its intensity: the rows of `FIRE_LIGHT`.
//  quality    `fireShot`: the benchmark level by day, a 4 m explosion's fire 0.2, 0.5 and 1.0 s after it starts (at 4 s),
//             the live frame (lockstep at 60 fps from the start, render scale 0.75: never 1, where the live frame has
//             no edge anti-aliasing) against our offline render at that scene time (no clamps, no denoiser), which has
//             the fireball's own light and no fire light. Live with the fire light and without (what it buys). ΔE (mean,
//             p99) and SSIM over the image, the fireball, the street before it, and 3 x 3 regions.
//  cost       games/voxel-level/level-destruction{,-smoke,-fire}.game.ts at 0.75, lockstep (frames as fast as the GPU
//             goes): the frame's CPU and GPU ms for 2 s before and 4 s after the 4 m explosion (`crater`); with fire,
//             `sustained` (7 a second for 10 s) and `peak` (20 within a second): frame times, fireballs burning, lights.
//             `--software`: instead, the fire route's crater under software traversal (THREERT_HWRT=0, loop guard 8,192).
//
//   node tools/bench-voxel-fire.ts [--only calibrate,quality,cost] [--spp 2048] [--software]
//
// One renderer at a time. A step that gives no answer in its time ends this script with exit 3 and leaves the renderer
// alone (never killed). Result: out/bench/voxel-fire/<time>.json and tables on stdout.
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fireRoomView, fireShotView } from "../games/voxel-level/level-fire.game.ts";
import { FIRE_LIGHT, FIRE_LIGHT_SIZES, FIREBALL_SIZE } from "../packages/voxel/src/explosions.ts";
import { bundleGame } from "./bundle-game.ts";
import { frameStats, rendererBin, repoRoot } from "./lib/bench.ts";
import { generate } from "./voxel-level/generate.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const only = opt("--only", "calibrate,quality,cost").split(",");
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
    /** From now on each frame steps `dt` s (the frame script, the grids and the shaders). */
    step(dt: number) { send({ cmd: "lockstep", dt }); },
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
  const ages = [0.02, 0.05, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9], life = 1.6, intensity = 30000, W = 640, H = 360;
  const rows: Record<string, unknown>[] = [];
  console.log(`\ncalibrate: the fireball's equivalent point light (cd per m² of size² per nit; floor | wall), offline ${SPP} spp`);
  for (const d of FIRE_LIGHT_SIZES) {
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
  // The table: per age and size the mean of the floor's and the wall's.
  const table = ages.map((u) => [u, ...FIRE_LIGHT_SIZES.map((d) => {
    const r = rows.find((x) => x.u === u && x.size === d)!;
    return [0, 1, 2].map((c) => +(((r.floor as number[])[c] + (r.wall as number[])[c]) / 2).toPrecision(3));
  })]);
  result.calibrate = { rows, table, current: FIRE_LIGHT };
  console.log(`  measured table (u, then rgb per size ${FIRE_LIGHT_SIZES.join(", ")} m):`);
  for (const row of table) console.log(`    ${JSON.stringify(row)},`);
}

/** The fire shot's regions at the fireball's age (s; it grows as the root of age / life): the fireball's box, the
 *  street below it, and 3 x 3. */
function shotRegions(age: number, r = 4, life = 1.6) {
  const v = fireShotView(1);
  const proj = projector(v.pos, v.target, v.fov);
  const c = [v.spot[0], v.spot[1] + 0.3 * r, v.spot[2]], rad = ((FIREBALL_SIZE * r) / 2) * Math.sqrt(Math.min(age / life, 1));
  const fire = box(corners([c[0] - rad, c[1] - rad, c[2] - rad], [c[0] + rad, c[1] + rad, c[2] + rad]).map(proj));
  const regions: [string, number[]][] = [["all", [0, 0, 1, 1]], ["fireball", fire], ["street", [clamp01(fire[0] - 0.15), fire[3], clamp01(fire[2] + 0.15), 1]]];
  if (fire[3] >= 0.98) regions.pop();
  for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) regions.push([`r${i}${j}`, [j / 3, i / 3, (j + 1) / 3, (i + 1) / 3]]);
  return regions;
}

function judge(ref: string, test: string, age: number) {
  return Object.fromEntries(shotRegions(age).map(([k, reg]) => [k, compare(ref, test, k === "all" ? undefined : reg)]));
}
function report(tag: string, j: Record<string, M>) {
  const short = ["all", "fireball", "street"].filter((k) => j[k]).map((k) => `${k} ${fmt(j[k])}${meets(j[k]) ? "" : " ✗"}`).join("  ");
  const grid = Object.entries(j).filter(([k]) => k.startsWith("r")).map(([k, m]) => `${k} ${m.mean_delta_e.toFixed(1)}/${m.p99_delta_e.toFixed(0)}/${m.ssim.toFixed(2)}`).join(" ");
  console.log(`  ${tag.padEnd(26)} ${short}\n  ${"".padEnd(26)} ${grid}`);
}

if (only.includes("quality")) {
  generate(1, "object");
  const at = 4, ages = [0.2, 0.5, 1.0];
  const refB = await bundle("shot-ref", "games/voxel-level/level-fire.game.ts", "fireShot", { light: false, at });
  const litB = await bundle("shot-lit", "games/voxel-level/level-fire.game.ts", "fireShot", { light: true, at });
  console.log(`\nquality: the live frame (lockstep 60 fps, scale ${SCALE}) against our offline render (${SPP} spp), the 4 m explosion's fire; ΔE mean / p99 / SSIM / luminance (✗: ΔE > 3 or SSIM < 0.9)`);
  for (const a of ages) console.log(`  regions at ${a} s: ${shotRegions(a).slice(1, 3).map(([k, r]) => `${k} ${r.map((x) => x.toFixed(3)).join(",")}`).join("; ")} (cells: ΔE mean / p99 / SSIM)`);
  const truths = ages.map((a) => offline(`shot-${a}`, refB, at + a));
  const rows: Record<string, unknown>[] = [];
  // The fire light twice (the live view's run-to-run spread: its radiance cache trains in an order the GPU decides).
  const lives: string[][] = [];
  for (const [tag, b] of [["with the fire light", litB], ["with it again", litB], ["without", refB]] as const) {
    const s = live(`shot-${tag}`, b);
    const shots: string[] = [];
    for (const a of ages) {
      const png = join(dir, `shot-${tag.replace(/\W+/g, "-")}-${a}.png`);
      const target = Math.round((at + a) * 60);
      await s.frames(target - s.frame - 1);
      await s.frames(1, png);
      shots.push(png);
    }
    const fireLine = s.lines.find((l) => l.includes("] fire {"));
    await s.quit();
    for (let k = 0; k < ages.length; k++) {
      const j = judge(truths[k].png, shots[k], ages[k]);
      rows.push({ tag, age: ages[k], j });
      report(`${tag}, ${ages[k]} s`, j);
    }
    console.log(`  (${fireLine ?? "no fire line"})`);
    lives.push(shots);
  }
  // Held: the same frame with time all but stopped (1e-5 s a frame) for 120 frames: the live view's own converged
  // picture of that moment. Against offline it is the live media's bias; against the real-time frame, the lag of the
  // temporal filter behind a fireball that changes every frame.
  for (let k = 0; k < ages.length; k++) {
    const s = live(`held-${ages[k]}`, litB);
    const png = join(dir, `shot-held-${ages[k]}.png`);
    // To the real-time snapshot's own frame (its age), then held there.
    await s.frames(Math.round((at + ages[k]) * 60));
    s.step(1e-5);
    await s.frames(120, png);
    await s.quit();
    const j = judge(truths[k].png, png, ages[k]), lag = judge(png, lives[0][k], ages[k]);
    rows.push({ tag: "held", age: ages[k], j }, { tag: "real-time vs held", age: ages[k], j: lag });
    report(`held, ${ages[k]} s`, j);
    report(`real-time vs held, ${ages[k]} s`, lag);
  }
  for (let k = 0; k < ages.length; k++) {
    const j = judge(lives[0][k], lives[1][k], ages[k]);
    rows.push({ tag: "live A/A", age: ages[k], j });
    report(`live run 2 vs run 1, ${ages[k]} s`, j);
  }
  // The offline reference's own noise: the same frame at half the samples.
  const half = offline(`shot-0.5-half`, refB, at + 0.5, SPP / 2);
  const own = judge(truths[1].png, half.png, 0.5);
  report(`offline ${SPP / 2} vs ${SPP} spp, 0.5 s`, own);
  result.quality = { rows, own };
}

if (only.includes("cost")) {
  generate(1, "object");
  type Frame = { interval: number; cpu: number; gpu: number; voxel?: [number, number]; guard?: number | null; rebuild: number; fallback: string | null };
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
      const explosions = s.lines.filter((l) => /^(\[game\] )?explosion \{/.test(l)).map((l) => JSON.parse(l.slice(l.indexOf("{"))) as { burning?: number; lit?: number });
      // The loose parts' top speed after the explosion (the game logs it every half second; step 10 c1's blast).
      const fastest = Math.max(0, ...s.lines.filter((l) => /^(\[game\] )?(bodies|explosion) \{/.test(l)).map((l) => (JSON.parse(l.slice(l.indexOf("{"))) as { fastest?: number }).fastest ?? 0));
      // The bodies' speeds just after the first explosion (the parts it cut loose, kicked: step 10 c1).
      const speeds = (explosions[0] as { speeds?: { n: number; p50: number; p90: number }; fastest?: number; kicked?: number } | undefined) ?? {};
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
        fastest,
        speeds_after_explosion: { kicked: speeds.kicked ?? 0, ...(speeds.speeds ?? { n: 0, p50: 0, p90: 0 }), max: speeds.fastest ?? 0 },
        rebuilds_after: after.filter((f) => f.rebuild > 0).map((f) => `${r2(f.rebuild)} ms${f.fallback ? ` (${f.fallback})` : ""}`),
        // The GPU ms per frame over the 4 s after, in 0.25 s bins (the fire burns 1.6 s; the smoke after).
        gpu_bins: Array.from({ length: Math.ceil(after.length / 15) }, (_, k) => r2(frameStats(after.slice(k * 15, k * 15 + 15).map((f) => f.gpu)).p50)),
      };
      rows.push(row);
      console.log(`  ${tag.padEnd(15)} ${scenario.padEnd(9)} before: CPU ${st(before.map((f) => f.cpu))}, GPU ${st(before.map((f) => f.gpu))};  explosion frame CPU ${row.explosion_frame.cpu}, GPU ${row.explosion_frame.gpu};  after: CPU ${st(after.map((f) => f.cpu))}, GPU ${st(after.map((f) => f.gpu))}${tag.startsWith("fire") ? `; burning at most ${row.burning_max}, lit ${row.lit_max}` : ""}${row.guard ? `; guard ${row.guard}` : ""}; rebuilds after: ${row.rebuilds_after.length ? row.rebuilds_after.join(", ") : "none"}; loose parts' top speed ${row.fastest} m/s; just after the explosion: ${row.speeds_after_explosion.kicked} kicked, ${row.speeds_after_explosion.n} moving, p50 ${row.speeds_after_explosion.p50} / p90 ${row.speeds_after_explosion.p90} / max ${row.speeds_after_explosion.max} m/s`);
      console.log(`  ${"".padEnd(25)} GPU p50 per 0.25 s after: ${row.gpu_bins.join(" ")}`);
    }
  }
  result.cost = rows;
}

const file = join(outDir, `${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
writeFileSync(file, JSON.stringify(result, null, 1) + "\n");
console.log(`-> ${file}`);

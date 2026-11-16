// The scene check: a short clip of each typical scene drawn by the live renderer, laid out as one grid video, so a
// broken picture shows at a glance; at a few moments of each clip, the frame against an offline render of the same
// moment (the standard). Run by .github/workflows/reel.yml on the reel runner (tools/reel/RUNNER.md), or by hand.
//
//   node tools/reel-check.ts [--clips a,b,...] [--out DIR] [--baseline run.json] [--truth-spp 256] [--no-truth]
//
// Each clip: lockstep at 60 fps, `settle` s not kept, then 4 s, every second frame kept (the video is 30 fps); the
// frames at the keyframes (and the frame after each, for flicker) are kept too. A keyframe is a moment the offline
// renderer can reproduce from the scene's clock, room and camera alone: before the clip's first event, and only for
// scenes whose state follows the clock (not physics). Offline renders are cached by bundle, settings and
// TRUTH_VERSION in THREERT_REEL_CACHE (default out/cache/reel-truth); a missing one is rendered with this build and
// marked `truth_fresh`.
// Checks: every kept frame for black, white or flat (broken); the renderer's error lines; at keyframes ΔE, SSIM and
// detail against offline and flicker; with --baseline (a previous run.json, normally main's), what got worse.
// A renderer that stops answering (a frame, a load past 15 min, an offline render past 30 min) or reports a panic or
// a lost device writes THREERT_REEL_INCIDENT (default out/reel-incident) and stops: the process is left alone (a hung
// GPU dispatch is never killed and retried), and the runner takes no job until a person removes the file.
// The grid's labels need an ffmpeg with drawtext (FFMPEG=<binary>; Homebrew's ffmpeg-full); without it, no labels.
// -> DIR/grid.mp4 (4 x 3 tiles of 480 x 270), DIR/<clip>/key-*.png + sheets, DIR/run.json, DIR/summary.md
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import sharp from "sharp";
import { installAssets } from "../showcase/assets.ts";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, waitGpuLock } from "./lib/bench.ts";
import { bistroCamera, ev, machineName, recordClip, type Clip, type Pose } from "./lib/reel.ts";

/** Bump when a change to the offline renderer changes what it converges to: every cached render is redone. */
const TRUTH_VERSION = 1;
const FPS = 60, EVERY = 2, SECONDS = 4, W = 960, H = 540;

interface CheckClip extends Clip {
  /** Keyframes (s into the recording) the offline renderer can reproduce. */
  truth?: number[];
  /** Paths under assets/ the clip needs; without one it is reported as not run. */
  needs?: string[];
}
const CLIPS: Record<string, CheckClip> = {
  crowd: { what: "Bistro night, 150 walkers: many lights, skinning, the light tree", src: "games/bistro-crowd/bistro-crowd.game.ts", room: "w150-v0", scale: 0.5, camera: (t) => bistroCamera(0.4 + 0.22 * t), truth: [2], needs: ["orca/bistro"] },
  tokyo: { what: "Tokyo from afternoon into night: sky, time of day, lamps coming on", src: "games/tokyo/tokyo-tod.game.ts", room: "shiba-15.6-17.7-10.5-r1600", settle: 4, truth: [3], needs: ["city/glb/shiba"] },
  kabukicho: { what: "Kabukichō in the rain: emissive signs, wet streets", src: "games/kabukicho/kabukicho.game.ts", scale: 0.5, truth: [2] },
  zeroday: { what: "Zero-Day: a large scene, glossy machines, small emitters", src: "games/zero-day/zero-day.game.ts", scale: 0.5, truth: [2], needs: ["orca/zeroday"] },
  voxels: { what: "Voxel level under explosions: edits, physics, fire and its light", src: "games/voxel-level/level-destruction-fire.game.ts", at: [[0.3, ev("show")]], needs: ["voxel/level/s1v2-object"] },
  home: { what: "Daylight Home: interior GI, then night with the lamps on", src: "showcase/daylight-home/daylight-home.game.ts", at: [[1.0, ev("time")]], truth: [0.5] },
  colour: { what: "Colour Lab: lamps switched, how fast the history follows", src: "showcase/colour-lab/colour-lab.game.ts", at: [[1.2, ev("red")], [2.4, ev("red")], [3.4, ev("green")]], truth: [0.6] },
  jewellery: { what: "Jeweller's Window: metals and gems turning under soft boxes", src: "showcase/jewellery/jewellery.game.ts", truth: [2] },
  refraction: { what: "Refraction: glass and transmission", src: "showcase/refraction/refraction.game.ts", truth: [2] },
  museum: { what: "Sculpture Gallery: walking up to marble and bronze", src: "showcase/museum/museum.game.ts", at: [[0.6, ev("next")]], truth: [0.3] },
  rooms: { what: "768 shadow-casting lamps behind walls: the light sampler", src: "scenes/lamp-rooms-wide.ts", scale: 0.5, truth: [2] },
  foliage: { what: "Ferns and plants in the sun: alpha cutouts in wind", src: "tools/foliage-scene.ts", truth: [2], needs: ["polyhaven/models/fern_02_1k.glb", "polyhaven/models/potted_plant_02_1k.glb", "polyhaven/models/potted_plant_04_1k.glb"] },
};
const ORDER = ["crowd", "tokyo", "kabukicho", "zeroday", "voxels", "home", "colour", "jewellery", "refraction", "museum", "rooms", "foliage"];

const args = process.argv.slice(2);
const opt = (k: string, d = "") => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const ids = opt("--clips") ? opt("--clips").split(",") : ORDER;
for (const id of ids) if (!CLIPS[id]) throw new Error(`reel-check: no clip ${id} (${ORDER.join(", ")})`);
const out = resolve(opt("--out", join(repoRoot, "out/reel-check", new Date().toISOString().replace(/[:.]/g, "-"))));
const cache = resolve(process.env.THREERT_REEL_CACHE ?? join(repoRoot, "out/cache/reel-truth"));
const incident = resolve(process.env.THREERT_REEL_INCIDENT ?? join(repoRoot, "out/reel-incident"));
const truthSpp = Number(opt("--truth-spp", "256"));
const noTruth = args.includes("--no-truth");
const baseline = opt("--baseline") && existsSync(opt("--baseline")) ? (JSON.parse(readFileSync(opt("--baseline"), "utf8")) as Run) : null;
if (existsSync(incident)) {
  console.error(`reel-check: ${incident} exists (an earlier run stopped on a renderer that did not answer); a person removes it after checking the machine`);
  process.exit(3);
}

type Key = { t: number; dE: number; p99: number; ssim: number; lum: number; detail: number; flicker: number; truth_fresh: boolean };
type ClipResult = { id: string; what: string; status: "ok" | "worse" | "broken" | "not run"; reasons: string[]; scale: number; load_s?: number; gpu_ms?: { p50: number; p95: number }; over_budget?: number; bad_frames?: number; keys: Key[]; warnings: string[] };
type Run = { version: 1; commit: string; ref: string; started: string; finished: string; seconds: number; machine: string; adapter: unknown; renderer: string; width: number; height: number; fps: number; status: string; clips: ClipResult[]; trigger: Record<string, string> };

const git = (...a: string[]) => execFileSync("git", a, { cwd: repoRoot, encoding: "utf8" }).trim();
const pad = (k: number) => String(k).padStart(4, "0");
const r3 = (x: number) => Math.round(x * 1000) / 1000;
const keyName = (t: number) => `key-${t.toFixed(2)}`;
const offlineCamera = (p: Pose) => ({ position: p.pos, target: p.target, ...(p.tan ? { fov: (2 * Math.atan(p.tan) * 180) / Math.PI } : {}) });

const luma = async (f: string, w?: number) => {
  const img = sharp(f).removeAlpha();
  const { data, info } = await (w ? img.resize(w) : img).raw().toBuffer({ resolveWithObject: true });
  const l = new Float32Array(info.width * info.height);
  for (let i = 0; i < l.length; i++) l[i] = 0.2126 * data[i * 3] + 0.7152 * data[i * 3 + 1] + 0.0722 * data[i * 3 + 2];
  return { l, w: info.width, h: info.height };
};
/** A frame that is black, white or flat (no picture). */
async function broken(f: string): Promise<string | null> {
  const { l } = await luma(f, 96);
  const mean = l.reduce((a, b) => a + b, 0) / l.length;
  const sd = Math.sqrt(l.reduce((a, b) => a + (b - mean) ** 2, 0) / l.length);
  return mean < 2 ? "black" : mean > 252 ? "white" : sd < 1 ? "flat" : null;
}
/** How much of the reference's detail a frame keeps: the regression of its luma gradients on the reference's. */
async function detail(ref: string, img: string): Promise<number> {
  const R = await luma(ref), T = await luma(img);
  let num = 0, den = 0;
  for (let y = 1; y < R.h - 1; y++) {
    for (let x = 1; x < R.w - 1; x++) {
      const i = y * R.w + x, rx = R.l[i + 1] - R.l[i - 1], ry = R.l[i + R.w] - R.l[i - R.w];
      num += rx * (T.l[i + 1] - T.l[i - 1]) + ry * (T.l[i + R.w] - T.l[i - R.w]);
      den += rx * rx + ry * ry;
    }
  }
  return num / den;
}
/** RMS of the luma change between two frames (0-255). */
async function flicker(a: string, b: string): Promise<number> {
  const A = await luma(a), B = await luma(b);
  let s = 0;
  for (let i = 0; i < A.l.length; i++) s += (A.l[i] - B.l[i]) ** 2;
  return Math.sqrt(s / A.l.length);
}
const compare = (ref: string, img: string, sheet: string) =>
  JSON.parse(execFileSync(rendererBin, ["compare", ref, img, "--sheet", sheet]).toString().trim().split("\n").pop()!) as { mean_delta_e: number; p99_delta_e: number; ssim: number; luminance_ratio: number };

/** Writes the incident file and stops, leaving any renderer process as it is (a hung GPU dispatch is never killed). */
function stopOnIncident(what: string): never {
  writeFileSync(incident, `${new Date().toISOString()} ${what}\n`);
  console.error(`reel-check: ${what}; wrote ${incident} and stopped`);
  finish(what);
  process.exit(3);
}

/** The offline render of a clip's moment (scene time `time`), cached; whether it was rendered now. */
async function truth(c: CheckClip, bundle: string, t: number, time: number): Promise<{ path: string; fresh: boolean }> {
  const o = { width: W, height: H, spp: truthSpp, time: r3(time), room: c.room ?? null, camera: c.camera ? offlineCamera(c.camera(t)) : null };
  const key = createHash("sha256").update(readFileSync(bundle)).update(JSON.stringify({ o, v: TRUTH_VERSION })).digest("hex").slice(0, 20);
  const path = join(cache, `${key}.png`);
  if (existsSync(path)) return { path, fresh: false };
  mkdirSync(cache, { recursive: true });
  const extra = [...(o.room ? ["--room", o.room] : []), ...(o.camera ? ["--camera", JSON.stringify(o.camera)] : [])];
  const tmp = `${path}.${process.pid}.png`;
  const p = spawn(rendererBin, ["render", bundle, "-o", tmp, "--assets", repoRoot, "--presets", join(repoRoot, "presets"), "--width", String(W), "--height", String(H), "--spp", String(truthSpp), "--time", String(o.time), ...extra], { stdio: ["ignore", "ignore", "inherit"] });
  const watchdog = setTimeout(() => stopOnIncident(`offline render of ${c.src} at ${o.time} s still running after 30 min`), 30 * 60_000);
  const code = await new Promise<number | null>((r) => p.on("exit", r));
  clearTimeout(watchdog);
  if (code !== 0 || !existsSync(tmp)) throw new Error(`offline render failed (exit ${code})`);
  renameSync(tmp, path);
  return { path, fresh: true };
}

/** What got worse against the baseline's same clip. */
function worse(r: ClipResult, b?: ClipResult): string[] {
  if (!b || b.status === "not run" || b.status === "broken") return [];
  const why: string[] = [];
  for (const k of r.keys) {
    const bk = b.keys.find((x) => x.t === k.t);
    if (!bk) continue;
    if (k.dE > bk.dE * 1.25 + 0.3) why.push(`ΔE at ${k.t} s ${bk.dE} -> ${k.dE}`);
    if (k.ssim < bk.ssim - 0.03) why.push(`SSIM at ${k.t} s ${bk.ssim} -> ${k.ssim}`);
    if (k.flicker > bk.flicker * 1.5 + 0.5) why.push(`flicker at ${k.t} s ${bk.flicker} -> ${k.flicker}`);
  }
  if (r.gpu_ms && b.gpu_ms && r.gpu_ms.p50 > b.gpu_ms.p50 * 1.15 + 0.3) why.push(`GPU ms p50 ${b.gpu_ms.p50} -> ${r.gpu_ms.p50}`);
  return why;
}


rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
installAssets();
await waitGpuLock("reel-check");
const started = new Date();
const adapter = (() => {
  try {
    return JSON.parse(execFileSync(rendererBin, ["info"], { encoding: "utf8" }).trim().split("\n").pop()!);
  } catch {
    return null;
  }
})();
const results: ClipResult[] = [];

for (const id of ids) {
  const c = CLIPS[id], dir = join(out, id), frames = join(dir, "frames");
  const scale = c.scale ?? 0.75;
  const res: ClipResult = { id, what: c.what, status: "ok", reasons: [], scale, keys: [], warnings: [] };
  results.push(res);
  const missing = (c.needs ?? []).filter((f) => !existsSync(join(repoRoot, "assets", f)));
  if (missing.length) {
    Object.assign(res, { status: "not run", reasons: [`missing: ${missing.map((f) => `assets/${f}`).join(", ")}`] });
    console.log(`${id}: not run (${res.reasons[0]})`);
    continue;
  }
  mkdirSync(frames, { recursive: true });
  const bundle = join(repoRoot, "out/bundles", `reel-check-${id}.js`);
  await bundleGame(join(repoRoot, c.src), bundle);
  const n = SECONDS * FPS;
  const keys = noTruth ? [] : (c.truth ?? []);
  const keyFrames = new Map<number, string>();
  for (const t of keys) {
    const k = Math.round(t * FPS);
    keyFrames.set(k, join(dir, `${keyName(t)}.png`)).set(k + 1, join(dir, `${keyName(t)}-next.png`));
  }
  const video = (k: number) => join(frames, `${pad(k / EVERY)}.png`);
  try {
    // Loading a large scene (Tokyo: 2.5 GB of tiles) takes minutes; a frame takes milliseconds.
    const r = await recordClip(c, bundle, { width: W, height: H, fps: FPS, frames: n, scale, loadMs: 15 * 60_000, frameLog: join(dir, "frames.jsonl"), label: `reel-check ${id}`, file: (k) => keyFrames.get(k) ?? (k % EVERY === 0 ? video(k) : null) });
    Object.assign(res, { load_s: r.load_s, gpu_ms: r.gpu_ms, over_budget: r.over_budget, warnings: r.warnings });
  } catch (e) {
    const msg = String((e as Error).message ?? e);
    Object.assign(res, { status: "broken", reasons: [`${/timed out/.test(msg) ? "renderer stopped answering" : "recording failed"}: ${msg.slice(0, 300)}`] });
    if (/timed out/.test(msg)) stopOnIncident(`${id}: the renderer stopped answering`);
    console.log(`${id}: broken (${msg.slice(0, 200)})`);
    continue;
  }
  if (res.warnings.some((l) => /panic|device lost/i.test(l))) {
    Object.assign(res, { status: "broken", reasons: ["renderer panic / device lost"] });
    stopOnIncident(`${id}: renderer panic / device lost`);
  }
  // Keyframes that fall on a video frame go into the video too.
  for (const [k, f] of keyFrames) if (k % EVERY === 0 && k < n && existsSync(f)) copyFileSync(f, video(k));
  const kept = readdirSync(frames).filter((f) => f.endsWith(".png")).sort();
  if (kept.length !== n / EVERY) res.reasons.push(`${kept.length} of ${n / EVERY} frames`);
  const bad: string[] = [];
  for (const f of kept) {
    const b = await broken(join(frames, f));
    if (b) bad.push(`${f} ${b}`);
  }
  res.bad_frames = bad.length;
  if (bad.length) res.reasons.push(`${bad.length} frame(s) with no picture (${bad.slice(0, 3).join(", ")})`);
  for (const t of keys) {
    try {
      const live = join(dir, `${keyName(t)}.png`);
      const tr = await truth(c, bundle, t, (c.settle ?? 4) + t);
      copyFileSync(tr.path, join(dir, `${keyName(t)}-offline.png`));
      const m = compare(tr.path, live, join(dir, `${keyName(t)}-sheet.png`));
      res.keys.push({ t, dE: r3(m.mean_delta_e), p99: r3(m.p99_delta_e), ssim: r3(m.ssim), lum: r3(m.luminance_ratio), detail: r3(await detail(tr.path, live)), flicker: r3(await flicker(live, join(dir, `${keyName(t)}-next.png`))), truth_fresh: tr.fresh });
    } catch (e) {
      res.reasons.push(`keyframe ${t} s: ${String((e as Error).message ?? e).slice(0, 200)}`);
    }
  }
  if (res.reasons.length) res.status = "broken";
  else {
    const why = worse(res, baseline?.clips.find((x) => x.id === id));
    if (why.length) Object.assign(res, { status: "worse", reasons: why });
  }
  console.log(`${id}: ${res.status}${res.reasons.length ? ` (${res.reasons.join("; ")})` : ""}; GPU ms p50 ${res.gpu_ms?.p50}; ${res.keys.map((k) => `ΔE ${k.dE} SSIM ${k.ssim}`).join(", ")}`);
}
const status = finish();
process.exit(status === "broken" ? 1 : 0);

/** run.json and summary.md, then the grid (its failure is reported, not fatal). Returns the run's status. */
function finish(incidentNote?: string): ClipResult["status"] {
  const rank = { ok: 0, "not run": 1, worse: 2, broken: 3 } as const;
  const status = results.reduce((s, r) => (rank[r.status] > rank[s] ? r.status : s), "ok" as ClipResult["status"]);
  const finished = new Date();
  const run: Run = {
    version: 1, commit: git("rev-parse", "HEAD"), ref: process.env.GITHUB_REF_NAME ?? git("rev-parse", "--abbrev-ref", "HEAD"),
    started: started.toISOString(), finished: finished.toISOString(), seconds: Math.round((+finished - +started) / 1000),
    machine: machineName(), adapter, renderer: rendererBin, width: W, height: H, fps: FPS / EVERY, status, clips: results,
    trigger: Object.fromEntries(Object.entries({ event: process.env.GITHUB_EVENT_NAME, actor: process.env.GITHUB_ACTOR, run_id: process.env.GITHUB_RUN_ID, pr: process.env.REEL_PR, baseline: baseline?.commit, incident: incidentNote }).filter(([, v]) => v)) as Record<string, string>,
  };
  const write = (grid: string) => {
    writeFileSync(join(out, "run.json"), JSON.stringify({ ...run, grid }, null, 1) + "\n");
    const icon = { ok: "🟢", worse: "🟠", broken: "🔴", "not run": "⚪" };
    const md = [
      `### Scene check ${icon[status]} ${status}`,
      `\`${run.commit.slice(0, 9)}\` on ${run.machine}, ${run.seconds} s${baseline ? `, against \`${baseline.commit.slice(0, 9)}\`` : ""}${incidentNote ? `. **Stopped: ${incidentNote}**` : ""}${grid === "ok" ? "" : `. Grid: ${grid}`}`,
      "",
      "| clip | | GPU ms p50 | ΔE / SSIM vs offline | flicker | notes |",
      "|---|---|---|---|---|---|",
      ...results.map((r) => `| ${r.id} | ${icon[r.status]} | ${r.gpu_ms?.p50 ?? ""} | ${r.keys.map((k) => `${k.dE} / ${k.ssim}${k.truth_fresh ? "*" : ""}`).join(", ")} | ${r.keys.map((k) => k.flicker).join(", ")} | ${r.reasons.join("; ").replace(/\|/g, "/")} |`),
      "",
      results.some((r) => r.keys.some((k) => k.truth_fresh)) ? "\\* offline rendered by this build (not cached)" : "",
    ].join("\n");
    writeFileSync(join(out, "summary.md"), md + "\n");
    return md;
  };
  write("not built");
  let grid = "ok";
  try {
    buildGrid();
  } catch (e) {
    grid = `failed (${String((e as Error).message ?? e).split("\n")[0].slice(0, 160)})`;
  }
  console.log(`\n${write(grid)}\n-> ${out}`);
  return status;
}

/** The grid: 4 x 3 tiles of 480 x 270, each with its clip's name and verdict (when ffmpeg has drawtext); a clip not
 *  recorded is a grey tile. FFMPEG names the binary (default ffmpeg on PATH). */
function buildGrid() {
  const ffmpeg = process.env.FFMPEG ?? "ffmpeg";
  const TW = 480, TH = 270, COLS = 4;
  const hasText = execFileSync(ffmpeg, ["-hide_banner", "-filters"], { encoding: "utf8" }).includes(" drawtext ");
  const font = ["/System/Library/Fonts/Supplemental/Arial.ttf", "/System/Library/Fonts/Helvetica.ttc", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"].find(existsSync);
  const tiles = ORDER.filter((id) => ids.includes(id)).map((id) => results.find((r) => r.id === id) ?? ({ id, status: "not run" } as ClipResult));
  const inputs: string[] = [], chains: string[] = [];
  tiles.forEach((r, i) => {
    const frames = join(out, r.id, "frames");
    if (existsSync(join(frames, "0000.png"))) inputs.push("-framerate", String(FPS / EVERY), "-i", join(frames, "%04d.png"));
    else inputs.push("-f", "lavfi", "-i", `color=c=0x303030:s=${TW}x${TH}:r=${FPS / EVERY}:d=${SECONDS}`);
    const colour = r.status === "ok" ? "white" : r.status === "worse" ? "orange" : "red";
    const text = hasText && font ? `,drawtext=fontfile=${font}:text='${`${r.id}  ${r.status}`.replace(/[:']/g, " ")}':x=8:y=8:fontsize=18:fontcolor=${colour}:box=1:boxcolor=black@0.55:boxborderw=4` : "";
    chains.push(`[${i}:v]scale=${TW}:${TH}:flags=lanczos,setsar=1${text}[v${i}]`);
  });
  if (!tiles.length) return;
  const layout = tiles.map((_, i) => `${(i % COLS) * TW}_${Math.floor(i / COLS) * TH}`).join("|");
  const filter = tiles.length > 1 ? `${chains.join(";")};${tiles.map((_, i) => `[v${i}]`).join("")}xstack=inputs=${tiles.length}:layout=${layout}:fill=black,format=yuv420p[out]` : `${chains[0]};[v0]format=yuv420p[out]`;
  const grid = join(out, "grid.mp4");
  execFileSync(ffmpeg, ["-hide_banner", "-loglevel", "error", "-y", ...inputs, "-filter_complex", filter, "-map", "[out]", "-shortest", "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-movflags", "+faststart", "-an", grid], { stdio: ["ignore", "inherit", "pipe"] });
  execFileSync(ffmpeg, ["-hide_banner", "-loglevel", "error", "-y", "-ss", "2", "-i", grid, "-frames:v", "1", join(out, "grid.jpg")], { stdio: ["ignore", "inherit", "pipe"] });
}

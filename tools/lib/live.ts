// Live-viewport measurements shared by the perf smoke and the perf suite (docs/WORKFLOW.md): the quiet-GPU check,
// a renderer run with its profile lines and frame log, Kabukichō still and on a scripted move, and the district drive.
// Every function takes the renderer binary and its paths, so the same measurement runs on a branch build and on a
// main commit built elsewhere (tools/lib/build-at.ts).
import { gltfCameraPath } from "./gltf-camera.ts";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { createInterface } from "node:readline";
import { existsSync, readFileSync, rmSync } from "node:fs";
import { frameStats, gpuBusy, acquireGpuLock } from "./bench.ts";

/** A renderer build to measure: binary, asset root (`assets/...`), presets directory, bundles directory. */
export interface Build {
  bin: string;
  assets: string;
  presets: string;
  bundles: string;
  /** What it is (a commit, "branch"), for reports. */
  label: string;
}

/**
 * Environment switches the measurements set, under both names a commit may read: the current one and the one before
 * the ThreeRT rename (bisects cross it). The old name is assembled so the rename script leaves it alone.
 */
export function switches(name: string, value: string): Record<string, string> {
  return { [`THREERT_${name}`]: value, [["AGENT", "3D_"].join("") + name]: value, [["A", "3D_"].join("") + name]: value };
}

/** GPU utilisation (%) from the IOAccelerator statistics, averaged over `ms` (null where the platform has none). */
export async function gpuUtilization(ms = 2000): Promise<number | null> {
  const read = (): number | null => {
    try {
      const out = execFileSync("ioreg", ["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"], { encoding: "utf8" });
      const m = /"Device Utilization %"=(\d+)/.exec(out);
      return m ? Number(m[1]) : null;
    } catch {
      return null;
    }
  };
  const xs: number[] = [];
  const t0 = performance.now();
  while (performance.now() - t0 < ms) {
    const v = read();
    if (v === null) return null;
    xs.push(v);
    await new Promise((r) => setTimeout(r, 200));
  }
  return xs.reduce((a, b) => a + b, 0) / xs.length;
}

/**
 * Whether the GPU is quiet enough to time: no known GPU process (UE, another live renderer, a playground) and the
 * device under 15 % busy over 3 s (that also catches a render farm or a hung job). Returns the reasons it is not.
 */
export async function quietGpu(): Promise<string[]> {
  const reasons = gpuBusy();
  // The desktop itself (window server, an app's UI) shows a few %; a renderer that just quit, a moment of tail.
  await new Promise((r) => setTimeout(r, 2000));
  const u = await gpuUtilization(3000);
  if (u !== null && u > 15) reasons.push(`GPU ${u.toFixed(0)} % busy`);
  return reasons;
}

/** `quietGpu`, then the GPU lock for this run (perf smoke, perf suite): the reasons it is not this run's to time. */
export async function takeGpu(tool?: string): Promise<string[]> {
  const reasons = await quietGpu();
  if (reasons.length) return reasons;
  const h = acquireGpuLock(tool);
  return h ? [`GPU lock: ${h.tool} (pid ${h.pid}) since ${h.started}`] : [];
}

/** A live renderer process: its stderr lines, a way to wait for one, commands, and quit. */
export interface Live {
  p: ChildProcess;
  lines: string[];
  send(cmd: object): void;
  wait(pred: (l: string) => boolean, timeoutMs?: number): Promise<string>;
  /** ms from launch to the first frame ("fps on"). */
  firstFrame: Promise<number>;
  quit(): Promise<void>;
}

/** `stdout`: the runner's stdout lines (what the game posts on the host channel: `{"host":[...]}`); `args`: more flags. */
export function startLive(b: Build, bundle: string, o: { scale?: string; env?: Record<string, string>; frameLog?: string; profile?: boolean; room?: string; width?: number; height?: number; bounces?: number; stdout?: (line: string) => void; args?: string[] } = {}): Live {
  const args = ["live", bundle, "--assets", b.assets, "--presets", b.presets, "--width", String(o.width ?? 1280), "--height", String(o.height ?? 720), "--scale", o.scale ?? "0.5"];
  if (o.bounces) args.push("--bounces", String(o.bounces));
  if (o.room) args.push("--room", o.room);
  if (o.frameLog) {
    rmSync(o.frameLog, { force: true });
    args.push("--frame-log", o.frameLog);
  }
  const t0 = performance.now();
  const env = { ...process.env, ...(o.profile ? switches("PROFILE", "1") : {}), ...o.env };
  if (o.args) args.push(...o.args);
  const p = spawn(b.bin, args, { stdio: ["pipe", o.stdout ? "pipe" : "ignore", "pipe"], env });
  if (o.stdout) createInterface({ input: p.stdout! }).on("line", o.stdout);
  const lines: string[] = [];
  let waiters: [(l: string) => boolean, (l: string) => void][] = [];
  createInterface({ input: p.stderr! }).on("line", (l) => {
    lines.push(l);
    waiters = waiters.filter(([pred, res]) => (pred(l) ? (res(l), false) : true));
  });
  const exited = new Promise<void>((r) => p.on("exit", () => r()));
  const wait = (pred: (l: string) => boolean, timeoutMs = 300_000) =>
    new Promise<string>((res, rej) => {
      const hit = lines.find(pred);
      if (hit) return res(hit);
      const t = setTimeout(() => rej(new Error(`live ${b.label}: timed out waiting (${lines.slice(-3).join(" | ")})`)), timeoutMs);
      waiters.push([pred, (l) => (clearTimeout(t), res(l))]);
      exited.then(() => rej(new Error(`live ${b.label}: exited (${lines.slice(-3).join(" | ")})`)));
    });
  return {
    p,
    lines,
    send: (c) => p.stdin!.write(JSON.stringify(c) + "\n"),
    wait,
    firstFrame: wait((l) => l.includes("fps on")).then(() => performance.now() - t0),
    quit: async () => {
      p.stdin!.write(JSON.stringify({ cmd: "quit" }) + "\n");
      const t = setTimeout(() => p.kill(), 10_000);
      await exited;
      clearTimeout(t);
    },
  };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const median = (xs: number[]) => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? s[Math.floor(s.length / 2)] : NaN;
};
/** GPU ms per frame from the renderer's "profile ms/frame: total X | ..." lines (averages over 120 frames). */
const profileTotals = (lines: string[]) => lines.map((l) => /^profile ms\/frame: total ([\d.]+)/.exec(l)).filter((m) => m).map((m) => Number(m![1]));

/** A scripted camera move for the perf gates: from `pos` looking along `fwd` (unit, y up), ping-pong over 2 s, `dist` m
 *  along the view (and `side` m across it) while the view yaws `yawDeg` degrees; `fovDeg` = vertical field of view. */
export interface GateMove { pos: number[]; fwd: number[]; dist: number; side: number; yawDeg: number; fovDeg: number }

function movePose(m: GateMove, f: number) {
  const yaw0 = Math.atan2(m.fwd[2], m.fwd[0]);
  const yaw = yaw0 - (f * m.yawDeg * Math.PI) / 180;
  const h = Math.hypot(m.fwd[0], m.fwd[2]);
  const along = [m.fwd[0] / h, 0, m.fwd[2] / h], across = [-along[2], 0, along[0]];
  const pos = m.pos.map((v, i) => v + f * (m.dist * along[i] + m.side * across[i]));
  const fwd = [Math.cos(yaw) * h, m.fwd[1], Math.sin(yaw) * h];
  return { pos, target: pos.map((v, i) => v + fwd[i] * 10), tan: Math.tan((m.fovDeg * Math.PI) / 360) };
}

/**
 * A scene (or a variant bundle of it) at the 0.5 floor: after `warmupS`, `seconds` still at the scene's camera, then
 * `seconds` on the scripted `move` (camera commands at 60 Hz). Medians of the profile totals (GPU ms per frame).
 * `events` are game events sent after the warm-up (e.g. "rain" to turn the rain off), `settleS` before measuring.
 */
export async function stillAndMoving(b: Build, bundle: string, move: GateMove | ((f: number) => { pos: number[]; target: number[]; tan: number }), o: { seconds?: number; warmupS?: number; events?: string[]; settleS?: number; env?: Record<string, string>; label?: string } = {}) {
  const seconds = o.seconds ?? 15, warmupS = o.warmupS ?? 8;
  const pose = typeof move === "function" ? move : (f: number) => movePose(move, f);
  const live = startLive(b, bundle, { profile: true, env: o.env });
  const first = await live.firstFrame;
  await sleep(warmupS * 1000);
  for (const e of o.events ?? []) live.send({ cmd: "event", type: e });
  if (o.events?.length) await sleep((o.settleS ?? 4) * 1000);
  let mark = live.lines.length;
  await sleep(seconds * 1000);
  const still = profileTotals(live.lines.slice(mark));
  const t0 = performance.now();
  for (let k = 0; performance.now() - t0 < (seconds + 2) * 1000; k++) {
    const f = (k % 120) / 60;
    live.send({ cmd: "camera", ...pose(f <= 1 ? f : 2 - f) });
    if (k === 120) mark = live.lines.length; // from the second second of the move on
    await sleep(Math.max(0, t0 + ((k + 1) * 1000) / 60 - performance.now()));
  }
  const moving = profileTotals(live.lines.slice(mark));
  await live.quit();
  if (!still.length || !moving.length) throw new Error(`${o.label ?? "scene"} ${b.label}: no profile lines (${live.lines.slice(-3).join(" | ")})`);
  return { first_frame_s: +(first / 1000).toFixed(2), still_ms: +median(still).toFixed(2), moving_ms: +median(moving).toFixed(2), samples: [still.length, moving.length] };
}

/** Bistro's gate move (scenes/bistro.ts STREET): 2.5 m down the street while the view yaws 12°, ping-pong over 2 s. */
export function bistroMove(street: { position: readonly number[]; target: readonly number[]; fov: number }): GateMove {
  const d = street.target.map((v, i) => v - street.position[i]);
  const n = Math.hypot(...d);
  return { pos: [...street.position], fwd: d.map((v) => v / n), dist: 2.5, side: 0, yawDeg: 12, fovDeg: street.fov };
}

/** Kabukichō (or a variant bundle of it) at the 0.5 floor, still then on its scripted move (`stillAndMoving`). */
export function kabukicho(b: Build, bundle: string, o: { seconds?: number; warmupS?: number; events?: string[]; settleS?: number; env?: Record<string, string> } = {}) {
  // The move as it always was: yaw from atan2(0.07, 0.995), 2.5 m along x and 0.3 m along z, y kept.
  const P0 = [-11.5, 1.7, -1.8], YAW0 = Math.atan2(0.07, 0.995), TAN = Math.tan((55 * Math.PI) / 360);
  const pose = (f: number) => {
    const yaw = YAW0 - (f * 12 * Math.PI) / 180;
    const pos = [P0[0] + 2.5 * f, P0[1], P0[2] + 0.3 * f];
    const fwd = [Math.cos(yaw), 0.07, Math.sin(yaw)];
    return { pos, target: pos.map((v, i) => v + fwd[i] * 10), tan: TAN };
  };
  return stillAndMoving(b, bundle, pose, { ...o, label: "kabukicho" });
}

/** Launches the bundle and quits at its first frame (warms the shader caches); returns ms to that frame. */
export async function firstFrame(b: Build, bundle: string): Promise<number> {
  const live = startLive(b, bundle);
  const ms = await live.firstFrame;
  await live.quit();
  return ms;
}

/** The median first frame (s) of `n` launches (after a warm one): one launch varies by ~0.1-0.2 s, which once read as
 *  a regression and sent a bisect after noise. */
export async function firstFrameMedian(b: Build, bundle: string, n = 3): Promise<number> {
  const s: number[] = [];
  for (let k = 0; k < n; k++) s.push(await firstFrame(b, bundle));
  s.sort((x, y) => x - y);
  return +(s[Math.floor(n / 2)] / 1000).toFixed(2);
}

/** A camera drive for the district benches: the pose at drive time t (s) over `seconds`. */
export interface Drive {
  seconds: number;
  at(t: number): { pos: number[]; target: number[]; tan: number };
}

/** Kabukichō's district drive: down the middle street (x from -10 to 515 m at z = 0, eye 1.7 m) at 15 m/s (a car in
 *  town), then back at 30 m/s. */
export const KABUKICHO_DRIVE: Drive = (() => {
  const X0 = -10, X1 = 515, EYE = 1.7, T = Math.tan((55 * Math.PI) / 360);
  const out = (X1 - X0) / 15, back = (X1 - X0) / 30;
  return {
    seconds: out + back,
    at(t: number) {
      const [x, dir] = t <= out ? [X0 + 15 * t, 1] : [X1 - 30 * (t - out), -1];
      return { pos: [x, EYE, 0], target: [x + dir * 30, EYE, 0], tan: T };
    },
  };
})();

/** Bistro's district drive (games/bistro-district): its own FBX camera path through the middle copy, the first 50 s
 *  (about 2 m/s, through the street, the square and back towards the bistro). */
export function bistroDrive(gltf: string, seconds = 50): Drive {
  const path = gltfCameraPath(gltf);
  return { seconds: Math.min(seconds, path.duration), at: (t) => path.at(t) };
}

/**
 * A district (or any scene) driven along `drive` (tools/bench-district.ts; default Kabukichō's): camera commands at
 * 60 Hz. GPU p50 / p95 and frame-interval p50 / p95 / p99 from the frame log, frames over 50 ms, first frame; the
 * frame-start interval p50 / p95 (`start`: start to start, what the pacer sets) where the frame log has it.
 */
export async function district(b: Build, bundle: string, frameLog: string, drive: Drive = KABUKICHO_DRIVE, o: { room?: string; profile?: boolean; env?: Record<string, string> } = {}) {
  const launch = performance.now();
  const live = startLive(b, bundle, { frameLog, room: o.room, profile: o.profile, env: o.env });
  const first = await live.firstFrame;
  live.send({ cmd: "camera", ...drive.at(0) });
  await sleep(3000);
  const tDrive = performance.now();
  const n = Math.round(drive.seconds * 60);
  for (let k = 1; k <= n; k++) {
    live.send({ cmd: "camera", ...drive.at(k / 60) });
    await sleep(Math.max(0, tDrive + (k * 1000) / 60 - performance.now()));
  }
  await live.quit();
  if (!existsSync(frameLog)) throw new Error(`district ${b.label}: no frame log`);
  type F = { t: number; interval: number; start?: number; gpu: number };
  const frames = readFileSync(frameLog, "utf8").trim().split("\n").map((l) => JSON.parse(l) as F);
  // Frame-log times start at the first frame; the drive started (tDrive - launch - first) ms after it.
  const driving = frames.filter((f) => f.t >= frames[0].t + (tDrive - launch - first));
  const gpu = frameStats(driving.map((f) => f.gpu)), iv = frameStats(driving.map((f) => f.interval));
  const starts = driving.map((f) => f.start).filter((v): v is number => typeof v === "number");
  const st = starts.length ? frameStats(starts) : undefined;
  // CPU per frame where the frame log has it: the frame script in all (`script`), its JavaScript (`js`), skinning and
  // re-transforming moving objects (`skin`), their upload with the dynamic BLAS / TLAS rebuild submitted (`upload`);
  // with THREERT_PROFILE_BLAS=1 also the rebuild's GPU time (`blas`); and the wait for the oldest frame in flight (`wait`:
  // GPU-bound).
  const cpu: Record<string, { p50: number; p95: number }> = {};
  for (const k of ["script", "js", "skin", "upload", "blas", "wait"] as const) {
    const xs = driving.map((f) => (f as unknown as Record<string, number>)[k]).filter((v) => typeof v === "number");
    if (xs.length) { const st = frameStats(xs); cpu[k] = { p50: +st.p50.toFixed(2), p95: +st.p95.toFixed(2) }; }
  }
  // GPU per pass (with `profile`): the mean of the profile lines printed while driving.
  const passes: Record<string, number> = {};
  const all = live.lines.filter((l) => l.startsWith("profile ms/frame:"));
  const prof = all.length > 1 ? all.slice(1) : all; // the first line may still hold the warm-up
  for (const l of prof) for (const m of l.split("|")[1]?.split(",") ?? []) { const [name, v] = m.trim().split(" "); if (name && v) passes[name] = (passes[name] ?? 0) + Number(v) / prof.length; }
  for (const k of Object.keys(passes)) passes[k] = +passes[k].toFixed(2);
  return { first_frame_s: +(first / 1000).toFixed(2), gpu_p50_ms: +gpu.p50.toFixed(2), gpu_p95_ms: +gpu.p95.toFixed(2), interval_p50_ms: +iv.p50.toFixed(2), interval_p95_ms: +iv.p95.toFixed(2), interval_p99_ms: +iv.p99.toFixed(1), ...(st ? { start_p50_ms: +st.p50.toFixed(2), start_p95_ms: +st.p95.toFixed(2) } : {}), over_50ms: driving.filter((f) => f.interval > 50).length, frames: driving.length, cpu, passes };
}

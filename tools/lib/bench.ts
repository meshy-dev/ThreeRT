// Dev-loop measurement rules (docs/ROADMAP.md § 4): measurements sized to ~300 ms, warm-up dropped, hitches counted
// separately (p95 / p99 / spikes), references cached by scene hash + renderer version, nothing measured while the GPU
// is busy with something else, and one summary line per benchmark: "<name>: <value> (×N vs baseline)".
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, statSync, unlinkSync, writeFileSync, writeSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

export const repoRoot = resolve(import.meta.dirname, "../..");
/** The renderer under test; THREERT_BIN measures another build (e.g. the previous commit's) with the same tools. */
export const rendererBin = process.env.THREERT_BIN ?? join(repoRoot, "renderer/target/release/threert-render");
const baselineFile = join(repoRoot, "out/bench/baselines.json");

/**
 * The GPU timing lock: every timed run (perf smoke and suite, the benches, milestone 1, layer B) holds this file
 * (pid, tool, start), so two measurements never overlap: the process scan below cannot see one that starts after a run
 * began. A lock whose process is gone, or one still unreadable 10 s after it was made, is stale and removed. The holder
 * releases it on exit; a run killed outright leaves it to the stale check. THREERT_GPU_LOCK moves it (tests).
 * Tools a holder runs (milestone 1 runs tools/compare/strongest.ts) share its lock: the holder puts its pid in
 * THREERT_GPU_LOCK_OWNER, which children inherit, and a lock held by that pid counts as theirs (they never remove it).
 */
export const GPU_LOCK = process.env.THREERT_GPU_LOCK ?? "/tmp/threert-gpu.lock";

export interface GpuLock {
  pid: number;
  tool: string;
  started: string;
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return (e as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** Who holds the GPU lock (null: nobody; a stale lock is removed). */
export function gpuLockHolder(): GpuLock | null {
  let text: string;
  try {
    text = readFileSync(GPU_LOCK, "utf8");
  } catch {
    return null;
  }
  try {
    const l = JSON.parse(text) as GpuLock;
    if (alive(l.pid)) return l;
  } catch {
    // Being written (create, then write) unless it has been like this for a while.
    try {
      if (Date.now() - statSync(GPU_LOCK).mtimeMs < 10_000) return { pid: -1, tool: "(being written)", started: "" };
    } catch {
      return null;
    }
  }
  try {
    unlinkSync(GPU_LOCK);
  } catch {
    // Someone else removed it first.
  }
  return null;
}

let lockHeld = false;

/** Whether the lock is this run's: held by this process, or by the tool that started it (THREERT_GPU_LOCK_OWNER). */
function ours(l: GpuLock): boolean {
  return l.pid === process.pid || String(l.pid) === process.env.THREERT_GPU_LOCK_OWNER;
}

/** Takes the GPU lock for this process (`tool`: its name in the lock); null when held now, else the other holder. */
export function acquireGpuLock(tool = (process.argv[1] ?? "node").split("/").pop()!): GpuLock | null {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      const fd = openSync(GPU_LOCK, "wx");
      writeSync(fd, JSON.stringify({ pid: process.pid, tool, started: new Date().toISOString() } satisfies GpuLock));
      closeSync(fd);
      if (!lockHeld) process.once("exit", releaseGpuLock);
      lockHeld = true;
      process.env.THREERT_GPU_LOCK_OWNER = String(process.pid);
      return null;
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== "EEXIST") throw e;
      const h = gpuLockHolder();
      if (h && ours(h)) return null;
      if (h) return h;
    }
  }
  return gpuLockHolder() ?? { pid: -1, tool: "(contended)", started: "" };
}

/** Releases the GPU lock if this process holds it. */
export function releaseGpuLock(): void {
  if (!lockHeld) return;
  lockHeld = false;
  try {
    if ((JSON.parse(readFileSync(GPU_LOCK, "utf8")) as GpuLock).pid === process.pid) unlinkSync(GPU_LOCK);
  } catch {
    // Gone already.
  }
}

/** The GPU lock for a run that uses the GPU without timing it (the render tests): waits while another run holds it,
 *  so its renders do not skew that run's timings; exits with code 2 after `maxMs`. */
export async function waitGpuLock(tool?: string, maxMs = 30 * 60_000): Promise<void> {
  const t0 = Date.now();
  let told = false;
  for (;;) {
    const h = acquireGpuLock(tool);
    if (!h) return;
    if (!told) console.error(`GPU lock held by ${h.tool} (pid ${h.pid}) since ${h.started}: waiting for it`);
    told = true;
    if (Date.now() - t0 > maxMs) {
      console.error(`GPU lock still held after ${Math.round(maxMs / 60_000)} min: not run`);
      process.exit(2);
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
}

/** The GPU lock for a run that only needs it (not a quiet GPU): exits with code 2 ("not measured") while another
 *  run holds it. */
export function requireGpuLock(tool?: string): void {
  const h = acquireGpuLock(tool);
  if (!h) return;
  console.error(`GPU lock held by ${h.tool} (pid ${h.pid}) since ${h.started}: not run (another measurement would be skewed)`);
  process.exit(2);
}

/**
 * Processes that would skew GPU measurements: UE editors, playgrounds, Blender, and renderers running a live view or
 * a render farm. Matched on the executable's name (not on command lines, which may only mention one).
 */
export function gpuBusy(): string[] {
  const ps = (field: string) => {
    try {
      return execFileSync("ps", ["-axo", `pid=,${field}=`], { encoding: "utf8", maxBuffer: 1 << 24 }).split("\n");
    } catch {
      return [];
    }
  };
  const args = new Map(ps("args").map((l) => l.trim()).filter(Boolean).map((l) => [l.split(" ")[0], l]));
  const self = String(process.pid);
  const out: string[] = [];
  const lock = gpuLockHolder();
  if (lock && !ours(lock)) out.push(`GPU lock: ${lock.tool} (pid ${lock.pid}) since ${lock.started}`);
  for (const l of ps("comm")) {
    const m = /^\s*(\d+) (.+)$/.exec(l);
    if (!m || m[1] === self) continue;
    const exe = m[2].trim().split("/").pop()!;
    const a = args.get(m[1]) ?? "";
    // Renderer and playground binaries by suffix: bisects run builds from before and after the ThreeRT rename.
    const gpu = /^(UnrealEditor|Blender)$/.test(exe) || /^[a-z0-9]+-playground$/.test(exe) || (/^[a-z0-9]+-render$/.test(exe) && / (live|play|view|farm)( |$)/.test(a));
    if (gpu) out.push(`${m[1]} ${a.replace(/\s+/g, " ").slice(0, 110)}`);
  }
  return out;
}

/** Refuses to measure on a busy GPU (pass `force` to measure anyway; results are then marked); a quiet GPU's run takes
 *  the GPU lock. */
export function requireQuietGpu(force = process.argv.includes("--force")): boolean {
  const busy = gpuBusy();
  if (!busy.length) {
    const h = acquireGpuLock();
    if (!h) return true;
    busy.push(`GPU lock: ${h.tool} (pid ${h.pid}) since ${h.started}`);
  }
  const msg = `GPU busy (${busy.length}): ${busy.slice(0, 3).join(" | ")}`;
  if (!force) throw new Error(`${msg}\n  timings would be wrong; stop those or pass --force`);
  console.error(`warning: ${msg} — measuring anyway (--force)`);
  return false;
}

/** Version of the renderer build (binary size + mtime) and its shaders: part of every cached reference's key. */
export function rendererVersion(bin = rendererBin): string {
  const h = createHash("sha256");
  const st = statSync(bin);
  h.update(`${st.size}|${st.mtimeMs}`);
  return h.digest("hex").slice(0, 12);
}

/** Path of a cached reference for (content, renderer version): `out/cache/refs/<hash>.<ext>`. */
export function cachedRef(content: string | Buffer, ext = "png"): { path: string; hit: boolean } {
  const key = createHash("sha256").update(content).update(rendererVersion()).digest("hex").slice(0, 20);
  const path = join(repoRoot, "out/cache/refs", `${key}.${ext}`);
  mkdirSync(dirname(path), { recursive: true });
  return { path, hit: existsSync(path) };
}

/** Frame-time statistics: median, p95, p99 and spikes (frames over 2x the median). */
export function frameStats(ms: number[]) {
  const s = [...ms].sort((a, b) => a - b);
  const q = (p: number) => s[Math.min(s.length - 1, Math.floor(p * s.length))] ?? NaN;
  const p50 = q(0.5);
  return { n: s.length, p50, p95: q(0.95), p99: q(0.99), max: s.at(-1) ?? NaN, spikes: ms.filter((v) => v > 2 * p50).length };
}

/**
 * Runs `fn` repeatedly for about `budgetMs` after dropping `warmup` runs; returns per-run times (ms). Throughput
 * benchmarks size their work this way instead of a fixed iteration count.
 */
export async function budgeted(fn: () => unknown | Promise<unknown>, budgetMs = 300, warmup = 2): Promise<number[]> {
  for (let i = 0; i < warmup; i++) await fn();
  const times: number[] = [];
  const t0 = performance.now();
  while (performance.now() - t0 < budgetMs || times.length < 3) {
    const t = performance.now();
    await fn();
    times.push(performance.now() - t);
  }
  return times;
}

type Baselines = Record<string, { value: number; at: string }>;

/**
 * One summary line, "<name>: <value><unit> (×N vs baseline)"; ×N > 1 means better. The first measurement (or
 * `--set-baseline`) becomes the baseline (out/bench/baselines.json).
 */
export function summary(name: string, value: number, opts: { unit?: string; lowerIsBetter?: boolean; digits?: number } = {}): string {
  const { unit = "", lowerIsBetter = true, digits = 2 } = opts;
  const all: Baselines = existsSync(baselineFile) ? JSON.parse(readFileSync(baselineFile, "utf8")) : {};
  const base = all[name];
  let rel = "baseline";
  if (base && !process.argv.includes("--set-baseline")) {
    const x = lowerIsBetter ? base.value / value : value / base.value;
    rel = `×${x.toFixed(2)} vs baseline ${base.value.toFixed(digits)}${unit}`;
  } else {
    all[name] = { value, at: new Date().toISOString() };
    mkdirSync(dirname(baselineFile), { recursive: true });
    writeFileSync(baselineFile, JSON.stringify(all, null, 1));
  }
  const line = `${name}: ${value.toFixed(digits)}${unit} (${rel})`;
  console.log(line);
  return line;
}

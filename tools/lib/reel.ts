// A clip of a game or scene drawn by the live renderer in lockstep: shared by the landing page's reel
// (tools/site-reel.ts) and the scene check (tools/reel-check.ts).
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { BISTRO } from "../../scenes/bistro.ts";
import { frameStats, rendererBin, repoRoot } from "./bench.ts";
import { gltfCameraPath } from "./gltf-camera.ts";
import { startLive, type Live } from "./live.ts";

export type Pose = { pos: number[]; target: number[]; tan?: number };
export type Cmd = Record<string, unknown>;
export interface Clip {
  what: string;
  src: string;
  room?: string;
  seconds?: number;
  /** Seconds of the scene's clock before the recording starts (the first frames' history; default 4). */
  settle?: number;
  /** Render scale (default 0.75). A recording cannot use dynamic resolution: its pauses read as headroom. */
  scale?: number;
  bounces?: number;
  /** Sent once, before settling. */
  before?: Cmd[];
  /** Sent at these times (s) into the recording. */
  at?: [number, Cmd][];
  /** The camera at time t (s), when the clip moves it. */
  camera?: (t: number) => Pose;
}
export const ev = (type: string, payload: unknown = {}): Cmd => ({ cmd: "event", type, payload });

let bistroPath: ReturnType<typeof gltfCameraPath> | undefined;
/** Bistro's own camera path at t (s). */
export const bistroCamera = (t: number) => (bistroPath ??= gltfCameraPath(join(repoRoot, BISTRO), "Camera")).at(t);

export interface Recording {
  frames: number;
  load_s: number;
  gpu_ms: { p50: number; p95: number };
  over_budget: number;
  /** The renderer's lines that read as trouble (fail, error, panic, missing). */
  warnings: string[];
}

/**
 * Records `frames` frames of a clip at `fps` after its settling frames. `file(k)` names the PNG of frame k, or null to
 * draw it without keeping it. Every frame is drawn, with its history carried from the frame before, as on screen.
 * `timeoutMs` bounds each wait on a frame, `loadMs` the wait for the first (default 300 s); a timeout throws and leaves
 * the process alone.
 */
export async function recordClip(c: Clip, bundle: string, o: { width: number; height: number; fps: number; frames: number; scale: number; room?: string; frameLog: string; file: (k: number) => string | null; timeoutMs?: number; loadMs?: number; label?: string; bin?: string }): Promise<Recording> {
  const timeout = o.timeoutMs ?? 120_000;
  const live: Live = startLive({ bin: o.bin ?? rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: o.label ?? "reel" }, bundle, { scale: String(o.scale), frameLog: o.frameLog, width: o.width, height: o.height, bounces: c.bounces, room: o.room ?? c.room, firstFrameMs: o.loadMs });
  // Lockstep from the first frame on: the scene's clock is the frame count, so a clip starts at the same moment of its
  // scene every time.
  live.send({ cmd: "lockstep", dt: 1 / o.fps });
  let ok = false;
  try {
    await live.firstFrame;
    for (const cmd of c.before ?? []) live.send(cmd);
    if (c.camera) live.send({ cmd: "camera", ...c.camera(0) });
    // The settling frames are drawn like the rest and not kept.
    live.send({ cmd: "frames", n: Math.round((c.settle ?? 4) * o.fps), sync: "settled" });
    await live.wait((l) => l.endsWith("sync settled"), Math.max(timeout, 300_000));
    const pending = [...(c.at ?? [])].sort((a, b) => a[0] - b[0]);
    for (let k = 0; k < o.frames; k++) {
      const t = k / o.fps;
      while (pending.length && pending[0][0] <= t) live.send(pending.shift()![1]);
      const f = o.file(k);
      const step: Cmd = { ...(c.camera ? { cmd: "camera", ...c.camera(t), frames: 1 } : { cmd: "frames", n: 1 }), ...(f ? { snapshot: f } : { sync: `k${k}` }) };
      live.send(step);
      await live.wait((l) => (f ? l.includes(f) : l.endsWith(`sync k${k}`)), timeout);
    }
    ok = true;
  } finally {
    // A renderer that stopped answering is left alone: killing a hung GPU dispatch can take the machine with it.
    if (ok) await live.quit().catch(() => {});
  }
  // (The log is complete once the renderer has quit; the recording is its last frames.)
  const first = await live.firstFrame;
  const logged = readFileSync(o.frameLog, "utf8").trim().split("\n").filter(Boolean).flatMap((l) => {
    try {
      return [JSON.parse(l) as { gpu: number }];
    } catch {
      return [];
    }
  }).slice(-o.frames);
  const g = frameStats(logged.map((x) => x.gpu));
  return {
    frames: o.frames,
    load_s: +(first / 1000).toFixed(1),
    gpu_ms: { p50: +g.p50.toFixed(2), p95: +g.p95.toFixed(2) },
    over_budget: logged.filter((x) => x.gpu > 1000 / o.fps).length,
    warnings: [...new Set(live.lines.filter((x) => /fail|error|panic|not found|missing|device lost/i.test(x)))].slice(0, 6).map((l) => l.slice(0, 240)),
  };
}

/** The machine, for a recording's record: CPU name (macOS), or the OS. */
export function machineName(): string {
  try {
    return execFileSync("sysctl", ["-n", "machdep.cpu.brand_string"], { encoding: "utf8" }).trim();
  } catch {
    return `${process.platform} ${process.arch}`;
  }
}

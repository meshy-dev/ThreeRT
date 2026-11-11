// The page of a ThreeRT game in a browser (docs/RUNTIME.md "The engine in a browser"). It does what the native process
// does around the script (renderer/src/script.rs `GameSession`, renderer/src/view.rs):
//   1. loads the engine's wasm module and makes `globalThis.__engine` of its exports (engine.ts);
//   2. runs the game bundle, the same file the native runtime loads in QuickJS (tools/bundle-game.ts: game code, the
//      scene binding, the node host), which installs `globalThis.__threert`;
//   3. `setup` builds the scene in the engine's world; the module opens a live view of it on the canvas (WebGPU);
//   4. every frame: the node host's `frame` (the frame script, world handlers' clock, the physics' events), then the
//      module's (physics ticks, commit, render, present);
//   5. a click on the canvas: the module picks what is under it and gives the event's payload (with the view ray),
//      the node host runs the `click` handlers, the module commits what they changed.
//
// URL parameters: `game` (the bundle, default game.js), `width` / `height` (the view, default 1280 x 720), `scale`
// (the traced resolution's share, default 0: dynamic, toward `fps`), `fps` (default 60), `lockstep=1` (no frame runs
// by itself: `threert.run(n, dt)` runs them, for scripted tests), `guard` (the loop guard's limit, iterations an
// invocation: default 65536; a new kernel's first runs take less), `bench=1` (no game: `threert.bench()`).
import { webEngine, type EngineExports } from "./engine.ts";
import { bench } from "./bench.ts";
import type { FrameRow } from "./frame.ts";

/** The session's exports (renderer/web/src/lib.rs) and wasm-bindgen's initialiser. */
interface Module extends EngineExports {
  default(): Promise<unknown>;
  provide(path: string, bytes: Uint8Array): void;
  start(canvas: HTMLCanvasElement, width: number, height: number, info: string, scale: number, fps: number, guard: number): Promise<string>;
  setLockstep(on: boolean): void;
  tick(dt: number): string;
  firstFrame(): number;
  frameBegin(dt: number): boolean;
  frameEnd(dt: number): string;
  click(u: number, v: number): string | undefined;
  commit(seq: number): boolean;
  physicsHash(): string;
}

/** The node host the bundle installs (packages/gameplay/src/node-host.ts). */
interface NodeHost {
  describe(): string;
  setup(roomId: string, seed?: number): string;
  event(type: string, payload: string, ts: number): string;
  frame(dt: number, now: number): void;
}

const params = new URLSearchParams(location.search);
const num = (k: string, d: number) => (params.has(k) ? Number(params.get(k)) : d);
const status = (text: string) => {
  const el = document.getElementById("status");
  if (el) el.textContent = text;
};

/** How far the page has got, for a test that waits on it: `module`, `game`, `setup`, `start` (inside the module's
 *  `start`: the GPU), `running`. */
const stage = (s: string) => Object.assign(window, { threertStage: s });

async function main() {
  const t0 = performance.now();
  stage("module");
  // (A URL the bundler does not follow: wasm-bindgen's module is served beside this one.)
  const wasm = (await import(new URL("./pkg/threert_web.js", import.meta.url).href)) as Module;
  await wasm.default();
  const t_module = performance.now() - t0;

  // The engine API, with the calls counted (and timed with `profile=1`: a clock read on either side of each).
  const counters = { calls: 0, ms: 0 };
  const profile = params.get("profile") === "1";
  const raw = webEngine(wasm) as unknown as Record<string, (...a: unknown[]) => unknown>;
  const engine: Record<string, (...a: unknown[]) => unknown> = {};
  for (const [name, f] of Object.entries(raw)) {
    engine[name] = profile
      ? (...a) => {
          const t = performance.now();
          try {
            return f(...a);
          } finally {
            counters.calls++;
            counters.ms += performance.now() - t;
          }
        }
      : (...a) => (counters.calls++, f(...a));
  }
  (globalThis as { __engine?: unknown }).__engine = engine;

  if (params.get("bench") === "1") {
    Object.assign(window, { threert: { bench: (n: number) => bench(webEngine(wasm), n, () => wasm.tick(1 / 60)) } });
    status("bench");
    return;
  }

  stage("game");
  // The game: one script, as the native runtime evaluates it.
  await new Promise<void>((ok, fail) => {
    const s = document.createElement("script");
    s.src = params.get("game") ?? "game.js";
    s.onload = () => ok();
    s.onerror = () => fail(new Error(`cannot load ${s.src}`));
    document.head.append(s);
  });
  const host = (globalThis as { __threert?: NodeHost }).__threert;
  if (!host) throw new Error("the bundle installed no node host");
  const describe = JSON.parse(host.describe()) as { handlers?: string[]; on?: string[]; frame?: boolean; keys?: Record<string, string> };
  const t1 = performance.now();
  const setup = JSON.parse(host.setup(params.get("room") ?? "local", params.has("seed") ? Number(params.get("seed")) : undefined)) as object;
  const t_setup = performance.now() - t1;
  const setup_calls = { ...counters };

  const canvas = document.getElementById("view") as HTMLCanvasElement;
  const [width, height] = [num("width", 1280), num("height", 720)];
  // The canvas' own pixels: the view's, times the display's pixel ratio where the page is shown smaller than them.
  canvas.width = width;
  canvas.height = height;
  const fps = num("fps", 60);
  stage("start");
  const t2 = performance.now();
  const info = JSON.parse(await wasm.start(canvas, width, height, JSON.stringify(describe), num("scale", 0), fps, num("guard", 0))) as Record<string, unknown>;
  const t_start = performance.now() - t2;

  const handles = (type: string) => [describe.handlers, describe.on].some((h) => h?.includes(type));
  const frames: FrameRow[] = [];
  let last = performance.now();
  let errors = 0;

  /** One frame, `dt` seconds after the one before. */
  const frame = (dt: number, interval: number): FrameRow => {
    const a = performance.now();
    const scripted = wasm.frameBegin(dt);
    const b = performance.now();
    const before = { ...counters };
    if (scripted) {
      // A frame script that throws does not stop the world (as natively): the frame's ticks are made all the same.
      try {
        host.frame(dt, Date.now());
      } catch (e) {
        if (errors++ < 5) console.error(`game: frame script: ${e instanceof Error ? e.message : e}`);
      }
    }
    const c = performance.now();
    const end = JSON.parse(wasm.frameEnd(dt)) as Omit<FrameRow, "interval" | "begin" | "script" | "calls" | "calls_ms" | "end">;
    const d = performance.now();
    const row: FrameRow = { interval, begin: b - a, script: c - b, calls: counters.calls - before.calls, calls_ms: counters.ms - before.ms, end: d - c, ...end };
    frames.push(row);
    if (frames.length > 20000) frames.splice(0, 10000);
    return row;
  };

  /** An event on this page's own node host (local play: the page is the authority), then what it changed committed. */
  const event = (type: string, payload: string) => {
    const t = performance.now();
    const out = JSON.parse(host.event(type, payload, Date.now())) as { log: string[]; seq: number };
    const script = performance.now() - t;
    const rebuilt = wasm.commit(out.seq);
    for (const l of out.log) console.log(`game: ${l}`);
    return { log: out.log, seq: out.seq, script_ms: script, commit_ms: performance.now() - t - script, rebuilt };
  };

  /** A click at (u, v) of the view: the pick, the `click` event with what is under it and the view ray. */
  const click = (u: number, v: number) => {
    if (!handles("click")) return null;
    const t = performance.now();
    const payload = wasm.click(u, v);
    if (payload === undefined) return null;
    const pick_ms = performance.now() - t;
    return { payload: JSON.parse(payload) as object, pick_ms, ...event("click", payload) };
  };
  canvas.addEventListener("click", (e) => {
    const r = canvas.getBoundingClientRect();
    const c = click((e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height);
    session.clicks.push(c);
  });
  addEventListener("keydown", (e) => {
    const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
    const type = describe.keys?.[key];
    if (type && !e.repeat) event(type, "{}");
  });

  const session = {
    info: { ...info, describe, setup, ms: { module: t_module, setup: t_setup, start: t_start, ...(info.ms as object) }, setup_calls },
    frames,
    clicks: [] as (ReturnType<typeof click> | null)[],
    lockstep: params.get("lockstep") === "1",
    /** `n` frames now, each `dt` seconds after the one before (lockstep: a scripted test's clock). */
    step: (n: number, dt = 1 / 60) => {
      for (let k = 0; k < n; k++) frame(dt, 0);
      return frames.length;
    },
    /**
     * `n` frames in lockstep, a few at a time with the browser's own frame between them (the canvas is shown, the
     * loop guard's count comes back): it stops early if the guard has stopped anything, and resolves to the frames
     * it ran.
     */
    run: async (n: number, dt = 1 / 60) => {
      let done = 0;
      while (done < n && !frames.at(-1)?.guard_trips) {
        for (let k = 0; k < 2 && done < n; k++, done++) frame(dt, 0);
        await new Promise(requestAnimationFrame);
      }
      return done;
    },
    click,
    event: (type: string, payload: unknown = {}) => event(type, JSON.stringify(payload)),
    physicsHash: () => wasm.physicsHash(),
    /** The GPU's time over the first frame, ms (0 until it is done): the pipelines' compile. */
    firstFrame: () => wasm.firstFrame(),
    /** The loop guard's count as last read back (null: the kernel has no guard). */
    guardTrips: () => frames.at(-1)?.guard_trips ?? null,
  };
  wasm.setLockstep(session.lockstep);
  Object.assign(window, { threert: session });

  // Frames at `fps` whatever the display's rate (a 120 Hz display shows each twice), as the native window.
  const tick = (now: number) => {
    requestAnimationFrame(tick);
    if (session.lockstep) return;
    const since = now - last;
    if (since < 1000 / fps - 2) return;
    last = now;
    const row = frame(Math.min(since / 1000, 0.1), since);
    if (row.frame % 30 === 0) status(`${(info.adapter as { name: string }).name} · ${(1000 / since).toFixed(0)} fps · GPU ${row.gpu_ms.toFixed(1)} ms · scale ${row.scale.toFixed(2)}${row.guard_trips ? ` · LOOP GUARD: ${row.guard_trips} invocations stopped, the image is wrong` : ""}`);
  };
  // (The first callback's own time may be from before `start` held the thread: the clock's now instead.)
  requestAnimationFrame(() => {
    last = performance.now();
    requestAnimationFrame(tick);
  });
  status("running");
  stage("running");
}

main().catch((e) => {
  status(`failed: ${e instanceof Error ? e.message : e}`);
  console.error(e);
  Object.assign(window, { threertError: String(e instanceof Error ? (e.stack ?? e.message) : e) });
});

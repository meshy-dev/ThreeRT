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
// by itself: `threert.step(n, dt)` runs them, for scripted tests), `bench=1` (no game: `threert.bench()`).
import { webEngine, type EngineExports, type WebEngine } from "./engine.ts";

/** The session's exports (renderer/web/src/lib.rs) and wasm-bindgen's initialiser. */
interface Module extends EngineExports {
  default(): Promise<unknown>;
  provide(path: string, bytes: Uint8Array): void;
  start(canvas: HTMLCanvasElement, width: number, height: number, info: string, scale: number, fps: number): Promise<string>;
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

/** A frame's numbers (ms): the page's own times around the module's (`frameEnd`'s JSON). */
export interface FrameRow {
  /** Since the frame before (the page's frame callback). */
  interval: number;
  /** `frameBegin`, the node host's `frame` (of which the engine's calls: `calls` of them, `calls_ms`), `frameEnd`. */
  begin: number;
  script: number;
  calls: number;
  calls_ms: number;
  end: number;
  physics: { ticks: number; step: number; sync: number; bodies: number; moved: number } | null;
  physics_ms: number;
  commit_ms: number;
  rebuilt: boolean;
  rebuild_ms: number;
  render_ms: number;
  gpu_ms: number;
  trace_ms: number;
  scale: number;
  frame: number;
  /** The kernel's invocations the loop guard has stopped since the view opened (0: every frame is sound). */
  guard_trips: number | null;
}

const params = new URLSearchParams(location.search);
const num = (k: string, d: number) => (params.has(k) ? Number(params.get(k)) : d);
const status = (text: string) => {
  const el = document.getElementById("status");
  if (el) el.textContent = text;
};

async function main() {
  const t0 = performance.now();
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
    Object.assign(window, { threert: { bench: (n: number) => bench(webEngine(wasm), n) } });
    status("bench");
    return;
  }

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
  const t2 = performance.now();
  const info = JSON.parse(await wasm.start(canvas, width, height, JSON.stringify(describe), num("scale", 0), fps)) as Record<string, unknown>;
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
    click,
    event: (type: string, payload: unknown = {}) => event(type, JSON.stringify(payload)),
    physicsHash: () => wasm.physicsHash(),
    /** The loop guard's count as last read back (null: the kernel has no guard). */
    guardTrips: () => frames.at(-1)?.guard_trips ?? null,
  };
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
  requestAnimationFrame((now) => {
    last = now;
    requestAnimationFrame(tick);
  });
  status("running");
}

/** The boundary's cost in this browser: each kind of engine call timed over many (ns a call), and the pose table of
 *  `n` bodies read (ms). The engine's world gets `n` objects with a body each. */
function bench(e: WebEngine, n: number) {
  const tri = [new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]), new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]), new Float32Array([0, 0, 1, 0, 0, 1]), new Uint32Array([0, 1, 2])] as const;
  e.geometry(1, ...tri);
  const mat = { name: "m", color: [1, 1, 1], opacity: 1, metalness: 0, roughness: 0.5, emissive: [0, 0, 0], emissiveIntensity: 0, doubleSided: false };
  e.material(2, mat as never);
  const desc = (k: number) => ({ name: `o${k}`, parent: null, castShadow: true, kind: "mesh", geometry: 1, material: 2 });
  const trs = new Float32Array([0, 0, 0, 0, 0, 0, 1, 1, 1, 1]);
  const t_build = performance.now();
  for (let k = 0; k < n; k++) {
    e.object(10 + k, desc(k) as never);
    trs[0] = (k % 100) * 2;
    trs[1] = 1;
    trs[2] = Math.floor(k / 100) * 2;
    e.transform(10 + k, trs);
    e.physics("body", { id: k + 1, object: 10 + k, slot: k, gravityScale: 0 });
    e.physics("collider", { id: k + 1, body: k + 1, shape: { type: "sphere", radius: 0.5 } });
  }
  const build_ms = performance.now() - t_build;
  /** ns a call: the best of five runs of `count` calls. */
  const time = (count: number, f: (k: number) => void) => {
    let best = Infinity;
    for (let r = 0; r < 5; r++) {
      const t = performance.now();
      for (let k = 0; k < count; k++) f(k);
      best = Math.min(best, performance.now() - t);
    }
    return Math.round((best / count) * 1e6 * 10) / 10;
  };
  let sink = 0;
  const calls = {
    "visible(id, bool)": time(1e6, (k) => e.visible(10 + (k % n), true)),
    "power(id, number)": time(1e6, (k) => e.power(10 + (k % n), 1)),
    "transform(id, Float32Array(10))": time(2e5, (k) => e.transform(10 + (k % n), trs)),
    "customData(id, [4])": time(2e5, (k) => e.customData(10 + (k % n), [1, 2, 3, 4])),
    "material(id, desc)": time(5e4, () => e.material(2, mat as never)),
    "object(id, desc)": time(5e4, (k) => e.object(10 + (k % n), desc(k % n) as never)),
    'physics("state", { body })': time(5e4, (k) => (sink += e.physics("state", { body: 1 + (k % n) }).length)),
    'physics("impulse", { body, impulse })': time(5e4, (k) => (sink += e.physics("impulse", { body: 1 + (k % n), impulse: [0, 0, 0] }).length)),
    'physics("ray", { origin, direction })': time(2e4, () => (sink += e.physics("ray", { origin: [0.2, 9, 0.1], direction: [0, -1, 0] }).length)),
    "physicsPoses(), not read": time(2e3, () => (sink += e.physicsPoses().length)),
  };
  /** ms to fetch the table and read every body's seven numbers: the best of 20. */
  const read = (m: number) => {
    let best = Infinity;
    for (let r = 0; r < 20; r++) {
      const t = performance.now();
      const p = e.physicsPoses();
      for (let k = 0; k < m * 7; k++) sink += p[k];
      best = Math.min(best, performance.now() - t);
    }
    return Math.round(best * 1e4) / 1e4;
  };
  const poses: Record<string, number> = {};
  for (const m of [1000, 10000]) if (m <= n) poses[`${m} bodies' poses read from the table, ms`] = read(m);
  return { bodies: n, build_ms, calls_ns: calls, poses, isolated: crossOriginIsolated, sink };
}

main().catch((e) => {
  status(`failed: ${e instanceof Error ? e.message : e}`);
  console.error(e);
  Object.assign(window, { threertError: String(e instanceof Error ? (e.stack ?? e.message) : e) });
});

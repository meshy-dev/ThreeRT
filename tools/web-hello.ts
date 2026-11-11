// The physics hello world in a browser (docs/RUNTIME.md "The engine in a browser"): games/physics/hello.game.ts, the
// file the native test runs, as a page (tools/build-web.ts) in this machine's Chrome, driven by Playwright. The same
// script as tools/physics-hello.ts and the same checks on the same numbers: frames in lockstep (1 / 60 s each: one
// tick), five clicks made with the mouse on the canvas (the page's own click path: pick, view ray, the node host's
// `click` handler), four boxes added, one kicked, where they rest and how far the kicked one slid.
//
//   node tools/web-hello.ts                 the scripted test (exit 1 on a failed check)
//   node tools/web-hello.ts --small         only the first step: a 160 x 90 view, fourteen frames (a new kernel's first run)
//   node tools/web-hello.ts --frames        then the frame times: the same clicks in real time, still and moving frames
//   node tools/web-hello.ts --bench         then the boundary's cost: engine calls and the pose table, in this Chrome
//   --native out.json   compare with the native test's numbers (node tools/physics-hello.ts --json out.json)
//   --json out.json     everything measured
//   --headless          no window (Chrome's new headless; WebGPU there is not checked on every version)
//   --view WxH --scale S  the scripted test's view and the traced share of it (default 1280x720, 0.5)
//   --guard N           the loop guard's limit for every page (default: 8192 for the small step, the page's 65536 after)
//   --cold              a fresh browser profile: the shaders compiled again (the first visit's times)
//
// A GPU job that does not end takes this machine's display with it (2026-11-11), and a page's shader runs on the same
// GPU as everything else. So: the kernel in the page has the loop guard (renderer/src/merged.rs `guard`), and its
// count is a check here (0, or the frames are wrong); the first step is small; every wait on the page has a timeout;
// and a timeout stops the run where it is: nothing is retried, no second page is opened (exit 3; see the message).
// The GPU lock is held.
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { chromium, type Page } from "playwright-core";
import { BOX, CAMERA, KICK, OFF, WALL_Z } from "../games/physics/hello.game.ts";
import { acquireGpuLock, repoRoot } from "./lib/bench.ts";
import { buildWeb, describeBuild, serveWeb } from "./lib/web.ts";
import type { FrameRow } from "../renderer/web/www/frame.ts";

const args = process.argv.slice(2);
const has = (f: string) => args.includes(f);
const opt = (f: string) => (has(f) ? args[args.indexOf(f) + 1] : undefined);

type V3 = [number, number, number];
const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const unit = (a: V3): V3 => { const l = Math.hypot(...a); return [a[0] / l, a[1] / l, a[2] / l]; };
/** Where a point of the world is in the image (u, v in 0..1), through the game's camera (as tools/physics-hello.ts). */
function project(p: V3): { u: number; v: number } {
  const c = [...CAMERA.position] as V3, fwd = unit(sub([...CAMERA.target] as V3, c));
  const right = unit(cross(fwd, [0, 1, 0])), up = cross(right, fwd);
  const d = sub(p, c), z = dot(d, fwd), t = Math.tan((CAMERA.fov * Math.PI) / 360);
  return { u: 0.5 + dot(d, right) / z / (2 * t * (CAMERA.width / CAMERA.height)), v: 0.5 - dot(d, up) / z / (2 * t) };
}

/** A wait on the page that did not come back: the run stops here (see the head of this file). */
class Stuck extends Error {}
const TIMEOUT = 30_000;
/** `f` in the page, or `Stuck` after TIMEOUT. */
async function inPage<T, A>(page: Page, what: string, f: (a: A) => T | Promise<T>, arg?: A): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  const late = new Promise<never>((_, fail) => (timer = setTimeout(() => fail(new Stuck(what)), TIMEOUT)));
  try {
    return await Promise.race([page.evaluate(f as (a: unknown) => T, arg), late]);
  } finally {
    clearTimeout(timer);
  }
}

const holder = acquireGpuLock("web-hello");
if (holder) { console.log(`not run: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }

const build = await buildWeb(join(repoRoot, "games/physics/hello.game.ts"));
console.log(describeBuild(build));
const { server, url } = await serveWeb(build.dir);
// A profile of its own, kept between runs: the second visit finds the shaders the first compiled (`--cold` drops it).
const profile = join(repoRoot, "out/web/chrome-profile");
if (has("--cold")) rmSync(profile, { recursive: true, force: true });
const context = await chromium.launchPersistentContext(profile, {
  channel: "chrome",
  headless: has("--headless"),
  viewport: { width: 1300, height: 780 },
  // Timestamp queries at full resolution (the frame's GPU time); nothing else is switched.
  args: ["--enable-webgpu-developer-features"],
});
// The loop guard's limit for the small first step (hello world's tree has 77 nodes: a sample is a few hundred
// iterations); the page's default, 65,536, after it.
const GUARD_FIRST = Number(opt("--guard") ?? 8192);
// The view of the scripted test and the frame times: `--view 640x360` (default the game's 1280 x 720), `--scale`
// (the traced share of it, default 0.5).
const [VW, VH] = (opt("--view") ?? `${CAMERA.width}x${CAMERA.height}`).split("x").map(Number);
const VIEW = `width=${VW}&height=${VH}&scale=${opt("--scale") ?? "0.5"}`;
// A tab of the context's own stays open throughout: closing a window's last tab and opening the next page races with
// Chrome's new-tab page (the test's page then never loads).
const keep = context.pages()[0] ?? (await context.newPage());
await keep.goto("about:blank");
const failures: string[] = [];
const check = (ok: boolean, what: string) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const out: Record<string, unknown> = { build: { wasm: build.wasm, js: build.js, game: build.game } };
let code = 0;

type Session = {
  info: Record<string, unknown>;
  frames: FrameRow[];
  clicks: unknown[];
  step(n: number, dt?: number): number;
  run(n: number, dt?: number): Promise<number>;
  click(u: number, v: number): unknown;
  event(type: string, payload?: unknown): unknown;
  guardTrips(): number | null;
  firstFrame(): number;
  bench(n: number): unknown;
};
/** What the callbacks below use of the page (they run there; this file is checked without the DOM's types). */
declare const document: { getElementById(id: string): { getBoundingClientRect(): { left: number; top: number; width: number; height: number } } };
declare const navigator: { gpu: { requestAdapter(): Promise<{ info: { vendor: string; architecture: string; device: string; description: string; isFallbackAdapter?: boolean }; isFallbackAdapter?: boolean; limits: { maxStorageBuffersPerShaderStage: number } }> } };

/** Opens the page with `query` and waits for the view (or for the page's own failure). */
async function open(query: string): Promise<{ page: Page; lines: string[]; info: Record<string, unknown> }> {
  const page = await context.newPage();
  const lines: string[] = [];
  page.on("console", (m) => lines.push(m.text()));
  page.on("pageerror", (e) => lines.push(`pageerror: ${e.message}`));
  // (`--guard N`: the loop guard's limit for every page of this run, not only the small first step's.)
  const guard = has("--guard") && !query.includes("guard=") ? `&guard=${GUARD_FIRST}` : "";
  await page.goto(`${url}?${query}${guard}`, { timeout: TIMEOUT });
  const ready = await Promise.race([
    page.waitForFunction(() => "threert" in globalThis || "threertError" in globalThis, undefined, { timeout: TIMEOUT }).then(() => true),
    new Promise<boolean>((ok) => setTimeout(() => ok(false), TIMEOUT)),
  ]).catch(() => false);
  if (!ready) {
    // How far the page got (main.ts `threertStage`): nothing means its script never ran (no GPU work on it); "start"
    // means it is inside the module's `start`, on the GPU.
    const stage = await Promise.race([page.evaluate(() => (globalThis as { threertStage?: string }).threertStage ?? null), new Promise<string>((ok) => setTimeout(() => ok("(no answer)"), 5000))]).catch(() => "(no answer)");
    throw new Stuck(`the page did not open its view (${query}); it is at ${page.url()}, stage ${stage}${stage === null ? " (its script never ran: not a GPU job)" : ""}; its console: ${lines.slice(-5).join(" | ")}`);
  }
  const error = await inPage(page, "reading the page's error", () => (globalThis as { threertError?: string }).threertError);
  if (error) throw new Error(`the page failed: ${error}`);
  const info = query.includes("bench=1") ? {} : await inPage(page, "reading the view's info", () => (globalThis as unknown as { threert: Session }).threert.info);
  return { page, lines, info };
}

/** A click with the mouse on the canvas where (u, v) of the view is shown. */
async function clickAt(page: Page, at: { u: number; v: number }) {
  const r = await inPage(page, "reading the canvas' place", () => { const b = document.getElementById("view")!.getBoundingClientRect(); return { x: b.left, y: b.top, w: b.width, h: b.height }; });
  await page.mouse.click(r.x + at.u * r.w, r.y + at.v * r.h);
}

const q = (xs: number[], p: number) => { const s = [...xs].sort((a, b) => a - b); return s.length ? s[Math.min(s.length - 1, Math.floor(p * s.length))] : NaN; };
const r3 = (v: number) => Math.round(v * 1000) / 1000;
const pair = (xs: number[]) => [r3(q(xs, 0.5)), r3(q(xs, 0.95))];

try {
  // 1. Small: the kernel's first frames in this browser, 160 x 90, in lockstep.
  {
    const { page, info } = await open(`width=160&height=90&scale=1&lockstep=1&guard=${GUARD_FIRST}`);
    const adapter = info.adapter as { name: string; type: string; backend: string };
    // What the browser itself says of the adapter (a fallback adapter is a software one: not what this measures).
    const gpu = await inPage(page, "asking the browser for its adapter", async () => {
      const a = await navigator.gpu.requestAdapter();
      return { vendor: a.info.vendor, architecture: a.info.architecture, device: a.info.device, description: a.info.description, fallback: a.isFallbackAdapter ?? a.info.isFallbackAdapter ?? null, maxStorageBuffers: a.limits.maxStorageBuffersPerShaderStage };
    });
    console.log(`web hello: ${JSON.stringify(gpu)}; the view: ${JSON.stringify(info.adapter)}, ${JSON.stringify(info.limits)}, loop guard ${info.loop_guard}, ${JSON.stringify(info.ms)}`);
    check(gpu.fallback !== true, `the adapter is the machine's GPU, not a fallback (${gpu.vendor} ${gpu.architecture})`);
    check(info.merged === true && (info.limits as { maxStorageBuffersPerShaderStage: number }).maxStorageBuffersPerShaderStage === 8, "the device was asked for eight storage buffers a stage, the merged layout fits them");
    check(typeof info.loop_guard === "number", `the kernel has the loop guard (${info.loop_guard} iterations an invocation)`);
    // One frame first, alone; then nine, then a few more for the count (it is read back a frame or two late).
    const t_first = performance.now();
    const first = await inPage(page, "the first frame at 160 x 90", () => (globalThis as unknown as { threert: Session }).threert.run(1));
    // (Until the browser's own next frame after it: with a kernel it has not compiled before, this is where it waits.)
    const first_wall = performance.now() - t_first;
    const more = await inPage(page, "thirteen more frames", () => (globalThis as unknown as { threert: Session }).threert.run(13));
    const trips = await inPage(page, "reading the loop guard's count", () => (globalThis as unknown as { threert: Session }).threert.guardTrips());
    check(first + more === 14 && trips === 0, `fourteen frames drawn at 160 x 90, the loop guard stopped nothing (${trips})`);
    // This page is the run's first: its first frame is every pipeline's first use in this browser session (`--cold`:
    // in this profile, so the shaders' compile; else what the browser kept of it).
    const compile = await inPage(page, "reading the first frame's time", () => (globalThis as unknown as { threert: Session }).threert.firstFrame());
    const row0 = await inPage(page, "reading the first frame's row", () => (globalThis as unknown as { threert: Session }).threert.frames[0]);
    console.log(`web hello: the first frame: ${r3(first_wall)} ms until the browser's next frame after it (the page's own time in it ${r3(row0.begin + row0.script + row0.end)} ms, recording ${r3(row0.render_ms)}); its work done on the GPU ${r3(compile)} ms after it was recorded`);
    out.small = { gpu, adapter, info, first_frame_gpu_ms: compile, first_frame_wall_ms: first_wall, first_frame: row0, cold: has("--cold") };
    await page.screenshot({ path: join(repoRoot, "out/web/hello-small.png") });
    await page.close();
    if (failures.length || has("--small")) throw new Error(failures.length ? "the small run failed: nothing larger is run" : "small");
  }

  // 2. The scripted test in lockstep (1280 x 720 traced at half unless told otherwise): tools/physics-hello.ts's steps.
  // (`--skip-scripted`: not this time; the frame-time page is then the first with its kernel.)
  if (!has("--skip-scripted")) {
    const { page, lines, info } = await open(`${VIEW}&lockstep=1`);
    // (`run` stops early if the loop guard stops anything: then the frames are short of what was asked, and it shows below.)
    const step = (n: number) => inPage(page, `${n} frames`, (k: number) => (globalThis as unknown as { threert: Session }).threert.run(k), n);
    // The clicks at the exact (u, v) the native test sends (`threert.click`: what the canvas' own click listener
    // calls with the mouse's place; a mouse lands on whole pixels, 2 cm apart on the ground at this distance, so its
    // boxes would not be the native run's to the millimetre). The mouse itself is checked after the report.
    const click = async (at: V3, settle: number) => {
      await inPage(page, "a click", (uv: { u: number; v: number }) => { const s = (globalThis as unknown as { threert: Session }).threert; s.clicks.push(s.click(uv.u, uv.v)); }, project(at));
      await step(settle);
    };
    await step(10);
    const column: V3 = [-1, 0, WALL_Z + OFF], lone: V3 = [1.5, 0, 2.5];
    await click(column, 120);
    await click([column[0], 1.2, WALL_Z], 150);
    await click([column[0], 1.7, WALL_Z], 180);
    await click(lone, 120);
    await click([lone[0], 0.3, lone[2]], 240);
    await inPage(page, "the report event", () => (globalThis as unknown as { threert: Session }).threert.event("report"));
    await step(2);
    // The mouse: a real click on the canvas, on the ground to the left: a fifth box appears over where it landed.
    const spot: V3 = [-3, 0, 3.5];
    await clickAt(page, project(spot));
    await step(4);
    const state = await inPage(page, "reading the run", () => { const s = (globalThis as unknown as { threert: Session }).threert; return { clicks: s.clicks, trips: s.guardTrips(), frames: s.frames.length, rebuilt: s.frames.filter((f) => f.rebuilt).length, moved: s.frames.filter((f) => (f.physics?.moved ?? 0) > 0).length }; });
    await page.screenshot({ path: join(repoRoot, "out/web/hello.png") });
    await page.close();

    const hello = lines.map((l) => /hello (\{.*\})/.exec(l)?.[1]).filter((x): x is string => !!x).map((x) => JSON.parse(x) as { click: number; add?: string; kick?: string; on?: string; at?: V3 });
    const report = lines.map((l) => /report (\{.*\})/.exec(l)?.[1]).filter((x): x is string => !!x).map((x) => JSON.parse(x) as { boxes: { name: string; at: V3; speed: number; asleep: boolean }[]; kicks: number }).pop();
    const near = (a: V3, b: V3, tol: number) => Math.hypot(...sub(a, b)) <= tol;
    const fmt = (a: V3) => `(${a.map((v) => v.toFixed(3)).join(", ")})`;
    console.log("web hello: the clicks");
    for (const h of hello) console.log(`  click ${h.click}: ${h.add ? `adds ${h.add} over the ${h.on} at ${fmt(h.at!)}` : h.kick ? `kicks ${h.kick}` : "meets nothing"}`);
    // (The mouse's box, added after the report, apart.)
    const mouse = hello.length === 6 ? hello.pop() : undefined;
    check(hello.filter((h) => h.add).length === 4 && hello.filter((h) => h.kick).length === 1, "4 boxes added, 1 kick");
    check(hello.filter((h) => h.add).map((h) => h.on).join() === "ground,wall,wall,ground", "added over the ground, the wall twice, the ground");
    check(hello.find((h) => h.kick)?.kick === "box3", "the kick meets box3");
    // A pixel of the view on the ground there is about 9 m / (view width) x 1.47 wide and a few times that deep.
    check(!!mouse?.add && mouse.on === "ground" && near(mouse.at!, [-3, OFF, 3.5], 60 / VW), `a mouse click on the canvas adds a box where it landed: ${mouse?.add} at ${mouse?.at ? fmt(mouse.at) : "?"}, (-3.000, 1.500, 3.500) within ${(60 / VW).toFixed(3)} m (a pixel)`);
    const boxes = report?.boxes ?? [];
    check(boxes.length === 4 && report?.kicks === 1, "the scene holds 4 boxes");
    if (boxes.length === 4) {
      const h = BOX / 2;
      for (const [k, y, tol] of [[0, h, 0.02], [1, h + BOX, 0.05], [2, h + 2 * BOX, 0.05]] as const) {
        check(near(boxes[k].at, [column[0], y, column[2]], tol), `box${k} rests at ${fmt(boxes[k].at)}: ${fmt([column[0], y, column[2]])} within ${tol} m`);
      }
      const from: V3 = [lone[0], h, lone[2]];
      const dir = unit([lone[0] - CAMERA.position[0], 0, lone[2] - CAMERA.position[2]]);
      const expected = (KICK * KICK) / (2 * 0.5 * 9.81);
      const moved = sub(boxes[3].at, from), along = dot(moved, dir), across = Math.hypot(...sub(moved, [dir[0] * along, 0, dir[2] * along]));
      check(Math.abs(along - expected) <= 0.15 * expected && across < 0.05, `box3 slid ${along.toFixed(3)} m along the kick (${across.toFixed(3)} m off it): ${expected.toFixed(3)} m within 15 %`);
      check(Math.abs(boxes[3].at[1] - h) < 0.01, `box3 is on the ground (y ${boxes[3].at[1].toFixed(4)})`);
      check(boxes.every((b) => b.asleep && b.speed === 0), "every box is at rest, asleep");
    }
    check(state.trips === 0, `the loop guard stopped nothing in ${state.frames} frames (${state.trips})`);
    // Software traversal: every added box and every frame a body moves rebuilds the scene (hardware: once, then in place).
    console.log(`web hello: ${state.frames} frames, ${state.rebuilt} of them rebuilt the scene (software traversal: each box added, each frame a body moved: ${state.moved})`);
    // The native test's numbers, if given: the same ticks, so the same boxes to the report's four decimals.
    const native = opt("--native");
    if (native && existsSync(native)) {
      const n = JSON.parse(readFileSync(native, "utf8")) as { hello: typeof hello; report: typeof report };
      const same = JSON.stringify(n.report) === JSON.stringify(report);
      check(same, `the report is the native run's, number for number${same ? "" : `: here ${JSON.stringify(report)}, natively ${JSON.stringify(n.report)}`}`);
      check(JSON.stringify(n.hello) === JSON.stringify(hello), "the clicks' log is the native run's");
    }
    out.scripted = { info, hello, report, ...state };
  }

  // 3. Frame times: the same clicks in real time (2.5 s apart), frames by what they did.
  if (has("--frames")) {
    const { page, info } = await open(`${VIEW}&fps=60${has("--profile-calls") ? "&profile=1" : ""}`);
    const sleep = (ms: number) => inPage(page, `${ms} ms of frames`, (t: number) => new Promise((ok) => setTimeout(ok, t)), ms);
    await sleep(4000);
    const column: V3 = [-1, 0, WALL_Z + OFF], lone: V3 = [1.5, 0, 2.5];
    for (const at of [column, [column[0], 1.2, WALL_Z] as V3, [column[0], 1.7, WALL_Z] as V3, lone, [lone[0], 0.3, lone[2]] as V3]) {
      await clickAt(page, project(at));
      await sleep(2500);
    }
    await sleep(3000);
    const data = await inPage(page, "reading the frames", () => { const s = (globalThis as unknown as { threert: Session }).threert; return { frames: s.frames, clicks: s.clicks, trips: s.guardTrips(), first: s.firstFrame() }; });
    await page.close();
    const F = data.frames;
    // The first frames (pipelines compiled on first use, caches filling) apart; then still frames, frames in which a
    // body moved (each rebuilds the scene), frames a click's box rebuilt.
    const rows = (name: string, f: FrameRow[]) => ({
      name,
      frames: f.length,
      interval: pair(f.map((x) => x.interval)),
      script: pair(f.map((x) => x.script - x.calls_ms)),
      calls: [Math.max(0, ...f.map((x) => x.calls)), ...pair(f.map((x) => x.calls_ms))],
      physics: pair(f.map((x) => (x.physics ? x.physics.step + x.physics.sync : 0))),
      commit: pair(f.map((x) => x.commit_ms)),
      rebuild: pair(f.map((x) => x.rebuild_ms)),
      record: pair(f.map((x) => x.render_ms)),
      page: pair(f.map((x) => x.begin + x.script + x.end)),
      gpu: pair(f.map((x) => x.gpu_ms)),
      trace: pair(f.map((x) => x.trace_ms)),
      scale: pair(f.map((x) => x.scale)),
    });
    const steady = F.slice(120);
    const table = [rows("still", steady.filter((f) => !f.rebuilt && !(f.physics && f.physics.moved > 0))), rows("a body moves", steady.filter((f) => f.physics && f.physics.moved > 0)), rows("rebuilt", steady.filter((f) => f.rebuilt))];
    console.log("web hello: frame times, ms p50 / p95 (interval; of the page's time: script, engine calls [most, p50, p95], physics, commit, of which rebuild, recording; GPU, of which trace)");
    for (const t of table) console.log(`  ${JSON.stringify(t)}`);
    console.log(`  the first frame on the GPU (every pipeline's first use: the shaders' compile): ${r3(data.first)} ms; first frames' intervals: ${F.slice(0, 6).map((f) => r3(f.interval)).join(", ")} ms; load: ${JSON.stringify(info.ms)}; ${JSON.stringify(info.load)}`);
    console.log(`  the clicks: ${JSON.stringify((data.clicks as ({ pick_ms: number; script_ms: number; commit_ms: number; rebuilt: boolean } | null)[]).map((c) => c && { pick: r3(c.pick_ms), script: r3(c.script_ms), commit: r3(c.commit_ms), rebuilt: c.rebuilt }))}`);
    check(data.trips === 0, `the loop guard stopped nothing in ${F.length} frames (${data.trips})`);
    out.frames = { info, table, first: F.slice(0, 30), clicks: data.clicks, first_frame_gpu_ms: data.first };
  }

  // 4. The boundary's cost in this browser (no GPU: the engine's world and physics alone).
  if (has("--bench")) {
    const { page } = await open("bench=1");
    const b = await inPage(page, "the boundary bench", () => (globalThis as unknown as { threert: Session }).threert.bench(10000));
    await page.close();
    console.log(`web hello: the boundary in this Chrome: ${JSON.stringify(b, null, 1)}`);
    out.bench = b;
  }
} catch (e) {
  if (e instanceof Stuck) {
    // No retry, no second page: whatever the GPU is doing, more work on it is what took the display down.
    console.log(`web hello: STOPPED, the page did not answer within ${TIMEOUT / 1000} s: ${e.message}`);
    console.log("web hello: nothing was retried. Leave the GPU alone, close the Chrome window by hand if it is still open, and look before running anything on the GPU again (the page's console above; `sample <pid>` of Chrome's GPU process; /Library/Logs/DiagnosticReports).");
    code = 3;
  } else if ((e as Error).message !== "small") {
    console.log(`web hello: ${(e as Error).message}`);
    code = 1;
  }
}
if (code !== 3) await context.close();
server.close();
const json = opt("--json");
if (json) { mkdirSync(dirname(json), { recursive: true }); writeFileSync(json, JSON.stringify({ ...out, failures }, null, 1)); }
console.log(code === 3 ? "web hello: STOPPED" : failures.length || code ? `web hello: FAIL (${failures.length})` : "web hello: pass");
process.exit(code || (failures.length ? 1 : 0));

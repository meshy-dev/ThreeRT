// The physics hello world on the browser's module without a browser (docs/RUNTIME.md "The engine in a browser"):
// renderer/web's wasm module loaded in Node (V8, the JavaScript engine Chrome has), `__engine` made of its exports by
// the page's own code (renderer/web/www/engine.ts), and the game bundle, the file the native runtime loads, run on
// it. No GPU and no view: what is checked is everything else of the browser's side: the exports, a descriptor's way
// across as JSON, the scene binding's calls arriving, the `click` world handler's ray query, the physics' ticks. The
// script is tools/physics-hello.ts's (five clicks, the same frames between them, 1 / 60 s each), the clicks given as
// the view rays the renderer would send, and the checks are its checks on the same numbers. CI runs it.
//
//   node tools/web-engine.ts [--native out.json] [--json out.json]
//   --native: the native test's numbers (node tools/physics-hello.ts --json out.json), compared number for number.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { BOX, CAMERA, KICK, OFF, WALL_Z } from "../games/physics/hello.game.ts";
import { repoRoot } from "./bundle-game.ts";
import { buildWeb, describeBuild } from "./lib/web.ts";
import { bench } from "../renderer/web/www/bench.ts";
import { webEngine, type EngineExports } from "../renderer/web/www/engine.ts";

type V3 = [number, number, number];
const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const unit = (a: V3): V3 => { const l = Math.hypot(...a); return [a[0] / l, a[1] / l, a[2] / l]; };

const args = process.argv.slice(2);
const opt = (f: string) => (args.includes(f) ? args[args.indexOf(f) + 1] : undefined);
const build = await buildWeb(join(repoRoot, "games/physics/hello.game.ts"));
console.log(describeBuild(build));
const wasm = (await import(pathToFileURL(join(build.dir, "pkg/threert_web.js")).href)) as EngineExports & { initSync(o: { module: BufferSource }): unknown; tick(dt: number): string; physicsHash(): string };
wasm.initSync({ module: readFileSync(join(build.dir, "pkg/threert_web_bg.wasm")) });
const engine = webEngine(wasm);
(globalThis as { __engine?: unknown }).__engine = engine;
// One for one: what the native runtime installs as `__engine` (script.rs `install_engine`), the page's `__engine`,
// the module's exports.
const natives = [...readFileSync(join(repoRoot, "renderer/src/script.rs"), "utf8").matchAll(/\be\.set\("(\w+)"/g)].map((m) => m[1]).sort();
const here = Object.keys(engine).sort();
const exported = here.filter((k) => typeof (wasm as unknown as Record<string, unknown>)[k] === "function");

// `--bench [n]`: the boundary's cost on this module in Node (renderer/web/www/bench.ts, what the page runs with
// `?bench=1`), each part's time as it ends; then nothing else.
if (args.includes("--bench")) {
  const n = Number(opt("--bench")) || 10000;
  const t = performance.now();
  const r = bench(engine, n, () => wasm.tick(1 / 60), (what, ms) => console.log(`  ${what}: ${ms.toFixed(0)} ms`));
  console.log(`web engine: the boundary in Node ${process.version}, ${((performance.now() - t) / 1000).toFixed(1)} s: ${JSON.stringify(r, null, 1)}`);
  process.exit(0);
}

// The game: one script, as the page evaluates it; its console is the test's record.
const lines: string[] = [];
const log = console.log;
console.log = (...a: unknown[]) => void lines.push(a.map(String).join(" "));
(0, eval)(readFileSync(join(build.dir, "game.js"), "utf8"));
const host = (globalThis as unknown as { __threert: { describe(): string; setup(room: string): string; event(type: string, payload: string, ts: number): string; frame(dt: number, now: number): void } }).__threert;
const describe = JSON.parse(host.describe()) as { on?: string[]; handlers?: string[] };
host.setup("local");

/** The view ray through a point of the world: what the renderer sends with a click (`LiveCore::ray`). */
function ray(p: V3) {
  const o = [...CAMERA.position] as V3;
  return { origin: o, direction: unit(sub(p, o)) };
}
const DT = 1 / 60;
let now = 1_000_000;
const step = (frames: number) => {
  for (let k = 0; k < frames; k++) {
    host.frame(DT, (now += 1000 * DT));
    wasm.tick(DT);
  }
};
const click = (at: V3, settle: number) => {
  host.event("click", JSON.stringify({ target: "", point: at, player: "you", ray: ray(at) }), (now += 1));
  step(settle);
};
step(10);
const column: V3 = [-1, 0, WALL_Z + OFF], lone: V3 = [1.5, 0, 2.5];
click(column, 120);
click([column[0], 1.2, WALL_Z], 150);
click([column[0], 1.7, WALL_Z], 180);
click(lone, 120);
click([lone[0], 0.3, lone[2]], 240);
host.event("report", "{}", (now += 1));
step(2);
console.log = log;

const hello = lines.map((l) => /hello (\{.*\})/.exec(l)?.[1]).filter((x): x is string => !!x).map((x) => JSON.parse(x) as { click: number; add?: string; kick?: string; on?: string; at?: V3 });
const report = lines.map((l) => /report (\{.*\})/.exec(l)?.[1]).filter((x): x is string => !!x).map((x) => JSON.parse(x) as { boxes: { name: string; at: V3; speed: number; asleep: boolean }[]; kicks: number }).pop();
const failures: string[] = [];
const check = (ok: boolean, what: string) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const near = (a: V3, b: V3, tol: number) => Math.hypot(...sub(a, b)) <= tol;
const fmt = (a: V3) => `(${a.map((v) => v.toFixed(3)).join(", ")})`;
console.log(`web engine: the module in Node ${process.version}; the game's handlers: ${JSON.stringify(describe.on)}`);
check(natives.length > 30 && natives.join() === here.join() && exported.length === here.length, `the engine API is the native one, one for one: ${natives.length} functions installed natively, ${here.length} in the page's __engine, ${exported.length} of them exports of the module${natives.join() === here.join() ? "" : `: native only ${natives.filter((n) => !here.includes(n))}, here only ${here.filter((n) => !natives.includes(n))}`}`);
for (const h of hello) console.log(`  click ${h.click}: ${h.add ? `adds ${h.add} over the ${h.on} at ${fmt(h.at!)}` : h.kick ? `kicks ${h.kick}` : "meets nothing"}`);
check(hello.filter((h) => h.add).length === 4 && hello.filter((h) => h.kick).length === 1, "4 boxes added, 1 kick");
check(hello.filter((h) => h.add).map((h) => h.on).join() === "ground,wall,wall,ground", "added over the ground, the wall twice, the ground");
check(hello.find((h) => h.kick)?.kick === "box3", "the kick meets box3");
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
const hash = wasm.physicsHash();
console.log(`web engine: the physics world's hash after the run: ${hash}`);
const native = opt("--native");
if (native && existsSync(native)) {
  const n = JSON.parse(readFileSync(native, "utf8")) as { hello: typeof hello; report: typeof report };
  const same = JSON.stringify(n.report) === JSON.stringify(report);
  check(same, `the report is the native run's, number for number${same ? "" : `: here ${JSON.stringify(report)}, natively ${JSON.stringify(n.report)}`}`);
}
const json = opt("--json");
if (json) writeFileSync(json, JSON.stringify({ hello, report, hash, failures }, null, 1));
console.log(failures.length ? `web engine: FAIL (${failures.length})` : "web engine: pass");
process.exit(failures.length ? 1 : 0);

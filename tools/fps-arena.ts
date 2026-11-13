// The first-person arena without a window (games/fps/arena.game.ts; docs/RUNTIME.md "The host channel", "Input",
// "HUD"): the game in the live renderer in lockstep, with this script as the app of its host channel. The app sends
// `launch` (the runner's `--launch`) and `input` messages (`{"cmd":"host","messages":[...]}` on stdin) and reads what
// the game posts (`{"host":[...]}` lines on stdout: its HUD). Checked on numbers:
//   1. launch: the player's name from `--launch` is in the HUD; the capsule rests a skin above the ground;
//   2. W held for a second: 4.5 m forward, and nothing more once the key is let go;
//   3. sprinting at the platform with frames of 0.1 s (0.75 m a frame, more than a wall is thick): stopped a skin
//      off its face, not inside it and not behind it;
//   4. walking into a wall at 45 degrees: it slides along the wall, and stops in the corner;
//   5. Space: a jump of v² / 2g, and back on the ground; held Space does not jump again;
//   6. the view turned onto a target by `look`, a press of the mouse button: tagged, once; the HUD says 1 / 5 with a
//      hit flash; a second press on it tags nothing; `restart` (the R key's handler) puts it back.
// Exit 1 on a failed check, 2 when the GPU lock is held. The GPU lock is held.
//   node tools/fps-arena.ts
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { HALF, HALF_HEIGHT, RADIUS, SKIN, SPRINT, START, TARGETS, WALK } from "../games/fps/arena.game.ts";
import type { Hud, HudText } from "@threert/gameplay";
import { acquireGpuLock, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";

const holder = acquireGpuLock("fps-arena");
if (holder) { console.log(`not run: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }
const bundle = join(repoRoot, "out/bundles/fps-arena.js");
execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/fps/arena.game.ts"), "-o", bundle], { stdio: "ignore" });

// What the game posted: every message, and the HUD as last described.
const posts: { t: string; hud?: Hud }[] = [];
const live = startLive(branchBuild(rendererBin), bundle, {
  width: 640, height: 360,
  args: ["--launch", JSON.stringify({ name: "Ada", room: "K7Q4MZ" })],
  stdout: (line) => { try { posts.push(...((JSON.parse(line) as { host?: { t: string }[] }).host ?? [])); } catch { /* not a line of ours */ } },
});
await live.firstFrame;
const DT = 1 / 60;
live.send({ cmd: "lockstep", dt: DT });
let id = 0;
const step = async (frames: number) => {
  const sync = `s${++id}`;
  live.send({ cmd: "frames", n: frames, sync });
  await live.wait((l) => l.includes(`live: sync ${sync}`));
};
/** An input message, as an app sends one a frame (here: when something changes; what is held stays held). */
const input = (m: object) => live.send({ cmd: "host", messages: [{ t: "input", captured: true, ...m }] });
interface Report { at: [number, number, number]; yaw: number; pitch: number; grounded: boolean; tagged: number; shots: number }
const report = async (): Promise<Report> => {
  const n = live.lines.length;
  live.send({ cmd: "event", type: "report" });
  await step(1);
  const line = live.lines.slice(n).map((l) => /arena (\{"at".*\})/.exec(l)?.[1]).find((x) => x);
  if (!line) throw new Error("no report");
  return JSON.parse(line) as Report;
};
const hud = (): Hud => posts.filter((p) => p.t === "hud").at(-1)?.hud ?? {};
const text = (h: Hud, label: string) => (h.items ?? []).find((i): i is HudText => i.type === "text" && i.label === label)?.text;

const failures: string[] = [];
const check = (ok: boolean, what: string) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const f = (v: number) => v.toFixed(3);
const REST = HALF_HEIGHT + RADIUS + SKIN;

// 1. Launch, and rest.
await step(30);
let r = await report();
check(JSON.stringify(hud()).includes('"Ada"'), "the HUD has the name the app launched the game with");
check(hud().crosshair?.style === "cross" && text(hud(), "Tagged") === `0 / ${TARGETS.length}`, `the HUD: a crosshair, Tagged ${text(hud(), "Tagged")}`);
check(r.grounded && Math.abs(r.at[1] - REST) < 2e-3 && r.at[0] === START[0] && r.at[2] === START[2], `at rest on the ground: y ${f(r.at[1])} (${f(REST)}: the capsule a skin above it)`);
const quiet = posts.length;
await step(30);
check(posts.length === quiet, "nothing posted while the HUD does not change");

// 2. Walk.
input({ keys: ["W"] });
await step(60);
input({ keys: [] });
await step(10);
r = await report();
check(Math.abs(START[2] - r.at[2] - WALK) < 0.02 && Math.abs(r.at[0]) < 1e-3, `W for 60 frames: ${f(START[2] - r.at[2])} m forward (${WALK} m/s), then still`);

// 3. Sprint at the platform's face (z = -8.8) with long frames.
live.send({ cmd: "lockstep", dt: 0.1 });
input({ keys: ["W", "Shift"] });
await step(40);
input({ keys: [] });
live.send({ cmd: "lockstep", dt: DT });
await step(2);
r = await report();
const face = -8.8 + RADIUS;
check(r.at[2] >= face && r.at[2] - face < 2 * SKIN + 1e-3, `sprinting ${f(SPRINT * 0.1)} m a frame at the platform: stopped at z ${f(r.at[2])}, ${f(r.at[2] - face)} m off its face`);

// 4. Back to the middle, then into the west wall at 45 degrees: it slides north along it into the corner.
live.send({ cmd: "event", type: "restart" });
input({ dx: -Math.PI / 4 }); // (to the left: yaw is the other way from look's dx)
await step(1);
input({ keys: ["W", "Shift"] });
await step(180);
r = await report();
const west = -HALF + RADIUS;
check(Math.abs(r.yaw - Math.PI / 4) < 1e-4, `look.dx of -45 degrees turned the view 45 degrees left (yaw ${f(r.yaw)})`);
check(r.at[0] >= west && r.at[0] - west < 2 * SKIN + 1e-3 && r.at[2] < START[2] - 8, `along the west wall: x ${f(r.at[0])} (${f(r.at[0] - west)} m off it), z ${f(r.at[2])}`);
await step(240);
input({ keys: [] });
await step(2);
r = await report();
const north = -HALF + RADIUS;
check(r.at[0] - west < 2 * SKIN + 1e-3 && r.at[2] >= north && r.at[2] - north < 2 * SKIN + 1e-3, `in the corner: (${f(r.at[0])}, ${f(r.at[2])}), inside both walls`);

// 5. Jump.
live.send({ cmd: "event", type: "restart" });
await step(5);
input({ keys: ["Space"], pressed: ["Space"] });
let top = 0;
for (let k = 0; k < 60; k++) {
  await step(1);
  top = Math.max(top, (await report()).at[1]);
}
r = await report();
const rise = 5 * 5 / (2 * 14);
check(Math.abs(top - REST - rise) < 0.06, `Space: up ${f(top - REST)} m (v² / 2g = ${f(rise)})`);
check(r.grounded && Math.abs(r.at[1] - REST) < 2e-3, "and down again; Space still held does not jump twice");
input({ keys: [] });

// 6. Turn onto a target and tag it.
const target = TARGETS[4];
const eye = [r.at[0], r.at[1] + 0.7, r.at[2]];
const d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
const yaw = Math.atan2(-d[0], -d[2]), pitch = Math.asin(d[1] / Math.hypot(...d));
input({ dx: r.yaw - yaw, dy: r.pitch - pitch });
await step(1);
input({ buttons: [0], pressed: ["Mouse0"] });
await step(1);
input({ buttons: [], released: ["Mouse0"] });
await step(2);
r = await report();
const tags = live.lines.filter((l) => l.includes('arena {"tagged"'));
check(r.tagged === 1 && r.shots === 1 && tags.length === 1 && tags[0].includes("target 5"), `the view ray tags ${/"tagged":"([^"]+)"/.exec(tags[0] ?? "")?.[1] ?? "nothing"} with one shot`);
check(text(hud(), "Tagged") === `1 / ${TARGETS.length}` && hud().flash?.kind === "hit" && hud().flash?.n === 1, `the HUD: Tagged ${text(hud(), "Tagged")}, a hit flash`);
input({ buttons: [0], pressed: ["Mouse0"] });
await step(1);
input({ buttons: [] });
await step(1);
r = await report();
check(r.tagged === 1 && r.shots === 2, "a second shot on it tags nothing more");
live.send({ cmd: "event", type: "restart" });
await step(2);
r = await report();
check(r.tagged === 0 && text(hud(), "Tagged") === `0 / ${TARGETS.length}`, "restart (the R key's handler): none tagged");

await live.quit();
const errors = live.lines.filter((l) => /frame script:|host messages failed|\[game error\]/.test(l));
check(errors.length === 0, `no script error${errors.length ? `: ${errors[0]}` : ""}`);
console.log(`fps arena: ${posts.length} messages posted by the game (${posts.filter((p) => p.t === "hud").length} HUD descriptions) over ${id} steps`);
if (failures.length) { console.error(`fps arena: ${failures.length} check(s) failed`); process.exit(1); }
console.log("fps arena: all checks pass");

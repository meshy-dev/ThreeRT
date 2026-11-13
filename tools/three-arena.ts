// The first-person arena in the three.js view, in a real browser (games/fps/arena.game.ts; tools/three/view.ts;
// docs/RUNTIME.md "The host channel"): headless Chrome driven over the DevTools protocol with the events a keyboard
// and a mouse send, so the page's own input path is what is checked (key codes, the pointer lock, the mouse's
// motion), with the engine's physics loaded (tools/three/physics.ts). Real time, so the distances are checked loosely;
// the exact numbers are tools/three/test/physics.test.ts's and tools/fps-arena.ts's.
//   1. launch: the name in the page's query is in the HUD, which is DOM over the canvas;
//   2. W held for a second: about 4.5 m forward;
//   3. a click takes the pointer lock and is not a shot; the mouse moved 100 px turns the view 0.25 rad;
//   4. Shift + W into the platform: stopped at its face;
//   5. a click with the pointer held is a shot.
// Writes a screenshot (out/three/arena.png). Exit 1 on a failed check.
//   node tools/three-arena.ts
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { RADIUS, SKIN, START, WALK } from "../games/fps/arena.game.ts";
import { bundlePage, launch, root, serve } from "./three/browser.ts";
import { buildScenePhysics, SCENE_PHYSICS } from "./three/physics-build.ts";

const lines: string[] = [];
const { html } = await bundlePage("fps-arena-view", join(root, "games/fps/arena.game.ts"), join(root, "tools/three/view.ts"));
buildScenePhysics();
const { server, origin } = await serve(root);
const browser = await launch((l) => lines.push(l));
const failures: string[] = [];
const check = (ok: boolean, what: string) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
try {
  const page = await browser.open(`${origin}${html}?physics=/${SCENE_PHYSICS}&name=Ada&room=K7Q4MZ`);
  await page.evaluate(`new Promise((ok, fail) => { const t0 = Date.now(); const k = setInterval(() => { if (document.querySelector(".hud-text")) { clearInterval(k); ok(true); } else if (Date.now() - t0 > 60000) { clearInterval(k); fail(new Error("no HUD after 60 s")); } }, 100); })`);
  await sleep(500);
  interface Report { at: [number, number, number]; yaw: number; pitch: number; grounded: boolean; tagged: number; shots: number }
  const report = async (): Promise<Report> => {
    const n = lines.length;
    await page.evaluate(`void __threert.event("report", "{}", Date.now())`);
    for (let k = 0; k < 50; k++) {
      const l = lines.slice(n).map((x) => /arena (\{"at".*\})/.exec(x)?.[1]).find((x) => x);
      if (l) return JSON.parse(l) as Report;
      await sleep(20);
    }
    throw new Error("no report");
  };
  const key = (type: "keyDown" | "keyUp", code: string, vk: number) => page.input("dispatchKeyEvent", { type, code, key: code.replace(/^Key/, "").toLowerCase(), windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk });
  const mouse = (type: "mousePressed" | "mouseReleased" | "mouseMoved", x: number, y: number, held = false) =>
    page.input("dispatchMouseEvent", { type, x, y, button: type === "mouseMoved" && !held ? "none" : "left", buttons: type === "mousePressed" || held ? 1 : 0, clickCount: type === "mouseMoved" ? 0 : 1 });
  const hud = () => page.evaluate<string>(`document.querySelector(".hud").innerText`);

  // 1. Launch and the HUD.
  let r = await report();
  const text = await hud();
  check(text.includes("Ada") && /0 \/ 5/.test(text) && /click to look around/i.test(text), `the HUD is on the page: ${JSON.stringify(text.replace(/\s+/g, " ").slice(0, 90))}`);
  check(r.grounded && r.at[0] === START[0] && Math.abs(r.at[1] - 0.92) < 2e-3, `the capsule rests on the engine's ground: y ${r.at[1]}`);
  // 2. W for a second.
  await key("keyDown", "KeyW", 87);
  await sleep(1000);
  await key("keyUp", "KeyW", 87);
  await sleep(100);
  r = await report();
  const walked = START[2] - r.at[2];
  check(Math.abs(walked - WALK) < 0.6, `W for a second: ${walked.toFixed(2)} m forward (${WALK} m/s)`);
  // 3. The pointer lock, and look.
  await mouse("mouseMoved", 640, 360);
  await mouse("mousePressed", 640, 360);
  await mouse("mouseReleased", 640, 360);
  await sleep(300);
  const locked = await page.evaluate<boolean>(`document.pointerLockElement === document.querySelector("canvas")`);
  r = await report();
  check(locked && r.shots === 0, `a click takes the pointer lock (${locked}) and is not a shot`);
  check(!/click to look around/i.test(await hud()), "the HUD's hint goes once the pointer is the game's");
  for (let k = 1; k <= 10; k++) await mouse("mouseMoved", 640 + 10 * k, 360);
  await sleep(100);
  r = await report();
  check(Math.abs(r.yaw + 0.25) < 0.02, `the mouse moved 100 px to the right: yaw ${r.yaw} (0.0025 rad a pixel)`);
  for (let k = 9; k >= 0; k--) await mouse("mouseMoved", 640 + 10 * k, 360);
  await sleep(100);
  mkdirSync(join(root, "out/three"), { recursive: true });
  await page.screenshot(join(root, "out/three/arena.png"));
  // 4. Sprint into the platform.
  await page.input("dispatchKeyEvent", { type: "keyDown", code: "ShiftLeft", key: "Shift", windowsVirtualKeyCode: 16, nativeVirtualKeyCode: 16 });
  await key("keyDown", "KeyW", 87);
  await sleep(2500);
  await key("keyUp", "KeyW", 87);
  await page.input("dispatchKeyEvent", { type: "keyUp", code: "ShiftLeft", key: "Shift", windowsVirtualKeyCode: 16, nativeVirtualKeyCode: 16 });
  await sleep(100);
  r = await report();
  const face = -8.8 + RADIUS;
  check(Math.abs(r.yaw) < 0.02 && r.at[2] >= face && r.at[2] - face < 2 * SKIN + 1e-3, `sprinting at the platform: stopped at z ${r.at[2]} (${(r.at[2] - face).toFixed(3)} m off its face)`);
  // 5. A shot.
  await mouse("mousePressed", 640, 360);
  await mouse("mouseReleased", 640, 360);
  await sleep(150);
  r = await report();
  check(r.shots === 1, "a click with the pointer held is a shot");
  const errors = lines.filter((l) => /\[page (exception|error)\]|frame script:/.test(l));
  check(errors.length === 0, `no error on the page${errors.length ? `: ${errors[0]}` : ""}`);
  await page.close();
} finally {
  browser.close();
  server.close();
}
if (failures.length) { console.error(`three arena: ${failures.length} check(s) failed`); process.exit(1); }
console.log("three arena: all checks pass (out/three/arena.png)");

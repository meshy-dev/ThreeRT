// ROADMAP § 0 step 1's comparison with the source app: jeantimex/tokyo itself (three.js WebGL) on the same compiled
// tiles, at the 5 poses of games/tokyo, against our offline render (tools/tokyo/truth.ts, rendered when missing): its
// frame time, and ΔE / SSIM / luminance. Headless Chrome over the DevTools protocol (tools/three/browser.ts); the app
// waits until every tile is in (`#loader.done`), then each pose is set on its camera (window.__app) and the tiles round
// it are let in before the shot. The shot is the canvas alone: the app's buttons, settings panel and credits are hidden
// for it (the credits stand wherever we show the city).
//
//   (the app, served:  npm --prefix <clone> run dev -- --host 127.0.0.1 --port 5280 --strictPort, with <clone>/public/
//    tiles, ortho and textures linked to assets/city/)
//   node tools/tokyo/three-app.ts [--url http://127.0.0.1:5280] [--area shinjuku] [--times day,night] [--spp 4096]
//
// The app's view: traffic, birds and clouds off, every tile loaded (radius 3000 m), day = Tokyo 12:00 (on the date of
// the run; our sun, games/tokyo SUN, is that of 2026-11-09), night = its ?night=1 (22:00). Frame time two ways: the
// app's own frame loop (rAF intervals: what its user sees, capped by the display rate), and its frame function run back
// to back with gl.finish() (CPU + GPU, finished). Its camera keeps 2 m above the ground (ours stands at 1.7 m); its
// controls are freed to look up for the canyon pose.
// Holds the GPU lock (tools/lib/bench.ts).
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { bundleGame } from "../bundle-game.ts";
import { repoRoot, requireGpuLock } from "../lib/bench.ts";
import { launch } from "../three/browser.ts";
import { compare, truth } from "./truth.ts";
import { POSES, SUN } from "../../games/tokyo/tokyo.game.ts";

requireGpuLock("tokyo-three-app");
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const base = opt("--url", "http://127.0.0.1:5280"), area = opt("--area", "shinjuku"), times = opt("--times", "day,night").split(","), spp = Number(opt("--spp", "4096"));
const out = join(repoRoot, "out/bench/tokyo", area, "three");
mkdirSync(out, { recursive: true });
const bundle = join(repoRoot, "out/bundles/tokyo.js");
await bundleGame(join(repoRoot, "games/tokyo/tokyo.game.ts"), bundle);
const result: Record<string, unknown> = {};

const browser = await launch((l) => { if (/exception|error/i.test(l)) console.log(`  ${l}`); });
try {
  for (const time of times) {
    const room = `${area}-${time}`;
    const url = `${base}/?area=${area}&traffic=0&birds=0&clouds=0&preview=0&radius=3000${time === "night" ? "&night=1" : "&time=12"}`;
    const page = await browser.open(url);
    const t0 = Date.now();
    await page.evaluate(`new Promise((ok, fail) => { const k = setInterval(() => { const l = document.getElementById("loader"); if (l && l.classList.contains("done") && window.__app) { clearInterval(k); ok(true); } }, 250); setTimeout(() => fail(new Error("the app did not finish loading")), 900000); })`, 905_000);
    const sun = await page.evaluate<number[]>(`__app.env.sunDir.toArray()`);
    console.log(`${room}: the app loaded in ${((Date.now() - t0) / 1000).toFixed(1)} s; its sun / moon direction ${sun.map((v) => v.toFixed(3)).join(", ")}`);
    result[`${room} app`] = { load_s: +((Date.now() - t0) / 1000).toFixed(1), sun };
    if (time === "day") {
      // the app's sun follows the date of the run; ours is fixed (SUN): say when they part by more than a degree
      const el = (SUN.elevation * Math.PI) / 180, az = (SUN.azimuth * Math.PI) / 180;
      const ours = [Math.sin(az) * Math.cos(el), Math.sin(el), Math.cos(az) * Math.cos(el)];
      const deg = (Math.acos(Math.min(1, ours.reduce((a, v, i) => a + v * sun[i], 0))) * 180) / Math.PI;
      if (deg > 1) console.log(`  (the app's sun is ${deg.toFixed(1)}° from ours: games/tokyo SUN is Tokyo's noon of 2026-11-09)`);
    }
    for (const [name, p] of Object.entries(POSES)) {
      // the camera at the pose; the controls freed to look up, without damping
      await page.evaluate(`(() => { const a = __app; a.controls.enableDamping = false; a.controls.maxPolarAngle = Math.PI; a.controls.minDistance = 0.1;
        a.camera.fov = ${p.fov}; a.camera.updateProjectionMatrix(); a.camera.position.set(${p.position.join(",")}); a.controls.target.set(${p.target.join(",")}); a.controls.update(); return true; })()`);
      // the tiles round the view in, then let it settle (shadows, the temporal antialiasing)
      await page.evaluate(`new Promise((ok) => { let last = -1, same = 0; const k = setInterval(() => { const n = __app.streamer.stats.loaded; same = n === last ? same + 1 : 0; last = n; if (same >= 8) { clearInterval(k); ok(n); } }, 250); })`);
      await new Promise((r) => setTimeout(r, 3000));
      const pos = await page.evaluate<number[]>(`__app.camera.position.toArray()`);
      // frame time: the app's own loop (rAF), then its frame function back to back, finished
      const raf = await page.evaluate<number[]>(`new Promise((ok) => { const t = []; let prev = performance.now(); const f = (now) => { t.push(now - prev); prev = now; if (t.length < 121) requestAnimationFrame(f); else ok(t.slice(1)); }; requestAnimationFrame(f); })`);
      const fin = await page.evaluate<number[]>(`(() => { const gl = __app.renderer.getContext(), t = []; for (let i = 0; i < 40; i++) { const s = performance.now(); __app.tick(); gl.finish(); t.push(performance.now() - s); } return t.slice(5); })()`);
      await page.evaluate(`(() => { if (!document.getElementById("shot")) document.head.insertAdjacentHTML("beforeend", '<style id="shot">#mode, #github, #credits, #hud, .lil-gui { visibility: hidden !important; }</style>'); return true; })()`);
      await new Promise((r) => setTimeout(r, 1000));
      const png = join(out, `three-${room}-${name}.png`);
      await page.screenshot(png);
      const pct = (xs: number[], q: number) => [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(q * xs.length))];
      const m = compare(truth(bundle, room, name, p, spp), png);
      const row = { frame_raf_p50_ms: +pct(raf, 0.5).toFixed(2), frame_raf_p95_ms: +pct(raf, 0.95).toFixed(2), frame_finished_p50_ms: +pct(fin, 0.5).toFixed(2), frame_finished_p95_ms: +pct(fin, 0.95).toFixed(2), camera: pos.map((v) => +v.toFixed(2)), ...m };
      result[`${room} ${name}`] = row;
      console.log(`  ${name.padEnd(18)} three.js: frame (rAF) p50 / p95 ${row.frame_raf_p50_ms} / ${row.frame_raf_p95_ms} ms, finished ${row.frame_finished_p50_ms} / ${row.frame_finished_p95_ms} ms; vs our offline render: ΔE ${m.mean_delta_e.toFixed(2)}  SSIM ${m.ssim.toFixed(3)}  luminance ${m.luminance_ratio.toFixed(3)}`);
    }
    await page.close();
  }
} finally {
  browser.close();
}
const json = join(out, "three-app.json");
writeFileSync(json, JSON.stringify(result, null, 1) + "\n");
console.log(`-> ${json}`);

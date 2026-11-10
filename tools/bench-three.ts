// three.js baseline vs ThreeRT vs UE5. The SAME scene code (scenes/*.ts and the games) drives three.js r187 through
// `ThreeEngine implements Engine` (tools/three/engine.ts) + the same SceneBinding the render node uses, rendered in
// headless Chrome (WebGL2 / ANGLE Metal), and ThreeRT's native renderer (offline converged and the 60 fps live
// viewport); every image is compared with the UE5 references (Path Tracer = ground truth, Lumen).
//
//   node tools/bench-three.ts [--scenes a,b] [--steps ours,three,report] [--frames 1,16,256] [--warmup 8] [--scale auto|0.5..1] [--assets DIR] [--calibrate-sky] [--refresh-three]
//
// Steps: `ours` = native offline (256 spp) + live viewport (--scale auto, `--warmup` s to settle, then history reset and
// snapshots after --frames frames); `three` = headless Chrome renders (needs ours.json for auto-exposure scenes),
// cached in out/cache/three by page content and query (--refresh-three renders anyway);
// `report` = compare with UE, tables, montages. `--calibrate-sky` re-derives sky.ts SKY_NITS_PER_LUX.
// Outputs: out/three/<view>.png (three.js, ACES + vignette; -probe / -agx / -neutral / -novignette variants),
// out/three/ours/*, out/bench/three.{json,md}, montages out/bench/three/<view>.jpg (three.js | ThreeRT live @16 |
// ThreeRT offline | UE). Kabukichō is benchmarked at UE view v1 (out/ue-kb); its game-camera three.js render
// (out/three/kabukicho.png) is the playground's reference.
import { execFileSync, spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { sources } from "./catalog.ts";
import { rendererBin } from "./lib/bench.ts";
import { bundleGame } from "./bundle-game.ts";
import { bundlePage, launch, root, serve } from "./three/browser.ts";

const bin = rendererBin; // THREERT_BIN measures another build
const arg = (k: string, d: string) => (process.argv.includes(k) ? process.argv[process.argv.indexOf(k) + 1] : d);
const all = sources();
const ids = arg("--scenes", Object.keys(all).join(",")).split(",").filter((id) => all[id]);
const steps = new Set(arg("--steps", "three,ours,report").split(","));
const liveFrames = arg("--frames", "1,16,256").split(",").map(Number);
const liveScale = arg("--scale", "auto"); // live viewport render scale (auto = dynamic resolution at 60 fps)
/** Asset root for scene files (default: the repo). */
const assets = resolve(arg("--assets", root));
const outThree = join(root, "out/three"), outOurs = join(root, "out/three/ours"), outBench = join(root, "out/bench");
for (const d of [outThree, outOurs, join(outBench, "three")]) mkdirSync(d, { recursive: true });

/** Kabukichō is measured against the UE ground truth of the actual UE level (out/ue-kb), camera view v1. */
const UE_KB_V1 = { loc: [-1150, -180, 170], rot: [4, 4, 0], fov: 55 };
function ueView(loc: number[], rot: number[], fov: number) {
  // tools/kabukicho/views.py: UE location (cm, Z up) + rotator (pitch, yaw) -> our eye + look-at target (m, Y up)
  const p = (rot[0] * Math.PI) / 180, y = (rot[1] * Math.PI) / 180;
  const fwd = [Math.cos(p) * Math.cos(y), Math.cos(p) * Math.sin(y), Math.sin(p)];
  const pos: [number, number, number] = [loc[0] / 100, loc[2] / 100, loc[1] / 100];
  return { position: pos, target: [pos[0] + fwd[0], pos[1] + fwd[2], pos[2] + fwd[1]] as [number, number, number], fov };
}
/** Views per catalog id: the scene's own camera, plus v1 for Kabukichō. */
const VIEWS: Record<string, { view: string; camera?: ReturnType<typeof ueView>; refs: { pt: string | null; lumen: string | null } }[]> = {};
for (const id of ids) {
  const ref = (m: string) => (existsSync(join(root, "out/ue", m, `${id}.png`)) ? join(root, "out/ue", m, `${id}.png`) : null);
  VIEWS[id] = [{ view: id, refs: { pt: ref("pathtracer"), lumen: ref("lumen") } }];
  if (id === "kabukicho") VIEWS[id].push({ view: "kabukicho-v1", camera: ueView(UE_KB_V1.loc, UE_KB_V1.rot, UE_KB_V1.fov), refs: { pt: null, lumen: join(root, "out/ue-kb/lumen-auto/v1.png") } });
}

const compare = (ref: string | null, img: string) => (ref && existsSync(ref) && existsSync(img) ? JSON.parse(execFileSync(bin, ["compare", ref, img]).toString()) : null);
const readJson = (p: string, d: any) => (existsSync(p) ? JSON.parse(readFileSync(p, "utf8")) : d);

// ------------------------------------------------------------------------------------------------ ThreeRT (native)

/** Offline converged (256 spp + OIDN) and the live viewport (60 fps, dynamic resolution) at every view. */
async function runOurs(id: string) {
  const bundle = join(root, "out/bundles", `${id}.js`);
  await bundleGame(all[id], bundle);
  const res: Record<string, any> = readJson(join(outOurs, "ours.json"), {});
  for (const v of VIEWS[id]) {
    const out = join(outOurs, `${v.view}-offline.png`);
    const args = ["render", bundle, "-o", out, "--assets", assets, "--spp", "256"];
    if (v.camera) args.push("--camera", JSON.stringify(v.camera));
    const stats = JSON.parse(execFileSync(bin, args, { maxBuffer: 1 << 26 }).toString().trim().split("\n").pop()!);
    res[v.view] = { offline: { ev100: stats.ev100, ms: stats.timings_ms.total_ms, trace_ms: stats.timings_ms.trace_ms, denoise: stats.denoise, width: stats.width, height: stats.height }, live: {} };
    console.log(`ours ${v.view}: offline ${(stats.timings_ms.total_ms / 1000).toFixed(1)} s, EV100 ${stats.ev100.toFixed(2)}`);
    const live = await runLive(bundle, v.view, stats.width, stats.height, v.camera);
    res[v.view].live = live;
  }
  writeFileSync(join(outOurs, "ours.json"), JSON.stringify(res, null, 1));
}

async function runLive(bundle: string, view: string, w: number, h: number, camera?: ReturnType<typeof ueView>) {
  const p = spawn(bin, ["live", bundle, "--assets", assets, "--width", String(w), "--height", String(h), "--fps", "60", "--scale", liveScale], { stdio: ["pipe", "ignore", "pipe"] });
  const send = (c: object) => p.stdin.write(JSON.stringify(c) + "\n");
  const got: Record<number, { spp: number; gpu_ms: number; scale: number }> = {};
  const done = new Promise<void>((ok, fail) => {
    const t = setTimeout(() => fail(new Error(`live ${view}: timed out`)), 180_000);
    createInterface({ input: p.stderr! }).on("line", (l) => {
      const m = /snapshot .*-live-(\d+)\.png at history (\d+) \((\d+) spp\/frame, GPU ([\d.]+) ms(?:, scale ([\d.]+))?\)/.exec(l);
      if (!m) return;
      got[Number(m[1])] = { spp: Number(m[3]), gpu_ms: Number(m[4]), scale: Number(m[5] ?? 1) };
      if (Object.keys(got).length === liveFrames.length) {
        clearTimeout(t);
        ok();
      }
    });
  });
  // Pipelines, texture pages and the dynamic resolution (60 fps budget) settle before the history reset.
  if (camera) send({ cmd: "camera", pos: camera.position, target: camera.target, tan: Math.tan((camera.fov * Math.PI) / 360) });
  await new Promise((r) => setTimeout(r, Number(arg("--warmup", "8")) * 1000));
  send({ cmd: "reset" });
  for (const k of liveFrames) send({ cmd: "snapshot", path: join(outOurs, `${view}-live-${k}.png`), after: k });
  await done;
  send({ cmd: "quit" });
  console.log(`ours ${view}: live ${liveFrames.map((k) => `@${k} ${got[k].gpu_ms} ms (scale ${got[k].scale})`).join(", ")}`);
  return got;
}

// ------------------------------------------------------------------------------------------------ three.js

async function runThree(idsToRun: string[]) {
  const ours = readJson(join(outOurs, "ours.json"), {});
  const { server, origin } = await serve(assets);
  // Chrome starts only when a view is not cached.
  const chrome: { b: Promise<Awaited<ReturnType<typeof launch>>> | null } = { b: null };
  const browser = () => ({
    run: async (url: string) => {
      chrome.b ??= launch((l) => (process.env.VERBOSE || /error|exception|warn/i.test(l) ? console.log(l.slice(0, 400)) : undefined));
      return (await chrome.b).run(url);
    },
  });
  const res: Record<string, any> = readJson(join(outThree, "three.json"), {});
  try {
    for (const id of idsToRun) {
      const { html, bytes, hash } = await bundlePage(id, all[id]);
      for (const v of VIEWS[id]) {
        const q = new URLSearchParams();
        const ev = ours[v.view]?.offline?.ev100;
        if (ev !== undefined) q.set("ev100", String(ev));
        if (v.camera) q.set("camera", JSON.stringify(v.camera));
        // Cached by the page's content (scene, binding, three.js) and its query (exposure, camera): three.js output
        // cannot change otherwise, so test:layer-b does not re-render it.
        const cache = join(root, "out/cache/three", createHash("sha256").update(hash).update(q.toString()).digest("hex").slice(0, 20));
        const view = (tm: string) => `${v.view}${tm === "aces" ? "" : `-${tm}`}.png`;
        if (existsSync(join(cache, "result.json")) && !process.argv.includes("--refresh-three")) {
          const r = JSON.parse(readFileSync(join(cache, "result.json"), "utf8"));
          for (const tm of r.variants as string[]) copyFileSync(join(cache, `${tm}.png`), join(outThree, view(tm)));
          delete r.variants;
          res[v.view] = r;
          console.log(`three ${v.view}: cached`);
          continue;
        }
        const t0 = Date.now();
        const r = await browser().run(`${origin}${html}?${q}`);
        if (!r.ok) {
          console.log(`three ${v.view}: FAILED ${r.error}`);
          res[v.view] = { ok: false, error: r.error };
          continue;
        }
        mkdirSync(cache, { recursive: true });
        const variants = Object.keys(r.images as Record<string, string>);
        for (const [tm, url] of Object.entries(r.images as Record<string, string>)) {
          const png = Buffer.from(url.split(",")[1], "base64");
          writeFileSync(join(outThree, view(tm)), png);
          writeFileSync(join(cache, `${tm}.png`), png);
        }
        delete r.images;
        res[v.view] = { ...r, bundle_kib: Math.round(bytes / 1024) };
        writeFileSync(join(cache, "result.json"), JSON.stringify({ ...res[v.view], variants }, null, 1));
        console.log(`three ${v.view}: ${((Date.now() - t0) / 1000).toFixed(1)} s, ${r.webgl}, GPU ${r.gpuMs?.toFixed(2) ?? "n/a"} ms, sync ${r.syncMs.toFixed(2)} ms, ${r.drawCalls} draws, dropped ${JSON.stringify(r.dropped)}`);
      }
    }
  } finally {
    if (chrome.b) (await chrome.b).close();
    server.close();
  }
  writeFileSync(join(outThree, "three.json"), JSON.stringify(res, null, 1));
}

// ------------------------------------------------------------------------------------------------ sky calibration

/** SKY_NITS_PER_LUX: horizontal sky irradiance of the native atmosphere (1 lux sun at 30 degrees, sun blocked) over
 *  that of the Sky addon's output (cosine-weighted over the upper hemisphere, no sun disc). */
async function calibrateSky() {
  const { preethamSky } = await import("./three/sky.ts");
  const { ELEVATION } = await import("./three/calib-sky.ts");
  const bundle = join(root, "out/three/calib-sky.js"), raw = join(root, "out/three/calib-sky.raw");
  await bundleGame(join(root, "tools/three/calib-sky.ts"), bundle);
  execFileSync(bin, ["render", bundle, "-o", join(root, "out/three/calib-sky.png"), "--assets", assets, "--spp", "1024", "--no-denoise", "--hdr", raw]);
  const f = new Float32Array(readFileSync(raw).buffer.slice(0));
  const n = Math.round(Math.sqrt(f.length / 3));
  const c = [0, 0, 0];
  let k = 0;
  for (let y = n / 2 - 8; y < n / 2 + 8; y++) for (let x = n / 2 - 8; x < n / 2 + 8; x++, k++) for (let ch = 0; ch < 3; ch++) c[ch] += f[(y * n + x) * 3 + ch];
  const ours = c.map((v) => (v / k) * Math.PI); // irradiance = pi x radiance of a white Lambertian surface
  const e = (ELEVATION * Math.PI) / 180, sun: [number, number, number] = [Math.cos(e), Math.sin(e), 0];
  const three = [0, 0, 0];
  const NT = 128, NP = 256;
  for (let i = 0; i < NT; i++) {
    const th = ((i + 0.5) / NT) * (Math.PI / 2);
    for (let j = 0; j < NP; j++) {
      const ph = ((j + 0.5) / NP) * 2 * Math.PI;
      const l = preethamSky([Math.sin(th) * Math.cos(ph), Math.cos(th), Math.sin(th) * Math.sin(ph)], sun);
      const w = Math.cos(th) * Math.sin(th) * (Math.PI / 2 / NT) * ((2 * Math.PI) / NP);
      for (let ch = 0; ch < 3; ch++) three[ch] += l[ch] * w;
    }
  }
  const Y = (v: number[]) => 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
  console.log(JSON.stringify({ elevation: ELEVATION, native_irradiance_lux_per_lux: ours, three_sky_irradiance: three, SKY_NITS_PER_LUX: Y(ours) / Y(three) }));
}
if (process.argv.includes("--calibrate-sky")) {
  await calibrateSky();
  process.exit(0);
}

// ------------------------------------------------------------------------------------------------ report

const ALWAYS_DROPPED = [
  "global illumination (three.js lights are direct only; IBL is unoccluded)",
  "ray traced reflections / shadows (screen-space or planar reflections are not in WebGLRenderer)",
  "UE post chain except the vignette (bloom, local exposure, UE filmic curve / gamut / blue correction; three.js: vignette pass + ACES)",
];

function report() {
  const three = readJson(join(outThree, "three.json"), {});
  const ours = readJson(join(outOurs, "ours.json"), {});
  const ueTime = (m: string) => readJson(join(root, `out/ue/${m}/timings_${m}_all.json`), {});
  const ue = { lumen: ueTime("lumen"), pathtracer: ueTime("pathtracer") };
  const rows: Record<string, any>[] = [];
  const montages: Record<string, string> = {};
  for (const id of ids) {
    for (const v of VIEWS[id]) {
      const t = three[v.view], o = ours[v.view];
      if (!t?.ok) continue;
      const kb = v.view === "kabukicho-v1";
      if (id === "kabukicho" && !kb) continue; // the game camera has no ground truth: only v1 is benchmarked
      const row = (renderer: string, img: string, extra: Record<string, unknown> = {}) => {
        if (!existsSync(img)) return null;
        const r = { scene: v.view, renderer, image: img.replace(root + "/", ""), vs_pt: compare(v.refs.pt, img), vs_lumen: compare(v.refs.lumen, img), ...extra };
        rows.push(r);
        return r;
      };
      const threeExtra = { frame_ms: +t.pipelinedMs.toFixed(2), frame_ms_no_shadows: +t.pipelinedNoShadowMs.toFixed(2), sync_ms: t.syncMs, timer_query_ms: t.gpuMs === null ? null : +t.gpuMs.toFixed(2), draw_calls: t.drawCalls, dropped: t.dropped };
      const base = row("three.js (ACES)", join(outThree, `${v.view}.png`), threeExtra);
      const probe = row("three.js + scene probe (ACES)", join(outThree, `${v.view}-probe.png`), { ...threeExtra, setup_ms: Math.round(t.probeMs ?? 0) });
      row("three.js (AgX)", join(outThree, `${v.view}-agx.png`), { tonemap_variant: true });
      row("three.js (Neutral)", join(outThree, `${v.view}-neutral.png`), { tonemap_variant: true });
      row("three.js (ACES, no vignette)", join(outThree, `${v.view}-novignette.png`), { tonemap_variant: true });
      for (const k of liveFrames) {
        const l = o?.live?.[k];
        if (l) row(`ThreeRT live @${k}f`, join(outOurs, `${v.view}-live-${k}.png`), { frame_ms: l.gpu_ms, render_scale: l.scale, spp_per_frame: l.spp });
      }
      if (o?.offline) row(`ThreeRT offline (256 spp, ${o.offline.denoise})`, join(outOurs, `${v.view}-offline.png`), { time_ms: o.offline.ms, ev100: +o.offline.ev100.toFixed(3) });
      if (v.refs.lumen && v.refs.pt) row("UE Lumen", v.refs.lumen, { time_ms: (ue.lumen[id]?.render_s ?? NaN) * 1000 });
      // Montage: three.js (the better of its two variants vs the primary reference) | ThreeRT live @16 | offline | UE.
      const primary = (r: any) => (r ? (v.refs.pt ? r.vs_pt : r.vs_lumen)?.mean_delta_e ?? Infinity : Infinity);
      const best = primary(probe) < primary(base) ? probe : base;
      const liveImg = join(outOurs, `${v.view}-live-${liveFrames.includes(16) ? 16 : liveFrames[0]}.png`);
      const panels = [best && join(root, best.image), liveImg, join(outOurs, `${v.view}-offline.png`), v.refs.pt ?? v.refs.lumen].filter((p): p is string => !!p && existsSync(p));
      const mont = join(outBench, "three", `${v.view}.jpg`);
      execFileSync(bin, ["montage", mont, "--width", "640", ...panels]);
      montages[v.view] = `${mont.replace(root + "/", "")} (${best?.renderer} | live @16f | offline | ${v.refs.pt ? "UE Path Tracer" : "UE Lumen"})`;
    }
  }
  const any3 = Object.values(three).find((t: any) => t?.ok) as any;
  writeFileSync(join(outBench, "three.json"), JSON.stringify({ generated: new Date().toISOString(), three: "r187 WebGLRenderer (WebGL2)", webgl: any3?.webgl, always_dropped: ALWAYS_DROPPED, montages, rows }, null, 1));

  const f2 = (m: any) => (m ? `${m.mean_delta_e} / ${m.ssim} / ${m.psnr_db}` : "–");
  const f1 = (m: any) => (m ? `${m.mean_delta_e} / ${m.ssim}` : "–");
  const ms = (r: any) => (r.frame_ms !== undefined ? `${r.frame_ms}${r.render_scale !== undefined ? ` (scale ${r.render_scale})` : r.frame_ms_no_shadows !== undefined ? ` (${r.frame_ms_no_shadows} w/o shadows)` : ""}` : r.time_ms !== undefined && Number.isFinite(r.time_ms) ? `${(r.time_ms / 1000).toFixed(1)} s` : "–");
  const md: string[] = [
    `three.js r187 WebGLRenderer on ${any3?.webgl ?? "?"} (headless Chrome), ThreeRT native renderer on the same machine. Metrics: mean ΔE76 / SSIM / PSNR dB (1280x720; light-room 960x540).`,
    "Kabukichō: camera view v1 against the UE ground truth of the actual UE level (UE Lumen, auto exposure; three.js uses the EV100 ThreeRT settled on).",
    "",
    "| scene | renderer | vs UE Path Tracer (ΔE / SSIM / PSNR) | vs UE Lumen (ΔE / SSIM) | frame ms (or time) |",
    "|---|---|---|---|---:|",
    ...rows.filter((r) => !r.tonemap_variant).map((r) => `| ${r.scene} | ${r.renderer} | ${f2(r.vs_pt)} | ${f1(r.vs_lumen)} | ${ms(r)} |`),
    "",
    "three.js post variants (vs UE Path Tracer, or UE Lumen for Kabukichō; ΔE / SSIM):",
    "",
    "| scene | ACES | AgX | Neutral | ACES without the vignette |",
    "|---|---|---|---|---|",
    ...[...new Set(rows.map((r) => r.scene))].map((sc) => {
      const g = (name: string) => {
        const r = rows.find((x) => x.scene === sc && x.renderer === name);
        return r ? f1(r.vs_pt ?? r.vs_lumen) : "–";
      };
      return `| ${sc} | ${g("three.js (ACES)")} | ${g("three.js (AgX)")} | ${g("three.js (Neutral)")} | ${g("three.js (ACES, no vignette)")} |`;
    }),
    "",
    "Dropped by three.js (scene elements using a feature three.js cannot express), besides " + ALWAYS_DROPPED.join("; ") + ":",
    "",
    ...[...new Set(rows.map((r) => r.scene))].map((sc) => {
      const d = rows.find((x) => x.scene === sc && x.dropped)?.dropped ?? {};
      return `- ${sc}: ${Object.keys(d).length ? Object.entries(d).map(([k, n]) => `${k} ×${n}`).join(", ") : "nothing scene-specific"}`;
    }),
    "",
    "Montages (three.js | ThreeRT live @16 frames | ThreeRT offline | UE):",
    "",
    ...Object.values(montages).map((m) => `- ${m}`),
  ];
  writeFileSync(join(outBench, "three.md"), md.join("\n") + "\n");
  console.log(md.join("\n"));
}

if (steps.has("ours")) for (const id of ids) await runOurs(id);
if (steps.has("three")) await runThree(ids);
if (steps.has("report")) report();

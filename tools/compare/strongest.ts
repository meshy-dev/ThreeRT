// docs/COMPARE.md part 1: three.js pushed as far as it goes vs ThreeRT, on the same scene code, the same camera and
// the same exposure, both measured against UE5 (path tracer for the reference scenes, Lumen for Kabukichō).
//
//   node tools/compare/strongest.ts [--scenes a,b] [--steps ours,three,report]
//
// three.js, two levels: "tuned" = the WebGL baseline of tools/three (shadow maps, PMREM sky IBL, MSAA, ACES: what a
// competent three.js developer ships), "strongest" = WebGPURenderer + r187's SSGI + SSR + TRAA + bloom
// (tools/compare/strongest-page.ts). ThreeRT: the live viewport at 60 fps (dynamic resolution) and at a fixed full
// resolution, 64 frames after the camera stops, plus the offline render (256 spp) for the exposure three.js reuses.
// Frame cost: three.js = back-to-back frames / GPU sync at full 1280 x 720; ThreeRT = GPU ms reported by the viewport.
// Outputs: out/compare/strongest/*, out/compare/strongest.{json,md}, montage out/compare/strongest.jpg.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { bundleGame } from "../bundle-game.ts";
import { sources } from "../catalog.ts";
import { requireQuietGpu } from "../lib/bench.ts";
import { bundlePage, launch, root, serve } from "../three/browser.ts";
import { bin, compare, montage, outDir, readJson, runLive } from "./lib.ts";

/** tools/kabukicho/views.py: UE location (cm, Z up) + rotator (pitch, yaw) -> eye + look-at target (m, Y up). */
function ueView(loc: number[], rot: number[], fov: number) {
  const p = (rot[0] * Math.PI) / 180, y = (rot[1] * Math.PI) / 180;
  const fwd = [Math.cos(p) * Math.cos(y), Math.cos(p) * Math.sin(y), Math.sin(p)];
  const pos = [loc[0] / 100, loc[2] / 100, loc[1] / 100];
  return { position: pos, target: [pos[0] + fwd[0], pos[1] + fwd[2], pos[2] + fwd[1]], fov };
}
const pt = (id: string) => join(root, "out/ue/pathtracer", `${id}.png`);
const SCENES = [
  { id: "material-grid", view: "material-grid", title: "材质球 · 日光", ref: pt("material-grid"), refName: "UE 路径追踪" },
  { id: "sun-room", view: "sun-room", title: "室内 · 窗外日光", ref: pt("sun-room"), refName: "UE 路径追踪" },
  { id: "neon-reflections", view: "neon-reflections", title: "夜景 · 霓虹 + 反射", ref: pt("neon-reflections"), refName: "UE 路径追踪" },
  { id: "glb-asset", view: "glb-asset", title: "glTF 模型 · 日光", ref: pt("glb-asset"), refName: "UE 路径追踪" },
  // Bistro at night (ROADMAP § 0 milestone 1): its reference is the UE Path Tracer (node tools/ue-bistro.ts).
  { id: "bistro", view: "bistro", title: "Bistro · 夜", ref: pt("bistro"), refName: "UE 路径追踪" },
  // Zero Day, Measure One, static (ROADMAP § 0 step 6): its reference is the UE Path Tracer too.
  { id: "zero-day", view: "zero-day", title: "Zero Day · 静态", ref: pt("zero-day"), refName: "UE 路径追踪" },
  // Kabukichō: the game, at UE camera view v1, against UE Lumen on the actual UE level (auto exposure).
  { id: "kabukicho", view: "kabukicho-v1", title: "歌舞伎町 · 夜", ref: join(root, "out/ue-kb/lumen-auto/v1.png"), refName: "UE Lumen", camera: ueView([-1150, -180, 170], [4, 4, 0], 55) },
];
const arg = (k: string, d: string) => (process.argv.includes(k) ? process.argv[process.argv.indexOf(k) + 1] : d);
const ids = arg("--scenes", SCENES.map((s) => s.id).join(",")).split(",");
const steps = new Set(arg("--steps", "ours,three,report").split(","));
const todo = SCENES.filter((s) => ids.includes(s.id));
const dir = join(outDir, "strongest");
mkdirSync(dir, { recursive: true });
const resFile = join(outDir, "strongest.json");
const res: Record<string, any> = readJson(resFile, {});
const save = () => writeFileSync(resFile, JSON.stringify(res, null, 1));
const all = sources();
const FRAMES = 64;

if (steps.has("ours")) {
  requireQuietGpu();
  for (const s of todo) {
    const bundle = join(root, "out/bundles", `${s.id}.js`);
    await bundleGame(all[s.id], bundle);
    const offline = join(dir, `${s.view}-ours-offline.png`);
    const args = ["render", bundle, "-o", offline, "--assets", root, "--spp", "256"];
    if (s.camera) args.push("--camera", JSON.stringify(s.camera));
    const st = JSON.parse(execFileSync(bin, args, { maxBuffer: 1 << 26 }).toString().trim().split("\n").pop()!);
    const r = (res[s.view] ??= {});
    r.ours_offline = { png: offline, ev100: st.ev100, s: +(st.timings_ms.total_ms / 1000).toFixed(1), width: st.width, height: st.height };
    const o = { width: st.width, height: st.height, frames: [FRAMES], camera: s.camera };
    r.ours_live = (await runLive(bundle, join(dir, `${s.view}-ours-auto`), { ...o, scale: "auto" }))[FRAMES];
    r.ours_full = (await runLive(bundle, join(dir, `${s.view}-ours-full`), { ...o, scale: "1" }))[FRAMES];
    console.log(`ours ${s.view}: EV ${st.ev100.toFixed(2)}, live ${r.ours_live.gpu_ms} ms @ scale ${r.ours_live.scale}, full res ${r.ours_full.gpu_ms} ms`);
    save();
  }
}

if (steps.has("three")) {
  requireQuietGpu();
  const { server, origin } = await serve();
  const browser = await launch((l) => (/error|exception/i.test(l) && !/favicon/.test(l) ? console.log(l.slice(0, 300)) : undefined));
  try {
    for (const s of todo) {
      const r = (res[s.view] ??= {});
      const q = new URLSearchParams();
      if (r.ours_offline) q.set("ev100", String(r.ours_offline.ev100));
      if (s.camera) q.set("camera", JSON.stringify(s.camera));
      // Each three.js level runs with and without the scene probe (a cube capture of the lit scene at the camera as
      // the environment, which stops sky light flooding interiors); the report keeps the one closer to UE per scene,
      // as a developer would pick it.
      // Tuned WebGL baseline (tools/three/page.ts): ACES image, steady-state frame time; its probe variant is a still.
      const web = await bundlePage(s.id, all[s.id]);
      const a = await browser.run(`${origin}${web.html}?${q}`, 900_000);
      if (!a.ok) throw new Error(`three tuned ${s.view}: ${a.error}`);
      const tuned = { frame_ms: +a.pipelinedMs.toFixed(2), sync_ms: +a.syncMs.toFixed(2), draw_calls: a.drawCalls, dropped: a.dropped };
      r.three_tuned = {};
      for (const [k, img] of [["sky", a.images.aces], ["probe", a.images.probe]] as const) {
        const png = join(dir, `${s.view}-three-tuned-${k}.png`);
        writeFileSync(png, Buffer.from(String(img).split(",")[1], "base64"));
        r.three_tuned[k] = { ...tuned, png, variant: k };
      }
      // Strongest: WebGPU + SSGI + SSR + TRAA + bloom, without / with the probe.
      const gpu = await bundlePage(`strong-${s.id}`, all[s.id], join(root, "tools/compare/strongest-page.ts"));
      r.three_strongest = {};
      for (const k of ["sky", "probe"]) {
        const b = await browser.run(`${origin}${gpu.html}?${q}&probe=${k === "probe" ? 1 : 0}`, 900_000);
        if (!b.ok) throw new Error(`three strongest ${s.view}: ${b.error}`);
        const png = join(dir, `${s.view}-three-strongest-${k}.png`);
        writeFileSync(png, Buffer.from(String(b.images.still).split(",")[1], "base64"));
        r.three_strongest[k] = { png, variant: k, frame_ms: +b.pipelinedMs.toFixed(2), sync_ms: +b.syncMs.toFixed(2), chain: b.chain, draw_calls: b.drawCalls, dropped: b.dropped };
      }
      console.log(`three ${s.view}: tuned ${r.three_tuned.sky.frame_ms} ms, strongest ${r.three_strongest.sky.frame_ms} / ${r.three_strongest.probe.frame_ms} ms (${r.three_strongest.sky.chain})`);
      save();
    }
  } finally {
    browser.close();
    server.close();
  }
}

if (steps.has("report")) {
  const rows = todo.filter((s) => res[s.view]?.three_strongest && res[s.view]?.ours_live).map((s) => {
    const r = res[s.view];
    const m = (x: any) => ({ ...x, vs_ue: compare(s.ref, x.png) });
    // The better of the sky / probe variants (lower ΔE against the reference).
    const best = (v: Record<string, any>) => Object.values(v).map(m).sort((p, q) => p.vs_ue.mean_delta_e - q.vs_ue.mean_delta_e)[0];
    return { ...s, three_tuned: best(r.three_tuned), three_strongest: best(r.three_strongest), ours_live: m(r.ours_live), ours_full: m(r.ours_full), ours_offline: m(r.ours_offline) };
  });
  writeFileSync(join(outDir, "strongest-report.json"), JSON.stringify({ generated: new Date().toISOString(), rows }, null, 1));
  const de = (x: any) => (x?.vs_ue ? x.vs_ue.mean_delta_e.toFixed(1) : "–");
  const fps = (ms: number) => (ms > 0 ? Math.min(999, Math.round(1000 / ms)) : 0);
  const dropped = (x: any) => Object.values((x?.dropped ?? {}) as Record<string, number>).reduce((a, b) => a + b, 0);
  const md = [
    "Mean ΔE76 vs UE (path tracer; Lumen for Kabukichō) · frame cost at 1280 x 720 (ThreeRT at 60 fps: GPU ms at its dynamic render scale). three.js: the better of its sky / scene-probe environment per scene.",
    "",
    "| scene | three.js tuned (WebGL) | three.js strongest (WebGPU + SSGI + SSR + TRAA) | ThreeRT live, 60 fps | ThreeRT live, full res | ThreeRT offline 256 spp |",
    "|---|---|---|---|---|---|",
    ...rows.map((r) => `| ${r.view} | ΔE ${de(r.three_tuned)} · ${r.three_tuned.frame_ms} ms (${r.three_tuned.variant}) | ΔE ${de(r.three_strongest)} · ${r.three_strongest.frame_ms} ms (${r.three_strongest.variant}) | ΔE ${de(r.ours_live)} · ${r.ours_live.gpu_ms} ms @ scale ${r.ours_live.scale} | ΔE ${de(r.ours_full)} · ${r.ours_full.gpu_ms} ms | ΔE ${de(r.ours_offline)} · ${r.ours_offline.s} s |`),
    "",
    ...rows.filter((r) => dropped(r.three_strongest)).map((r) => `- ${r.view}: three.js cannot express ${Object.entries(r.three_strongest.dropped).map(([k, n]) => `${k} ×${n}`).join(", ")}`),
  ];
  writeFileSync(join(outDir, "strongest.md"), md.join("\n") + "\n");
  console.log(md.join("\n"));
  const { server, origin } = await serve();
  const browser = await launch();
  try {
    const spec = {
      title: "Part 1 · three.js 拉满 vs ThreeRT：同一份场景代码、同一相机、同一曝光",
      subtitle: "ΔE = 与 UE 参考的平均色差（越小越接近）；ms = 每帧 GPU 时间，1280×720。ThreeRT 实时视口锁 60 fps，自动调渲染比例；相机停下 64 帧后截图",
      columns: ["three.js 常规调优（WebGL）", "three.js 最强（WebGPU+SSGI+SSR+TRAA）", "ThreeRT 实时（60 fps）", "UE5 参考答案"],
      tileWidth: 480,
      rows: rows.map((r) => ({
        label: `${r.title}\n${r.view}`,
        cells: [
          { img: r.three_tuned.png, caption: [`ΔE ${de(r.three_tuned)} · ${r.three_tuned.frame_ms} ms（${fps(r.three_tuned.frame_ms)} fps）`, `阴影贴图 + ${r.three_tuned.variant === "probe" ? "场景探针" : "天空"} IBL + MSAA + ACES`] },
          { img: r.three_strongest.png, caption: [`ΔE ${de(r.three_strongest)} · ${r.three_strongest.frame_ms} ms（${fps(r.three_strongest.frame_ms)} fps）`, dropped(r.three_strongest) > 3 ? `丢失：材质函数 ×${r.three_strongest.dropped["material functions (WGSL; the standard library: wetness, puddles, grime, windows, world mapping, animated emission)"] ?? 0}、发光面照明 ×${r.three_strongest.dropped["emissive surfaces as light sources (three.js emission lights nothing)"] ?? 0} 等` : `SSGI + SSR + TRAA + 泛光${r.three_strongest.variant === "probe" ? " + 场景探针" : ""}`] },
          { img: r.ours_live.png, caption: [`ΔE ${de(r.ours_live)} · ${r.ours_live.gpu_ms} ms（60 fps，渲染比例 ${r.ours_live.scale}）`, `全分辨率 ${r.ours_full.gpu_ms} ms · ΔE ${de(r.ours_full)}`] },
          { img: existsSync(r.ref) ? r.ref : null, caption: ["参考答案", r.refName] },
        ],
      })),
      footer: ["同一台 Mac（Apple GPU）；three.js 在 headless Chrome 里跑（WebGL2 / WebGPU，均走 Metal），ThreeRT 原生 wgpu。three.js 用 ThreeRT 离线渲染定下的曝光，ThreeRT 实时用自身自动曝光的结果。"],
    };
    await montage(browser, origin, join(outDir, "strongest.jpg"), spec);
    // The copy docs/COMPARE.md shows.
    await montage(browser, origin, join(root, "docs/images/compare-strongest.jpg"), { ...spec, tileWidth: 400, quality: 0.8 });
  } finally {
    browser.close();
    server.close();
  }
  console.log(`montage: ${join(outDir, "strongest.jpg")}`);
}

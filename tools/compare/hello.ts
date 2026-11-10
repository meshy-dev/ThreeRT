// docs/COMPARE.md part 2: what you get out of the box. The same scene written as a hello world twice, in plain
// three.js (compare/hello/three/<id>.js) and in the ThreeRT DSL (compare/hello/threert/<id>.ts): same geometry,
// materials, light placement and camera, only business code, every renderer setting left at its default.
//
//   node tools/compare/hello.ts [--scenes a,b] [--steps three,ours,report]
//
// three.js renders twice: as written (WebGLRenderer defaults) and with the usual tutorial additions
// (compare/hello/three/polish.js: ACES, shadow maps, RoomEnvironment). ThreeRT renders in its live viewport (60 fps
// pacing, dynamic resolution, auto exposure), 64 frames after the camera stops. UE5's path tracer is the reference.
// Outputs: out/compare/hello/*, out/compare/hello.{json,md}, montage out/compare/hello.jpg.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { bundleGame } from "../bundle-game.ts";
import { requireQuietGpu } from "../lib/bench.ts";
import { bundlePage, launch, root, serve } from "../three/browser.ts";
import { alignExposure, codeLines, compare, type LiveShot, montage, outDir, readJson, runLive } from "./lib.ts";

const SCENES = [
  // exposure: three.js polish tier's toneMappingExposure, picked by eye so the brightness is close to the reference.
  { id: "material-grid", title: "材质球 · 日光", environment: true, exposure: 0.15 },
  { id: "sun-room", title: "室内 · 窗外日光", environment: true, exposure: 0.27 },
  // extraLine: the one line of intent a night scene needs in ThreeRT (its default auto exposure brightens the night, like
  // UE's); rendered as a variant next to the all-defaults image.
  { id: "neon-reflections", title: "夜景 · 霓虹 + 反射", environment: false, exposure: 1.6, extraLine: "camera.setManualExposure(6);" },
  { id: "glb-asset", title: "glTF 模型 · 日光", environment: true, exposure: 0.12 },
];
const arg = (k: string, d: string) => (process.argv.includes(k) ? process.argv[process.argv.indexOf(k) + 1] : d);
const ids = arg("--scenes", SCENES.map((s) => s.id).join(",")).split(",");
const steps = new Set(arg("--steps", "three,ours,report").split(","));
const dir = join(outDir, "hello");
mkdirSync(dir, { recursive: true });
const resFile = join(outDir, "hello.json");
const res: Record<string, any> = readJson(resFile, {});
const save = () => writeFileSync(resFile, JSON.stringify(res, null, 1));
const threeSrc = (id: string) => join(root, "compare/hello/three", `${id}.js`);
const oursSrc = (id: string) => join(root, "compare/hello/threert", `${id}.ts`);
const ueRef = (id: string) => join(root, "out/ue/pathtracer", `${id}.png`);
const LIVE_FRAMES = 64;

if (steps.has("three")) {
  const { server, origin } = await serve();
  const browser = await launch((l) => (/error|exception/i.test(l) && !/favicon/.test(l) ? console.log(l.slice(0, 300)) : undefined));
  try {
    for (const s of SCENES.filter((s) => ids.includes(s.id))) {
      const { html } = await bundlePage(`hello-${s.id}`, threeSrc(s.id), join(root, "tools/compare/hello-page.ts"));
      for (const tier of ["default", "polish"]) {
        const r = await browser.run(`${origin}${html}?tier=${tier}&exposure=${s.exposure}${s.environment ? "" : "&env=0"}`, 120_000);
        if (!r.ok) throw new Error(`three ${s.id} ${tier}: ${r.error}`);
        const png = join(dir, `${s.id}-three-${tier}.png`);
        writeFileSync(png, Buffer.from(String(r.image).split(",")[1], "base64"));
        (res[s.id] ??= {})[`three_${tier}`] = { png, frame_ms: +r.frameMs.toFixed(2) };
        console.log(`three ${s.id} ${tier}: ${r.frameMs.toFixed(2)} ms`);
      }
      save();
    }
  } finally {
    browser.close();
    server.close();
  }
}

if (steps.has("ours")) {
  requireQuietGpu();
  for (const s of SCENES.filter((s) => ids.includes(s.id))) {
    const bundle = join(dir, `${s.id}.bundle.js`);
    await bundleGame(oursSrc(s.id), bundle);
    const shots: Record<number, LiveShot> = await runLive(bundle, join(dir, `${s.id}-ours`), { width: 1280, height: 720, scale: "auto", frames: [LIVE_FRAMES] });
    (res[s.id] ??= {}).ours = shots[LIVE_FRAMES];
    console.log(`ours ${s.id}: GPU ${shots[LIVE_FRAMES].gpu_ms} ms, scale ${shots[LIVE_FRAMES].scale}`);
    if ("extraLine" in s) {
      const src = join(dir, `${s.id}.extra-line.ts`);
      writeFileSync(src, readFileSync(oursSrc(s.id), "utf8").replace("  scene.add(camera);", `  ${s.extraLine}\n  scene.add(camera);`));
      await bundleGame(src, bundle);
      res[s.id].ours_extra = (await runLive(bundle, join(dir, `${s.id}-ours-extra`), { width: 1280, height: 720, scale: "auto", frames: [LIVE_FRAMES] }))[LIVE_FRAMES];
    }
    save();
  }
}

if (steps.has("report")) {
  const polishLines = codeLines(join(root, "compare/hello/three/polish.js"));
  const rows: Record<string, any>[] = [];
  const { server, origin } = await serve();
  const browser = await launch();
  try {
    for (const s of SCENES.filter((s) => ids.includes(s.id) && res[s.id])) {
      const r = res[s.id];
      // three.js: the file plus the 4 lines every page needs (renderer, size, canvas, render call).
      const threeLines = codeLines(threeSrc(s.id)) + 4, oursLines = codeLines(oursSrc(s.id));
      // ΔE as rendered, and with the overall brightness matched to UE (exposure is each engine's own default here:
      // three.js has none, ThreeRT auto-exposes, the UE reference used the scene's manual exposure).
      const m = async (img: string) => {
        const aligned = img.replace(/\.png$/, "-aligned.png");
        const ev = await alignExposure(browser, origin, ueRef(s.id), img, aligned);
        return { png: img, vs_ue_pt: compare(ueRef(s.id), img), vs_ue_pt_aligned: compare(ueRef(s.id), aligned), exposure_offset_ev: +ev.toFixed(2) };
      };
      rows.push({
        ...s,
        three_lines: threeLines,
        three_polish_lines: threeLines + polishLines,
        ours_lines: oursLines,
        three_default: { ...r.three_default, ...(await m(r.three_default.png)) },
        three_polish: { ...r.three_polish, ...(await m(r.three_polish.png)) },
        ours: { ...r.ours, ...(await m(r.ours.png)) },
        ours_extra: r.ours_extra ? { ...r.ours_extra, ...(await m(r.ours_extra.png)) } : null,
      });
    }
    writeFileSync(join(outDir, "hello-report.json"), JSON.stringify({ generated: new Date().toISOString(), polish_lines: polishLines, rows }, null, 1));
    const de = (x: any) => `${x.vs_ue_pt_aligned.mean_delta_e.toFixed(1)}`;
    const raw = (x: any) => `${x.vs_ue_pt.mean_delta_e.toFixed(1)}`;
    const md = [
      "Mean ΔE76 vs the UE 5.8 path tracer, brightness-aligned (as rendered in brackets); lines of code.",
      "",
      "| scene | three.js as written | three.js + tutorial polish | ThreeRT defaults (60 fps live) |",
      "|---|---|---|---|",
      ...rows.map((r) => `| ${r.id} | ${r.three_lines} lines · ΔE ${de(r.three_default)} (${raw(r.three_default)}) | ${r.three_polish_lines} lines · ΔE ${de(r.three_polish)} (${raw(r.three_polish)}) | ${r.ours_lines} lines · ΔE ${de(r.ours)} (${raw(r.ours)}) · GPU ${r.ours.gpu_ms} ms at scale ${r.ours.scale}${r.ours_extra ? `; + \`${r.extraLine}\`: ΔE ${de(r.ours_extra)} (${raw(r.ours_extra)})` : ""} |`),
    ];
    writeFileSync(join(outDir, "hello.md"), md.join("\n") + "\n");
    console.log(md.join("\n"));

    const ev = (x: any) => (Math.abs(x.exposure_offset_ev) < 0.3 ? "亮度接近参考" : `比参考${x.exposure_offset_ev > 0 ? "暗" : "亮"} ${Math.abs(x.exposure_offset_ev).toFixed(1)} 档`);
    const spec = {
      title: "Part 2 · Hello World 开箱画面：同一场景，只写业务代码，渲染设置全用默认",
      subtitle: "左两列 three.js r187（WebGLRenderer）；第三列 ThreeRT（实时视口 60 fps，相机停下 64 帧后）；右列 UE5 路径追踪作参考答案。ΔE = 亮度对齐后与参考的平均色差，越小越像",
      columns: ["three.js · 照写", `three.js · 再加教程常用 ${polishLines} 行`, "ThreeRT · 全默认", "UE5 路径追踪（参考答案）"],
      tileWidth: 480,
      rows: rows.map((r) => ({
        label: `${r.title}\n${r.id}`,
        cells: [
          { img: r.three_default.png, caption: [`${r.three_lines} 行 · ΔE ${de(r.three_default)}`, `无阴影 / 无色调映射 / 无环境光照 · ${ev(r.three_default)}`] },
          { img: r.three_polish.png, caption: [`${r.three_polish_lines} 行 · ΔE ${de(r.three_polish)}`, `${r.environment ? "ACES + 阴影 + RoomEnvironment" : "ACES + 阴影"} + 手调曝光 ${r.exposure} · ${ev(r.three_polish)}`] },
          { img: r.ours.png, caption: [`${r.ours_lines} 行 · ΔE ${de(r.ours)}`, r.ours_extra ? `${ev(r.ours)}；加 1 行夜景曝光后 ΔE ${de(r.ours_extra)}` : `60 fps（GPU ${r.ours.gpu_ms} ms，渲染比例 ${r.ours.scale}）· ${ev(r.ours)}`] },
          { img: ueRef(r.id), caption: ["参考答案", "UE 5.8 Path Tracer（离线，场景原手动曝光）"] },
        ],
      })),
      footer: [
        "行数 = 非空非注释行；three.js 含建渲染器 / 设尺寸 / 挂画布 / render 4 行。光强按各自引擎的常规写法：three.js 用其官方示例的典型值（太阳 3、半球光 3），ThreeRT 用物理单位（勒克斯 / 尼特）。",
        "曝光：three.js 没有曝光概念（亮度取决于手调光强）；ThreeRT 默认自动曝光（夜景会像手机相机一样被提亮）；UE 参考用场景原来的手动曝光。ΔE 先把整体亮度对齐再算，只比光照和材质。",
      ],
    };
    await montage(browser, origin, join(outDir, "hello.jpg"), spec);
    // The copy docs/COMPARE.md shows.
    await montage(browser, origin, join(root, "docs/images/compare-hello.jpg"), { ...spec, tileWidth: 400, quality: 0.8 });
  } finally {
    browser.close();
    server.close();
  }
  console.log(`montage: ${join(outDir, "hello.jpg")}`);
}

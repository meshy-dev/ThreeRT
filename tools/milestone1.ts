// Milestone 1 (docs/ROADMAP.md "How we are judged", § 0): ThreeRT live vs three.js (strongest / tuned) vs our offline
// render, for Bistro night and Zero Day, in one command.
//
//   node tools/milestone1.ts [--scenes bistro,zero-day] [--reuse] [--ref-spp n] [--clip-seconds 6] [--clip-fps 10]
//
// The reference is our unbiased offline render: no clamps (clampIndirect off), no denoiser, --ref-spp samples (default:
// the scene's, Bistro 4096, Zero Day 16384). It is cached by the bundle and the committed renderer tree (git rev-parse
// HEAD:renderer), so it is rendered again only when either changes. A second render at a quarter of the samples, same settings, shows the reference's own noise (its
// ΔE to the reference is about twice the reference's noise floor).
// Ours, measured now unless --reuse finds them in out/compare/strongest.json (tools/compare/strongest.ts): ThreeRT live
// at 60 fps pacing (dynamic resolution) and at full resolution, three.js tuned (WebGL) and strongest (WebGPU), the
// variant (sky / scene probe) closer to the reference.
// Outputs per scene in out/milestone1/<scene>/: montage.jpg (our offline render | ThreeRT live | three.js strongest |
// three.js tuned, each labelled with its frame rate and ΔE / SSIM), move.webp (animated WebP: ThreeRT live | three.js
// along the scene's FBX camera move, each with the frame time measured along it, <= 5 MB), metrics.json, section.md. The
// section goes into docs/COMPARE.md between the scene's milestone-1 markers, the montage into
// docs/images/milestone1-<scene>.jpg.
// With more than one scene, out/milestone1/montage.jpg holds them all, one row each.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { BISTRO, EV100 as BISTRO_EV } from "../scenes/bistro.ts";
import { ZERO_DAY, EV100 as ZERO_DAY_EV } from "../scenes/zero-day.ts";
import { bundleGame } from "./bundle-game.ts";
import { compare, montage, readJson } from "./compare/lib.ts";
import { frameStats, requireQuietGpu, rendererBin } from "./lib/bench.ts";
import { gltfCameraPath } from "./lib/gltf-camera.ts";
import { startLive } from "./lib/live.ts";
import { bundlePage, launch, root, serve } from "./three/browser.ts";

type Box = [number, number, number, number];
interface SceneDef {
  title: string;
  label: string;
  file: string;
  gltf: string;
  camera: string;
  ev: number;
  /** Regions of the view (fractions of the frame: x0, y0, x1, y1). */
  boxes: Record<string, Box>;
  /** The reference's samples per pixel when --ref-spp is not given. */
  refSpp?: number;
}
const SCENES: Record<string, SceneDef> = {
  bistro: {
    title: "Bistro night", label: "Bistro night\nstreet", file: "scenes/bistro.ts", gltf: BISTRO, camera: "Camera", ev: BISTRO_EV,
    boxes: { "left facade": [0, 0.083, 0.258, 0.556], "shop and lamp": [0.258, 0.167, 0.508, 0.611], street: [0.156, 0.667, 0.859, 1], square: [0.617, 0.056, 0.898, 0.458], right: [0.859, 0.417, 1, 0.972] },
  },
  "zero-day": {
    // Glossy machine tops reflecting ~10 k small emitters: at 4096 spp its 1024 spp render still read ΔE 3.3 against
    // it (Bistro 1.8), as much as ThreeRT live's distance; 16 k quarters the reference's own noise.
    title: "Zero Day", label: "Zero Day\nMeasure One", file: "scenes/zero-day.ts", gltf: ZERO_DAY, camera: "DynamicCamera2", ev: ZERO_DAY_EV, refSpp: 16384,
    boxes: { ceiling: [0.25, 0, 0.75, 0.17], towers: [0.27, 0.17, 0.7, 0.55], "left wall": [0, 0.1, 0.22, 0.75], "right mirror": [0.78, 0, 0.95, 0.9], floor: [0.2, 0.75, 0.75, 1] },
  },
};

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const reuse = args.includes("--reuse");
const ids = opt("--scenes", Object.keys(SCENES).join(",")).split(",");
for (const id of ids) if (!SCENES[id]) throw new Error(`unknown scene ${id} (${Object.keys(SCENES).join(", ")})`);
const sppOf = (s: SceneDef) => Number(opt("--ref-spp", String(s.refSpp ?? 4096)));
const clipSeconds = Number(opt("--clip-seconds", "6"));
const clipFps = Number(opt("--clip-fps", "10"));
const python = join(root, "assets/.cache/orca-venv/bin/python");
if (!existsSync(python)) throw new Error(`${python} missing: bun scripts/orca.ts makes it (Pillow, numpy)`);
const py = (dir: string, job: object) => {
  const f = join(dir, "job.json");
  writeFileSync(f, JSON.stringify(job));
  return execFileSync(python, ["-I", join(root, "tools/milestone1/compose.py"), f], { encoding: "utf8" });
};
const fps = (ms: number | undefined) => (ms && ms > 0 ? Math.min(60, Math.round(1000 / ms)) : 0);
const UNBIASED = ["--no-denoise", "--override", JSON.stringify({ native: { clampIndirect: 1e9 } })];

// 1. ThreeRT live / full, three.js tuned / strongest, for every scene not already measured.
const strongestFile = join(root, "out/compare/strongest.json");
const missing = ids.filter((id) => !reuse || !readJson(strongestFile, {})[id]?.three_strongest);
if (missing.length) {
  requireQuietGpu();
  execFileSync("node", [join(root, "tools/compare/strongest.ts"), "--scenes", missing.join(","), "--steps", "ours,three"], { stdio: "inherit" });
}
const measured = readJson(strongestFile, {});

// 2. The references: our unbiased offline render, cached by bundle + renderer tree.
const rendererTree = execFileSync("git", ["rev-parse", "HEAD:renderer"], { cwd: root, encoding: "utf8" }).trim();
const offline = async (id: string) => {
  const s = SCENES[id];
  const dir = join(root, "out/milestone1", id);
  mkdirSync(dir, { recursive: true });
  const bundle = join(root, "out/bundles", `${id}.js`);
  await bundleGame(join(root, s.file), bundle);
  const key = createHash("sha256").update(readFileSync(bundle)).update(rendererTree).digest("hex").slice(0, 12);
  const render = (spp: number) => {
    const png = join(dir, `offline-unbiased-${spp}-${key}.png`), stats = png.replace(/\.png$/, ".json");
    if (!existsSync(png)) {
      requireQuietGpu();
      execFileSync(rendererBin, ["render", bundle, "-o", png, "--assets", root, "--spp", String(spp), "--stats", stats, ...UNBIASED], { stdio: ["ignore", "ignore", "inherit"] });
    }
    const st = readJson(stats, null) as { timings_ms?: { total_ms?: number } } | null;
    return { png, s: st?.timings_ms?.total_ms ? +(st.timings_ms.total_ms / 1000).toFixed(0) : null };
  };
  const spp = sppOf(s);
  return { dir, bundle, key, spp, ref: render(spp), noise: render(Math.max(1, spp / 4)) };
};

// 3. Per scene: metrics, the montage row, the clip.
const { server, origin } = await serve();
const browser = await launch();
const rows: Parameters<typeof montage>[3]["rows"] = [];
const columns = ["Our offline render (reference)", "ThreeRT live", "three.js strongest (WebGPU)", "three.js tuned (WebGL)"];
const subtitle = (spp: string) => `ΔE76 / SSIM against our unbiased offline render (no clamps, no denoiser, ${spp} spp); frame rates on one Mac (Apple M5), nothing else on the GPU`;
const sppUsed: number[] = [];
try {
  for (const id of ids) {
    const s = SCENES[id];
    const r = measured[id];
    if (!r?.ours_live || !r?.three_strongest) throw new Error(`${id}: no measurements in ${strongestFile}`);
    const { dir, bundle, ref, noise, spp: refSpp } = await offline(id);
    sppUsed.push(refSpp);
    const best = (v: Record<string, { png: string; frame_ms: number; variant: string }>) => Object.values(v).map((x) => ({ ...x, m: compare(ref.png, x.png) })).sort((a, b) => (a.m?.mean_delta_e ?? 1e9) - (b.m?.mean_delta_e ?? 1e9))[0];
    const tuned = best(r.three_tuned), strongest = best(r.three_strongest);
    const noiseName = `ThreeRT offline ${refSpp / 4} spp (reference noise)`;
    const images: Record<string, string> = { "ThreeRT live": r.ours_live.png, "ThreeRT full resolution": r.ours_full.png, "three.js strongest": strongest.png, "three.js tuned": tuned.png, [noiseName]: noise.png };
    py(dir, { regions: { ref: ref.png, images, boxes: s.boxes, out: join(dir, "regions.json") } });
    const regions = JSON.parse(readFileSync(join(dir, "regions.json"), "utf8"));
    const overall = Object.fromEntries(Object.entries(images).map(([k, p]) => [k, compare(ref.png, p)]));
    const de = (k: string) => (overall[k] ? overall[k]!.mean_delta_e.toFixed(1) : "–");
    const ss = (k: string) => (overall[k] ? overall[k]!.ssim.toFixed(3) : "–");
    const timing: Record<string, string> = {
      "ThreeRT live": `${r.ours_live.gpu_ms} ms GPU at scale ${r.ours_live.scale} (60 fps pacing)`,
      "ThreeRT full resolution": `${r.ours_full.gpu_ms} ms GPU (${fps(r.ours_full.gpu_ms)} fps)`,
      "three.js strongest": `${strongest.frame_ms} ms (${fps(strongest.frame_ms)} fps), ${strongest.variant}`,
      "three.js tuned": `${tuned.frame_ms} ms (${fps(tuned.frame_ms)} fps), ${tuned.variant}`,
      [noiseName]: `${noise.s ?? "?"} s`,
    };
    // The clip: the scene's FBX camera move from 0 s. ThreeRT live in lockstep at 60 fps (the clip's frames as
    // snapshots, GPU time from the frame log); three.js tuned per pose (its frame time measured at each pose).
    const path = gltfCameraPath(join(root, s.gltf), s.camera);
    const n = Math.round(clipSeconds * clipFps);
    const poses = Array.from({ length: n }, (_, k) => path.at(k / clipFps));
    const fresh = (name: string) => {
      const d = join(dir, name);
      rmSync(d, { recursive: true, force: true });
      mkdirSync(d, { recursive: true });
      return d;
    };
    const ours = fresh("move-threert");
    const frameLog = join(ours, "frames.jsonl");
    requireQuietGpu();
    const b = { bin: rendererBin, assets: root, presets: join(root, "presets"), bundles: join(root, "out/bundles"), label: "milestone1" };
    const live = startLive(b, bundle, { scale: "auto", frameLog });
    await live.firstFrame;
    live.send({ cmd: "camera", ...poses[0] });
    await new Promise((ok) => setTimeout(ok, 4000));
    live.send({ cmd: "lockstep", dt: 1 / 60 });
    const t0 = performance.now();
    for (const [k, p] of poses.entries()) {
      const f = join(ours, `${String(k).padStart(4, "0")}.png`);
      live.send({ cmd: "camera", ...p, frames: Math.round(60 / clipFps), snapshot: f });
      await live.wait((l) => l.includes(f), 60_000);
    }
    const t1 = performance.now();
    await live.quit();
    // Frame-log times start at the first frame; keep the frames of the move (the last (t1 - t0) ms of the log).
    type F = { t: number; gpu: number };
    const logged = existsSync(frameLog) ? readFileSync(frameLog, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l) as F) : [];
    const moveFrames = logged.filter((f) => f.t >= (logged.at(-1)?.t ?? 0) - (t1 - t0));
    const oursGpu = moveFrames.length ? frameStats(moveFrames.map((f) => f.gpu)) : null;
    const three = fresh("move-three");
    const page = await bundlePage(`${id}-move`, join(root, s.file));
    const pq = new URLSearchParams({ probe: "0", frames: "4", path: JSON.stringify(poses.map((p) => ({ position: p.pos, target: p.target, fov: (2 * Math.atan(p.tan) * 180) / Math.PI }))) });
    const res = await browser.run(`${origin}${page.html}?${pq}`, 1_800_000);
    if (!res.ok) throw new Error(`three.js move (${id}): ${res.error}`);
    (JSON.parse(res.images.path) as string[]).forEach((u, k) => writeFileSync(join(three, `${String(k).padStart(4, "0")}.jpg`), Buffer.from(u.split(",")[1], "base64")));
    const threeMs = (res.pathMs as number[] | undefined)?.length ? frameStats(res.pathMs) : null;
    const frames = (d: string) => readdirSync(d).filter((f) => /\.(png|jpg)$/.test(f)).sort().map((f) => join(d, f));
    const gif = py(dir, {
      gif: {
        out: join(dir, "move.webp"), max_bytes: 5_000_000, width: 1280, fps: clipFps,
        tiles: [
          { label: oursGpu ? `ThreeRT live · ${fps(oursGpu.p50)} fps · GPU p50 ${oursGpu.p50.toFixed(1)} / p95 ${oursGpu.p95.toFixed(1)} ms` : `ThreeRT live · ${fps(r.ours_live.gpu_ms)} fps (GPU ${r.ours_live.gpu_ms} ms)`, frames: frames(ours) },
          { label: threeMs ? `three.js WebGL tuned · ${fps(threeMs.p50)} fps · p50 ${threeMs.p50.toFixed(1)} / p95 ${threeMs.p95.toFixed(1)} ms` : `three.js WebGL tuned · ${fps(tuned.frame_ms)} fps`, frames: frames(three) },
        ],
      },
    });
    console.log(`${id} move.webp: ${gif.trim()}`);
    const move = {
      seconds: clipSeconds, poses: n,
      threert_gpu_ms: oursGpu ? { p50: +oursGpu.p50.toFixed(2), p95: +oursGpu.p95.toFixed(2), frames: moveFrames.length } : null,
      three_tuned_ms: threeMs ? { p50: +threeMs.p50.toFixed(2), p95: +threeMs.p95.toFixed(2) } : null,
    };

    // The montage: each tile's frame rate at the still pose, and along the camera move where it was measured.
    const moveFps = (st: { p50: number } | null) => (st ? ` · camera move: ${fps(st.p50)} fps (p50 ${st.p50.toFixed(1)} ms)` : "");
    const tile = (k: string, ms: number) => [`${fps(ms)} fps · ΔE ${de(k)} · SSIM ${ss(k)}`];
    const row = {
      label: s.label,
      cells: [
        { img: ref.png, caption: [`reference · ${refSpp} spp · ${ref.s ?? "?"} s`, `no clamps, no denoiser · ${refSpp / 4} spp vs it: ΔE ${de(noiseName)}`] },
        { img: r.ours_live.png, caption: [...tile("ThreeRT live", r.ours_live.gpu_ms), `GPU ${r.ours_live.gpu_ms} ms at scale ${r.ours_live.scale}${moveFps(oursGpu)}`] },
        { img: strongest.png, caption: [...tile("three.js strongest", strongest.frame_ms), `${strongest.frame_ms} ms · SSGI + SSR + TRAA (${strongest.variant})`] },
        { img: tuned.png, caption: [...tile("three.js tuned", tuned.frame_ms), `${tuned.frame_ms} ms · 16 automatic lights${moveFps(threeMs)}`] },
      ],
    };
    rows.push(row);
    await montage(browser, origin, join(dir, "montage.jpg"), {
      title: `${s.title} · ThreeRT vs three.js vs our offline render (same camera, 1280 × 720, EV100 ${s.ev})`,
      subtitle: subtitle(String(refSpp)), columns, tileWidth: 480, rows: [row],
    });

    // The table and the COMPARE.md section.
    const reg = Object.keys(s.boxes);
    const tableRows = Object.keys(images).map((k) => `| ${k} | ${timing[k] ?? ""} | ${de(k)} | ${ss(k)} | ${reg.map((g) => regions[k]?.[g]?.delta_e ?? "–").join(" | ")} | ${regions[k]?.all?.luminance_ratio ?? "–"} |`);
    const section = [
      `### Milestone 1: ${s.title} (${new Date().toISOString().slice(0, 10)})`,
      "",
      `Same Mac (Apple M5), the scene's FBX camera (first pose), 1280 × 720, EV100 ${s.ev}, nothing else on the GPU. ΔE76, SSIM and per-region ΔE against our unbiased offline render (no clamps, no denoiser, ${refSpp} spp, ${ref.s ?? "?"} s); the ${refSpp / 4} spp row shows the reference's own noise.`,
      "",
      `| renderer | frame cost | ΔE | SSIM | ${reg.join(" | ")} | luminance vs reference |`,
      `|---|---|---:|---:|${reg.map(() => "---:").join("|")}|---:|`,
      ...tableRows,
      "",
      `Along the camera move (${clipSeconds} s from 0 s, ${n} poses): ThreeRT live GPU p50 / p95 ${move.threert_gpu_ms ? `${move.threert_gpu_ms.p50} / ${move.threert_gpu_ms.p95} ms (${fps(move.threert_gpu_ms.p50)} fps)` : "–"} (60 fps pacing, dynamic resolution); three.js WebGL tuned ${move.three_tuned_ms ? `${move.three_tuned_ms.p50} / ${move.three_tuned_ms.p95} ms (${fps(move.three_tuned_ms.p50)} fps)` : "–"}.`,
      "",
      `![${s.title} montage](images/milestone1-${id}.jpg) · the motion clip (≤ 5 MB, not committed): \`${relative(root, join(dir, "move.webp"))}\``,
    ].join("\n");
    writeFileSync(join(dir, "section.md"), section + "\n");
    writeFileSync(join(dir, "metrics.json"), JSON.stringify({ at: new Date().toISOString(), reference: { png: relative(root, ref.png), spp: refSpp, s: ref.s, renderer_tree: rendererTree }, timing, overall, regions, move }, null, 1) + "\n");
    copyFileSync(join(dir, "montage.jpg"), join(root, "docs/images", `milestone1-${id}.jpg`));
    const compareMd = join(root, "docs/COMPARE.md");
    const md = readFileSync(compareMd, "utf8");
    const [a, z] = [`<!-- milestone1:${id} -->`, `<!-- /milestone1:${id} -->`];
    const block = `${a}\n${section}\n${z}`;
    writeFileSync(compareMd, md.includes(a) ? md.replace(new RegExp(`${a}[\\s\\S]*?${z}`), block) : md.replace(/\n## Rules of the comparison/, `\n${block}\n\n## Rules of the comparison`));
    console.log(`${id}: montage ${join(dir, "montage.jpg")}, clip ${join(dir, "move.webp")}, section in docs/COMPARE.md`);
  }
  if (rows.length > 1) {
    await montage(browser, origin, join(root, "out/milestone1/montage.jpg"), {
      title: "Milestone 1 · ThreeRT vs three.js vs our offline render (each scene's camera, 1280 × 720)",
      subtitle: subtitle([...new Set(sppUsed)].join(" / ")), columns, tileWidth: 480, rows,
    });
    console.log(`all scenes: ${join(root, "out/milestone1/montage.jpg")}`);
  }
} finally {
  browser.close();
  server.close();
}

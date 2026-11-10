// three.js compatibility set (ROADMAP § 6 step 2f): tests/three-compat/*.js are snippets of three.js scene code as
// official examples and LLM-written code have it, run unchanged against the DSL (`import * as THREE from
// "@threert/scene"`). The page around them (DOM, window, requestAnimationFrame, asset loaders) is left out: a ThreeRT
// game's setup and frame replace it. A snippet's default export builds the scene and returns { scene, camera,
// frame?(t), check?(), expect? }.
//
// A snippet passes when
//  - it runs: built, bound to a recording engine and synced, its frame stepped (t = 0, 0.5, 1) and synced again,
//    without an exception;
//  - its `check()` (the three.js behaviour the snippet relies on: a raycast's hit, a transform's result) is true;
//  - the DSL reported nothing `unsupported:` (`compatNotes`; `mapped:` notes, the user's chosen mappings, are fine);
//  - the renderer draws it (offline, 160 x 90, 16 spp, linear HDR; the frame stepped once, t = 1, as the page's first
//    render would have): finite, not black, not flat (coefficient of variation of luminance > 0.05), and colourful
//    where `expect.colorful` says (at least 1 % of the pixels clearly coloured: chroma > 0.3).
//
// Also printed: the THREE.X names the snippets use that the DSL lacks, by how many snippets use them.
//
//   node tools/three-compat.ts [filter] [--no-render] [--json out.json]
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot } from "./lib/bench.ts";

const args = process.argv.slice(2);
const filter = args.find((a) => !a.startsWith("--") && args[args.indexOf(a) - 1] !== "--json");
const render = !args.includes("--no-render");
const dir = join(repoRoot, "tests/three-compat");
const out = join(repoRoot, "out/three-compat");
mkdirSync(out, { recursive: true });
const files = readdirSync(dir).filter((f) => f.endsWith(".js") && (!filter || f.includes(filter))).sort();

const scene = await import("@threert/scene");
const notes = () => ((scene as { compatNotes?: () => string[] }).compatNotes?.() ?? []);

type Row = { id: string; area: string; source: string; ran: boolean; checked: boolean; unsupported: string[]; mapped: string[]; rendered: boolean | null; pass: boolean; why: string };
const rows: Row[] = [];
for (const f of files) {
  const src = readFileSync(join(dir, f), "utf8");
  const area = /^\/\/ area: (\w+)/m.exec(src)?.[1] ?? "?";
  const source = /^\/\/ source: (.*)$/m.exec(src)?.[1] ?? "?";
  const row: Row = { id: f.replace(/\.js$/, ""), area, source, ran: false, checked: false, unsupported: [], mapped: [], rendered: null, pass: false, why: "" };
  const before = notes().length;
  let expect: { colorful?: boolean } = {};
  try {
    const mod = await import(pathToFileURL(join(dir, f)).href);
    const r = mod.default();
    expect = r.expect ?? {};
    const engine = new scene.RecordingEngine();
    const binding = new scene.SceneBinding(engine, r.scene, r.camera);
    binding.sync();
    for (const t of [0, 0.5, 1]) {
      r.frame?.(t);
      binding.sync();
    }
    row.ran = true;
    row.checked = r.check ? r.check() === true : true;
    if (!row.checked) row.why = "check failed";
  } catch (e) {
    row.why = String((e as Error).message ?? e).split("\n")[0].slice(0, 120);
  }
  const added = notes().slice(before);
  row.unsupported = added.filter((n) => n.startsWith("unsupported:"));
  row.mapped = added.filter((n) => n.startsWith("mapped:"));
  if (row.ran && row.checked && !row.unsupported.length && render) {
    try {
      const wrapper = join(out, `${row.id}.entry.js`);
      writeFileSync(wrapper, `import build from ${JSON.stringify(join(dir, f))};\nexport default function () {\n  const r = build();\n  r.frame?.(1);\n  return { scene: r.scene, camera: r.camera, options: { name: ${JSON.stringify(row.id)}, width: 160, height: 90 } };\n}\n`);
      const bundle = (await bundleGame(wrapper, join(out, `${row.id}.bundle.js`))).out;
      const raw = join(out, `${row.id}.raw`);
      execFileSync(rendererBin, ["render", bundle, "-o", join(out, `${row.id}.png`), "--assets", repoRoot, "--spp", "16", "--width", "160", "--height", "90", "--no-denoise", "--hdr", raw], { stdio: ["ignore", "ignore", "pipe"] });
      const px = new Float32Array(readFileSync(raw).buffer.slice(0));
      const ch = px.length / (160 * 90);
      let sum = 0, sum2 = 0, chroma = 0, finite = true;
      for (let i = 0; i < 160 * 90; i++) {
        const [r0, g0, b0] = [px[i * ch], px[i * ch + 1], px[i * ch + 2]];
        if (!Number.isFinite(r0 + g0 + b0)) finite = false;
        const l = 0.2126 * r0 + 0.7152 * g0 + 0.0722 * b0;
        sum += l;
        sum2 += l * l;
        if ((Math.max(r0, g0, b0) - Math.min(r0, g0, b0)) / Math.max(1e-6, Math.max(r0, g0, b0)) > 0.3) chroma++;
      }
      const n = 160 * 90, mean = sum / n, cv = Math.sqrt(Math.max(0, sum2 / n - mean * mean)) / Math.max(mean, 1e-9);
      row.rendered = finite && mean > 1e-6 && cv > 0.05 && (!expect.colorful || chroma / n > 0.01);
      if (!row.rendered) row.why = `render: ${finite ? "" : "not finite, "}mean ${mean.toExponential(2)}, cv ${cv.toFixed(3)}, coloured ${((100 * chroma) / n).toFixed(1)} %`;
    } catch (e) {
      row.rendered = false;
      row.why = `render failed: ${String((e as { stderr?: Buffer }).stderr ?? e).trim().split("\n").slice(-1)[0].slice(0, 120)}`;
    }
  }
  if (row.unsupported.length && !row.why) row.why = row.unsupported.join("; ").slice(0, 120);
  row.pass = row.ran && row.checked && !row.unsupported.length && (row.rendered ?? !render);
  rows.push(row);
  console.log(`${row.pass ? "PASS" : "FAIL"} ${row.area.padEnd(9)} ${row.id.padEnd(34)} ${row.why}`);
}
// The THREE.X names still missing from the DSL, by how many snippets use them (the order to fill gaps in).
const missing = new Map<string, number>();
for (const f of files) {
  for (const n of new Set([...readFileSync(join(dir, f), "utf8").matchAll(/THREE\.(\w+)/g)].map((m) => m[1]))) if (!(n in scene)) missing.set(n, (missing.get(n) ?? 0) + 1);
}
if (missing.size) console.log(`\nmissing exports (snippets using them): ${[...missing].sort((a, b) => b[1] - a[1]).map(([n, c]) => `${n} ${c}`).join(", ")}`);
const areas = [...new Set(rows.map((r) => r.area))].sort();
console.log("\n| area | pass / snippets |\n| --- | --- |");
for (const a of areas) console.log(`| ${a} | ${rows.filter((r) => r.area === a && r.pass).length} / ${rows.filter((r) => r.area === a).length} |`);
const passed = rows.filter((r) => r.pass).length;
console.log(`| **all** | **${passed} / ${rows.length}** (${((100 * passed) / rows.length).toFixed(0)} %) |`);
if (args.includes("--json")) writeFileSync(args[args.indexOf("--json") + 1], JSON.stringify(rows, null, 1));

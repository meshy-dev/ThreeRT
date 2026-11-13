// Lighting on voxels (ROADMAP § 6 step 4; docs/VOXEL.md, "Measured: lighting on voxels"): the benchmark level at night
// (games/voxel-level/level-night.game.ts), its lamps' emissive voxels as the only light besides the night sky's glow,
// ThreeRT live against our offline render of the same world.
//
//  quality   three shots on the level's camera path, and shot 0 after the lamp ahead was blown up in setup (its pieces
//            0.3 s into their flight, hanging there): live 64 frames after a history reset, camera held still, at the
//            0.5 floor (layer B's protocol) and at the dynamic resolution the game runs at ("auto"), against our
//            unbiased offline render (no clamps, no denoiser, 16384 spp: at 4096 it still differs from 16384 by ΔE 0.8
//            / SSIM 0.97; cached in out/cache/truth): mean ΔE and SSIM over the image and over its 3 x 3 regions; a
//            second run at the floor beside it (the live view's run-to-run spread: its radiance cache trains in an
//            order the GPU decides, ROADMAP Known issues).
//  lamp      the radiance the lamp ahead of shot 0 adds: our offline render with the lamps on and off (and with the lamp
//            blown up, its glowing pieces in flight, on and off), mean luminance over the ground and the first 3 m
//            above it around its foot.
//  subtree   when emitters added in place get a light-tree subtree of their own (the supervisor's rule): the same lamp
//            in the static tree (its pole cut in setup), and after the same cut in place at frame 3 in the subtree of
//            emitters added in place (ltree.rs `set_dynamic`), at the same frames, against the offline render of that
//            world over what it lights: mean ΔE of the
//            64-frame shot, and the per-pixel variance of the luminance over `--repeat` such shots.
//  blast     the lamp blown up live at 5 s (a 1 m explosion; its pieces fly, the glowing ones lighting what they pass,
//            and settle back after 3 s), `--rounds` runs: the explosion frame (the worse of its two), frames over 50
//            ms, frame start p95 over the run after its first second, the in-place edits' fallbacks and rebuilds.
//
//   node tools/bench-voxel-light.ts [--only quality,lamp,subtree,blast] [--rounds 4] [--repeat 4] [--spp 4096]
//
// Holds the GPU lock. Result: out/bench/voxel-light/<time>.json and tables on stdout.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import sharp from "sharp";
import { blastLamp, type Night, SHOTS } from "../games/voxel-level/level-night.game.ts";
import { cameraPath } from "../games/voxel-level/plan.ts";
import { bundleGame } from "./bundle-game.ts";
import { frameStats, rendererBin, repoRoot, requireGpuLock } from "./lib/bench.ts";
import { startLive } from "./lib/live.ts";
import { generate } from "./voxel-level/generate.ts";

requireGpuLock("bench-voxel-light");
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const only = opt("--only", "quality,lamp,subtree,blast").split(",");
const rounds = Number(opt("--rounds", "4"));
const repeat = Number(opt("--repeat", "4"));
const SPP = Number(opt("--spp", "16384"));
const TRUTH_VERSION = 1;
const dir = join(repoRoot, "out/voxel-light");
const truthDir = join(repoRoot, "out/cache/truth");
const outDir = join(repoRoot, "out/bench/voxel-light");
for (const d of [dir, truthDir, outDir]) mkdirSync(d, { recursive: true });
generate(1, "object");

/** A variant's bundle (an entry module calling nightGame with its options). */
async function bundle(name: string, o: Night): Promise<string> {
  const entry = join(dir, `${name}.entry.ts`);
  writeFileSync(entry, `import { nightGame } from ${JSON.stringify(join(repoRoot, "games/voxel-level/level-night.game.ts"))};\nexport default nightGame(${JSON.stringify(o)});\n`);
  return (await bundleGame(entry, join(dir, `${name}.js`))).out;
}

/** Our unbiased offline render of a bundle (png and linear HDR), cached by bundle content, renderer and spp. */
function truth(name: string, b: string, spp = SPP): { png: string; hdr: string } {
  const key = createHash("sha256").update(readFileSync(b)).update(readFileSync(rendererBin)).update(`truth ${TRUTH_VERSION} ${spp}`).digest("hex").slice(0, 12);
  const png = join(truthDir, `${name}-${key}.png`), hdr = join(truthDir, `${name}-${key}.raw`);
  if (!existsSync(png) || !existsSync(hdr)) {
    console.log(`  offline ${name} (${spp} spp)…`);
    execFileSync(rendererBin, ["render", b, "-o", png, "--hdr", hdr, "--assets", repoRoot, "--spp", String(spp), "--no-denoise", "--override", JSON.stringify({ native: { clampIndirect: 1e9 } })], { stdio: ["ignore", "ignore", "inherit"] });
  }
  return { png, hdr };
}

type M = { mean_delta_e: number; ssim: number; luminance_ratio: number };
const compare = (ref: string, test: string, region?: number[]): M => JSON.parse(execFileSync(rendererBin, ["compare", ref, test, ...(region ? ["--region", region.join(",")] : [])]).toString().trim().split("\n").pop()!);
/** Overall and per 3 x 3 region (row-major from the top left). */
function judge(ref: string, test: string) {
  const all = compare(ref, test);
  const regions: M[] = [];
  for (let r = 0; r < 3; r++) for (let c = 0; c < 3; c++) regions.push(compare(ref, test, [c / 3, r / 3, (c + 1) / 3, (r + 1) / 3]));
  return { all, regions, worst_de: Math.max(...regions.map((m) => m.mean_delta_e)), worst_ssim: Math.min(...regions.map((m) => m.ssim)) };
}

/** Live shots of a bundle: after the first frame and `settleMs`, `shots` x (history reset, `frames` frames, snapshot). */
async function liveShots(name: string, b: string, shots: number, o: { frames?: number; scale?: string; env?: Record<string, string> } = {}): Promise<string[]> {
  const frames = o.frames ?? 64, settleMs = 6000;
  const live = startLive({ bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: dir, label: name }, b, { scale: o.scale ?? "0.5", env: o.env });
  await live.firstFrame;
  await new Promise((ok) => setTimeout(ok, settleMs));
  const out: string[] = [];
  for (let k = 0; k < shots; k++) {
    const png = join(dir, `${name}-live-${frames}-${k}.png`);
    live.send({ cmd: "reset" });
    live.send({ cmd: "snapshot", path: png, after: frames });
    await live.wait((l: string) => l.includes(png), 300_000);
    out.push(png);
  }
  await live.quit();
  return out;
}

// Around the lamp's foot (a 6 m square, from the ground to 3 m up) as an image region of shot 0: the box of its corners
// projected.
const cam = cameraPath(1)[SHOTS[0]];
function project(p: number[]): [number, number] {
  const f = cam.target.map((t, i) => t - cam.pos[i]), fl = Math.hypot(...f), fw = f.map((v) => v / fl);
  const r = [-fw[2], 0, fw[0]], rl = Math.hypot(...r), rt = r.map((v) => v / rl);
  const up = [rt[1] * fw[2] - rt[2] * fw[1], rt[2] * fw[0] - rt[0] * fw[2], rt[0] * fw[1] - rt[1] * fw[0]];
  const d = p.map((v, i) => v - cam.pos[i]);
  const z = d[0] * fw[0] + d[1] * fw[1] + d[2] * fw[2], x = d[0] * rt[0] + d[2] * rt[2], y = d[0] * up[0] + d[1] * up[1] + d[2] * up[2];
  const t = Math.tan((55 * Math.PI) / 360);
  return [0.5 + x / (z * t * (16 / 9)) / 2, 0.5 - y / (z * t) / 2];
}
const lamp = blastLamp(1);
const corners = [-3, 3].flatMap((dx) => [-3, 3].flatMap((dz) => [0, 3].map((dy) => project([lamp.ground[0] + dx, lamp.ground[1] + dy, lamp.ground[2] + dz]))));
const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const LIT = [Math.min(...corners.map((c) => c[0])), Math.min(...corners.map((c) => c[1])), Math.max(...corners.map((c) => c[0])), Math.max(...corners.map((c) => c[1]))].map(clamp01);

/** Mean luminance of a linear HDR render (RGB f32, 1280 x 720) over a region. */
function hdrLum(raw: string, [u0, v0, u1, v1]: number[], w = 1280, h = 720): number {
  const px = new Float32Array(readFileSync(raw).buffer.slice(0));
  let s = 0, n = 0;
  for (let y = Math.floor(v0 * h); y < Math.ceil(v1 * h); y++) for (let x = Math.floor(u0 * w); x < Math.ceil(u1 * w); x++) {
    const i = (y * w + x) * 3;
    s += 0.2126 * px[i] + 0.7152 * px[i + 1] + 0.0722 * px[i + 2];
    n++;
  }
  return s / n;
}

/** Per-pixel variance of the (linear) luminance over shots, its mean over a region, relative to the mean luminance². */
async function variance(pngs: string[], [u0, v0, u1, v1]: number[]) {
  const imgs = await Promise.all(pngs.map((p) => sharp(p).removeAlpha().raw().toBuffer({ resolveWithObject: true })));
  const { width: w, height: h } = imgs[0].info;
  const lin = (c: number) => ((c /= 255) <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  let vs = 0, ms = 0, n = 0;
  for (let y = Math.floor(v0 * h); y < Math.ceil(v1 * h); y++) for (let x = Math.floor(u0 * w); x < Math.ceil(u1 * w); x++) {
    const i = (y * w + x) * 3;
    const l = imgs.map((im) => 0.2126 * lin(im.data[i]) + 0.7152 * lin(im.data[i + 1]) + 0.0722 * lin(im.data[i + 2]));
    const m = l.reduce((a, b) => a + b, 0) / l.length;
    vs += l.reduce((a, b) => a + (b - m) ** 2, 0) / (l.length - 1);
    ms += m;
    n++;
  }
  return { var: vs / n, mean: ms / n, rel: vs / n / Math.max((ms / n) ** 2, 1e-12) };
}

const r2 = (v: number) => +v.toFixed(2);
const r3 = (v: number) => +v.toFixed(3);
const result: Record<string, unknown> = { at: new Date().toISOString(), spp: SPP, lit_region: LIT.map(r3), lamp };
const fmt = (m: M) => `${m.mean_delta_e.toFixed(2)} / ${m.ssim.toFixed(3)}`;

if (only.includes("quality")) {
  const rows: Record<string, unknown>[] = [];
  const variants: [string, Night][] = [...[0, 1, 2].map((s): [string, Night] => [`night-${s}`, { shot: s }]), ["blast-setup-0", { blast: "lamp", when: "setup", advance: 0.3 }]];
  console.log(`\nquality: live (64 frames) vs our offline render (${SPP} spp); ΔE / SSIM`);
  console.log(`variant          0.5 floor: all  worst region    run 2 all       run 2 vs 1      auto: all       worst region`);
  for (const [name, o] of variants) {
    const b = await bundle(name, o);
    const t = truth(name, b);
    const [a] = await liveShots(name, b, 1);
    const [c] = await liveShots(`${name}-b`, b, 1);
    const [d] = await liveShots(`${name}-auto`, b, 1, { scale: "auto" });
    const ja = judge(t.png, a), jc = judge(t.png, c), jd = judge(t.png, d), aa = compare(a, c);
    rows.push({ name, run1: ja, run2: jc, aa, auto: jd });
    const worst = (j: typeof ja) => `${j.worst_de.toFixed(2)} / ${j.worst_ssim.toFixed(3)}`;
    console.log(`${name.padEnd(16)} ${fmt(ja.all).padEnd(15)} ${worst(ja).padEnd(15)} ${fmt(jc.all).padEnd(15)} ${fmt(aa).padEnd(15)} ${fmt(jd.all).padEnd(15)} ${worst(jd)}`);
    for (const [tag, j] of [["floor", ja], ["auto", jd]] as const) console.log(`  ${tag} regions ΔE / SSIM  ${j.regions.map((m) => `${m.mean_delta_e.toFixed(2)}/${m.ssim.toFixed(3)}`).join(" ")}`);
  }
  result.quality = rows;
}

if (only.includes("lamp")) {
  const on = truth("night-0", await bundle("night-0", { shot: 0 }));
  const off = truth("dark-0", await bundle("dark-0", { shot: 0, lamps: false }));
  const bon = truth("blast-setup-0", await bundle("blast-setup-0", { blast: "lamp", when: "setup", advance: 0.3 }));
  const boff = truth("blast-setup-dark-0", await bundle("blast-setup-dark-0", { blast: "lamp", when: "setup", advance: 0.3, lamps: false }));
  const v = { on: hdrLum(on.hdr, LIT), off: hdrLum(off.hdr, LIT), blast_on: hdrLum(bon.hdr, LIT), blast_off: hdrLum(boff.hdr, LIT) };
  result.lamp = v;
  console.log(`\nlamp: mean luminance (nits) of the ground around the lamp's foot (region ${LIT.map((x) => x.toFixed(3)).join(",")}), offline`);
  console.log(`  lamps on ${v.on.toPrecision(4)}, off ${v.off.toPrecision(4)} (x${(v.on / Math.max(v.off, 1e-9)).toFixed(0)}); lamp blown up, pieces in flight: glowing ${v.blast_on.toPrecision(4)}, not ${v.blast_off.toPrecision(4)}`);
}

if (only.includes("subtree")) {
  const tb = await bundle("pole-setup-0", { blast: "pole", when: "setup" });
  const lb = await bundle("pole-live-0", { blast: "pole", when: 3 });
  const t = truth("pole-setup-0", tb);
  const rows: Record<string, unknown>[] = [];
  for (const [tag, b] of [["static tree", tb], ["in place, own subtree", lb]] as const) {
    const shots = await liveShots(`subtree-${tag.replace(/[^a-z]+/g, "-")}`, b, repeat, {});
    const de = shots.map((s) => compare(t.png, s, LIT));
    const v = await variance(shots, LIT);
    rows.push({ tag, de: de.map((m) => m.mean_delta_e), de_mean: r3(de.reduce((a, m) => a + m.mean_delta_e, 0) / de.length), ssim_mean: r3(de.reduce((a, m) => a + m.ssim, 0) / de.length), lum_ratio: r3(de.reduce((a, m) => a + m.luminance_ratio, 0) / de.length), var_rel: v.rel, var: v.var });
  }
  const [s, d] = rows as { de_mean: number; var: number }[];
  const over = (x: { de_mean: number; var: number }) => x.de_mean - s.de_mean > 0.5 || x.var > 1.5 * s.var;
  result.subtree = { rows, own: { de_diff: r3(d.de_mean - s.de_mean), var_ratio: r3(d.var / s.var), over: over(d) } };
  console.log(`\nsubtree: the lamp's emitters in the static tree, and moved in place to the subtree of emitters added in place; ${repeat} live shots of 64 frames each, over what it lights`);
  for (const r of rows) console.log(`  ${String(r.tag).padEnd(24)} ΔE ${r.de_mean} (${(r.de as number[]).map((x) => x.toFixed(2)).join(" ")}), SSIM ${r.ssim_mean}, luminance ${r.lum_ratio} of offline, variance ${(r.var as number).toExponential(3)} (relative ${(r.var_rel as number).toExponential(3)})`);
  for (const [tag, x] of [["own subtree", d]] as const) console.log(`  ${tag}: ΔE ${x.de_mean - s.de_mean >= 0 ? "+" : ""}${r3(x.de_mean - s.de_mean)}, variance x${r3(x.var / s.var)} of the static tree's: ${over(x) ? "over the rule's line (ΔE +0.5 or variance +50 %)" : "within the rule's lines"}`);
}

if (only.includes("blast")) {
  const b = await bundle("blast-live-0", { blast: "lamp", when: 300 });
  const runs: Record<string, unknown>[] = [];
  type Frame = { interval: number; start?: number; cpu: number; edit: number; rebuild: number; fallback: string | null; voxel?: [number, number] };
  for (let round = 1; round <= rounds; round++) {
    const log = join(dir, "blast-frames.jsonl");
    const live = startLive({ bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: dir, label: "blast" }, b, { frameLog: log });
    await live.firstFrame;
    // The explosion at frame 300 of the frame script (5 s), its pieces settle 3 s later: 10 s in all.
    await live.wait((l: string) => l.includes("settled\":1"), 60_000);
    await new Promise((ok) => setTimeout(ok, 1000));
    await live.quit();
    const F = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l) as Frame);
    const ex = F.findIndex((f) => (f.voxel?.[0] ?? 0) > 0), st = F.findIndex((f) => (f.voxel?.[1] ?? 0) > 0);
    const after = F.slice(60);
    const starts = after.map((f) => f.start ?? f.interval);
    const row = {
      round, frames: F.length, explosion_frame_ms: r2(Math.max(F[ex].interval, F[ex + 1]?.interval ?? 0)), explosion_cpu_ms: r2(Math.max(F[ex].cpu, F[ex + 1]?.cpu ?? 0)), settle_frame_ms: st >= 0 ? r2(Math.max(F[st].interval, F[st + 1]?.interval ?? 0)) : null,
      flight_edit_p50: r2(frameStats(F.slice(ex + 2, st > 0 ? st : undefined).map((f) => f.edit)).p50), over_50: after.filter((f) => f.interval > 50).length, start_p95: r2(frameStats(starts).p95), start_max: r2(Math.max(...starts)),
      fallbacks: F.filter((f) => f.fallback).map((f) => f.fallback), rebuilds: F.filter((f) => f.rebuild > 0).length,
    };
    runs.push(row);
    console.log(JSON.stringify(row));
  }
  result.blast = runs;
}

const file = join(outDir, `${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
writeFileSync(file, JSON.stringify(result, null, 1) + "\n");
console.log(`-> ${file}`);

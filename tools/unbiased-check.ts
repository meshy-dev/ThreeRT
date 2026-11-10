// The parity suite's unbiasedness spot check (docs/WORKFLOW.md, "Scheduled suites on main"): the offline path tracer,
// as it renders with hardware ray tracing, against the software BVH at many more samples. Both with no clamps and no
// denoiser, raw HDR at 320 x 180; per scene, the image mean and 4 x 4 region means of luminance. The software BVH is the
// referee: it traverses every triangle and runs every alpha test itself, so a hardware path that loses hits or stops
// short shows here (the shadow rays' 16-leaf limit read up to 2.3 % bright in a Bistro region; their candidate loop lost
// hits from run to run).
//
//   node tools/unbiased-check.ts [--scenes bistro,bistro-15s,zero-day,foliage,procedural,voxel] [--spp N] [--truth-spp 16384]
//
// The referee renders are cached in out/cache/truth by bundle content, size, spp, camera and TRUTH_VERSION (bump it
// when the estimator changes what it converges to). Pass: the mean within MEAN_TOL and every region within the scene's
// `tol`, set from the noise of equivalent renders (two seeds of the hardware render against the referee: Bistro and the
// ferns within 0.2 %, Zero Day's glossy machine tops 1.5 % at 4096 spp, hence 16384 there). It caught Zero Day's
// emissive quads under masked decals (a region 3.5 % dark, trace_hw.wgsl). Exit 1 on a fail.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { BISTRO } from "../scenes/bistro.ts";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, requireQuietGpu } from "./lib/bench.ts";
import { bistroDrive } from "./lib/live.ts";

const TRUTH_VERSION = 1;
const W = 320, H = 180;
const MEAN_TOL = 0.005;
// `at`: a pose on Bistro's camera path (s) instead of the scene's camera; `spp` of the hardware render; `tol`: regions.
const SCENES: Record<string, { src: string; what: string; at?: number; spp: number; tol: number }> = {
  bistro: { src: "scenes/bistro.ts", spp: 4096, tol: 0.01, what: "20 k small emitters, alpha-cutout foliage" },
  "bistro-15s": { src: "scenes/bistro.ts", at: 15, spp: 4096, tol: 0.01, what: "the square 15 s along the camera path: light through dense foliage" },
  "zero-day": { src: "scenes/zero-day.ts", spp: 16384, tol: 0.02, what: "10 k emitters, glossy metal, decals over emitters" },
  foliage: { src: "tools/foliage-scene.ts", spp: 4096, tol: 0.01, what: "potted ferns (alpha cutouts) in sun" },
  procedural: { src: "tools/procedural-scene.ts", spp: 4096, tol: 0.01, what: "procedural spheres and boxes (geometry functions, AABB candidates) beside triangles" },
  voxel: { src: "tools/voxel-scene.ts", spp: 4096, tol: 0.01, what: "voxel objects (brick DDA, three box granularities): terrain, a hollow house, a sphere, a lamp" },
};
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const ids = opt("--scenes", Object.keys(SCENES).join(",")).split(",");
const sppOpt = args.includes("--spp") ? Number(opt("--spp", "0")) : 0;
const truthSpp = Number(opt("--truth-spp", "16384"));
const dir = join(repoRoot, "out/cache/truth");
const work = join(repoRoot, "out/unbiased-check");
mkdirSync(dir, { recursive: true });
mkdirSync(work, { recursive: true });
requireQuietGpu();

const path = bistroDrive(join(repoRoot, BISTRO));
const cameraAt = (t: number) => {
  const c = path.at(t);
  return { position: c.pos, target: c.target, fov: (2 * Math.atan(c.tan) * 180) / Math.PI };
};
const render = (bundle: string, out: string, n: number, software: boolean, camera: string[]) =>
  execFileSync(rendererBin, ["render", bundle, "-o", out.replace(/\.raw$/, ".png"), "--assets", repoRoot, "--spp", String(n), "--width", String(W), "--height", String(H), "--no-denoise", "--override", JSON.stringify({ native: { clampIndirect: 1e9 } }), "--hdr", out, ...camera], { stdio: ["ignore", "ignore", "inherit"], env: { ...process.env, THREERT_FREEZE_TIME: "1", ...(software ? { THREERT_HWRT: "0" } : {}) } });
const lum = (file: string) => {
  const b = readFileSync(file);
  const f = new Float32Array(b.buffer, b.byteOffset, b.byteLength / 4);
  const y = new Float64Array(W * H);
  for (let i = 0; i < W * H; i++) y[i] = 0.2126 * f[i * 3] + 0.7152 * f[i * 3 + 1] + 0.0722 * f[i * 3 + 2];
  return y;
};
const regions = (y: Float64Array) => {
  const r = new Array(16).fill(0);
  for (let py = 0; py < H; py++) for (let px = 0; px < W; px++) r[Math.floor((py * 4) / H) * 4 + Math.floor((px * 4) / W)] += y[py * W + px];
  return r.map((v) => v / ((W * H) / 16));
};

let failed = 0;
console.log(`hardware offline / software BVH (${truthSpp} spp), luminance, ${W} x ${H}, no clamps, no denoiser:`);
for (const id of ids) {
  const s = SCENES[id];
  if (!s) throw new Error(`unknown scene ${id} (${Object.keys(SCENES).join(", ")})`);
  const bundle = join(repoRoot, "out/bundles", `${id}.js`);
  await bundleGame(join(repoRoot, s.src), bundle);
  const camera = s.at === undefined ? [] : ["--camera", JSON.stringify(cameraAt(s.at))];
  const key = createHash("sha256").update(readFileSync(bundle)).update(`unbiased ${TRUTH_VERSION} ${W}x${H} ${truthSpp}${camera.length ? ` ${camera.join(" ")}` : ""}`).digest("hex").slice(0, 12);
  const truth = join(dir, `${id}-sw-${key}.raw`);
  if (!existsSync(truth)) {
    console.log(`  rendering ${id}'s software-BVH referee (${truthSpp} spp, once per bundle)…`);
    render(bundle, truth, truthSpp, true, camera);
  }
  const hw = join(work, `${id}-hw.raw`);
  const spp = sppOpt || s.spp;
  render(bundle, hw, spp, false, camera);
  const a = lum(truth), b = lum(hw);
  const ma = a.reduce((p, c) => p + c, 0) / a.length, mb = b.reduce((p, c) => p + c, 0) / b.length;
  const ra = regions(a), rb = regions(b);
  const ratios = rb.map((v, i) => (ra[i] > 1e-6 ? v / ra[i] : 1));
  const worst = ratios.reduce((w, v) => (Math.abs(v - 1) > Math.abs(w - 1) ? v : w), 1);
  const ok = Math.abs(mb / ma - 1) <= MEAN_TOL && ratios.every((v) => Math.abs(v - 1) <= s.tol);
  if (!ok) failed++;
  console.log(`${ok ? "PASS" : "FAIL"} ${id.padEnd(10)} ${String(spp).padStart(5)} spp  mean ${(mb / ma).toFixed(4)} (±${MEAN_TOL})  worst region ${worst.toFixed(3)} (±${s.tol})  [${ratios.map((v) => v.toFixed(3)).join(" ")}]  (${s.what})`);
}
if (failed) {
  console.error(`unbiased check: ${failed} scene(s) off the software BVH`);
  process.exitCode = 1;
}

// The ΔE / SSIM tool (tools/compare/image-metrics.ts, delta-e.ts): CIEDE2000 against Sharma, Wu and Dalal's published
// test data, SSIM and ΔE76 on images whose answer is known, regions, the bars, and the command line end to end.
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Rgb8, ciede2000, grid, linearToLab, measure, parseRegion, pixelRect, resize, ssimMap } from "../image-metrics.ts";
import { main, readImage, writePng } from "../delta-e.ts";

// Sharma, Wu, Dalal, "The CIEDE2000 color-difference formula: implementation notes, supplementary test data, and
// mathematical observations" (2005), table 1: L1 a1 b1, L2 a2 b2, ΔE00.
const SHARMA: number[][] = [
  [50, 2.6772, -79.7751, 50, 0, -82.7485, 2.0425],
  [50, 3.1571, -77.2803, 50, 0, -82.7485, 2.8615],
  [50, 2.8361, -74.02, 50, 0, -82.7485, 3.4412],
  [50, -1.3802, -84.2814, 50, 0, -82.7485, 1.0],
  [50, -1.1848, -84.8006, 50, 0, -82.7485, 1.0],
  [50, -0.9009, -85.5211, 50, 0, -82.7485, 1.0],
  [50, 0, 0, 50, -1, 2, 2.3669],
  [50, -1, 2, 50, 0, 0, 2.3669],
  [50, 2.49, -0.001, 50, -2.49, 0.0009, 7.1792],
  [50, 2.49, -0.001, 50, -2.49, 0.001, 7.1792],
  [50, 2.49, -0.001, 50, -2.49, 0.0011, 7.2195],
  [50, 2.49, -0.001, 50, -2.49, 0.0012, 7.2195],
  [50, -0.001, 2.49, 50, 0.0009, -2.49, 4.8045],
  [50, -0.001, 2.49, 50, 0.001, -2.49, 4.8045],
  [50, -0.001, 2.49, 50, 0.0011, -2.49, 4.7461],
  [50, 2.5, 0, 50, 0, -2.5, 4.3065],
  [50, 2.5, 0, 73, 25, -18, 27.1492],
  [50, 2.5, 0, 61, -5, 29, 22.8977],
  [50, 2.5, 0, 56, -27, -3, 31.903],
  [50, 2.5, 0, 58, 24, 15, 19.4535],
  [50, 2.5, 0, 50, 3.1736, 0.5854, 1.0],
  [50, 2.5, 0, 50, 3.2972, 0, 1.0],
  [50, 2.5, 0, 50, 1.8634, 0.5757, 1.0],
  [50, 2.5, 0, 50, 3.2592, 0.335, 1.0],
  [60.2574, -34.0099, 36.2677, 60.4626, -34.1751, 39.4387, 1.2644],
  [63.0109, -31.0961, -5.8663, 62.8187, -29.7946, -4.0864, 1.263],
  [61.2901, 3.7196, -5.3901, 61.4292, 2.248, -4.962, 1.8731],
  [35.0831, -44.1164, 3.7933, 35.0232, -40.0716, 1.5901, 1.8645],
  [22.7233, 20.0904, -46.694, 23.0331, 14.973, -42.5619, 2.0373],
  [36.4612, 47.858, 18.3852, 36.2715, 50.5065, 21.2231, 1.4146],
  [90.8027, -2.0831, 1.441, 91.1528, -1.6435, 0.0447, 1.4441],
  [90.9257, -0.5406, -0.9208, 88.6381, -0.8985, -0.7239, 1.5381],
  [6.7747, -0.2908, -2.4247, 5.8714, -0.0985, -2.2286, 0.6377],
  [2.0776, 0.0795, -1.135, 0.9033, -0.0636, -0.5514, 0.9082],
];

test("CIEDE2000 matches Sharma et al.'s 34 test pairs to 4 decimals, both ways round", () => {
  for (const [L1, a1, b1, L2, a2, b2, want] of SHARMA) {
    assert.ok(Math.abs(ciede2000(L1, a1, b1, L2, a2, b2) - want) < 5e-5, `${[L1, a1, b1, L2, a2, b2]}: ${ciede2000(L1, a1, b1, L2, a2, b2)} vs ${want}`);
    assert.ok(Math.abs(ciede2000(L2, a2, b2, L1, a1, b1) - want) < 5e-5);
  }
  assert.equal(ciede2000(50, 10, -10, 50, 10, -10), 0);
});

test("Lab of sRGB white is (100, 0, 0) and of black (0, 0, 0)", () => {
  const [L, a, b] = linearToLab(1, 1, 1);
  assert.ok(Math.abs(L - 100) < 0.01 && Math.abs(a) < 0.05 && Math.abs(b) < 0.05, `${L} ${a} ${b}`);
  assert.deepEqual(linearToLab(0, 0, 0).map((v) => Math.round(v * 1e6) / 1e6), [0, 0, 0]);
});

const solid = (w: number, h: number, c: [number, number, number]): Rgb8 => {
  const data = new Uint8Array(w * h * 3);
  for (let i = 0; i < w * h; i++) data.set(c, i * 3);
  return { width: w, height: h, data };
};
const noise = (w: number, h: number, seed: number): Rgb8 => {
  let s = seed;
  const data = new Uint8Array(w * h * 3).map(() => ((s = (s * 1103515245 + 12345) >>> 0), s >>> 24));
  return { width: w, height: h, data };
};

test("an image against itself: ΔE 0, SSIM 1, luminance ratio 1, every region the same", () => {
  const img = noise(40, 30, 7);
  const { result } = measure(img, img, { regions: grid(2, 2), bars: { de2000: 1, ssim: 0.98 } });
  assert.equal(result.overall.de2000, 0);
  assert.equal(result.overall.de76, 0);
  assert.equal(result.overall.ssim, 1);
  assert.equal(result.overall.luminance_ratio, 1);
  assert.equal(result.regions.length, 4);
  for (const r of result.regions) assert.equal(r.de2000, 0), assert.equal(r.ssim, 1), assert.equal(r.pixels, 300);
  assert.equal(result.pass, true);
  assert.deepEqual(result.failed, []);
});

test("a solid colour shift: every pixel's ΔE is the pair's CIEDE2000, the luminance ratio follows, the bars fail", () => {
  const a = solid(16, 16, [120, 80, 40]), b = solid(16, 16, [130, 80, 40]);
  const lin = (c: number) => ((c / 255 + 0.055) / 1.055) ** 2.4;
  const la = linearToLab(lin(120), lin(80), lin(40)), lb = linearToLab(lin(130), lin(80), lin(40));
  const { result } = measure(a, b, { bars: { de2000: 1, ssim: 0.98 } });
  assert.ok(Math.abs(result.overall.de2000 - ciede2000(...la, ...lb)) < 1e-3);
  assert.ok(Math.abs(result.overall.de76 - Math.hypot(la[0] - lb[0], la[1] - lb[1], la[2] - lb[2])) < 1e-3);
  assert.ok(result.overall.de2000 < result.overall.de76, "ΔE2000 is the smaller in a saturated orange");
  assert.ok(result.overall.luminance_ratio > 1);
  // Flat images have no structure to lose: SSIM's luminance term alone, just under 1.
  assert.ok(result.overall.ssim > 0.99 && result.overall.ssim < 1);
  assert.equal(result.pass, false);
  assert.match(result.failed!.join(), /ΔE2000/);
});

test("SSIM is near 0 for two unrelated noises and symmetric", () => {
  const a = noise(32, 32, 1), b = noise(32, 32, 2);
  const ya = new Float64Array(32 * 32).map((_, i) => a.data[i * 3]);
  const yb = new Float64Array(32 * 32).map((_, i) => b.data[i * 3]);
  const m = (map: Float32Array) => map.reduce((s, v) => s + v, 0) / map.length;
  const ab = m(ssimMap(ya, yb, 32, 32)), ba = m(ssimMap(yb, ya, 32, 32));
  assert.ok(Math.abs(ab - ba) < 1e-9);
  assert.ok(ab < 0.2, `two noises: ${ab}`);
});

test("regions see only their part: an error in the bottom-right quarter is that region's, and the worst", () => {
  const a = solid(20, 20, [100, 100, 100]);
  const b = solid(20, 20, [100, 100, 100]);
  for (let y = 10; y < 20; y++) for (let x = 10; x < 20; x++) b.data.set([200, 40, 40], (y * 20 + x) * 3);
  const { result, maps } = measure(a, b, { regions: [...grid(2, 2), parseRegion("bottom=0,0.5,1,1")] });
  const by = Object.fromEntries(result.regions.map((r) => [r.name, r]));
  assert.equal(by.r0c0.de2000, 0);
  assert.equal(by.r0c1.de2000, 0);
  assert.equal(by.r1c0.de2000, 0);
  assert.ok(by.r1c1.de2000 > 20);
  assert.ok(Math.abs(by.bottom.de2000 - by.r1c1.de2000 / 2) < 1e-2);
  assert.ok(Math.abs(result.overall.de2000 - by.r1c1.de2000 / 4) < 1e-2);
  assert.equal(by.r1c1.over5, 1);
  assert.equal(result.worst, "r1c1");
  assert.equal(maps.de2000[0], 0);
  assert.ok(maps.de2000[19 * 20 + 19] > 20);
});

test("aligning the luminance takes a pure exposure difference out", () => {
  const a = noise(24, 24, 3);
  const b: Rgb8 = { ...a, data: a.data.map((v) => Math.round(v * 0.7)) };
  const raw = measure(a, b).result, aligned = measure(a, b, { alignLuminance: true }).result;
  assert.ok(aligned.gain > 1.3, `gain ${aligned.gain}`);
  assert.ok(aligned.overall.de2000 < raw.overall.de2000 / 2, `${aligned.overall.de2000} vs ${raw.overall.de2000}`);
});

test("a test at another resolution is resized to the reference's; region bounds cover whole pixels", () => {
  const a = solid(40, 20, [50, 150, 250]);
  const { result } = measure(a, solid(20, 10, [50, 150, 250]));
  assert.equal(result.width, 40);
  assert.equal(result.overall.de2000, 0);
  assert.equal(resize(a, 40, 20), a);
  assert.deepEqual(pixelRect([0, 0, 1, 1], 7, 5), [0, 0, 7, 5]);
  assert.deepEqual(pixelRect([0.99, 0.99, 1, 1], 7, 5), [6, 4, 7, 5]);
  assert.throws(() => parseRegion("x=0,0,0.5"));
  assert.throws(() => parseRegion("x=0.5,0,0.2,1"));
});

// A gradient with a little noise, deterministic: the pair below has an answer from the renderer's `compare`.
const gradient = (w: number, h: number, seed: number): Rgb8 => {
  let s = seed;
  const data = new Uint8Array(w * h * 3);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++)
      for (let c = 0; c < 3; c++) {
        s = (s * 1103515245 + 12345) >>> 0;
        const base = c === 0 ? (x * 255) / (w - 1) : c === 1 ? (y * 255) / (h - 1) : 128;
        data[(y * w + x) * 3 + c] = Math.max(0, Math.min(255, Math.round(base + ((s >>> 24) - 128) / 8)));
      }
  return { width: w, height: h, data };
};

test("ΔE76, p95, p99, SSIM and luminance are the renderer's `compare` numbers on the same pair", () => {
  // `threert-render compare a.png b.png` on these two images (renderer/src/compare.rs): mean_delta_e 9.96,
  // p95_delta_e 19.74, p99_delta_e 25.97, ssim 0.8795, luminance_ratio 1.003.
  const { result } = measure(gradient(24, 16, 1), gradient(24, 16, 2));
  assert.equal(Math.round(result.overall.de76 * 100) / 100, 9.96);
  assert.equal(result.overall.de76_p95, 19.74);
  assert.equal(result.overall.de76_p99, 25.97);
  assert.equal(result.overall.ssim, 0.8795);
  assert.equal(result.overall.luminance_ratio, 1.003);
});

test("a test at another size is resized as the renderer's `compare` resizes it (Lanczos 3, the image crate's)", () => {
  // image::imageops::resize(&img, 4, 3, FilterType::Lanczos3) of gradient(6, 4, 3), as compare.rs resizes a test.
  const want = [20, 18, 120, 87, 17, 121, 165, 16, 133, 235, 14, 128, 18, 130, 128, 81, 135, 134, 162, 129, 122, 237, 132, 122, 10, 237, 141, 90, 244, 127, 166, 242, 115, 237, 235, 130];
  assert.deepEqual(Array.from(resize(gradient(6, 4, 3), 4, 3).data), want);
});

test("the command line: one pair and a manifest, JSON and PNGs written, the bar's exit code", async () => {
  const dir = mkdtempSync(join(tmpdir(), "delta-e-"));
  try {
    const ref = noise(48, 32, 11);
    const close: Rgb8 = { ...ref, data: ref.data.map((v) => Math.min(v + 1, 255)) };
    const far: Rgb8 = { ...ref, data: ref.data.map((v, i) => (i % 3 === 0 ? 255 - v : v)) };
    await writePng(ref, join(dir, "ref.png"));
    await writePng(close, join(dir, "close.png"));
    await writePng(far, join(dir, "far.png"));
    assert.deepEqual((await readImage(join(dir, "close.png"))).data, close.data);

    const out = join(dir, "out");
    const log = console.log, err = console.error;
    console.log = console.error = () => {};
    let one: number, many: number;
    try {
      one = await main([join(dir, "ref.png"), join(dir, "close.png"), "--out", out, "--grid", "2x1", "--region", "top=0,0,1,0.5", "--bar", "step2"]);
      writeFileSync(join(dir, "poses.json"), JSON.stringify({ regions: [{ name: "left", rect: [0, 0, 0.5, 1] }], poses: [{ name: "a", reference: "ref.png", test: "close.png" }, { name: "b", reference: "ref.png", test: "far.png" }] }));
      many = await main(["--manifest", join(dir, "poses.json"), "--out", out, "--grid", "0x0", "--bar", "step2"]);
    } finally {
      console.log = log;
      console.error = err;
    }
    assert.equal(one, 0);
    const r = JSON.parse(readFileSync(join(out, "close.json"), "utf8"));
    assert.equal(r.pass, true);
    assert.deepEqual(r.regions.map((g: { name: string }) => g.name), ["r0c0", "r0c1", "top"]);
    assert.ok(existsSync(join(out, "close-heat.png")) && existsSync(join(out, "close-sheet.png")));
    const sheetImg = await readImage(join(out, "close-sheet.png"));
    assert.deepEqual([sheetImg.width, sheetImg.height], [144, 32]);

    assert.equal(many, 1, "pose b misses the bar");
    const s = JSON.parse(readFileSync(join(out, "summary.json"), "utf8"));
    assert.equal(s.summary.poses, 2);
    assert.equal(s.summary.pass, false);
    assert.equal(s.summary.worst.de2000.pose, "b");
    assert.ok(s.summary.failed.every((f: string) => f.startsWith("b: ")));
    assert.deepEqual(s.poses[0].regions.map((g: { name: string }) => g.name), ["left"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

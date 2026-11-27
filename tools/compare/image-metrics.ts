// Image metrics against a reference (our offline render): per-pixel CIEDE2000 and CIE76 in CIELAB (D65), SSIM, the
// luminance ratio, overall and per region, and the heat maps that show where the error is. Pure TypeScript on 8-bit
// sRGB buffers, so it runs without the renderer built; `tools/delta-e.ts` is its command line.
//
// - ΔE76 and SSIM are computed exactly as the renderer's `compare` (renderer/src/compare.rs) computes them, so the
//   numbers in docs/COMPARE.md and the benches stay comparable: ΔE76 on sRGB -> linear -> XYZ -> Lab (D65), SSIM on
//   gamma-encoded BT.601 luma with an 11 x 11 Gaussian (σ 1.5), clamped edges, C1 = (0.01 · 255)², C2 = (0.03 · 255)².
//   Sources: CIELAB and ΔE76 from CIE 15:2004 (Colorimetry), sRGB's transfer function and its matrix to XYZ from IEC
//   61966-2-1; SSIM, its window, σ and constants from Wang, Bovik, Sheikh and Simoncelli, "Image Quality Assessment:
//   From Error Visibility to Structural Similarity", IEEE Trans. Image Processing 13(4), 2004; BT.601 luma from ITU-R
//   BT.601.
// - ΔE2000 is CIEDE2000 (Sharma, Wu, Dalal 2005, kL = kC = kH = 1) on the same Lab values: perceptually more uniform
//   than ΔE76 in saturated blues and near grey, and smaller than ΔE76 for the same pair in saturated colours.

export interface Rgb8 {
  width: number;
  height: number;
  /** 3 bytes per pixel, sRGB, rows top to bottom. */
  data: Uint8Array;
}

/** [u0, v0, u1, v1] in 0..1 of the image, v down. */
export type Rect = [number, number, number, number];

export interface Region {
  name: string;
  rect: Rect;
}

export interface Stats {
  /** Mean / p95 / p99 CIEDE2000. */
  de2000: number;
  de2000_p95: number;
  de2000_p99: number;
  /** Mean / p95 / p99 CIE76: the renderer's `compare` mean_delta_e / p95_delta_e / p99_delta_e. */
  de76: number;
  de76_p95: number;
  de76_p99: number;
  /** Mean SSIM over the region's pixels (each pixel's SSIM from its 11 x 11 window over the whole image). */
  ssim: number;
  /** Mean linear luminance of the test over the reference's. */
  luminance_ratio: number;
  /** Share of pixels with ΔE2000 above 5 (clearly visible). */
  over5: number;
  pixels: number;
}

export interface Bars {
  /** Highest mean ΔE2000 that passes. */
  de2000?: number;
  /** Highest mean ΔE76 that passes. */
  de76?: number;
  /** Lowest SSIM that passes. */
  ssim?: number;
}

export interface Result {
  width: number;
  height: number;
  /** Gain applied to the test's linear RGB before measuring (1 unless aligned), and the same in EV. */
  gain: number;
  overall: Stats;
  regions: (Stats & { name: string; rect: Rect })[];
  /** The worst region by mean ΔE2000. */
  worst: string | null;
  /** Present when bars were given: whether `overall` meets them, and which it misses. */
  pass?: boolean;
  failed?: string[];
}

export interface Maps {
  de2000: Float32Array;
  de76: Float32Array;
  ssim: Float32Array;
}

const toLinear = new Float64Array(256).map((_, i) => {
  const c = i / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
});

/** CIELAB (D65) of a linear sRGB colour, as compare.rs `lab`. */
export function linearToLab(r: number, g: number, b: number): [number, number, number] {
  const x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
  const y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  const z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
  const f = (t: number) => (t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116);
  const fx = f(x), fy = f(y), fz = f(z);
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
}

/** CIEDE2000 between two Lab colours (kL = kC = kH = 1). */
export function ciede2000(L1: number, a1: number, b1: number, L2: number, a2: number, b2: number): number {
  const rad = Math.PI / 180;
  const C1 = Math.hypot(a1, b1), C2 = Math.hypot(a2, b2);
  const Cb7 = ((C1 + C2) / 2) ** 7;
  const G = 0.5 * (1 - Math.sqrt(Cb7 / (Cb7 + 25 ** 7)));
  const a1p = (1 + G) * a1, a2p = (1 + G) * a2;
  const C1p = Math.hypot(a1p, b1), C2p = Math.hypot(a2p, b2);
  const hue = (b: number, a: number) => {
    if (a === 0 && b === 0) return 0;
    const h = Math.atan2(b, a) / rad;
    return h < 0 ? h + 360 : h;
  };
  const h1p = hue(b1, a1p), h2p = hue(b2, a2p);
  const dLp = L2 - L1;
  const dCp = C2p - C1p;
  let dhp = 0;
  if (C1p * C2p !== 0) {
    dhp = h2p - h1p;
    if (dhp > 180) dhp -= 360;
    else if (dhp < -180) dhp += 360;
  }
  const dHp = 2 * Math.sqrt(C1p * C2p) * Math.sin((dhp / 2) * rad);
  const Lbp = (L1 + L2) / 2;
  const Cbp = (C1p + C2p) / 2;
  let hbp = h1p + h2p;
  if (C1p * C2p !== 0) {
    if (Math.abs(h1p - h2p) <= 180) hbp /= 2;
    else hbp = h1p + h2p < 360 ? (hbp + 360) / 2 : (hbp - 360) / 2;
  }
  const T = 1 - 0.17 * Math.cos((hbp - 30) * rad) + 0.24 * Math.cos(2 * hbp * rad) + 0.32 * Math.cos((3 * hbp + 6) * rad) - 0.2 * Math.cos((4 * hbp - 63) * rad);
  const dTheta = 30 * Math.exp(-(((hbp - 275) / 25) ** 2));
  const Cbp7 = Cbp ** 7;
  const RC = 2 * Math.sqrt(Cbp7 / (Cbp7 + 25 ** 7));
  const l50 = (Lbp - 50) ** 2;
  const SL = 1 + (0.015 * l50) / Math.sqrt(20 + l50);
  const SC = 1 + 0.045 * Cbp;
  const SH = 1 + 0.015 * Cbp * T;
  const RT = -Math.sin(2 * dTheta * rad) * RC;
  const l = dLp / SL, c = dCp / SC, h = dHp / SH;
  return Math.sqrt(l * l + c * c + h * h + RT * c * h);
}

/** Gaussian SSIM map on two luma planes, as compare.rs `ssim` (whose result is this map's mean). */
export function ssimMap(a: Float64Array, b: Float64Array, w: number, h: number): Float32Array {
  const sigma = 1.5, r = 5;
  const k: number[] = [];
  for (let i = -r; i <= r; i++) k.push(Math.exp(-(i * i) / (2 * sigma * sigma)));
  const ks = k.reduce((s, v) => s + v, 0);
  for (let i = 0; i < k.length; i++) k[i] /= ks;
  const tmp = new Float64Array(w * h);
  const blur = (src: Float64Array): Float64Array => {
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++) {
        let s = 0;
        for (let j = 0; j < k.length; j++) s += src[y * w + Math.min(Math.max(x + j - r, 0), w - 1)] * k[j];
        tmp[y * w + x] = s;
      }
    const out = new Float64Array(w * h);
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++) {
        let s = 0;
        for (let j = 0; j < k.length; j++) s += tmp[Math.min(Math.max(y + j - r, 0), h - 1) * w + x] * k[j];
        out[y * w + x] = s;
      }
    return out;
  };
  const n = w * h;
  const prod = (p: Float64Array, q: Float64Array) => {
    const o = new Float64Array(n);
    for (let i = 0; i < n; i++) o[i] = p[i] * q[i];
    return o;
  };
  const ma = blur(a), mb = blur(b);
  const saa = blur(prod(a, a)), sbb = blur(prod(b, b)), sab = blur(prod(a, b));
  const c1 = (0.01 * 255) ** 2, c2 = (0.03 * 255) ** 2;
  const map = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const mua = ma[i], mub = mb[i];
    const va = saa[i] - mua * mua, vb = sbb[i] - mub * mub, cov = sab[i] - mua * mub;
    map[i] = ((2 * mua * mub + c1) * (2 * cov + c2)) / ((mua * mua + mub * mub + c1) * (va + vb + c2));
  }
  return map;
}

/** Per output pixel along one axis: the first input pixel and the normalised Lanczos 3 weights, as the image crate's
 *  `vertical_sample` / `horizontal_sample` (image 0.25) compute them, in f32. */
function lanczosTaps(size: number, newSize: number): { left: number; ws: Float32Array }[] {
  const f = Math.fround;
  const sinc = (t: number) => (t === 0 ? 1 : f(f(Math.sin(f(t * f(Math.PI)))) / f(t * f(Math.PI))));
  const kernel = (x: number) => (Math.abs(x) < 3 ? f(sinc(x) * sinc(f(x / 3))) : 0);
  const ratio = f(size / newSize), sratio = ratio < 1 ? 1 : ratio, support = f(3 * sratio);
  const taps: { left: number; ws: Float32Array }[] = [];
  for (let o = 0; o < newSize; o++) {
    const input = f(f(o + 0.5) * ratio);
    const left = Math.min(Math.max(Math.floor(f(input - support)), 0), size - 1);
    const right = Math.min(Math.max(Math.ceil(f(input + support)), left + 1), size);
    const centre = f(input - 0.5);
    const ws = new Float32Array(right - left);
    let sum = 0;
    for (let i = left; i < right; i++) sum = f(sum + (ws[i - left] = kernel(f(f(i - centre) / sratio))));
    for (let k = 0; k < ws.length; k++) ws[k] = f(ws[k] / sum);
    taps.push({ left, ws });
  }
  return taps;
}

/** Lanczos 3 resize of `img` to w x h (the reference's size), for a test rendered at another resolution: the renderer's
 *  `compare` resize (`image::imageops::resize`, FilterType::Lanczos3), so a test at another size reads the same there
 *  and here. Vertical pass first into f32, then horizontal, clamped and rounded to 8 bits. */
export function resize(img: Rgb8, w: number, h: number): Rgb8 {
  if (img.width === w && img.height === h) return img;
  const f = Math.fround;
  const W = img.width, H = img.height;
  const tmp = new Float32Array(W * h * 3);
  for (const [y, { left, ws }] of lanczosTaps(H, h).entries())
    for (let x = 0; x < W; x++)
      for (let c = 0; c < 3; c++) {
        let t = 0;
        for (let k = 0; k < ws.length; k++) t = f(t + f(img.data[((left + k) * W + x) * 3 + c] * ws[k]));
        tmp[(y * W + x) * 3 + c] = t;
      }
  const out = new Uint8Array(w * h * 3);
  for (const [x, { left, ws }] of lanczosTaps(W, w).entries())
    for (let y = 0; y < h; y++)
      for (let c = 0; c < 3; c++) {
        let t = 0;
        for (let k = 0; k < ws.length; k++) t = f(t + f(tmp[(y * W + left + k) * 3 + c] * ws[k]));
        out[(y * w + x) * 3 + c] = Math.round(Math.min(Math.max(t, 0), 255));
      }
  return { width: w, height: h, data: out };
}

/** A cols x rows grid of regions named "r<row>c<col>" (row 0 at the top). */
export function grid(cols: number, rows: number): Region[] {
  const out: Region[] = [];
  for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) out.push({ name: `r${r}c${c}`, rect: [c / cols, r / rows, (c + 1) / cols, (r + 1) / rows] });
  return out;
}

/** Pixel bounds [x0, y0, x1, y1) of a rect on a w x h image (at least one pixel). */
export function pixelRect([u0, v0, u1, v1]: Rect, w: number, h: number): [number, number, number, number] {
  const x0 = Math.min(Math.max(Math.floor(u0 * w), 0), w - 1), y0 = Math.min(Math.max(Math.floor(v0 * h), 0), h - 1);
  return [x0, y0, Math.max(Math.min(Math.ceil(u1 * w), w), x0 + 1), Math.max(Math.min(Math.ceil(v1 * h), h), y0 + 1)];
}

const round = (v: number, d: number) => Math.round(v * 10 ** d) / 10 ** d;

function quantile(sorted: Float32Array, q: number): number {
  return sorted.length ? sorted[Math.min(Math.floor(sorted.length * q), sorted.length - 1)] : NaN;
}

export interface MeasureOptions {
  regions?: Region[];
  /** Scale the test's linear RGB by one gain so its mean luminance matches the reference's (exposure taken out). */
  alignLuminance?: boolean;
  bars?: Bars;
}

/** Everything measured on `test` against `reference`, and the per-pixel maps for the heat maps. */
export function measure(reference: Rgb8, test: Rgb8, o: MeasureOptions = {}): { result: Result; maps: Maps } {
  const { width: w, height: h } = reference;
  test = resize(test, w, h);
  const n = w * h;
  const A = reference.data, B = test.data;
  const lum = (d: Uint8Array, i: number) => 0.2126 * toLinear[d[i]] + 0.7152 * toLinear[d[i + 1]] + 0.0722 * toLinear[d[i + 2]];
  let gain = 1;
  if (o.alignLuminance) {
    let la = 0, lb = 0;
    for (let i = 0; i < n * 3; i += 3) (la += lum(A, i)), (lb += lum(B, i));
    gain = lb > 0 ? la / lb : 1;
  }
  const encode = (c: number) => {
    const v = c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055;
    return Math.round(Math.min(Math.max(v, 0), 1) * 255);
  };
  const de2000 = new Float32Array(n), de76 = new Float32Array(n);
  const la = new Float64Array(n), lb = new Float64Array(n);
  const ya = new Float64Array(n), yb = new Float64Array(n);
  for (let p = 0; p < n; p++) {
    const i = p * 3;
    const ar = toLinear[A[i]], ag = toLinear[A[i + 1]], ab = toLinear[A[i + 2]];
    let br = toLinear[B[i]], bg = toLinear[B[i + 1]], bb = toLinear[B[i + 2]];
    if (gain !== 1) (br = Math.min(br * gain, 1)), (bg = Math.min(bg * gain, 1)), (bb = Math.min(bb * gain, 1));
    const [L1, a1, b1] = linearToLab(ar, ag, ab);
    const [L2, a2, b2] = linearToLab(br, bg, bb);
    de76[p] = Math.hypot(L1 - L2, a1 - a2, b1 - b2);
    de2000[p] = ciede2000(L1, a1, b1, L2, a2, b2);
    la[p] = 0.2126 * ar + 0.7152 * ag + 0.0722 * ab;
    lb[p] = 0.2126 * br + 0.7152 * bg + 0.0722 * bb;
    ya[p] = 0.299 * A[i] + 0.587 * A[i + 1] + 0.114 * A[i + 2];
    yb[p] = gain === 1 ? 0.299 * B[i] + 0.587 * B[i + 1] + 0.114 * B[i + 2] : 0.299 * encode(br) + 0.587 * encode(bg) + 0.114 * encode(bb);
  }
  const ssim = ssimMap(ya, yb, w, h);
  const stats = (rect: Rect): Stats => {
    const [x0, y0, x1, y1] = pixelRect(rect, w, h);
    const m = (x1 - x0) * (y1 - y0);
    const d2 = new Float32Array(m), d7 = new Float32Array(m);
    let s2 = 0, s7 = 0, ss = 0, sa = 0, sb = 0, over = 0, k = 0;
    for (let y = y0; y < y1; y++)
      for (let x = x0; x < x1; x++, k++) {
        const p = y * w + x;
        d2[k] = de2000[p];
        d7[k] = de76[p];
        s2 += de2000[p];
        s7 += de76[p];
        ss += ssim[p];
        sa += la[p];
        sb += lb[p];
        if (de2000[p] > 5) over++;
      }
    d2.sort();
    d7.sort();
    return {
      de2000: round(s2 / m, 3),
      de2000_p95: round(quantile(d2, 0.95), 2),
      de2000_p99: round(quantile(d2, 0.99), 2),
      de76: round(s7 / m, 3),
      de76_p95: round(quantile(d7, 0.95), 2),
      de76_p99: round(quantile(d7, 0.99), 2),
      ssim: round(ss / m, 4),
      luminance_ratio: round(sb / Math.max(sa, 1e-9), 3),
      over5: round(over / m, 4),
      pixels: m,
    };
  };
  const overall = stats([0, 0, 1, 1]);
  const regions = (o.regions ?? []).map((r) => ({ name: r.name, rect: r.rect, ...stats(r.rect) }));
  const worst = regions.length ? regions.reduce((a, b) => (b.de2000 > a.de2000 ? b : a)).name : null;
  const result: Result = { width: w, height: h, gain: round(gain, 4), overall, regions, worst };
  if (o.bars) {
    const failed = check(overall, o.bars);
    result.pass = failed.length === 0;
    result.failed = failed;
  }
  return { result, maps: { de2000, de76, ssim } };
}

/** The bars `s` misses, as readable strings (empty when it passes). */
export function check(s: Stats, bars: Bars): string[] {
  const f: string[] = [];
  if (bars.de2000 !== undefined && !(s.de2000 <= bars.de2000)) f.push(`ΔE2000 ${s.de2000} > ${bars.de2000}`);
  if (bars.de76 !== undefined && !(s.de76 <= bars.de76)) f.push(`ΔE76 ${s.de76} > ${bars.de76}`);
  if (bars.ssim !== undefined && !(s.ssim >= bars.ssim)) f.push(`SSIM ${s.ssim} < ${bars.ssim}`);
  return f;
}

/** Black -> red -> yellow -> white for t in 0..1, as compare.rs `heat`. */
export function heat(t: number): [number, number, number] {
  const c = (v: number) => Math.floor(Math.min(Math.max(v, 0), 1) * 255);
  return [c(t * 3), c(t * 3 - 1), c(t * 3 - 2)];
}

export interface HeatOptions {
  /** ΔE2000 at full scale (white). */
  scale?: number;
  /** Region outlines drawn over the heat map (cyan), the worst one in magenta. */
  regions?: Region[];
  worst?: string | null;
}

/** The ΔE2000 heat map as an RGB image of the reference's size. */
export function heatImage(map: Float32Array, w: number, h: number, o: HeatOptions = {}): Rgb8 {
  const scale = o.scale ?? 10;
  const data = new Uint8Array(w * h * 3);
  for (let p = 0; p < w * h; p++) data.set(heat(map[p] / scale), p * 3);
  for (const r of o.regions ?? []) {
    const [x0, y0, x1, y1] = pixelRect(r.rect, w, h);
    const col = r.name === o.worst ? [255, 0, 255] : [0, 200, 255];
    const put = (x: number, y: number) => data.set(col, (y * w + x) * 3);
    for (let x = x0; x < x1; x++) put(x, y0), put(x, y1 - 1);
    for (let y = y0; y < y1; y++) put(x0, y), put(x1 - 1, y);
  }
  return { width: w, height: h, data };
}

/** reference | test | heat side by side (the renderer's `compare --sheet` layout). */
export function sheet(reference: Rgb8, test: Rgb8, heatMap: Rgb8): Rgb8 {
  const { width: w, height: h } = reference;
  test = resize(test, w, h);
  const data = new Uint8Array(w * 3 * h * 3);
  for (let y = 0; y < h; y++)
    [reference, test, heatMap].forEach((img, k) => data.set(img.data.subarray(y * w * 3, (y + 1) * w * 3), (y * w * 3 + k * w) * 3));
  return { width: w * 3, height: h, data };
}

/** Parse "name=u0,v0,u1,v1". */
export function parseRegion(s: string): Region {
  const m = /^([^=]+)=(.+)$/.exec(s);
  if (!m) throw new Error(`region "${s}": expected name=u0,v0,u1,v1`);
  const v = m[2].split(",").map(Number);
  if (v.length !== 4 || v.some((x) => !Number.isFinite(x) || x < 0 || x > 1) || v[2] <= v[0] || v[3] <= v[1])
    throw new Error(`region "${s}": expected four numbers in 0..1 with u0 < u1, v0 < v1`);
  return { name: m[1], rect: v as Rect };
}

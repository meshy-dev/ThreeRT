// Shared pieces of the three.js vs ThreeRT comparison (docs/COMPARE.md): the native live viewport runner, image
// metrics, labelled montages (drawn by headless Chrome on a canvas, so no image library is needed) and line counts.
import { execFileSync, spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { rendererBin } from "../lib/bench.ts";
import { type Browser, root } from "../three/browser.ts";

export const bin = rendererBin;
export const outDir = join(root, "out/compare");

export interface LiveShot {
  frames: number;
  png: string;
  gpu_ms: number;
  scale: number;
  spp: number;
}

/** The live viewport (60 fps pacing, `scale` = auto for dynamic resolution or a fixed render scale): settle `warmup`
 *  seconds, reset the history, snapshot after each of `frames` frames on a still camera. */
export async function runLive(bundle: string, outPrefix: string, o: { width: number; height: number; scale: string; frames: number[]; warmup?: number; camera?: { position: number[]; target: number[]; fov: number }; assets?: string }): Promise<Record<number, LiveShot>> {
  const p = spawn(bin, ["live", bundle, "--assets", o.assets ?? root, "--width", String(o.width), "--height", String(o.height), "--fps", "60", "--scale", o.scale], { stdio: ["pipe", "ignore", "pipe"] });
  const send = (c: object) => p.stdin.write(JSON.stringify(c) + "\n");
  const got: Record<number, LiveShot> = {};
  const done = new Promise<void>((ok, fail) => {
    const t = setTimeout(() => fail(new Error(`live ${outPrefix}: timed out`)), 240_000);
    p.on("exit", (c) => (Object.keys(got).length < o.frames.length ? fail(new Error(`live ${outPrefix}: exited ${c}`)) : undefined));
    createInterface({ input: p.stderr! }).on("line", (l) => {
      const m = /snapshot .*-live-(\d+)\.png at history (\d+) \((\d+) spp\/frame, GPU ([\d.]+) ms(?:, scale ([\d.]+))?\)/.exec(l);
      if (!m) return;
      const k = Number(m[1]);
      got[k] = { frames: k, png: `${outPrefix}-live-${k}.png`, spp: Number(m[3]), gpu_ms: Number(m[4]), scale: Number(m[5] ?? 1) };
      if (Object.keys(got).length === o.frames.length) {
        clearTimeout(t);
        ok();
      }
    });
  });
  if (o.camera) send({ cmd: "camera", pos: o.camera.position, target: o.camera.target, tan: Math.tan((o.camera.fov * Math.PI) / 360) });
  await new Promise((r) => setTimeout(r, (o.warmup ?? 8) * 1000));
  send({ cmd: "reset" });
  for (const k of o.frames) send({ cmd: "snapshot", path: `${outPrefix}-live-${k}.png`, after: k });
  await done;
  send({ cmd: "quit" });
  await new Promise((r) => p.once("exit", r));
  return got;
}

export interface Metrics {
  mean_delta_e: number;
  psnr_db: number;
  ssim: number;
}
/** `threert-render compare`: mean ΔE76 / PSNR / SSIM of `img` against `ref`. */
export const compare = (ref: string | null, img: string): Metrics | null => (ref && existsSync(ref) && existsSync(img) ? JSON.parse(execFileSync(bin, ["compare", ref, img]).toString()) : null);

/** Brightness-aligned copy of `img` (written to `out`): one gain on linear RGB so its mean luminance matches `ref`'s,
 *  i.e. the exposure difference taken out. Returns the gain in EV (log2). Runs in headless Chrome (canvas decode). */
export async function alignExposure(browser: Browser, origin: string, ref: string, img: string, out: string): Promise<number> {
  const html = `<!doctype html><body><script type="module">
const load = (src) => new Promise((ok, fail) => { const i = new Image(); i.onload = () => ok(i); i.onerror = fail; i.src = src; });
const [a, b] = await Promise.all([load(${JSON.stringify("/" + relative(root, ref))}), load(${JSON.stringify("/" + relative(root, img))})]);
const px = (im) => { const c = document.createElement("canvas"); c.width = b.width; c.height = b.height; const g = c.getContext("2d"); g.drawImage(im, 0, 0, b.width, b.height); return [c, g, g.getImageData(0, 0, b.width, b.height)]; };
const lin = new Float32Array(256).map((_, i) => { const v = i / 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; });
const enc = (v) => Math.round(255 * Math.min(1, v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055));
const mean = (d) => { let s = 0; for (let i = 0; i < d.length; i += 4) s += 0.2126 * lin[d[i]] + 0.7152 * lin[d[i + 1]] + 0.0722 * lin[d[i + 2]]; return s / (d.length / 4); };
const [, , ra] = px(a), [cb, gb, rb] = px(b);
const k = mean(ra.data) / Math.max(1e-6, mean(rb.data));
for (let i = 0; i < rb.data.length; i += 4) for (let c = 0; c < 3; c++) rb.data[i + c] = enc(lin[rb.data[i + c]] * k);
gb.putImageData(rb, 0, 0);
window.__result = { ok: true, ev: Math.log2(k), png: cb.toDataURL("image/png") };
</script>`;
  const page = join(outDir, "align.html");
  writeFileSync(page, html);
  const r = await browser.run(`${origin}/${relative(root, page)}`, 60_000);
  writeFileSync(out, Buffer.from(String(r.png).split(",")[1], "base64"));
  return r.ev;
}

export const readJson = (p: string, d: any) => (existsSync(p) ? JSON.parse(readFileSync(p, "utf8")) : d);

/** Lines of code a person writes: non-blank, non-comment lines. */
export function codeLines(file: string): number {
  return readFileSync(file, "utf8").split("\n").filter((l) => l.trim() && !l.trim().startsWith("//")).length;
}

export interface MontageCell {
  img: string | null;
  /** Caption lines drawn under the tile (first line bold). */
  caption: string[];
}
export interface MontageSpec {
  title: string;
  subtitle?: string;
  columns: string[];
  rows: { label: string; cells: MontageCell[] }[];
  tileWidth: number;
  footer?: string[];
  /** JPEG quality (default 0.9). */
  quality?: number;
}

/** Draws a labelled grid of images in headless Chrome and writes it as a JPEG (`out`). */
export async function montage(browser: Browser, origin: string, out: string, spec: MontageSpec): Promise<void> {
  const url = (p: string | null) => (p ? "/" + relative(root, p) : null);
  const data = { ...spec, rows: spec.rows.map((r) => ({ ...r, cells: r.cells.map((c) => ({ ...c, img: url(c.img) })) })) };
  const html = `<!doctype html><meta charset="utf-8"><body style="margin:0"><script type="module">
const S = ${JSON.stringify(data)};
const load = (src) => new Promise((ok) => { if (!src) return ok(null); const i = new Image(); i.onload = () => ok(i); i.onerror = () => ok(null); i.src = src; });
const W = S.tileWidth, H = Math.round(W * 9 / 16), pad = 10, labelW = 150, capH = 46, headH = S.subtitle ? 78 : 52, colH = 30;
const footH = (S.footer?.length ?? 0) * 20 + (S.footer?.length ? 16 : 0);
const cv = document.createElement("canvas");
cv.width = labelW + S.columns.length * (W + pad) + pad;
cv.height = headH + colH + S.rows.length * (H + capH + pad) + footH + pad;
document.body.appendChild(cv);
const g = cv.getContext("2d");
const font = (px, bold) => (g.font = (bold ? "600 " : "") + px + "px -apple-system, 'PingFang SC', 'Helvetica Neue', sans-serif");
g.fillStyle = "#16181c"; g.fillRect(0, 0, cv.width, cv.height);
g.fillStyle = "#fff"; font(24, true); g.fillText(S.title, pad + 4, 34);
if (S.subtitle) { g.fillStyle = "#aab"; font(15); g.fillText(S.subtitle, pad + 4, 60); }
S.columns.forEach((c, j) => { g.fillStyle = "#e8e8f0"; font(16, true); g.fillText(c, labelW + pad + j * (W + pad), headH + 20); });
for (let i = 0; i < S.rows.length; i++) {
  const r = S.rows[i], y = headH + colH + i * (H + capH + pad);
  g.fillStyle = "#e8e8f0"; font(16, true);
  r.label.split("\\n").forEach((l, k) => { if (k) { g.fillStyle = "#99a"; font(13); } g.fillText(l, pad + 4, y + 22 + k * 18); });
  for (let j = 0; j < r.cells.length; j++) {
    const c = r.cells[j], x = labelW + pad + j * (W + pad);
    const im = await load(c.img);
    if (im) g.drawImage(im, x, y, W, H); else { g.fillStyle = "#2a2d33"; g.fillRect(x, y, W, H); g.fillStyle = "#777"; font(14); g.fillText("n/a", x + W / 2 - 10, y + H / 2); }
    c.caption.forEach((l, k) => { g.fillStyle = k ? "#b8bcc8" : "#fff"; font(k ? 13 : 14, !k); g.fillText(l, x + 2, y + H + 18 + k * 17); });
  }
}
(S.footer ?? []).forEach((l, k) => { g.fillStyle = "#99a"; font(13); g.fillText(l, pad + 4, cv.height - footH - pad + 16 + k * 20); });
window.__result = { ok: true, jpg: cv.toDataURL("image/jpeg", S.quality ?? 0.9) };
</script>`;
  const page = join(outDir, "montage.html");
  writeFileSync(page, html);
  const r = await browser.run(`${origin}/${relative(root, page)}`, 120_000);
  writeFileSync(out, Buffer.from(String(r.jpg).split(",")[1], "base64"));
}

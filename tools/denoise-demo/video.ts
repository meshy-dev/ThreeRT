// The denoiser demo's videos from tools/denoise-demo/record.ts's frames (ffmpeg):
//   split: the whole frame, ThreeRT's denoiser on the left half, MetalFX's on the right (the same frame), at 60 fps:
//          the speed it was rendered at.
//   3up:   1:1 crops of the middle of the frame: the input (the trace, not denoised), ThreeRT's, MetalFX's, at half speed.
//   node tools/denoise-demo/video.ts [--from 0 --to 659] [--fps-builtin 49 --fps-metalfx 53]
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const dir = resolve(flag("--dir", join(root, "out/kaleido-demo")));
const out = join(dir, "video");
mkdirSync(out, { recursive: true });
const from = Number(flag("--from", "0")), to = Number(flag("--to", "659"));
const font = flag("--font", "/System/Library/Fonts/Hiragino Sans GB.ttc");
const fpsOf = (m: string) => flag(`--fps-${m}`, "");
const enc = () => ["-frames:v", String(to - from + 1), "-c:v", "libx264", "-preset", "slow", "-crf", "14", "-pix_fmt", "yuv420p", "-movflags", "+faststart"];
const input = (m: string, rate: number) => ["-framerate", String(rate), "-start_number", String(from), "-i", join(dir, m, "f%04d.png")];
// Labels: a transparent PNG drawn by overlay.py (this machine's ffmpeg has no drawtext), laid over the frame.
type Label = { text: string; x: number; y: number; size: number; align?: "left" | "right" | "center" };
let n = 0;
const labels = (w: number, h: number, ls: Label[]) => {
  const f = join(out, `labels-${n++}.png`);
  execFileSync("python3", ["-I", join(import.meta.dirname, "overlay.py"), f, String(w), String(h), font, JSON.stringify(ls)]);
  return f;
};
const ffmpeg = (a: string[]) => execFileSync("ffmpeg", ["-y", "-loglevel", "error", ...a], { stdio: "inherit" });
const fps = (m: string) => (fpsOf(m) ? `  ·  ${fpsOf(m)} fps` : "");

// Split screen: the left half of ThreeRT's frame beside the right half of MetalFX's.
const caption = "同一帧 · 输出 1920×1080 · 渲染 960×540（50%）· 每像素每帧 1 条光线 · 按渲染速度 60 fps 播放";
const splitLabels = (cap: boolean) => labels(1920, 1080, [
  { text: `ThreeRT 内置降噪${cap ? fps("builtin") : ""}`, x: 28, y: 28, size: 40 },
  { text: `MetalFX 联合降噪${cap ? fps("metalfx") : ""}`, x: 28, y: 28, size: 40, align: "right" },
  ...(cap ? [{ text: caption, x: 0, y: -30, size: 26, align: "center" as const }] : []),
]);
const split = "[0]crop=960:1080:0:0[l];[1]crop=960:1080:960:0[r];[l][r]hstack=2,drawbox=x=958:y=0:w=4:h=1080:color=white@0.85:t=fill[v];[v][2]overlay=0:0";
ffmpeg([...input("builtin", 60), ...input("metalfx-denoised", 60), "-i", splitLabels(true), "-filter_complex", split,
  ...enc(), join(out, "kaleido-split.mp4")]);

// Three 1:1 crops of the middle (no resampling: each method's own pixels), at half speed.
const [cw, ch, cx, cy] = [640, 720, 640, 260];
const threeLabels = (cap: boolean) => labels(1920, 720, [
  { text: "输入：光追原始信号（未降噪）", x: 24, y: 24, size: 30 },
  { text: "ThreeRT 内置降噪", x: 664, y: 24, size: 30 },
  { text: "MetalFX 联合降噪", x: 1304, y: 24, size: 30 },
  ...(cap ? [{ text: "画面中央 1:1 裁切 · 半速播放", x: 0, y: -24, size: 24, align: "center" as const }] : []),
]);
const three = [0, 1, 2].map((i) => `[${i}]crop=${cw}:${ch}:${cx}:${cy}[c${i}]`).join(";") + ";[c0][c1][c2]hstack=3," +
  "drawbox=x=638:y=0:w=4:h=720:color=white@0.85:t=fill,drawbox=x=1278:y=0:w=4:h=720:color=white@0.85:t=fill[v];[v][3]overlay=0:0";
ffmpeg([...input("off", 30), ...input("builtin", 30), ...input("metalfx-denoised", 30), "-i", threeLabels(true), "-filter_complex", three,
  ...enc(), join(out, "kaleido-3up.mp4")]);

// Stills of both at a few moments.
for (const k of [222, 300, 480]) {
  const f = (m: string) => ["-i", join(dir, m, `f${String(k).padStart(4, "0")}.png`)];
  ffmpeg([...f("builtin"), ...f("metalfx-denoised"), "-i", splitLabels(false), "-filter_complex", split, "-frames:v", "1", join(out, `kaleido-split-f${k}.png`)]);
  ffmpeg([...f("off"), ...f("builtin"), ...f("metalfx-denoised"), "-i", threeLabels(false), "-filter_complex", three, "-frames:v", "1", join(out, `kaleido-3up-f${k}.png`)]);
}
console.log(`videos and stills in ${out}`);

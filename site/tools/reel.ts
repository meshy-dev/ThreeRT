// Lays the landing page's reel out and encodes it, from the clips tools/site-reel.ts recorded (out/site-reel/<clip>):
//   node tools/site-reel.ts all      (and the OpenStrike clips: site/README.md, Media)
//   bun site/tools/reel.ts [--crf 27] [--crf-sm 28]
// -> site/media/reel/reel.mp4      the grid for a wide screen: 4 x 3 clips, 1920 x 1080, 60 fps, an 8 s loop
//    site/media/reel/reel-tall.mp4 the grid for a phone held upright: 2 x 4 clips, 720 x 1440
//    site/media/reel/reel.webp, reel-tall.webp   their first frames: what the page shows until the reel plays
//    site/media/reel/reel.json     the clips in each grid, in order, with how each was recorded
// A clip is 8.5 s of frames: the last half second is faded over the first, so every tile loops without a cut.
// H.264, so every browser plays it. Needs ffmpeg on PATH. Sources can be moved with THREERT_OUT (default <repo>/out).
import { $ } from "bun";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import sharp from "sharp";

const site = resolve(import.meta.dir, "..");
const OUT = process.env.THREERT_OUT ?? join(site, "../out");
const src = join(OUT, "site-reel");
const media = join(site, "media/reel");
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);

/** The grids, row by row. The page's words sit over the left of the wide one and the top of the tall one. */
const WIDE = { cols: 4, rows: 3, tile: [480, 360], clips: ["crowd", "os-assault", "tokyo", "kabukicho", "home", "jewellery", "os-inferno", "museum", "voxels", "colour", "showroom", "os-dust2"] };
const TALL = { cols: 2, rows: 4, tile: [360, 360], clips: ["kabukicho", "os-assault", "home", "jewellery", "crowd", "os-inferno", "museum", "colour"] };
/** Between tiles: the page's ink, a pixel on each side. */
const GAP = 1, INK = "0x060709";

const meta = (id: string) => {
  const f = join(src, id, "meta.json");
  if (!existsSync(f)) throw new Error(`${id}: not recorded (${f} is missing)`);
  const m = JSON.parse(readFileSync(f, "utf8"));
  const n = readdirSync(join(src, id)).filter((x) => /^\d{4}\.png$/.test(x)).length;
  if (n !== m.frames) throw new Error(`${id}: ${n} frames, meta.json says ${m.frames}`);
  return m;
};

async function grid(name: string, g: typeof WIDE, crf: string) {
  const ms = g.clips.map(meta);
  const { fps, seconds, tail } = ms[0];
  if (ms.some((m) => m.fps !== fps || m.seconds !== seconds || m.tail !== tail)) throw new Error("the clips differ in length or frame rate");
  const main = Math.round(seconds * fps), [tw, th] = g.tile, [iw, ih] = [tw - 2 * GAP, th - 2 * GAP];
  const inputs = g.clips.flatMap((id) => ["-framerate", String(fps), "-i", join(src, id, "%04d.png")]);
  // A tile: the middle of the frame at the tile's shape, its tail faded over its start, the gap round it.
  const chains = g.clips.map((_, i) =>
    `[${i}:v]crop='min(iw,ih*${iw}/${ih})':'min(ih,iw*${ih}/${iw})',scale=${iw}:${ih}:flags=lanczos,split[m${i}][t${i}];` +
    `[m${i}]trim=end_frame=${main},setpts=PTS-STARTPTS[a${i}];` +
    `[t${i}]trim=start_frame=${main},setpts=PTS-STARTPTS,format=yuva444p,fade=t=out:st=0:d=${tail}:alpha=1[b${i}];` +
    `[a${i}][b${i}]overlay=eof_action=pass,pad=${tw}:${th}:${GAP}:${GAP}:color=${INK}[v${i}]`);
  const layout = g.clips.map((_, i) => `${(i % g.cols) * tw}_${Math.floor(i / g.cols) * th}`).join("|");
  const filter = `${chains.join(";")};${g.clips.map((_, i) => `[v${i}]`).join("")}xstack=inputs=${g.clips.length}:layout=${layout},format=yuv420p[out]`;
  const out = join(media, `${name}.mp4`);
  await $`ffmpeg -hide_banner -loglevel error -y ${inputs} -filter_complex ${filter} -map [out] -r ${fps} -c:v libx264 -preset slower -crf ${crf} -profile:v high -g ${fps} -color_primaries bt709 -color_trc bt709 -colorspace bt709 -movflags +faststart -an ${out}`;
  const still = join(src, `${name}-first.png`);
  await $`ffmpeg -hide_banner -loglevel error -y -i ${out} -frames:v 1 ${still}`;
  await sharp(still).webp({ quality: 84, effort: 6 }).toFile(join(media, `${name}.webp`));
  console.log(`${name}.mp4: ${g.cols} x ${g.rows} of ${tw} x ${th}, ${(statSync(out).size / 1048576).toFixed(1)} MiB`);
  return { width: g.cols * tw, height: g.rows * th, cols: g.cols, rows: g.rows, fps, seconds, clips: ms.map((m) => ({ id: m.id, what: m.what, width: m.width, height: m.height, scale: m.scale, machine: m.machine ?? null, commit: m.commit ?? null, gpu_ms: m.gpu_ms ?? null })) };
}

mkdirSync(media, { recursive: true });
const wide = await grid("reel", WIDE, opt("--crf", "27"));
const tall = await grid("reel-tall", TALL, opt("--crf-sm", "28"));
writeFileSync(join(media, "reel.json"), JSON.stringify({ at: new Date().toISOString(), wide, tall }, null, 1) + "\n");

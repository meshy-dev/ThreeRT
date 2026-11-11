// Encodes the landing page's hero clip from the frames tools/site-hero.ts recorded (out/site-hero/bistro):
//   node tools/site-hero.ts ...      (ThreeRT live + three.js along Bistro's camera move; site/README.md has the run)
//   bun site/tools/hero.ts [--crf 27] [--crf-sm 28]
// -> site/media/bistro/hero.mp4, hero-720.mp4        both sides in one file, three.js on top of ThreeRT, 60 fps, a
//                                                     loop (there and back); H.264 so every browser plays it.
//                                                     hero.mp4 is the size it was recorded at, hero-720.mp4 for phones
//    site/media/bistro/hero-three.webp, hero-rt.webp  the clip's first frame: what the page shows until it plays
//    site/media/bistro/hero.json                      how it was recorded (machine, size, scale) and, from the probe
//                                                     at the same settings, the GPU time a frame: the footnote's source
// One file for both sides, because two videos drift: js/threert.js draws each half onto its side of the divider.
// Needs ffmpeg on PATH. Sources can be moved with THREERT_OUT (default <repo>/out).
import { $ } from "bun";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import sharp from "sharp";

const site = resolve(import.meta.dir, "..");
const OUT = process.env.THREERT_OUT ?? join(site, "../out");
const src = join(OUT, "site-hero/bistro");
const media = join(site, "media/bistro");
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);

const meta = JSON.parse(readFileSync(join(src, "meta.json"), "utf8"));
const frames = (d: string, ext: string) => readdirSync(join(src, d)).filter((f) => new RegExp(`^\\d{4}\\.${ext}$`).test(f)).sort();
const rt = frames("threert", "png"), three = frames("three", "jpg");
const n = meta.poses as number;
if (three.length !== n) throw new Error(`three.js: ${three.length} frames, the recording has ${n} poses`);
if (rt.length !== 2 * n - 2) throw new Error(`ThreeRT: ${rt.length} frames, expected ${2 * n - 2} (there and back)`);

// three.js has no history: its way back is its way out, reversed (the same order ThreeRT was recorded in).
const order = [...three.keys(), ...Array.from({ length: n - 2 }, (_, k) => n - 2 - k)];
const list = join(src, "three.ffconcat");
writeFileSync(list, "ffconcat version 1.0\n" + order.map((k) => `file 'three/${three[k]}'\nduration ${1 / meta.fps}`).join("\n") + "\n");

mkdirSync(media, { recursive: true });
const encode = async (name: string, height: number, crf: string) => {
  const out = join(media, name);
  const scale = height === meta.height ? "" : `,scale=-2:${2 * height}:flags=lanczos`;
  // yuv420p + High profile + faststart: plays everywhere and starts before it has all arrived. A key frame a second.
  await $`ffmpeg -hide_banner -loglevel error -y -f concat -safe 0 -i ${list} -framerate ${meta.fps} -i ${join(src, "threert/%04d.png")} -filter_complex ${`[0:v][1:v]vstack=inputs=2${scale},format=yuv420p`} -r ${meta.fps} -c:v libx264 -preset slower -crf ${crf} -profile:v high -g ${meta.fps} -color_primaries bt709 -color_trc bt709 -colorspace bt709 -movflags +faststart -an ${out}`;
  return `${name} ${(statSync(out).size / 1048576).toFixed(1)} MiB`;
};
const sizes = [await encode("hero.mp4", meta.height, opt("--crf", "27")), await encode("hero-720.mp4", 720, opt("--crf-sm", "28"))];

await sharp(join(src, "three", three[0])).webp({ quality: 88, effort: 6 }).toFile(join(media, "hero-three.webp"));
await sharp(join(src, "threert", rt[0])).webp({ quality: 88, effort: 6 }).toFile(join(media, "hero-rt.webp"));
// The times to quote are the probe's (frames back to back, as on screen), when it ran at the recorded settings.
const probe = existsSync(join(src, "probe.json")) ? JSON.parse(readFileSync(join(src, "probe.json"), "utf8")) : null;
const same = probe && probe.width === meta.width && probe.height === meta.height && String(probe.option_scale) === String(meta.option_scale) && probe.bounces === meta.bounces;
if (!same) console.warn("warning: no probe at the recorded settings (node tools/site-hero.ts --probe ...): hero.json has no back-to-back GPU times");
const { threert, ...how } = meta;
writeFileSync(join(media, "hero.json"), JSON.stringify({ ...how, seconds_loop: +(rt.length / meta.fps).toFixed(2), realtime: same ? { at: probe.at, ...probe.threert } : null }, null, 1) + "\n");
console.log(`${sizes.join(", ")} -> ${media}`);

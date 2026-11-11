// Builds site/media from the benchmark outputs: the renders the landing page shows, re-encoded as WebP (sharp) and
// never retouched. Run after the comparisons it draws on:
//   node tools/milestone1.ts            -> out/milestone1/<scene>/ (reference, camera-move frames, metrics.json)
//   node tools/compare/strongest.ts     -> out/compare/strongest/  (Bistro / Zero-Day stills, both engines)
//   node tools/compare/hello.ts         -> out/compare/hello/      (hello-world stills)
//   bun site/tools/media.ts
// Sources can be moved with THREERT_OUT (default <repo>/out) and THREERT_COMPARE (default $THREERT_OUT/compare).
import { mkdirSync, readFileSync, readdirSync, copyFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import sharp from "sharp";

const site = resolve(import.meta.dir, "..");
const root = resolve(site, "..");
const OUT = process.env.THREERT_OUT ?? join(root, "out");
const CMP = process.env.THREERT_COMPARE ?? join(OUT, "compare");
const media = join(site, "media");

let n = 0;
async function webp(src: string, dst: string, o: { quality?: number; width?: number } = {}) {
  if (!existsSync(src)) throw new Error(`missing source: ${src}`);
  const out = join(media, dst);
  mkdirSync(dirname(out), { recursive: true });
  let img = sharp(src);
  if (o.width) img = img.resize(o.width);
  await img.webp({ quality: o.quality ?? 86, effort: 6 }).toFile(out);
  n++;
}

// 1. Bistro night and Zero-Day (milestone 1): the first pose of each scene's camera path, 1280 × 720.
for (const id of ["bistro", "zero-day"]) {
  const m1 = join(OUT, "milestone1", id);
  const metrics = JSON.parse(readFileSync(join(m1, "metrics.json"), "utf8"));
  // The unbiased offline render the run was judged against (its path is relative to the checkout that ran it).
  await webp(join(dirname(OUT), metrics.reference.png), `${id}/ref.webp`, { quality: 88 });
  const strong = join(CMP, "strongest");
  await webp(join(strong, `${id}-ours-auto-live-64.png`), `${id}/live.webp`, { quality: 88 });
  await webp(join(strong, `${id}-three-strongest-sky.png`), `${id}/strongest.webp`, { quality: 88 });
  await webp(join(strong, `${id}-three-tuned-sky.png`), `${id}/tuned.webp`, { quality: 88 });
  // The 6 s camera move along the scene's own path: 60 poses, both engines in lockstep, 960 px wide.
  for (const [dir, tag] of [["move-threert", "rt"], ["move-three", "three"]] as const) {
    const frames = readdirSync(join(m1, dir)).filter((f) => /^\d{4}\.(png|jpg)$/.test(f)).sort();
    if (frames.length !== 60) throw new Error(`${id}/${dir}: ${frames.length} frames, expected 60`);
    for (const [k, f] of frames.entries()) await webp(join(m1, dir, f), `${id}/move/${tag}-${String(k).padStart(2, "0")}.webp`, { quality: 74, width: 960 });
  }
}

// 2. Hello world: three.js as written vs ThreeRT with every setting at its default, and the ThreeRT source shown.
mkdirSync(join(media, "code"), { recursive: true });
for (const s of ["material-grid", "sun-room", "glb-asset"]) {
  await webp(join(CMP, "hello", `${s}-three-default.png`), `hello/${s}-three.webp`);
  await webp(join(CMP, "hello", `${s}-ours-live-64.png`), `hello/${s}-rt.webp`);
  copyFileSync(join(root, "compare/hello/threert", `${s}.ts`), join(media, "code", `${s}.threert.ts`));
}

console.log(`${n} images -> ${media} (device stills: bun site/tools/device-stills.ts)`);

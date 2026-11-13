// Bakes what the landing page shows of threert.app into the site: bun site/tools/platform.ts
//
// The page asks the platform for its apps when it loads (js/platform.js, index.html). What this writes is what stands
// until that answer comes, and for good where it does not: no JavaScript, a crawler, the platform out of reach.
//   site/index.html            the tiles between the two `apps:` comments (the same markup the page builds live)
//   site/media/apps/<name>.webp   each tile's cover, 640 x 360, so the grid costs a few dozen KiB instead of the
//                              platform's full-size covers; the page keeps a copy for as long as the app's cover is
//                              the one it was made from (the tile's data-cover)
//   site/media/openstrike/     OpenStrikeRT's cover (two widths) and two of its screenshots: the section's pictures
//                              are this site's own files, not another host's
// The pictures are the platform's own (first-party apps, or covers their creators published), re-encoded and nothing
// else. Run it when the catalogue has moved on (a new first-party app, a cover that changed), then commit.
import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import sharp from "sharp";
import { FEATURED, LIST, PLATFORM, pick, tile } from "../js/platform.js";

const site = resolve(import.meta.dir, "..");
const BEGIN = "<!-- apps:begin (site/tools/platform.ts) -->", END = "<!-- apps:end -->";
/** The featured game's screenshots the section shows next to its cover (1-based, as the platform numbers them). */
const SHOTS = [3, 6];

async function get(url: string): Promise<Response> {
  const r = await fetch(url, { signal: AbortSignal.timeout(60_000) });
  if (!r.ok) throw new Error(`${url}: ${r.status}`);
  return r;
}
const picture = async (url: string) => Buffer.from(await (await get(url)).arrayBuffer());
async function webp(src: Buffer, file: string, width: number, height: number, quality: number) {
  await sharp(src).resize(width, height, { fit: "cover" }).webp({ quality, effort: 6 }).toFile(file);
  return `${file.slice(site.length + 1)} (${Math.round(readFileSync(file).length / 1024)} KiB)`;
}

// 1. The tiles.
const apps = pick((await (await get(LIST)).json()).items);
if (!apps.length) throw new Error(`${LIST} lists nothing the page can show: index.html is left as it was`);
const dir = join(site, "media/apps");
mkdirSync(dir, { recursive: true });
for (const f of readdirSync(dir)) if (!apps.some((a) => `${a.name}.webp` === f)) rmSync(join(dir, f));
for (const a of apps) console.log(await webp(await picture(a.cover), join(dir, `${a.name}.webp`), 640, 360, 78));

const page = join(site, "index.html"), html = readFileSync(page, "utf8");
const from = html.indexOf(BEGIN), to = html.indexOf(END);
if (from < 0 || to < from) throw new Error(`site/index.html: no ${BEGIN} ... ${END}`);
const pad = "      ";
const baked = html.slice(0, from + BEGIN.length) + "\n" + apps.map((a) => pad + tile(a, `media/apps/${a.name}.webp`)).join("\n") + "\n" + pad + html.slice(to);
if (baked !== html) writeFileSync(page, baked);
console.log(`site/index.html: ${apps.map((a) => a.name).join(", ")}${baked === html ? " (unchanged)" : ""}`);

// 2. The featured game's pictures.
const game = (await (await get(`${PLATFORM}/api/v1/apps/${FEATURED}`)).json()).app;
const os = join(site, "media/openstrike");
mkdirSync(os, { recursive: true });
const cover = await picture(game.cover);
console.log(await webp(cover, join(os, "cover.webp"), 1600, 900, 80));
console.log(await webp(cover, join(os, "cover-800.webp"), 800, 450, 78));
for (const n of SHOTS) {
  const url = game.screenshots?.[n - 1];
  if (!url) throw new Error(`${FEATURED} has no screenshot ${n}`);
  console.log(await webp(await picture(url), join(os, `shot-${n}.webp`), 960, 540, 78));
}
console.log(`${game.title} ${game.version}${game.listed === false ? " (unlisted on the platform for now)" : ""}`);

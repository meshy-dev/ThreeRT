// A companion to the renderer's `threert-render compare`: where in the frame the error is. The headline columns are
// compare's (ΔE76 mean / p99, SSIM, the same numbers to the second decimal); on top of them, CIEDE2000, a table per
// region (a grid and named rects, the worst named), a heat map with the regions drawn on it, and bars that exit 1.
// Needs only Node (no renderer build, no GPU), so it also runs on captures from another machine.
//
//   node tools/compare/delta-e.ts <reference> <test> [options]
//   node tools/compare/delta-e.ts --manifest poses.json [options]
//
// Options:
//   --out <dir>              where the JSON and PNGs go (default out/delta-e)
//   --name <name>            file stem for a single pair (default: the test's file name)
//   --grid <cols>x<rows>     a grid of regions r<row>c<col> (default 3x3; 0x0 for none)
//   --region name=u0,v0,u1,v1  a named region (0..1, v down), repeatable: "street=0,0.6,1,1"
//   --align                  take the exposure out first: one gain on the test's linear RGB to the reference's luminance
//   --max-de2000 <v> --max-de76 <v> --min-ssim <v>   bars on the overall numbers; a miss exits 1
//   --bar step2              the streamed city's bar (ROADMAP § 0 step 2: ΔE ≤ 1.0, SSIM ≥ 0.98), ΔE read as ΔE76
//                            mean, the number `compare` reports; add --max-de2000 for a CIEDE2000 bar
//   --scale <v>              ΔE2000 at the heat map's full scale (default 10)
//
// A manifest measures several poses in one run (the streamed city's fixed poses, live against offline or streamed
// against everything resident) and adds a summary (mean and worst over the poses):
//   { "regions": [{ "name": "street", "rect": [0, 0.6, 1, 1] }],
//     "poses": [{ "name": "golden-gai", "reference": "out/cache/truth/a.png", "test": "out/tokyo/a-live.png" }] }
// Paths in a manifest are relative to the manifest's directory. A pose may carry its own "regions" (added to the
// manifest's and the grid).
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";
import { type Bars, type Region, type Result, type Rgb8, type Stats, check, grid, heatImage, measure, parseRegion, sheet } from "./image-metrics.ts";

const repoRoot = resolve(import.meta.dirname, "../..");

export async function readImage(path: string): Promise<Rgb8> {
  const { data, info } = await sharp(path).removeAlpha().toColourspace("srgb").raw().toBuffer({ resolveWithObject: true });
  if (info.channels !== 3) throw new Error(`${path}: ${info.channels} channels after removing alpha`);
  return { width: info.width, height: info.height, data: new Uint8Array(data.buffer, data.byteOffset, data.length) };
}

export const writePng = (img: Rgb8, path: string) => sharp(Buffer.from(img.data), { raw: { width: img.width, height: img.height, channels: 3 } }).png().toFile(path);

export interface Pair {
  name: string;
  reference: string;
  test: string;
  regions?: Region[];
}

export interface Options {
  out: string;
  regions: Region[];
  align: boolean;
  bars?: Bars;
  scale: number;
}

export interface PoseReport extends Result {
  name: string;
  reference: string;
  test: string;
  heat: string;
  sheet: string;
}

export interface Summary {
  poses: number;
  mean: Pick<Stats, "de2000" | "de76" | "ssim">;
  worst: { de2000: { pose: string; value: number }; ssim: { pose: string; value: number } };
  pass?: boolean;
  failed?: string[];
}

/** Measure one pair and write `<name>.json`, `<name>-heat.png` and `<name>-sheet.png` into `o.out`. */
export async function comparePair(p: Pair, o: Options): Promise<PoseReport> {
  const [ref, test] = await Promise.all([readImage(p.reference), readImage(p.test)]);
  const regions = [...o.regions, ...(p.regions ?? [])];
  const { result, maps } = measure(ref, test, { regions, alignLuminance: o.align, bars: o.bars });
  mkdirSync(o.out, { recursive: true });
  const heatPath = join(o.out, `${p.name}-heat.png`), sheetPath = join(o.out, `${p.name}-sheet.png`);
  const h = heatImage(maps.de2000, ref.width, ref.height, { scale: o.scale, regions, worst: result.worst });
  await Promise.all([writePng(h, heatPath), writePng(sheet(ref, test, h), sheetPath)]);
  const report: PoseReport = { name: p.name, reference: p.reference, test: p.test, heat: heatPath, sheet: sheetPath, ...result };
  writeFileSync(join(o.out, `${p.name}.json`), JSON.stringify(report, null, 2) + "\n");
  return report;
}

/** Mean and worst over poses; the bars, when given, apply to every pose. */
export function summarize(reports: PoseReport[], bars?: Bars): Summary {
  const mean = (k: "de2000" | "de76" | "ssim") => Math.round((reports.reduce((s, r) => s + r.overall[k], 0) / reports.length) * 1e4) / 1e4;
  const worstDe = reports.reduce((a, b) => (b.overall.de2000 > a.overall.de2000 ? b : a));
  const worstSsim = reports.reduce((a, b) => (b.overall.ssim < a.overall.ssim ? b : a));
  const s: Summary = {
    poses: reports.length,
    mean: { de2000: mean("de2000"), de76: mean("de76"), ssim: mean("ssim") },
    worst: { de2000: { pose: worstDe.name, value: worstDe.overall.de2000 }, ssim: { pose: worstSsim.name, value: worstSsim.overall.ssim } },
  };
  if (bars) {
    s.failed = reports.flatMap((r) => check(r.overall, bars).map((f) => `${r.name}: ${f}`));
    s.pass = s.failed.length === 0;
  }
  return s;
}

const fmt = (r: PoseReport) => {
  const regions = r.regions.length ? `  worst region ${r.worst} (ΔE2000 ${r.regions.find((g) => g.name === r.worst)!.de2000})` : "";
  const o = r.overall;
  return `${r.name}: ΔE76 / p99 / SSIM ${o.de76.toFixed(2)} / ${o.de76_p99.toFixed(1)} / ${o.ssim.toFixed(3)}  ΔE2000 ${o.de2000} (p95 ${o.de2000_p95})  luminance ${o.luminance_ratio}${regions}${r.pass === false ? `  FAIL: ${r.failed!.join(", ")}` : r.pass ? "  pass" : ""}`;
};

/** The command line; returns the exit code. Logs go to stderr, the JSON (one pair's report or the manifest's) to stdout. */
export async function main(argv: string[]): Promise<number> {
  const pos: string[] = [];
  const named: Region[] = [];
  let out = join(repoRoot, "out/delta-e"), name: string | undefined, manifest: string | undefined, gridSpec = "3x3", align = false, scale = 10;
  const bars: Bars = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      const v = argv[++i];
      if (v === undefined) throw new Error(`${a} needs a value`);
      return v;
    };
    const num = () => {
      const v = Number(val());
      if (!Number.isFinite(v)) throw new Error(`${a}: not a number`);
      return v;
    };
    if (a === "--out") out = resolve(val());
    else if (a === "--name") name = val();
    else if (a === "--manifest") manifest = resolve(val());
    else if (a === "--grid") gridSpec = val();
    else if (a === "--region") named.push(parseRegion(val()));
    else if (a === "--align") align = true;
    else if (a === "--max-de2000") bars.de2000 = num();
    else if (a === "--max-de76") bars.de76 = num();
    else if (a === "--min-ssim") bars.ssim = num();
    else if (a === "--bar") {
      const b = val();
      if (b !== "step2") throw new Error(`--bar ${b}: only step2 is known`);
      bars.de76 = 1.0;
      bars.ssim = 0.98;
    } else if (a === "--scale") scale = num();
    else if (a.startsWith("--")) throw new Error(`unknown option ${a}`);
    else pos.push(a);
  }
  const g = /^(\d+)x(\d+)$/.exec(gridSpec);
  if (!g) throw new Error(`--grid ${gridSpec}: expected <cols>x<rows>`);
  const regions = [...(+g[1] && +g[2] ? grid(+g[1], +g[2]) : []), ...named];
  const o: Options = { out, regions, align, bars: Object.keys(bars).length ? bars : undefined, scale };
  let pairs: Pair[];
  if (manifest) {
    const m = JSON.parse(readFileSync(manifest, "utf8"));
    const base = dirname(manifest);
    o.regions = [...regions, ...(m.regions ?? [])];
    pairs = (m.poses ?? []).map((p: Pair) => ({ ...p, reference: resolve(base, p.reference), test: resolve(base, p.test) }));
    if (!pairs.length) throw new Error(`${manifest}: no poses`);
  } else {
    if (pos.length !== 2) throw new Error("usage: delta-e.ts <reference> <test> [options] | --manifest poses.json [options]");
    pairs = [{ name: name ?? basename(pos[1], extname(pos[1])), reference: resolve(pos[0]), test: resolve(pos[1]) }];
  }
  const reports: PoseReport[] = [];
  for (const p of pairs) {
    const r = await comparePair(p, o);
    console.error(fmt(r));
    reports.push(r);
  }
  if (!manifest) {
    console.log(JSON.stringify(reports[0]));
    return reports[0].pass === false ? 1 : 0;
  }
  const summary = summarize(reports, o.bars);
  const all = { summary, poses: reports };
  writeFileSync(join(out, "summary.json"), JSON.stringify(all, null, 2) + "\n");
  console.error(`${summary.poses} poses: mean ΔE76 ${summary.mean.de76}  ΔE2000 ${summary.mean.de2000}  SSIM ${summary.mean.ssim}; worst ΔE2000 ${summary.worst.de2000.value} (${summary.worst.de2000.pose}), worst SSIM ${summary.worst.ssim.value} (${summary.worst.ssim.pose})${summary.pass === false ? "  FAIL" : summary.pass ? "  pass" : ""}`);
  console.log(JSON.stringify(all));
  return summary.pass === false ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (e) => {
      console.error(e instanceof Error ? e.message : e);
      process.exit(2);
    },
  );
}

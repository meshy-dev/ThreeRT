// Scene catalog for the playground client: every reference scene + playable game, with UE5 reference images and the
// measured metrics, so each scene can be opened live and A/B'd against UE5 during acceptance. Each entry runs from its
// bundle (out/bundles/<id>.js, node tools/build-scenes.ts).
//   node tools/catalog.ts            -> out/bundles/catalog.json   (also imported by the playground backend)
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(import.meta.dirname, "..");
export const bundlesDir = join(root, "out/bundles");
/** Interactive entries: catalog id (= UE reference name) -> game source. */
export const GAMES: Record<string, string> = {
  "kabukicho": "games/kabukicho/kabukicho.game.ts",
  // LOD / streaming acceptance (docs/ROADMAP.md § 5): the 3 x 3 district (37.8 M triangles, 3,197 instanced props) and
  // its middle row.
  "kabukicho-district": "games/kabukicho/district.game.ts",
  "kabukicho-district-row": "games/kabukicho/district-row.game.ts",
  "light-room": "games/light-room/light-room.game.ts",
  "mannequin-plaza": "games/mannequin/mannequin.game.ts",
  // Zero Day, Measure One, its FBX take played (ROADMAP § 0 step 8).
  "zero-day-animated": "games/zero-day/zero-day.game.ts",
  // The C slice on Bistro night (ROADMAP § 0 step 9): 60 s shot, rain from 15 s, explosion at 40 s.
  "bistro-slice": "games/bistro-slice/bistro-slice.game.ts",
  // The district on Bistro (ROADMAP § 0 step 10): 3 x 3 copies, 25.5 M triangles.
  "bistro-district": "games/bistro-district/bistro-district.game.ts",
  // three.js's official instancing examples, their code unchanged (ROADMAP § 6 step 2e): 1,000 instances turning.
  "instancing": "games/instancing/instancing.game.ts",
  // Physics hello world (ROADMAP § 6 step 7): click adds a box with a rigid body, click a box kicks it.
  "physics-hello": "games/physics/hello.game.ts",
};
const TITLES: Record<string, string> = { "bistro-district": "Bistro District (3 × 3)", "bistro-slice": "Bistro Night · C slice", "zero-day-animated": "Zero Day (animated)", kabukicho: "Kabukichō Night", "kabukicho-district": "Kabukichō District (3 × 3)", "kabukicho-district-row": "Kabukichō District (middle row)" };

/** Catalog id -> absolute source path: the reference scenes (scenes/*.ts) and the games. */
export function sources(): Record<string, string> {
  const scenes = readdirSync(join(root, "scenes")).filter((f) => f.endsWith(".ts")).map((f) => [f.replace(/\.ts$/, ""), join(root, "scenes", f)]);
  return Object.fromEntries([...scenes, ...Object.entries(GAMES).map(([id, src]) => [id, join(root, src)])]);
}

const describe = (file: string) => {
  if (!existsSync(file)) return "";
  const first = readFileSync(file, "utf8").split("\n").find((l) => l.startsWith("//"));
  return first ? first.replace(/^\/\/\s*/, "") : "";
};

export type CatalogEntry = ReturnType<typeof buildCatalog>[number];
/** `threert-render compare` output. */
type Metrics = { mean_delta_e: number; psnr_db: number; ssim: number };
type RealtimeRow = { frames: number; gpu_ms: number; vs_lumen: Metrics | null; vs_pt: Metrics | null };

export function buildCatalog() {
  const quality = existsSync(join(root, "out/bench/quality.json")) ? JSON.parse(readFileSync(join(root, "out/bench/quality.json"), "utf8")).rows : [];
  const live = existsSync(join(root, "out/bench/live/live.json")) ? JSON.parse(readFileSync(join(root, "out/bench/live/live.json"), "utf8")) : [];
  return Object.entries(sources())
    .map(([id, source]) => {
      const ref = (m: string) => (existsSync(join(root, "out/ue", m, `${id}.png`)) ? join(root, "out/ue", m, `${id}.png`) : null);
      const lumen = quality.find((r: any) => r.scene === id && r.spp === "UE Lumen");
      const pt = quality.find((r: any) => r.scene === id && r.spp === "UE PT 1024");
      const rt = live.filter((r: any) => r.scene === id);
      return {
        id,
        kind: GAMES[id] ? "game" : "scene",
        title: TITLES[id] ?? id.replace(/-/g, " ").replace(/\b\w/g, (c: string) => c.toUpperCase()),
        description: describe(source),
        source,
        bundle: join(bundlesDir, `${id}.js`),
        // three.js baseline: the same scene code rendered by three.js r187 (node tools/bench-three.ts), same camera.
        references: { lumen: ref("lumen"), pathtracer: ref("pathtracer"), three: existsSync(join(root, "out/three", `${id}.png`)) ? join(root, "out/three", `${id}.png`) : null },
        ue: { lumen_s: lumen?.ms ? lumen.ms / 1000 : null, pathtracer_s: pt?.ms ? pt.ms / 1000 : null, lumen_vs_pt: lumen?.vs_ue_pt ?? null },
        realtime: rt.map((r: any): RealtimeRow => ({ frames: r.frames, gpu_ms: r.gpu_ms, vs_lumen: r.vs_ue_lumen, vs_pt: r.vs_ue_pt })) as RealtimeRow[],
      };
    })
    .sort((a, b) => (a.kind === b.kind ? a.id.localeCompare(b.id) : a.kind === "game" ? -1 : 1));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const entries = buildCatalog();
  writeFileSync(join(bundlesDir, "catalog.json"), JSON.stringify({ generated: new Date().toISOString(), scenes: entries }, null, 1));
  console.log(entries.map((e) => `${e.kind.padEnd(5)} ${e.id.padEnd(18)} bundle: ${existsSync(e.bundle) ? "yes" : "no "}  refs: ${e.references.lumen ? "lumen" : "-"} ${e.references.pathtracer ? "pt" : "-"} ${e.references.three ? "three" : "-"}`).join("\n"));
}

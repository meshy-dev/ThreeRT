// Bundles every catalog entry (the reference scenes in scenes/*.ts and the games) for the renderer:
// out/bundles/<id>.js, run with `threert-render play | live | render <bundle> --assets <repo root>`.
//
//   node tools/build-scenes.ts [cornell-box kabukicho ...]
import { join } from "node:path";
import { bundleGame, repoRoot } from "./bundle-game.ts";
import { bundlesDir, sources } from "./catalog.ts";

const only = process.argv.slice(2);
for (const [id, src] of Object.entries(sources())) {
  if (only.length && !only.includes(id)) continue;
  const r = await bundleGame(src, join(bundlesDir, `${id}.js`));
  console.log(`${r.out.replace(repoRoot + "/", "").padEnd(34)} ${(r.bytes / 1024).toFixed(0).padStart(5)} KiB ${r.ms.toFixed(0)} ms`);
}

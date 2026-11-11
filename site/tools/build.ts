// A static copy of the landing page for any file host: bun site/tools/build.ts [outDir]  (default out/site)
// The page plus the three.js files its import map names; no bundling, nothing rewritten.
import { cpSync, copyFileSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { VENDOR } from "./vendor.ts";

const site = resolve(import.meta.dir, "..");
const out = resolve(process.argv[2] ?? join(site, "../out/site"));
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
copyFileSync(join(site, "index.html"), join(out, "index.html"));
for (const d of ["css", "js", "brand", "media"]) cpSync(join(site, d), join(out, d), { recursive: true });
for (const [rel, file] of Object.entries(VENDOR)) {
  mkdirSync(dirname(join(out, rel)), { recursive: true });
  copyFileSync(file, join(out, rel));
}
console.log(`landing page -> ${out}`);

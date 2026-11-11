// Puts the files the showcase cases read where the scenes look for them: showcase/<id>/assets/* (committed with the
// case: models and textures from elsewhere, repacked small, each named in the case's CREDITS.md) -> the asset store,
// assets/showcase/<id>/*, which is the path the scene code uses and a package carries.
//   node showcase/assets.ts          (tools/showcase.ts and the client's release run it; it copies only what changed)
import { copyFileSync, existsSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

const here = import.meta.dirname, root = resolve(here, "..");

/** Copies every case's files into the store; returns how many it wrote. */
export function installAssets(): number {
  let n = 0;
  for (const id of readdirSync(here)) {
    const src = join(here, id, "assets");
    if (!existsSync(src)) continue;
    const dst = join(root, "assets/showcase", id);
    mkdirSync(dst, { recursive: true });
    for (const f of readdirSync(src)) {
      if (f.startsWith(".")) continue;
      const a = statSync(join(src, f)), to = join(dst, f);
      if (!a.isFile() || (existsSync(to) && statSync(to).size === a.size && statSync(to).mtimeMs >= a.mtimeMs)) continue;
      copyFileSync(join(src, f), to);
      n++;
    }
  }
  return n;
}

if (process.argv[1] === import.meta.filename) console.log(`${installAssets()} file(s) -> ${join(root, "assets/showcase")}`);

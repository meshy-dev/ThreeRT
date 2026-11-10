// Initialises a checkout (the main clone or any of its git worktrees): the shared asset store, then the assets.
//
//   bun scripts/setup.ts [--no-fetch] [--store DIR]
//
// Assets (Poly Haven textures, generated Kabukichō textures, Khronos / Quaternius models; ~0.7 GB, not committed) live
// once per machine in a store next to the main clone, `<main clone>/../threert-assets` (THREERT_ASSET_STORE or --store to
// put it elsewhere). Each checkout's `assets/` is a relative symlink to it, so scenes keep referencing repo-relative
// `assets/...` paths and every worktree sees the same files. Nothing is deleted: files from the old layout (a real
// `assets/` directory, or a link into another worktree's) are copied into the store (APFS clones on macOS: no extra
// space), a real directory is kept as `assets.pre-store`, and other worktrees are only reported. Then each fetcher
// downloads what is missing (they skip what exists).
import { $ } from "bun";
import { cpSync, existsSync, lstatSync, mkdirSync, readdirSync, readlinkSync, renameSync, symlinkSync, unlinkSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

const args = process.argv.slice(2);
const opt = (k: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : undefined);
const root = resolve(import.meta.dir, "..");

// The main clone: the parent of the common git directory (a linked worktree's own .git is a file pointing there).
const common = (await $`git rev-parse --path-format=absolute --git-common-dir`.cwd(root).text()).trim();
const store = resolve(opt("--store") ?? process.env.THREERT_ASSET_STORE ?? join(dirname(common), "..", "threert-assets"));
mkdirSync(store, { recursive: true });
console.log(`asset store: ${store}`);

const lstat = (p: string) => {
  try {
    return lstatSync(p);
  } catch {
    return null;
  }
};

/** Copies what the store lacks from an old assets directory (never overwriting; clones on APFS). */
async function fill(from: string) {
  for (const name of readdirSync(from)) {
    const a = join(from, name), b = join(store, name);
    if (process.platform === "darwin") await $`/bin/cp -cRn ${a} ${dirname(b)}/`.quiet();
    else cpSync(a, b, { recursive: true, force: false, errorOnExist: false });
  }
}

// This checkout's `assets/` -> the store.
const at = join(root, "assets");
const target = relative(root, store);
const st = lstat(at);
if (st?.isSymbolicLink()) {
  const old = resolve(root, readlinkSync(at));
  if (old !== store) {
    if (lstat(old)?.isDirectory()) {
      console.log(`copying ${old} into the store`);
      await fill(old);
    }
    unlinkSync(at);
    symlinkSync(target, at);
  } else if (readlinkSync(at) !== target) {
    unlinkSync(at);
    symlinkSync(target, at);
  }
} else if (st?.isDirectory()) {
  console.log(`copying ${at} into the store; the directory stays as assets.pre-store (delete it once you are happy)`);
  await fill(at);
  renameSync(at, `${at}.pre-store`);
  symlinkSync(target, at);
} else if (!st) {
  symlinkSync(target, at);
}
console.log(`  ${at} -> ${readlinkSync(at)}`);

// Other worktrees: report only (run this script there to switch them).
const worktrees = (await $`git worktree list --porcelain`.cwd(root).text()).split("\n").filter((l) => l.startsWith("worktree ")).map((l) => l.slice(9));
for (const wt of worktrees) {
  if (resolve(wt) === root) continue;
  const w = lstat(join(wt, "assets"));
  const where = !w ? "no assets/" : w.isSymbolicLink() ? `assets -> ${readlinkSync(join(wt, "assets"))}` : "assets/ is a directory";
  if (!w?.isSymbolicLink() || resolve(wt, readlinkSync(join(wt, "assets"))) !== store) console.log(`  ${wt}: ${where} (run \`bun scripts/setup.ts\` there)`);
}

if (args.includes("--no-fetch")) process.exit(0);

// Fetchers (each skips what exists). Kabukichō's generated textures need python3; Quaternius needs Google Chrome; the
// ORCA scenes need Blender.
const a = (p: string) => join(root, "assets", p);
const helmet = a("DamagedHelmet.glb");
if (!existsSync(helmet)) {
  console.log("fetching DamagedHelmet.glb");
  await $`curl -fL -C - --retry 5 -o ${helmet} https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models/DamagedHelmet/glTF-Binary/DamagedHelmet.glb`;
}
await $`node tools/fetch-polyhaven.ts`.cwd(root);
await $`node tools/fetch-polyhaven.ts --set kabukicho-ue --res 2k`.cwd(root);
if (!existsSync(a("kabukicho/textures"))) await $`node tools/kabukicho-assets.ts`.cwd(root);
await $`node tools/fetch-khronos.ts`.cwd(root);
const chrome = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
if (existsSync(chrome) || existsSync(a("quaternius/ual/UAL1_Standard.glb"))) await $`node tools/fetch-quaternius.ts`.cwd(root);
else console.log("skipping Quaternius (needs Google Chrome, or CHROME=<binary>): the mannequin scenes and animation/skinned-pose test need it");
// Bistro and Zero Day (ORCA, FBX + DDS -> glTF; needs Blender, skips itself without it; ~2 GB download once).
await $`bun scripts/orca.ts`.cwd(root);
// The voxel benchmark level (games/voxel-level, ROADMAP § 6 step 2): generated, not downloaded (~4 s, ~0.3 GB; skips a
// level already built). Other box granularities: `node tools/voxel-level/generate.ts --granularity brick|object`.
await $`node tools/voxel-level/generate.ts --granularity region`.cwd(root);
console.log((await $`du -sh ${store}`.text()).trim());

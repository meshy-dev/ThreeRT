// Builds the renderer and game bundles of another commit (a main baseline, a bisect step) without touching the
// checkout: a detached worktree in out/perf-base/<sha>, one cargo target directory shared by all of them (incremental
// builds), and a node_modules made of links to this checkout's, except the workspace packages, which point at the
// commit's own sources. Assets are this checkout's.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, symlinkSync } from "node:fs";
import { dirname, join } from "node:path";
import { repoRoot } from "./bench.ts";
import type { Build } from "./live.ts";

const baseDir = join(repoRoot, "out/perf-base");
const git = (...a: string[]) => execFileSync("git", a, { cwd: repoRoot, encoding: "utf8" }).trim();

/** The full sha of a revision. */
export const resolveSha = (rev: string) => git("rev-parse", `${rev}^{commit}`);

/** The renderer binary's name at a commit (its Cargo package name). */
function binName(dir: string): string {
  const m = /^name\s*=\s*"([^"]+)"/m.exec(readFileSync(join(dir, "renderer/Cargo.toml"), "utf8"));
  if (!m) throw new Error(`no package name in ${dir}/renderer/Cargo.toml`);
  return m[1];
}

/** node_modules for `dir`: links to this checkout's packages, the workspace ones to `dir`'s own sources. */
function linkNodeModules(dir: string) {
  const nm = join(dir, "node_modules");
  if (existsSync(nm)) return;
  mkdirSync(nm);
  const workspaces: Record<string, string> = {};
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  for (const w of pkg.workspaces ?? []) {
    const dirs = w.endsWith("/*") ? readdirSync(join(dir, w.slice(0, -2))).map((d) => join(w.slice(0, -2), d)) : [w];
    for (const d of dirs) {
      const pj = join(dir, d, "package.json");
      if (existsSync(pj)) workspaces[JSON.parse(readFileSync(pj, "utf8")).name] = join(dir, d);
    }
  }
  const src = join(repoRoot, "node_modules");
  for (const e of readdirSync(src)) {
    if (e.startsWith(".")) continue;
    if (e.startsWith("@")) {
      mkdirSync(join(nm, e));
      for (const s of readdirSync(join(src, e))) if (!workspaces[`${e}/${s}`]) symlinkSync(join(src, e, s), join(nm, e, s));
    } else if (!workspaces[e]) symlinkSync(join(src, e), join(nm, e));
  }
  for (const [name, d] of Object.entries(workspaces)) {
    const at = join(nm, name);
    mkdirSync(dirname(at), { recursive: true });
    if (!existsSync(at)) symlinkSync(d, at);
  }
}

/**
 * The commit `sha` built: renderer (release) and the bundles of `games` (paths relative to the commit's root, e.g.
 * games/kabukicho/kabukicho.game.ts) plus `extra` game / scene files from this checkout (copied in at the same path,
 * replacing the commit's own, so relative imports between them hold; bundled against the commit's packages; one that
 * does not build there is left out). Reuses whatever an earlier call built.
 */
export function buildAt(sha: string, games: string[], extra: string[] = []): Build & { built: string[] } {
  const dir = join(baseDir, sha.slice(0, 12));
  if (!existsSync(dir)) {
    mkdirSync(baseDir, { recursive: true });
    git("worktree", "add", "--detach", dir, sha);
  }
  linkNodeModules(dir);
  const target = join(baseDir, "target");
  const name = binName(dir);
  const bin = join(dir, "bin", name);
  if (!existsSync(bin)) {
    console.log(`building ${sha.slice(0, 12)} (${name}) ...`);
    execFileSync("cargo", ["build", "--release"], { cwd: join(dir, "renderer"), env: { ...process.env, CARGO_TARGET_DIR: target }, stdio: ["ignore", "ignore", "inherit"] });
    mkdirSync(join(dir, "bin"), { recursive: true });
    execFileSync("cp", [join(target, "release", name), bin]);
  }
  const bundles = join(dir, "out/bundles");
  mkdirSync(bundles, { recursive: true });
  const built: string[] = [];
  const bundle = (src: string, out: string) => {
    if (existsSync(out)) return true;
    try {
      execFileSync("node", [join(dir, "tools/bundle-game.ts"), src, "-o", out], { cwd: dir, stdio: ["ignore", "ignore", "pipe"] });
      return true;
    } catch {
      return false;
    }
  };
  for (const g of games) if (bundle(join(dir, g), join(bundles, `${g.split("/").pop()!.replace(/\.(game\.)?ts$/, "")}.js`))) built.push(g);
  // All copied before any is bundled: they may import each other.
  for (const g of extra) {
    mkdirSync(dirname(join(dir, g)), { recursive: true });
    execFileSync("cp", [join(repoRoot, g), join(dir, g)]);
  }
  for (const g of extra) {
    const at = join(dir, g);
    if (bundle(at, join(bundles, `${g.split("/").pop()!.replace(/\.(game\.)?ts$/, "")}.js`))) built.push(g);
  }
  return { bin, assets: repoRoot, presets: join(dir, "presets"), bundles, label: sha.slice(0, 12), built };
}

/** This checkout as a build (its release renderer and out/bundles; bundles are made by the caller). */
export function branchBuild(bin: string): Build {
  return { bin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "branch" };
}

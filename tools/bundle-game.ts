// Bundles a game for the render node: game code + the ThreeRT runtime (@threert/scene, @threert/gameplay) + the node
// host, as one self-contained script for the renderer's embedded JS engine (`threert-render play <bundle>`). The bundle
// is the unit a render node receives: it builds, edits and animates the scene where it is rendered. Assets the game
// references by path (Poly Haven GLBs, textures) resolve against the asset root given to the renderer (the repo root).
//
// A plain scene module (scenes/*.ts: default export () => {scene, camera, options}) bundles as a game without state or
// handlers.
//
//   node tools/bundle-game.ts games/kabukicho/kabukicho.game.ts [-o out/bundles/kabukicho.js]
import { build } from "esbuild";
import { mkdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

export const repoRoot = resolve(import.meta.dirname, "..");

/** Default bundle path of a game / scene module: out/bundles/<name>.js. */
export function bundlePath(src: string): string {
  return join(repoRoot, "out/bundles", basename(src).replace(/(\.game)?\.[jt]s$/, ".js"));
}

/** Bundles `src` (a game or a scene module) to `out`; returns the bundle path. */
export async function bundleGame(src: string, out = bundlePath(src)): Promise<{ out: string; bytes: number; ms: number }> {
  const t0 = performance.now();
  const result = await build({
    stdin: {
      contents: [
        `import mod from ${JSON.stringify(resolve(src))};`,
        `import { defineGame, installNodeHost } from "@threert/gameplay";`,
        `installNodeHost(typeof mod === "function" ? defineGame({ state: {}, setup: () => mod(), handlers: {} }) : mod);`,
      ].join("\n"),
      resolveDir: repoRoot,
      sourcefile: "threert-node-entry.js",
    },
    bundle: true,
    // The runtime packages resolve from this repo for game / scene files anywhere.
    nodePaths: [join(repoRoot, "node_modules")],
    format: "iife",
    platform: "neutral",
    target: "es2022",
    minifySyntax: true,
    legalComments: "none",
    write: false,
    logLevel: "warning",
  });
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, result.outputFiles[0].text);
  return { out, bytes: result.outputFiles[0].contents.length, ms: performance.now() - t0 };
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const src = args.find((a) => a.endsWith(".ts") || a.endsWith(".js"));
  if (!src) throw new Error("usage: node tools/bundle-game.ts <game.ts | scene.ts> [-o out.js]");
  const r = await bundleGame(src, args.includes("-o") ? resolve(args[args.indexOf("-o") + 1]) : undefined);
  console.log(`${r.out.replace(repoRoot + "/", "")}: ${(r.bytes / 1024).toFixed(0)} KiB, ${r.ms.toFixed(0)} ms`);
}

// Builds a ThreeRT game as a page (docs/RUNTIME.md "The engine in a browser"): the engine's wasm module
// (renderer/web), wasm-bindgen's JavaScript for it, the page (renderer/web/www) and the game's bundle, the same file
// the native runtime loads. Prints the sizes. `--serve` keeps a server on it (WebGPU needs localhost or https).
//
//   node tools/build-web.ts [games/physics/hello.game.ts] [-o out/web/dist] [--serve [port]]
//
// Needs the wasm32 target (rustup target add wasm32-unknown-unknown) and the wasm-bindgen CLI of the version in
// renderer/web/Cargo.toml (cargo install wasm-bindgen-cli --version <it> --locked).
import { join, resolve } from "node:path";
import { repoRoot } from "./bundle-game.ts";
import { buildWeb, describeBuild, serveWeb } from "./lib/web.ts";

const args = process.argv.slice(2);
const game = args.find((a) => a.endsWith(".ts") || a.endsWith(".js")) ?? join(repoRoot, "games/physics/hello.game.ts");
const dir = args.includes("-o") ? resolve(args[args.indexOf("-o") + 1]) : undefined;
const b = await buildWeb(resolve(game), dir);
console.log(describeBuild(b));
console.log(`web build: ${b.dir.replace(repoRoot + "/", "")}`);
if (args.includes("--serve")) {
  const port = Number(args[args.indexOf("--serve") + 1]);
  const { url } = await serveWeb(b.dir, Number.isFinite(port) ? port : 8787);
  console.log(`web build: ${url} (Ctrl-C stops it)`);
}

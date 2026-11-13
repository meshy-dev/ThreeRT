// Opens a scene / game module in three.js in an ordinary browser: bundles it with the interactive view
// (tools/three/view.ts), serves the repo root (assets/ included) on localhost and opens the page.
//
//   node tools/three/open.ts scenes/bistro.ts [--port 8123] [--no-open] [--physics | --no-physics]
//   node tools/three/open.ts games/fps/arena.game.ts --launch 'room=K7Q4MZ&name=Ada'
//
// A game is played in the page (view.ts): its frame script, its keys and clicks, its host channel. One whose source
// names the physics (`physics`, `Shapecaster`, `intersectPhysics`; `--physics` / `--no-physics` decide otherwise) gets
// the engine's own, built here as a wasm32 module (tools/three/physics-build.ts: cargo and the wasm32 target).
// `--launch` is the game's part of the page's query (the host channel's `launch` message).
//
// The page stays up until Ctrl-C. URL parameters of the view: ev100, autoLights, width / height, and those view.ts
// lists.
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { bundlePage, root, serve } from "./browser.ts";
import { buildScenePhysics, SCENE_PHYSICS } from "./physics-build.ts";

const args = process.argv.slice(2);
const src = args.find((a, i) => !a.startsWith("--") && args[i - 1] !== "--port" && args[i - 1] !== "--launch");
if (!src) {
  console.error("usage: node tools/three/open.ts <scene or game .ts> [--port N] [--no-open] [--physics | --no-physics] [--launch 'room=..&name=..']");
  process.exit(2);
}
const id = `${basename(src).replace(/\.(game\.)?ts$/, "")}-view`;
const { html, bytes } = await bundlePage(id, resolve(src), join(root, "tools/three/view.ts"));
const { origin } = await serve(root, Number(args.includes("--port") ? args[args.indexOf("--port") + 1] : 0));
const query = new URLSearchParams(args.includes("--launch") ? args[args.indexOf("--launch") + 1] : "");
const physics = args.includes("--physics") || (!args.includes("--no-physics") && /\bphysics\b|Shapecaster|intersectPhysics/.test(readFileSync(resolve(src), "utf8")));
if (physics) {
  buildScenePhysics();
  query.set("physics", `/${SCENE_PHYSICS}`);
}
const url = `${origin}${html}${query.size ? `?${query}` : ""}`;
console.log(`${src}: ${(bytes / 1024).toFixed(0)} KiB page\n${url}\n(Ctrl-C to stop)`);
if (!args.includes("--no-open")) spawn(process.platform === "darwin" ? "open" : "xdg-open", [url], { stdio: "ignore", detached: true }).unref();

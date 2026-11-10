// Opens a scene / game module in three.js in an ordinary browser: bundles it with the interactive view
// (tools/three/view.ts), serves the repo root (assets/ included) on localhost and opens the page.
//
//   node tools/three/open.ts scenes/bistro.ts [--port 8123] [--no-open]
//
// The page stays up until Ctrl-C. URL parameters of the view: ev100, autoLights, width / height.
import { spawn } from "node:child_process";
import { basename, join, resolve } from "node:path";
import { bundlePage, root, serve } from "./browser.ts";

const args = process.argv.slice(2);
const src = args.find((a, i) => !a.startsWith("--") && args[i - 1] !== "--port");
if (!src) {
  console.error("usage: node tools/three/open.ts <scene or game .ts> [--port N] [--no-open]");
  process.exit(2);
}
const id = `${basename(src).replace(/\.(game\.)?ts$/, "")}-view`;
const { html, bytes } = await bundlePage(id, resolve(src), join(root, "tools/three/view.ts"));
const { origin } = await serve(root, Number(args.includes("--port") ? args[args.indexOf("--port") + 1] : 0));
const url = `${origin}${html}`;
console.log(`${src}: ${(bytes / 1024).toFixed(0)} KiB page\n${url}\n(Ctrl-C to stop)`);
if (!args.includes("--no-open")) spawn(process.platform === "darwin" ? "open" : "xdg-open", [url], { stdio: "ignore", detached: true }).unref();

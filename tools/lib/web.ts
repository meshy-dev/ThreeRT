// A ThreeRT game as a page (docs/RUNTIME.md "The engine in a browser"): the build (the engine's wasm module, its
// JavaScript, the page, the game's bundle) and a static server for it. Used by tools/build-web.ts and
// tools/web-hello.ts.
import { execFileSync } from "node:child_process";
import { build } from "esbuild";
import { copyFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { open, stat } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import { homedir } from "node:os";
import { extname, join, normalize } from "node:path";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";
import { bundleGame, repoRoot } from "../bundle-game.ts";

const crate = join(repoRoot, "renderer/web");
/** The wasm-bindgen CLI's version: the `wasm-bindgen` crate's in renderer/web/Cargo.toml (they must be the same). */
const BINDGEN = /wasm-bindgen = "=([\d.]+)"/.exec(readFileSync(join(crate, "Cargo.toml"), "utf8"))![1];

function bindgen(): string {
  const local = join(homedir(), ".cargo/bin/wasm-bindgen");
  const bin = existsSync(local) ? local : "wasm-bindgen";
  let version = "";
  try {
    version = execFileSync(bin, ["--version"], { encoding: "utf8" }).trim().split(" ").pop() ?? "";
  } catch {}
  if (version !== BINDGEN) throw new Error(`wasm-bindgen ${BINDGEN} is needed (found ${version || "none"}): cargo install wasm-bindgen-cli --version ${BINDGEN} --locked`);
  return bin;
}

export interface WebBuild {
  dir: string;
  /** The module's size, bytes: as built, gzip -9, brotli 11; and the JavaScript beside it (wasm-bindgen's + the page's). */
  wasm: { raw: number; gzip: number; brotli: number };
  js: { raw: number; gzip: number; brotli: number };
  game: { raw: number; gzip: number; brotli: number };
  ms: { cargo: number; bindgen: number; page: number };
}

const sizes = (files: string[]) => {
  const bytes = Buffer.concat(files.map((f) => readFileSync(f)));
  return { raw: bytes.length, gzip: gzipSync(bytes, { level: 9 }).length, brotli: brotliCompressSync(bytes, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_LGWIN]: 24 } }).length };
};

/** Builds the page of `game` (a game or scene module) into `dir`: index.html, main.js, pkg/ (the module), game.js. */
export async function buildWeb(game: string, dir = join(repoRoot, "out/web/dist")): Promise<WebBuild> {
  const t0 = performance.now();
  execFileSync("cargo", ["build", "--release", "--locked", "--target", "wasm32-unknown-unknown"], { cwd: crate, stdio: ["ignore", "ignore", "inherit"] });
  const t1 = performance.now();
  mkdirSync(join(dir, "pkg"), { recursive: true });
  execFileSync(bindgen(), ["--target", "web", "--no-typescript", "--out-dir", join(dir, "pkg"), join(crate, "target/wasm32-unknown-unknown/release/threert_web.wasm")], { stdio: ["ignore", "ignore", "inherit"] });
  const t2 = performance.now();
  await build({ entryPoints: [join(crate, "www/main.ts")], bundle: true, format: "esm", target: "es2022", outfile: join(dir, "main.js"), nodePaths: [join(repoRoot, "node_modules")], logLevel: "warning" });
  copyFileSync(join(crate, "www/index.html"), join(dir, "index.html"));
  await bundleGame(game, join(dir, "game.js"));
  return {
    dir,
    wasm: sizes([join(dir, "pkg/threert_web_bg.wasm")]),
    js: sizes([join(dir, "pkg/threert_web.js"), join(dir, "main.js")]),
    game: sizes([join(dir, "game.js")]),
    ms: { cargo: t1 - t0, bindgen: t2 - t1, page: performance.now() - t2 },
  };
}

const types: Record<string, string> = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".json": "application/json", ".png": "image/png", ".jpg": "image/jpeg", ".glb": "model/gltf-binary" };

/**
 * Serves `dir` on localhost (a secure context: WebGPU needs one), cross-origin isolated so the page's clock is not
 * coarsened (5 µs instead of 100 µs). Port 0: any free one. Resolves to the server and its URL.
 */
export function serveWeb(dir: string, port = 0): Promise<{ server: Server; url: string }> {
  const server = createServer(async (req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url ?? "/", "http://x").pathname)).replace(/^(\.\.[/\\])+/, "");
    const file = join(dir, path === "/" ? "index.html" : path);
    try {
      if (!file.startsWith(dir)) throw new Error("outside");
      const size = (await stat(file)).size;
      const f = await open(file);
      res.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream", "content-length": size, "cross-origin-opener-policy": "same-origin", "cross-origin-embedder-policy": "require-corp", "cache-control": "no-store" });
      f.createReadStream().pipe(res).on("close", () => f.close());
    } catch {
      res.writeHead(404).end("not found");
    }
  });
  return new Promise((ok) => server.listen(port, "127.0.0.1", () => ok({ server, url: `http://localhost:${(server.address() as { port: number }).port}/` })));
}

/** A build's line for a log. */
export function describeBuild(b: WebBuild): string {
  const kb = (n: number) => `${(n / 1024).toFixed(0)} KiB`;
  const row = (s: { raw: number; gzip: number; brotli: number }) => `${kb(s.raw)} (gzip ${kb(s.gzip)}, brotli ${kb(s.brotli)})`;
  return `web build: module ${row(b.wasm)}, its JavaScript and the page's ${row(b.js)}, the game's bundle ${row(b.game)}; cargo ${(b.ms.cargo / 1000).toFixed(1)} s, wasm-bindgen ${(b.ms.bindgen / 1000).toFixed(1)} s`;
}

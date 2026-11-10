// Node side of the three.js baseline: bundles a scene / game module with the three.js page (tools/three/page.ts),
// serves the repo root over HTTP (scene assets resolve as /assets/...), and drives headless Chrome over the DevTools
// protocol (Node's built-in WebSocket, no puppeteer).
import { createHash } from "node:crypto";
import { build } from "esbuild";
import { spawn, type ChildProcess } from "node:child_process";
import { createReadStream, existsSync, mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { extname, join, normalize, resolve } from "node:path";

export const root = resolve(import.meta.dirname, "../..");
export const pagesDir = join(root, "out/three/pages");
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

/** Bundles `src` (scene module or game) + a three.js page (default: the WebGL baseline, tools/three/page.ts) for the
 *  browser: out/three/pages/<id>.{js,html}. */
export async function bundlePage(id: string, src: string, page = join(root, "tools/three/page.ts")): Promise<{ html: string; bytes: number; hash: string }> {
  mkdirSync(pagesDir, { recursive: true });
  const r = await build({
    stdin: {
      contents: [`import mod from ${JSON.stringify(resolve(src))};`, `import { run } from ${JSON.stringify(page)};`, `run(mod);`].join("\n"),
      resolveDir: root,
      sourcefile: `three-${id}.js`,
    },
    bundle: true,
    nodePaths: [join(root, "node_modules")],
    format: "esm",
    platform: "browser",
    target: "es2022",
    minifySyntax: true,
    legalComments: "none",
    write: false,
    logLevel: "warning",
  });
  writeFileSync(join(pagesDir, `${id}.js`), r.outputFiles[0].text);
  writeFileSync(join(pagesDir, `${id}.html`), `<!doctype html><meta charset="utf-8"><title>${id} (three.js)</title><body style="margin:0;background:#000"><script type="module" src="./${id}.js"></script>\n`);
  // The page's content hash: scene code, the engine binding and three.js all go into it (bench-three's cache key).
  return { html: `/out/three/pages/${id}.html`, bytes: r.outputFiles[0].contents.length, hash: createHash("sha256").update(r.outputFiles[0].contents).digest("hex") };
}

const TYPES: Record<string, string> = {
  ".html": "text/html", ".js": "text/javascript", ".json": "application/json", ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
  ".glb": "model/gltf-binary", ".gltf": "model/gltf+json", ".bin": "application/octet-stream", ".exr": "application/octet-stream", ".hdr": "application/octet-stream",
  ".ktx2": "image/ktx2", ".webp": "image/webp",
};

/** Static file server for the repo root (read-only, no directory listings); `assets/...` may also come from another
 * asset root (the renderer's `--assets`). */
export function serve(assets = root, port = 0): Promise<{ server: Server; origin: string }> {
  const server = createServer((req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url ?? "/", "http://x").pathname));
    let file = join(root, path);
    if (!file.startsWith(root + "/")) return void res.writeHead(403).end();
    const alt = join(assets, path);
    if (assets !== root && path.startsWith("/assets/") && alt.startsWith(assets + "/") && !existsSync(file)) file = alt;
    try {
      const st = statSync(file);
      if (!st.isFile()) throw new Error("not a file");
      res.writeHead(200, { "content-type": TYPES[extname(file).toLowerCase()] ?? "application/octet-stream", "content-length": st.size, "cache-control": "no-store" });
      createReadStream(file).pipe(res);
    } catch {
      res.writeHead(404).end();
    }
  });
  return new Promise((ok) => server.listen(port, "127.0.0.1", () => ok({ server, origin: `http://127.0.0.1:${(server.address() as { port: number }).port}` })));
}

/** Minimal DevTools protocol client over one browser WebSocket (flattened sessions). */
class Cdp {
  private id = 0;
  private readonly calls = new Map<number, { ok: (v: any) => void; fail: (e: Error) => void }>();
  readonly listeners: ((method: string, params: any, session?: string) => void)[] = [];
  private readonly ws: WebSocket;
  constructor(ws: WebSocket) {
    this.ws = ws;
    ws.addEventListener("message", (ev) => {
      const m = JSON.parse(String(ev.data));
      if (m.id !== undefined) {
        const c = this.calls.get(m.id);
        this.calls.delete(m.id);
        if (m.error) c?.fail(new Error(`${m.error.message} ${m.error.data ?? ""}`));
        else c?.ok(m.result);
      } else for (const l of this.listeners) l(m.method, m.params, m.sessionId);
    });
  }
  send(method: string, params: object = {}, sessionId?: string): Promise<any> {
    const id = ++this.id;
    return new Promise((ok, fail) => {
      this.calls.set(id, { ok, fail });
      this.ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }
}

export interface Browser {
  /** Opens `url` in a fresh tab, waits for `window.__result` (timeout ms), closes the tab and returns the result. */
  run(url: string, timeoutMs?: number): Promise<any>;
  /** Opens `url` in a fresh 1280 x 720 tab and keeps it open: evaluate expressions in it, take screenshots. */
  open(url: string): Promise<Page>;
  close(): void;
}

export interface Page {
  /** The value of `expression` (awaited if it is a promise), as JSON. */
  evaluate<T = unknown>(expression: string, timeoutMs?: number): Promise<T>;
  /** A PNG of the viewport. */
  screenshot(file: string): Promise<void>;
  close(): Promise<void>;
}

export async function launch(log: (line: string) => void = () => {}): Promise<Browser> {
  const profile = mkdtempSync(join(tmpdir(), "threert-three-"));
  const proc: ChildProcess = spawn(CHROME, [
    "--headless=new", "--use-angle=metal", "--enable-gpu", "--ignore-gpu-blocklist", "--enable-webgl", "--no-first-run",
    "--no-default-browser-check", "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
    "--disable-backgrounding-occluded-windows", "--disable-gpu-watchdog", "--force-device-scale-factor=1", "--window-size=1280,720",
    "--disable-breakpad", "--disable-crash-reporter", // no crashpad handler left behind holding our stderr pipe
    "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  const wsUrl = await new Promise<string>((ok, fail) => {
    let buf = "";
    const t = setTimeout(() => fail(new Error("Chrome did not start")), 20000);
    proc.stderr!.on("data", (d) => {
      buf += String(d);
      const m = /DevTools listening on (ws:\/\/\S+)/.exec(buf);
      if (m) {
        clearTimeout(t);
        ok(m[1]);
      }
    });
    proc.on("exit", (c) => fail(new Error(`Chrome exited (${c}): ${buf.slice(-2000)}`)));
  });
  const ws = new WebSocket(wsUrl);
  await new Promise((ok, fail) => {
    ws.addEventListener("open", ok, { once: true });
    ws.addEventListener("error", fail, { once: true });
  });
  const cdp = new Cdp(ws);
  cdp.listeners.push((method, p) => {
    if (method === "Runtime.consoleAPICalled") log(`[page ${p.type}] ${p.args.map((a: any) => a.value ?? a.description ?? "").join(" ")}`);
    else if (method === "Runtime.exceptionThrown") log(`[page exception] ${p.exceptionDetails?.exception?.description ?? p.exceptionDetails?.text}`);
    else if (method === "Log.entryAdded" && p.entry.level !== "verbose") log(`[page log ${p.entry.level}] ${p.entry.text} ${p.entry.url ?? ""}`);
  });
  return {
    async run(url, timeoutMs = 600_000) {
      const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
      const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
      try {
        await cdp.send("Runtime.enable", {}, sessionId);
        await cdp.send("Log.enable", {}, sessionId);
        await cdp.send("Page.enable", {}, sessionId);
        await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false }, sessionId);
        const loaded = new Promise<void>((ok) => {
          const l = (method: string, _p: unknown, session?: string) => {
            if (method === "Page.loadEventFired" && session === sessionId) {
              cdp.listeners.splice(cdp.listeners.indexOf(l), 1);
              ok();
            }
          };
          cdp.listeners.push(l);
        });
        await cdp.send("Page.navigate", { url }, sessionId);
        await loaded;
        const r = await cdp.send("Runtime.evaluate", {
          expression: `new Promise((ok, fail) => { const t0 = Date.now(); const k = setInterval(() => { if (window.__result) { clearInterval(k); ok(JSON.stringify(window.__result)); } else if (Date.now() - t0 > ${timeoutMs}) { clearInterval(k); fail(new Error("timeout")); } }, 200); })`,
          awaitPromise: true, returnByValue: true, timeout: timeoutMs + 5000,
        }, sessionId);
        if (r.exceptionDetails) throw new Error(`page: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
        return JSON.parse(r.result.value);
      } finally {
        await cdp.send("Target.closeTarget", { targetId }).catch(() => {});
      }
    },
    async open(url) {
      const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
      const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
      await cdp.send("Runtime.enable", {}, sessionId);
      await cdp.send("Log.enable", {}, sessionId);
      await cdp.send("Page.enable", {}, sessionId);
      await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false }, sessionId);
      const loaded = new Promise<void>((ok) => {
        const l = (method: string, _p: unknown, session?: string) => {
          if (method === "Page.loadEventFired" && session === sessionId) {
            cdp.listeners.splice(cdp.listeners.indexOf(l), 1);
            ok();
          }
        };
        cdp.listeners.push(l);
      });
      await cdp.send("Page.navigate", { url }, sessionId);
      await loaded;
      return {
        async evaluate(expression, timeoutMs = 600_000) {
          const r = await cdp.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true, timeout: timeoutMs }, sessionId);
          if (r.exceptionDetails) throw new Error(`page: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
          return r.result.value;
        },
        async screenshot(file) {
          const r = await cdp.send("Page.captureScreenshot", { format: "png" }, sessionId);
          writeFileSync(file, Buffer.from(r.data, "base64"));
        },
        async close() {
          await cdp.send("Target.closeTarget", { targetId }).catch(() => {});
        },
      };
    },
    close() {
      ws.close();
      proc.kill();
      // Chrome's crashpad handler inherits stderr and can outlive Chrome: let go of the pipe, or it keeps the
      // process from exiting (strongest.ts once hung after its last page for that).
      proc.stderr?.destroy();
      setTimeout(() => rmSync(profile, { recursive: true, force: true }), 500);
    },
  };
}

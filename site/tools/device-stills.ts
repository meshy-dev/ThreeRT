// Renders the two 3D device rows of the "Everywhere" section to stills, the page's fallback when WebGL is off:
//   bun site/tools/device-stills.ts   -> site/media/devices/{native,web}.webp
// Headless Chrome with SwiftShader (software WebGL), so it never touches the GPU. Starts its own server.
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import sharp from "sharp";

const site = resolve(import.meta.dir, "..");
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const port = 4199;

const server = Bun.spawn(["bun", join(site, "serve.ts"), String(port)], { stdout: "ignore", stderr: "inherit" });
const profile = mkdtempSync(join(tmpdir(), "threert-stills-"));
const chrome = spawn(CHROME, ["--headless=new", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
try {
  const ws = await new Promise<string>((ok, fail) => {
    let b = "";
    const t = setTimeout(() => fail(new Error("Chrome did not start")), 20000);
    chrome.stderr!.on("data", (d) => { b += d; const m = /ws:\/\/\S+/.exec(b); if (m) { clearTimeout(t); ok(m[0]); } });
  });
  const sock = new WebSocket(ws);
  await new Promise((r) => sock.addEventListener("open", r, { once: true }));
  let id = 0;
  const pend = new Map<number, (m: any) => void>();
  sock.addEventListener("message", (e) => { const m = JSON.parse(String(e.data)); pend.get(m.id)?.(m); pend.delete(m.id); });
  const send = (method: string, params = {}, sessionId?: string) => new Promise<any>((r) => { const i = ++id; pend.set(i, r); sock.send(JSON.stringify({ id: i, method, params, sessionId })); });
  const { result: { targetId } } = await send("Target.createTarget", { url: "about:blank" });
  const { result: { sessionId } } = await send("Target.attachToTarget", { targetId, flatten: true });
  await send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 2, mobile: false }, sessionId);
  await send("Page.navigate", { url: `http://127.0.0.1:${port}/` }, sessionId);
  await Bun.sleep(3000);
  await send("Runtime.evaluate", { expression: "document.documentElement.style.scrollBehavior = 'auto'; scrollTo(0, document.querySelector('#everywhere .rail').getBoundingClientRect().top + scrollY - 60)" }, sessionId);
  await Bun.sleep(9000); // textures load, the rows settle
  mkdirSync(join(site, "media/devices"), { recursive: true });
  for (const [sel, name] of [["#shelfNative", "native"], ["#shelfWeb", "web"]]) {
    const r = await send("Runtime.evaluate", { expression: `(() => { const b = document.querySelector('${sel}').getBoundingClientRect(); return JSON.stringify({ x: b.left + scrollX, y: b.top + scrollY, width: b.width, height: b.height }); })()`, returnByValue: true }, sessionId);
    const shot = await send("Page.captureScreenshot", { format: "png", clip: { ...JSON.parse(r.result.result.value), scale: 1 } }, sessionId);
    await sharp(Buffer.from(shot.result.data, "base64")).webp({ quality: 86 }).toFile(join(site, `media/devices/${name}.webp`));
    console.log(`media/devices/${name}.webp`);
  }
} finally {
  chrome.kill();
  server.kill();
  rmSync(profile, { recursive: true, force: true });
}

// The host channel end to end (docs/RUNTIME.md "The host channel"), on games/host/echo.game.ts: a game that has the
// app open a socket, says hello, answers the first messages that come back, and has the app fetch a page.
//
//   1. `threert-render live`, with this script as the app: the game's posts are read from the runner's stdout
//      (`{"host":[...]}`), and answered on its stdin (`{"cmd":"host","messages":[...]}`). Checked: the order of the
//      game's posts (net.open, net.send x 3, net.close; net.fetch), that a batch of messages reaches its listeners,
//      that what they emit is committed (the state's count is in the HUD), and that the HUD is posted as it changes.
//   2. `threert-render play` (a window, for about three seconds), which is an app itself: the same game against a
//      WebSocket and HTTP server on this machine, through the binary's own sockets. Checked: the server got the
//      game's three messages in order, the game got the three answers, the close and the page.
//   3. the three.js view in headless Chrome (tools/three/view.ts), an app too: the same game against the same server,
//      through the browser's WebSocket and fetch, its HUD read off the page.
//
// Exit 1 on a failed check, 2 when the GPU lock is held. The GPU lock is held.
//   node tools/host-echo.ts [--no-play] [--no-three]
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { createInterface } from "node:readline";
import { join } from "node:path";
import type { Duplex } from "node:stream";
import { ROUNDS } from "../games/host/echo.game.ts";
import type { Hud, HudText } from "@threert/gameplay";
import { acquireGpuLock, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";
import { bundlePage, launch as launchChrome, root, serve } from "./three/browser.ts";

const holder = acquireGpuLock("host-echo");
if (holder) { console.log(`not run: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }
const bundle = join(repoRoot, "out/bundles/host-echo.js");
execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/host/echo.game.ts"), "-o", bundle], { stdio: "ignore" });
const failures: string[] = [];
const check = (ok: boolean, what: string) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Post = { t: string; id?: string; url?: string; data?: string; hud?: Hud };
const messages = (h: Hud | undefined) => (h?.items ?? []).find((i): i is HudText => i.type === "text" && i.label === "Messages")?.text;
const answers = (lines: string[]) => lines.map((l) => /echo got (".*")/.exec(l)?.[1]).filter((x): x is string => !!x).map((x) => JSON.parse(x) as string);

/** A WebSocket and HTTP server on this machine: it answers every text message with "echo: <text>". */
async function echoServer(): Promise<{ port: number; got: string[]; closeCode(): number; close(): void }> {
  const got: string[] = [];
  let closeCode = 0;
  const server = createServer((_req, res) => res.writeHead(200, { "content-type": "text/plain", "access-control-allow-origin": "*" }).end("hello from http"));
  server.on("upgrade", (req, socket: Duplex) => {
    const accept = createHash("sha1").update(`${req.headers["sec-websocket-key"]}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest("base64");
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const frame = (op: number, payload: Buffer) => socket.write(Buffer.concat([Buffer.from([0x80 | op, payload.length]), payload]));
    let buf: Buffer = Buffer.alloc(0);
    socket.on("data", (d: Buffer) => {
      buf = Buffer.concat([buf, d]);
      // (Short masked frames, as this game sends them.)
      while (buf.length >= 6 && buf.length >= 6 + (buf[1] & 127)) {
        const n = buf[1] & 127, mask = buf.subarray(2, 6);
        const payload = Buffer.from(buf.subarray(6, 6 + n).map((b, k) => b ^ mask[k % 4]));
        const op = buf[0] & 15;
        buf = buf.subarray(6 + n);
        if (op === 1) {
          got.push(payload.toString());
          frame(1, Buffer.from(`echo: ${payload.toString()}`));
        } else if (op === 8) {
          closeCode = n >= 2 ? payload.readUInt16BE(0) : 1005;
          frame(8, payload.subarray(0, 2));
          socket.end();
        }
      }
    });
    socket.on("error", () => {});
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  return { port: (server.address() as { port: number }).port, got, closeCode: () => closeCode, close: () => void server.close() };
}

// ---------------------------------------------------------------------------------------------- 1. live
{
  const posts: Post[] = [];
  let handled = 0;
  const live = startLive(branchBuild(rendererBin), bundle, {
    width: 320, height: 180,
    args: ["--launch", JSON.stringify({ url: "ws://app.test/echo", fetch: "http://app.test/hello" })],
    stdout: (line) => { try { posts.push(...((JSON.parse(line) as { host?: Post[] }).host ?? [])); } catch { /* not ours */ } },
  });
  await live.firstFrame;
  live.send({ cmd: "lockstep", dt: 1 / 60 });
  let id = 0;
  const step = async (frames: number) => {
    const sync = `s${++id}`;
    live.send({ cmd: "frames", n: frames, sync });
    await live.wait((l) => l.includes(`live: sync ${sync}`));
  };
  // The app: what an embedder does with each post, a frame at a time, until the game closes its socket.
  let closed = false;
  for (let frame = 0; frame < 60 && !closed; frame++) {
    await step(1);
    const out: object[] = [];
    for (const p of posts.slice(handled)) {
      if (p.t === "net.open") out.push({ t: "net.opened", id: p.id });
      else if (p.t === "net.send") out.push({ t: "net.messages", id: p.id, list: [`echo: ${p.data}`] });
      else if (p.t === "net.fetch") out.push({ t: "net.fetched", id: p.id, status: 200, body: "hello from the app" });
      else if (p.t === "net.close") { out.push({ t: "net.closed", id: p.id, code: 1000, reason: "" }); closed = true; }
    }
    handled = posts.length;
    if (out.length) live.send({ cmd: "host", messages: out });
  }
  await step(2);
  await live.quit();
  const net = posts.filter((p) => p.t.startsWith("net."));
  console.log("host echo: live, this script as the app");
  check(net.map((p) => p.t).join() === ["net.open", "net.fetch", ...Array(ROUNDS).fill("net.send"), "net.close"].join(), `the game's posts: ${net.map((p) => p.t).join(", ")}`);
  check(net[0].url === "ws://app.test/echo" && net[1].url === "http://app.test/hello", "launch: the addresses the app opened the game with");
  check(net.filter((p) => p.t === "net.send").map((p) => p.data).join() === "hello,again 1,again 2", `it sent ${net.filter((p) => p.t === "net.send").map((p) => p.data).join(", ")}`);
  check(answers(live.lines).join() === "echo: hello,echo: again 1,echo: again 2", `its listeners got ${answers(live.lines).join(", ")}`);
  check(live.lines.some((l) => l.includes('echo fetched 200 "hello from the app"')) && live.lines.some((l) => l.includes("echo closed 1000")), "and the page, and the close");
  const huds = posts.filter((p) => p.t === "hud").map((p) => messages(p.hud));
  check(huds.at(-1) === String(ROUNDS) && new Set(huds).size === ROUNDS + 1, `what the listeners emitted is in the state: the HUD counted ${[...new Set(huds)].join(", ")}`);
}

// ---------------------------------------------------------------------------------------------- 2. play
if (!process.argv.includes("--no-play")) {
  const { port, got, closeCode, close } = await echoServer();
  const launch = JSON.stringify({ url: `ws://127.0.0.1:${port}/socket`, fetch: `http://127.0.0.1:${port}/hello` });
  const p = spawn(rendererBin, ["play", bundle, "--assets", repoRoot, "--presets", join(repoRoot, "presets"), "--width", "480", "--height", "270", "--launch", launch], { stdio: ["pipe", "pipe", "pipe"] });
  const lines: string[] = [];
  const posts: Post[] = [];
  createInterface({ input: p.stderr! }).on("line", (l) => lines.push(l));
  createInterface({ input: p.stdout! }).on("line", (l) => { try { const m = JSON.parse(l) as { type?: string; messages?: Post[] }; if (m.type === "host") posts.push(...(m.messages ?? [])); } catch { /* not ours */ } });
  const exited = new Promise<void>((r) => p.on("exit", () => r()));
  const done = () => lines.some((l) => l.includes("echo closed")) && lines.some((l) => l.includes("echo fetched"));
  for (let k = 0; k < 100 && !done() && p.exitCode === null; k++) await sleep(100);
  await sleep(300);
  p.stdin!.write(JSON.stringify({ cmd: "quit" }) + "\n");
  const kill = setTimeout(() => p.kill(), 5000);
  await exited;
  clearTimeout(kill);
  close();
  console.log("host echo: play, the window's own sockets");
  check(got.join() === "hello,again 1,again 2", `the server got ${got.join(", ") || "nothing"}`);
  check(answers(lines).join() === "echo: hello,echo: again 1,echo: again 2", `the game got ${answers(lines).join(", ") || "nothing"}`);
  check(closeCode() === 1000 && lines.some((l) => l.includes("echo closed 1000")), `the game's close reached the server (${closeCode()}) and came back`);
  check(lines.some((l) => l.includes('echo fetched 200 "hello from http"')), "the page was fetched");
  check(messages(posts.filter((q) => q.t === "hud").at(-1)?.hud) === String(ROUNDS), "the window passes the HUD on");
  const errors = lines.filter((l) => /host messages:|frame script:|\[game error\]/.test(l));
  check(errors.length === 0, `no script error${errors.length ? `: ${errors[0]}` : ""}`);
}

// ---------------------------------------------------------------------------------------------- 3. three.js
if (!process.argv.includes("--no-three")) {
  const { port, got, closeCode, close } = await echoServer();
  const lines: string[] = [];
  const { html } = await bundlePage("host-echo-view", join(repoRoot, "games/host/echo.game.ts"), join(root, "tools/three/view.ts"));
  const { server, origin } = await serve(root);
  const browser = await launchChrome((l) => lines.push(l));
  let shown = "";
  try {
    const page = await browser.open(`${origin}${html}?url=${encodeURIComponent(`ws://127.0.0.1:${port}/socket`)}&fetch=${encodeURIComponent(`http://127.0.0.1:${port}/hello`)}`);
    const done = () => lines.some((l) => l.includes("echo closed")) && lines.some((l) => l.includes("echo fetched"));
    for (let k = 0; k < 300 && !done(); k++) await sleep(100);
    await sleep(300);
    shown = await page.evaluate<string>(`document.querySelector(".hud")?.innerText ?? ""`);
    await page.close();
  } finally {
    browser.close();
    server.close();
    close();
  }
  console.log("host echo: the three.js view, the browser's sockets");
  check(got.join() === "hello,again 1,again 2", `the server got ${got.join(", ") || "nothing"}`);
  check(answers(lines).join() === "echo: hello,echo: again 1,echo: again 2", `the game got ${answers(lines).join(", ") || "nothing"}`);
  check(closeCode() === 1000 && lines.some((l) => l.includes("echo closed 1000")), `the game's close reached the server (${closeCode()}) and came back`);
  check(lines.some((l) => l.includes('echo fetched 200 "hello from http"')), "the page was fetched");
  check(/MESSAGES\s+3/i.test(shown) && /closed \(1000\)/.test(shown), `the HUD on the page: ${JSON.stringify(shown.replace(/\s+/g, " "))}`);
}

if (failures.length) { console.error(`host echo: ${failures.length} check(s) failed`); process.exit(1); }
console.log("host echo: all checks pass");

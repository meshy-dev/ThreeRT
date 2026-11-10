// Fetches Quaternius' Universal Animation Library [Standard] (CC0): the UE-style mannequin (13.7 k triangles, a 65-bone
// skeleton with UE5 mannequin bone names) and 43 animation clips, as one GLB -> assets/quaternius/ual/UAL1_Standard.glb.
// itch.io only hands out the free download through its page, so this clicks through it in headless Chrome.
// Then, with Blender, the walkers' level of detail: UAL1_Standard_lod1.glb, the same skeleton and clips at 3.5 k
// triangles (tools/quaternius/lod.py; skinned characters are rebuilt into the dynamic BLAS every frame, and the Bistro
// C slice's four walkers were 1.44 ms of it).
//
//   node tools/fetch-quaternius.ts [--force]
import { spawn, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";
import { repoRoot } from "./bundle-game.ts";

const PAGE = "https://quaternius.itch.io/universal-animation-library/purchase";
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const out = join(repoRoot, "assets/quaternius/ual");
const work = join(repoRoot, "out/downloads/quaternius");
const glb = join(out, "UAL1_Standard.glb");
const lod = join(out, "UAL1_Standard_lod1.glb");
/** Triangles kept by the walkers' LOD (13.8 k -> 3.5 k). */
const LOD_RATIO = 0.25;

if (existsSync(glb) && !process.argv.includes("--force")) console.log(`${glb} exists (--force to fetch again)`);
else await fetchPack();
makeLod();

/** The LOD, made again when it is missing or older than the pack. */
function makeLod() {
  if (existsSync(lod) && statSync(lod).mtimeMs >= statSync(glb).mtimeMs) return;
  const blender = [process.env.BLENDER, "/Applications/Blender.app/Contents/MacOS/Blender"].find((b) => b && existsSync(b)) ?? "blender";
  const tmp = `${lod}.tmp.glb`;
  const r = spawnSync(blender, ["-b", "--factory-startup", "--python-exit-code", "1", "--python", join(repoRoot, "tools/quaternius/lod.py"), "--", glb, tmp, String(LOD_RATIO)], { encoding: "utf8" });
  if (r.status !== 0 || !existsSync(tmp)) {
    console.log(`skipping the walkers' LOD: needs Blender (brew install --cask blender, or BLENDER=<binary>)${r.error ? "" : `; Blender exited ${r.status}`}`);
    return;
  }
  renameSync(tmp, lod);
  console.log(`${lod} (${(r.stdout.split("\n").find((l) => l.startsWith("lod:")) ?? "").slice(5)})`);
}

async function fetchPack() {
  rmSync(work, { recursive: true, force: true });
  mkdirSync(join(work, "zip"), { recursive: true });

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const port = 9300 + Math.floor(Math.random() * 500);
  const chrome = spawn(CHROME, ["--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${join(work, "profile")}`, "about:blank"], { stdio: "ignore" });
  try {
    let ws: WebSocket | undefined;
    for (let i = 0; i < 50 && !ws; i++) {
      await sleep(200);
      try {
        const page = ((await (await fetch(`http://127.0.0.1:${port}/json`)).json()) as { type: string; webSocketDebuggerUrl: string }[]).find((t) => t.type === "page");
        if (page) ws = new WebSocket(page.webSocketDebuggerUrl);
      } catch {}
    }
    if (!ws) throw new Error("Chrome did not start");
    await new Promise((r) => ws!.addEventListener("open", r));
    let id = 0;
    const pending = new Map<number, (m: any) => void>();
    ws.addEventListener("message", (e) => {
      const m = JSON.parse(String(e.data));
      pending.get(m.id)?.(m);
      pending.delete(m.id);
    });
    const send = (method: string, params = {}) => new Promise<any>((r) => (pending.set(++id, r), ws!.send(JSON.stringify({ id, method, params }))));
    const evaluate = async (expression: string) => (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result?.result?.value;
    await send("Browser.setDownloadBehavior", { behavior: "allow", downloadPath: join(work, "zip") });
    await send("Page.navigate", { url: PAGE });
    await sleep(4000);
    // "No thanks, just take me to the downloads" (the pack is free; you can support the author on itch.io / Patreon).
    await evaluate("document.querySelector('.direct_download_btn').click()");
    await sleep(5000);
    await evaluate("document.querySelector('.download_btn').click()");
    let zip: string | undefined;
    for (let i = 0; i < 180 && !zip; i++) {
      await sleep(1000);
      zip = readdirSync(join(work, "zip")).find((f) => f.endsWith(".zip"));
    }
    if (!zip) throw new Error(`no download from ${PAGE} (itch.io may be rate limiting; try again later)`);
    const x = join(work, "x");
    if (spawnSync("unzip", ["-q", "-o", join(work, "zip", zip), "-d", x]).status !== 0) throw new Error(`unzip ${zip} failed`);
    const root = join(x, readdirSync(x)[0]);
    mkdirSync(out, { recursive: true });
    copyFileSync(join(root, "Unreal-Godot/UAL1_Standard.glb"), join(out, "UAL1_Standard.glb"));
    copyFileSync(join(root, "License.txt"), join(out, "License.txt"));
    console.log(`${out}/UAL1_Standard.glb (CC0, Quaternius)`);
  } finally {
    chrome.kill();
  }
}

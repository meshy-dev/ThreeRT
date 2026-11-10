// GPU throughput of a warm render node: frames/s and paths/s per scene, preset and traversal backend.
//   node tools/bench-throughput.ts [--runs 3] [--backends hw,sw] [--bin path] [--scenes a,b]
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const arg = (k: string, d: string) => {
  const i = process.argv.indexOf(k);
  return i > 0 ? process.argv[i + 1] : d;
};
const bin = arg("--bin", join(root, "renderer/target/release/threert-render"));
const runs = Number(arg("--runs", "3"));
const backends = arg("--backends", "hw,sw").split(",");
const scenes = arg("--scenes", "cornell-box,material-grid,sun-room,neon-reflections,glb-asset").split(",");
const presets = [
  { name: "preview", spp: 32 },
  { name: "cinematic", spp: 256 },
];
const outDir = join(root, "out/bench");
mkdirSync(join(outDir, "frames"), { recursive: true });

const median = (v: number[]) => [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)];
const results: Record<string, unknown>[] = [];
for (const backend of backends) {
  const t0 = performance.now();
  const proc = spawn(bin, ["serve"], { stdio: ["pipe", "pipe", "inherit"], env: { ...process.env, THREERT_HWRT: backend === "hw" ? "1" : "0" } });
  const lines = createInterface({ input: proc.stdout })[Symbol.asyncIterator]();
  const ready = JSON.parse((await lines.next()).value as string);
  const processStartMs = performance.now() - t0;
  let id = 0;
  const render = async (scene: string, preset: string, spp: number) => {
    proc.stdin.write(JSON.stringify({ id: ++id, game: join(root, "out/bundles", `${scene}.js`), assets: root, out: join(outDir, "frames", `tp-${scene}-${preset}.png`), spp }) + "\n");
    const m = JSON.parse((await lines.next()).value as string);
    if (!m.ok) throw new Error(m.error);
    return m.stats;
  };
  for (const scene of scenes) {
    await render(scene, "warmup", 4);
    for (const p of presets) {
      const totals: number[] = [];
      let last: { samples_per_sec: number; hwrt: boolean; timings_ms: Record<string, number> } | undefined;
      for (let r = 0; r < runs; r++) {
        last = await render(scene, p.name, p.spp);
        totals.push(last!.timings_ms.total_ms);
      }
      const row = { backend, hwrt: last!.hwrt, adapter: ready.adapter, scene, preset: p.name, spp: p.spp, frame_ms: median(totals), mpaths_s: +(last!.samples_per_sec / 1e6).toFixed(1), timings_ms: last!.timings_ms };
      results.push(row);
      console.log(JSON.stringify({ backend, scene, preset: p.name, frame_ms: row.frame_ms, mpaths_s: row.mpaths_s }));
    }
  }
  results.push({ backend, process_start_ms: Math.round(processStartMs), gpu_init_ms: ready.init_ms });
  proc.stdin.end();
}
writeFileSync(join(outDir, `throughput-${backends.join("-")}.json`), JSON.stringify(results, null, 1));

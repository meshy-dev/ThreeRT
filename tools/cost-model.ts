// Cost per screenshot from measured frame times (out/bench/throughput-*.json) and published unit prices.
//   node tools/cost-model.ts [--utilization 0.5] [--cpu-frame-s <seconds per frame on a CF standard-4 container>]
// Prices (USD, checked 2026-11): Cloudflare Workers/DO/R2/Containers list prices; GPU hourly rates are market medians.
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const arg = (k: string, d: string) => {
  const i = process.argv.indexOf(k);
  return i > 0 ? process.argv[i + 1] : d;
};
const utilization = Number(arg("--utilization", "0.5"));
const cpuFrameS = Number(arg("--cpu-frame-s", "NaN"));

const PRICE = {
  workerRequest: 0.3 / 1e6,
  workerCpuMs: 0.02 / 1e6,
  doRequest: 0.15 / 1e6,
  doGbS: 12.5 / 1e6,
  r2ClassA: 4.5 / 1e6,
  r2ClassB: 0.36 / 1e6,
  containerVcpuS: 0.00002,
  containerGibS: 0.0000025,
  gpuHour: { "RTX 4090 (median, $0.48/h)": 0.48, "NVIDIA L4 (AWS g6, $0.80/h)": 0.8, "A10G (AWS g5, $1.01/h)": 1.01 },
};

// Per screenshot control-plane usage (event -> commit -> render job -> result upload -> client fetch):
// gateway requests: event, render, job poll x2, result PUT, PNG GET; DO: room event + snapshot, pool enqueue/dispatch/complete;
// Dynamic Worker: one handler execution; R2: PNG put + code / asset reads (node cache misses amortized to ~2).
const control = {
  workerRequests: 6 + 1,
  workerCpuMs: 15,
  doRequests: 6,
  doActiveS: 1.5,
  r2A: 1,
  r2B: 3,
};
const controlCost =
  control.workerRequests * PRICE.workerRequest +
  control.workerCpuMs * PRICE.workerCpuMs +
  control.doRequests * PRICE.doRequest +
  control.doActiveS * 0.128 * PRICE.doGbS +
  control.r2A * PRICE.r2ClassA +
  control.r2B * PRICE.r2ClassB;

const tpPath = join(root, "out/bench/throughput-hw-sw.json");
const rows = existsSync(tpPath) ? (JSON.parse(readFileSync(tpPath, "utf8")) as Record<string, unknown>[]) : [];
const frames = rows.filter((r) => r.backend === "hw" && r.preset) as { scene: string; preset: string; frame_ms: number }[];
const mean = (preset: string) => {
  const v = frames.filter((f) => f.preset === preset).map((f) => f.frame_ms / 1000);
  return v.reduce((a, b) => a + b, 0) / Math.max(1, v.length);
};
const usd = (v: number) => (v < 0.001 ? `$${(v * 1000).toFixed(4)}m` : `$${v.toFixed(4)}`);

const out: string[] = [];
out.push(`Control plane (Workers + DO + R2 + Dynamic Worker) per screenshot: ${usd(controlCost)} (m = 1/1000 USD)`);
out.push(`GPU node utilization assumed ${utilization * 100}% (scale-to-zero pool; idle tail billed).`);
out.push("");
out.push("| backend | preset | frame time (s) | GPU $/h | $/screenshot (GPU) | + control plane | screenshots per $1 |");
out.push("|---|---|---:|---:|---:|---:|---:|");
for (const preset of ["preview", "cinematic"]) {
  const t = mean(preset);
  for (const [name, hourly] of Object.entries(PRICE.gpuHour)) {
    // Frame times measured on an Apple M5 (10-core GPU). Discrete RT GPUs are faster, so this is an upper bound.
    const gpu = (t * hourly) / 3600 / utilization;
    out.push(`| ${name} (M5-measured time) | ${preset} | ${t.toFixed(2)} | ${hourly} | ${usd(gpu)} | ${usd(gpu + controlCost)} | ${Math.floor(1 / (gpu + controlCost))} |`);
  }
  if (Number.isFinite(cpuFrameS)) {
    const s = cpuFrameS * (preset === "cinematic" ? 8 : 1);
    const c = s * (4 * PRICE.containerVcpuS + 12 * PRICE.containerGibS);
    out.push(`| CF Container standard-4 (lavapipe CPU) | ${preset} | ${s.toFixed(1)} | ${(3600 * (4 * PRICE.containerVcpuS + 12 * PRICE.containerGibS)).toFixed(3)} | ${usd(c)} | ${usd(c + controlCost)} | ${Math.floor(1 / (c + controlCost))} |`);
  }
}
const md = out.join("\n");
writeFileSync(join(root, "out/bench/cost.md"), md + "\n");
console.log(md);

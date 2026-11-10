// Checks the GPU timing lock (tools/lib/bench.ts) with child processes on a private lock file: a holder makes others
// busy and shares the lock with the tools it runs, releases on exit, a killed holder's lock is stale, a freshly created
// (still empty) lock counts as held for 10 s.
//
//   node tools/check-gpu-lock.ts
import { spawn, spawnSync } from "node:child_process";
import { existsSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const lock = `/tmp/threert-gpu-lock-check-${process.pid}.lock`;
const env = { ...process.env, THREERT_GPU_LOCK: lock };
const bench = join(import.meta.dirname, "lib/bench.ts");
const run = (code: string) => spawnSync(process.execPath, ["--input-type=module", "-e", code], { env, encoding: "utf8" });
const probe = `import { acquireGpuLock, gpuBusy } from ${JSON.stringify(bench)}; const h = acquireGpuLock("probe"); console.log(JSON.stringify({ holder: h, busy: gpuBusy().filter((b) => b.startsWith("GPU lock")) }));`;
const failures: string[] = [];
const check = (ok: boolean, what: string) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures.push(what);
};

rmSync(lock, { force: true });
// A holder that waits for stdin to close (then exits normally).
const holder = spawn(process.execPath, ["--input-type=module", "-e", `import { acquireGpuLock } from ${JSON.stringify(bench)}; console.log(JSON.stringify(acquireGpuLock("holder"))); process.stdin.resume(); process.stdin.on("end", () => process.exit(0));`], { env, stdio: ["pipe", "pipe", "inherit"] });
const first = await new Promise<string>((r) => holder.stdout.once("data", (d) => r(String(d).trim())));
check(first === "null", "a free lock is taken");
const busy = JSON.parse(run(probe).stdout);
check(busy.holder?.tool === "holder" && busy.holder?.pid === holder.pid, "another process sees the holder");
check(busy.busy.length === 1, "gpuBusy reports it");
const refused = run(`import { requireGpuLock } from ${JSON.stringify(bench)}; requireGpuLock("second");`);
check(refused.status === 2, "requireGpuLock exits 2 while it is held");
// A tool the holder runs shares its lock (milestone 1 runs tools/compare/strongest.ts): it may measure, and leaves the
// lock to its parent.
const child = run(`process.env.THREERT_GPU_LOCK_OWNER = ${JSON.stringify(String(holder.pid))}; ${probe}`);
const shared = JSON.parse(child.stdout);
check(shared.holder === null && shared.busy.length === 0, "a child of the holder shares the lock");
check(existsSync(lock), "the child leaves the lock to its parent");
holder.stdin.end();
await new Promise((r) => holder.once("exit", r));
check(!existsSync(lock), "released on a normal exit");

// A holder killed outright leaves a stale lock: the next taker clears it.
const killed = spawn(process.execPath, ["--input-type=module", "-e", `import { acquireGpuLock } from ${JSON.stringify(bench)}; acquireGpuLock("killed"); console.log("held"); setInterval(() => {}, 1000);`], { env, stdio: ["ignore", "pipe", "inherit"] });
await new Promise((r) => killed.stdout.once("data", r));
killed.kill("SIGKILL");
await new Promise((r) => killed.once("exit", r));
check(existsSync(lock), "a killed holder leaves its file");
const after = JSON.parse(run(probe).stdout);
check(after.holder === null, "a dead holder's lock is stale: taken");
rmSync(lock, { force: true });

// A lock being written (created, still empty) counts as held; an empty one older than 10 s is stale.
writeFileSync(lock, "");
check(JSON.parse(run(probe).stdout).holder?.tool === "(being written)", "an empty fresh lock counts as held");
const old = (Date.now() - 20_000) / 1000;
utimesSync(lock, old, old);
check(JSON.parse(run(probe).stdout).holder === null, "an empty lock older than 10 s is stale");
rmSync(lock, { force: true });

// A test run waits for the holder (waitGpuLock), then takes the lock.
const holder2 = spawn(process.execPath, ["--input-type=module", "-e", `import { acquireGpuLock } from ${JSON.stringify(bench)}; acquireGpuLock("holder2"); console.log("held"); process.stdin.resume(); process.stdin.on("end", () => process.exit(0));`], { env, stdio: ["pipe", "pipe", "inherit"] });
await new Promise((r) => holder2.stdout.once("data", r));
const waiter = spawn(process.execPath, ["--input-type=module", "-e", `import { waitGpuLock, gpuLockHolder } from ${JSON.stringify(bench)}; await waitGpuLock("waiter"); console.log(JSON.stringify(gpuLockHolder()));`], { env, stdio: ["ignore", "pipe", "ignore"] });
let waited = "";
waiter.stdout.on("data", (d) => (waited += d));
// "close" (not "exit"): it comes after the child's stdout has been read to its end. Listened for from the start, so a
// waiter that finished during the pause below is still seen (and reported) rather than hanging the check.
const waiterClosed = new Promise((r) => waiter.once("close", r));
await new Promise((r) => setTimeout(r, 3000));
check(waited === "", "waitGpuLock waits while the lock is held");
holder2.stdin.end();
await waiterClosed;
check(JSON.parse(waited.trim() || "null")?.tool === "waiter", "and takes it once released");
rmSync(lock, { force: true });

if (failures.length) {
  console.log(`gpu lock: ${failures.length} failed`);
  process.exit(1);
}
console.log("gpu lock: ok");

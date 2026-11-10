// The physics' determinism check (ROADMAP § 6 step 7): physics-core run natively and as a wasm32 module, the browser's
// target. Both must end in the same bits:
//   - the script (renderer/physics-core/src/check.rs: a tower of boxes hit by a ball, a hinged door, a rope, a ball
//     joint and a motor-driven slider, 600 ticks): the two hashes are compared with each other and the recorded one;
//   - 1,000 random worlds (`check::case`: tumbling convex hulls, bodies of several colliders, joints with limits and
//     motors, 90 ticks each), every one compared, and their folded hash with the recorded one. The script alone has
//     one hull that happens to agree; these are the shapes whose mass properties need an eigen decomposition, where
//     the platforms' math functions first differed.
// Rapier's `enhanced-determinism` (physics-core's default) and Rapier built without `std` are what make them agree
// (physics-core/Cargo.toml); `--without` builds without the former and reports (they differ; no exit code).
//   node tools/physics-determinism.ts [--without]      (needs the wasm32-unknown-unknown target)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const crate = join(import.meta.dirname, "../renderer/physics-core");
const cargo = (args: string[]) => execFileSync("cargo", args, { cwd: crate, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"], maxBuffer: 1 << 26 });
const without = process.argv.includes("--without") ? ["--no-default-features"] : [];
const run = ["run", "--release", "--locked", "--quiet", ...without, "--example", "determinism"];
const native = cargo(run).trim();
const nativeCases = cargo([...run, "--", "--cases"]).trim().split("\n").map((l) => l.split(" "));
cargo(["build", "--release", "--locked", "--quiet", ...without, "--target", "wasm32-unknown-unknown", "--example", "determinism_wasm"]);
const wasm = readFileSync(join(crate, "target/wasm32-unknown-unknown/release/examples/determinism_wasm.wasm"));
const { instance } = await WebAssembly.instantiate(wasm, {});
const hex = (h: bigint) => BigInt.asUintN(64, h).toString(16).padStart(16, "0");
const browser = hex((instance.exports.physics_determinism as (ticks: number) => bigint)(600));
const source = readFileSync(join(crate, "src/check.rs"), "utf8");
const constant = (name: string) => new RegExp(`${name}: u64 = 0x([0-9a-f_]+);`).exec(source)![1].replaceAll("_", "").padStart(16, "0");
const label = without.length ? " (without enhanced-determinism)" : "";
console.log(`physics determinism${label}: script: native ${native}, wasm32 ${browser}, recorded ${constant("EXPECTED")}`);

// The random cases, one by one; and all of them folded as check::cases() folds them.
const kinds = [...source.matchAll(/\("([^"]+)", (\d+)\)/g)].map((m) => m[1]);
const differ = kinds.map(() => [] as string[]), count = kinds.map(() => 0);
let folded = 0xcbf29ce484222325n;
const caseOf = instance.exports.physics_determinism_case as (kind: number, seed: number) => bigint;
for (const [kind, seed, hash] of nativeCases) {
  const w = hex(caseOf(Number(kind), Number(seed)));
  count[Number(kind)]++;
  if (w !== hash) differ[Number(kind)].push(`seed ${seed}: native ${hash}, wasm32 ${w}`);
  folded = BigInt.asUintN(64, (folded ^ BigInt(`0x${hash}`)) * 0x100000001b3n);
}
let failed = native !== browser || native !== constant("EXPECTED");
kinds.forEach((name, k) => {
  console.log(`physics determinism${label}: ${name}: ${count[k] - differ[k].length} of ${count[k]} equal natively and in wasm32`);
  for (const d of differ[k].slice(0, 3)) console.log(`  ${d}`);
  failed ||= differ[k].length > 0;
});
console.log(`physics determinism${label}: cases: native ${hex(folded)}, recorded ${constant("EXPECTED_CASES")}`);
failed ||= hex(folded) !== constant("EXPECTED_CASES");
if (without.length) process.exit(0);
if (failed) {
  console.error("physics determinism: the hashes differ");
  process.exit(1);
}

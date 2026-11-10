// The voxel engine's determinism check (ROADMAP § 6 step 6): voxel-core's edit script (renderer/voxel-core/src/check.rs:
// explosions by hardness, cuts, paint, added voxels, parts falling and landing, on a small level) run natively and as a
// wasm32 module, the browser's target. Both must leave the same level, bit for bit: their hashes are compared with each
// other and with the recorded one. Exit 1 when they differ.
//   node tools/voxel-determinism.ts        (builds both; needs the wasm32-unknown-unknown target)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const crate = join(import.meta.dirname, "../renderer/voxel-core");
const cargo = (args: string[]) => execFileSync("cargo", args, { cwd: crate, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
const native = cargo(["run", "--release", "--locked", "--quiet", "--example", "determinism"]).trim();
cargo(["build", "--release", "--locked", "--quiet", "--target", "wasm32-unknown-unknown", "--example", "determinism_wasm"]);
const wasm = readFileSync(join(crate, "target/wasm32-unknown-unknown/release/examples/determinism_wasm.wasm"));
const { instance } = await WebAssembly.instantiate(wasm, {});
const hash = (instance.exports.voxel_determinism as () => bigint)();
const browser = BigInt.asUintN(64, hash).toString(16).padStart(16, "0");
const recorded = /EXPECTED: u64 = 0x([0-9a-f_]+);/.exec(readFileSync(join(crate, "src/check.rs"), "utf8"))![1].replaceAll("_", "");
console.log(`voxel determinism: native ${native}, wasm32 ${browser}, recorded ${recorded}`);
if (native !== browser || native !== recorded) {
  console.error("voxel determinism: the hashes differ");
  process.exit(1);
}

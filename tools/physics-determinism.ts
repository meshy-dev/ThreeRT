// The physics' determinism check (ROADMAP § 6 step 7): physics-core's script (renderer/physics-core/src/check.rs: a
// tower of boxes hit by a ball, a hinged door, a rope, a ball joint and a motor-driven slider, 600 ticks) run natively
// and as a wasm32 module, the browser's target. Both must end in the same bits: their hashes are compared with each
// other and with the recorded one. Rapier's `enhanced-determinism` (physics-core's default) is what makes them agree;
// `--without` builds without it and reports the two hashes (they differ on this machine; no exit code).
//   node tools/physics-determinism.ts [--without]      (needs the wasm32-unknown-unknown target)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const crate = join(import.meta.dirname, "../renderer/physics-core");
const cargo = (args: string[]) => execFileSync("cargo", args, { cwd: crate, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
const without = process.argv.includes("--without") ? ["--no-default-features"] : [];
const native = cargo(["run", "--release", "--locked", "--quiet", ...without, "--example", "determinism"]).trim();
cargo(["build", "--release", "--locked", "--quiet", ...without, "--target", "wasm32-unknown-unknown", "--example", "determinism_wasm"]);
const wasm = readFileSync(join(crate, "target/wasm32-unknown-unknown/release/examples/determinism_wasm.wasm"));
const { instance } = await WebAssembly.instantiate(wasm, {});
const hash = (instance.exports.physics_determinism as (ticks: number) => bigint)(600);
const browser = BigInt.asUintN(64, hash).toString(16).padStart(16, "0");
const recorded = /EXPECTED: u64 = 0x([0-9a-f_]+);/.exec(readFileSync(join(crate, "src/check.rs"), "utf8"))![1].replaceAll("_", "");
console.log(`physics determinism${without.length ? " (without enhanced-determinism)" : ""}: native ${native}, wasm32 ${browser}, recorded ${recorded}`);
if (without.length) process.exit(0);
if (native !== browser || native !== recorded) {
  console.error("physics determinism: the hashes differ");
  process.exit(1);
}

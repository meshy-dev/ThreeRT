// The physics' determinism check (ROADMAP § 6 step 7): physics-core run natively and as a wasm32 module, the browser's
// target. Both must end in the same bits:
//   - the script (renderer/physics-core/src/check.rs: a tower of boxes hit by a ball, a hinged door, a rope, a ball
//     joint and a motor-driven slider, 600 ticks): the two hashes are compared with each other and the recorded one;
//   - 1,000 random worlds (`check::case`: tumbling convex hulls, bodies of several colliders, joints with limits and
//     motors, 90 ticks each), every one compared, and their folded hash with the recorded one. The script alone has
//     one hull that happens to agree; these are the shapes whose mass properties need an eigen decomposition, where
//     the platforms' math functions first differed (48 of the 1,000 before glamx was patched).
// Both are run twice: as physics-core builds on its own, and with `std` in the crate graph (the crate's `std-in-graph`
// feature: what a larger program's other dependencies do, the renderer's among them), where Rapier's crates take
// f32's own math functions unless they are kept from it (renderer/third_party/glamx/PATCH.md). Without the patch the
// first run passes and the second fails.
// Then the renderer's dependency graph, built for wasm32 (renderer/examples/physics_bits.rs), against the recorded
// hashes: the engine's library in a browser.
// Then Rapier's parallel solver (physics-core's `parallel` feature: what the renderer's native build steps the world
// with, ROADMAP § 6 step 8), on 2 and on 4 threads: the script and every one of the random worlds against the
// sequential native build's, which the steps above hold to wasm32 and the recorded hashes. So native parallel =
// native sequential = wasm32 = recorded: a native build steps as a browser does.
// `--without` builds without Rapier's `enhanced-determinism` instead and reports (the hashes differ; no exit code).
//   node tools/physics-determinism.ts [--without]      (needs the wasm32-unknown-unknown target)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const crate = join(import.meta.dirname, "../renderer/physics-core");
const cargo = (args: string[]) => execFileSync("cargo", args, { cwd: crate, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"], maxBuffer: 1 << 26 });
const without = process.argv.includes("--without");
const source = readFileSync(join(crate, "src/check.rs"), "utf8");
const constant = (name: string) => new RegExp(`${name}: u64 = 0x([0-9a-f_]+);`).exec(source)![1].replaceAll("_", "").padStart(16, "0");
const kinds = [...source.matchAll(/\("([^"]+)", (\d+)\)/g)].map((m) => m[1]);
const hex = (h: bigint) => BigInt.asUintN(64, h).toString(16).padStart(16, "0");
const builds: [string, string[]][] = without ? [[" (without enhanced-determinism)", ["--no-default-features"]]] : [["", []], [" (std in the crate graph)", ["--features", "std-in-graph"]]];
let failed = false;
// The sequential native build's script and cases (the first build's), for the parallel build below.
let sequential: { script: string; cases: string } | null = null;

for (const [label, features] of builds) {
  const run = ["run", "--release", "--locked", "--quiet", ...features, "--example", "determinism"];
  const native = cargo(run).trim();
  const casesOut = cargo([...run, "--", "--cases"]).trim();
  const nativeCases = casesOut.split("\n").map((l) => l.split(" "));
  sequential ??= { script: native, cases: casesOut };
  cargo(["build", "--release", "--locked", "--quiet", ...features, "--target", "wasm32-unknown-unknown", "--example", "determinism_wasm"]);
  const wasm = readFileSync(join(crate, "target/wasm32-unknown-unknown/release/examples/determinism_wasm.wasm"));
  const { instance } = await WebAssembly.instantiate(wasm, {});
  const browser = hex((instance.exports.physics_determinism as (ticks: number) => bigint)(600));
  console.log(`physics determinism${label}: script: native ${native}, wasm32 ${browser}, recorded ${constant("EXPECTED")}`);
  failed ||= native !== browser || native !== constant("EXPECTED");

  // The random cases, one by one; and all of them folded as check::cases() folds them.
  const differ = kinds.map(() => [] as string[]), count = kinds.map(() => 0);
  let folded = 0xcbf29ce484222325n;
  const caseOf = instance.exports.physics_determinism_case as (kind: number, seed: number) => bigint;
  for (const [kind, seed, hash] of nativeCases) {
    const w = hex(caseOf(Number(kind), Number(seed)));
    count[Number(kind)]++;
    if (w !== hash) differ[Number(kind)].push(`seed ${seed}: native ${hash}, wasm32 ${w}`);
    folded = BigInt.asUintN(64, (folded ^ BigInt(`0x${hash}`)) * 0x100000001b3n);
  }
  kinds.forEach((name, k) => {
    console.log(`physics determinism${label}: ${name}: ${count[k] - differ[k].length} of ${count[k]} equal natively and in wasm32`);
    for (const d of differ[k].slice(0, 3)) console.log(`  ${d}`);
    failed ||= differ[k].length > 0;
  });
  console.log(`physics determinism${label}: cases: native ${hex(folded)}, recorded ${constant("EXPECTED_CASES")}`);
  failed ||= hex(folded) !== constant("EXPECTED_CASES");
}
// Rapier's parallel solver: the same bits as the sequential build, whatever the thread count.
if (!without && sequential) {
  const run = ["run", "--release", "--locked", "--quiet", "--features", "parallel", "--example", "determinism"];
  for (const threads of ["2", "4"]) {
    const parallel = (args: string[]) => execFileSync("cargo", args, { cwd: crate, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"], maxBuffer: 1 << 26, env: { ...process.env, RAYON_NUM_THREADS: threads } }).trim();
    const [script, cases] = [parallel(run), parallel([...run, "--", "--cases"])];
    const [mine, theirs] = [cases.split("\n"), sequential.cases.split("\n")];
    const differ = mine.filter((l, k) => l !== theirs[k]);
    console.log(`physics determinism (parallel, ${threads} threads): script ${script}, sequential ${sequential.script}, recorded ${constant("EXPECTED")}; ${mine.length - differ.length} of ${theirs.length} worlds equal the sequential build's`);
    for (const d of differ.slice(0, 3)) console.log(`  ${d}`);
    failed ||= script !== sequential.script || script !== constant("EXPECTED") || mine.length !== theirs.length || differ.length > 0;
  }
}
// The renderer's own dependency graph built for wasm32 (the engine's library in a browser): the same recorded hashes.
// (It holds the wasm32 side; a glamx patch left out shows natively, in the renderer's own test, not here.)
if (!without) {
  const renderer = join(import.meta.dirname, "../renderer");
  execFileSync("cargo", ["build", "--release", "--locked", "--quiet", "--target", "wasm32-unknown-unknown", "--example", "physics_bits"], { cwd: renderer, stdio: ["ignore", "ignore", "inherit"] });
  // (The voxel adapter is in the renderer's library, whose clock and logging are the browser's: imports this check
  // has no use for. Each is given a function that returns 0; the scripts read no clock and log nothing.)
  const module = await WebAssembly.compile(readFileSync(join(renderer, "target/wasm32-unknown-unknown/release/examples/physics_bits.wasm")));
  const imports: Record<string, Record<string, () => number>> = {};
  for (const i of WebAssembly.Module.imports(module)) (imports[i.module] ??= {})[i.name] = () => 0;
  const bits = { instance: await WebAssembly.instantiate(module, imports) };
  const [script, cases] = [hex((bits.instance.exports.physics_script as () => bigint)()), hex((bits.instance.exports.physics_cases as () => bigint)())];
  console.log(`physics determinism (the renderer's graph, wasm32): script ${script}, cases ${cases}`);
  failed ||= script !== constant("EXPECTED") || cases !== constant("EXPECTED_CASES");
  // The voxel bodies' script (renderer/src/voxel.rs `VoxelWorld::check_script`: ROADMAP § 6 step 8): the adapter between
  // the voxels and the physics. The native side is the renderer's test `voxel_bodies_step_to_the_recorded_bits`.
  const voxel = hex((bits.instance.exports.voxel_bodies_script as () => bigint)());
  const recorded = /EXPECTED_SCRIPT: u64 = 0x([0-9a-f_]+);/.exec(readFileSync(join(renderer, "src/voxel.rs"), "utf8"))![1].replaceAll("_", "").padStart(16, "0");
  console.log(`physics determinism (the renderer's graph, wasm32): voxel bodies ${voxel}, recorded ${recorded}`);
  failed ||= voxel !== recorded;
}
if (without) process.exit(0);
if (failed) {
  console.error("physics determinism: the hashes differ");
  process.exit(1);
}

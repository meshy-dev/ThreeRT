// Where UE's and our default auto exposure land on the night scene compare/hello/threert/neon-reflections.ts
// (docs/COMPARE.md). UE: out/ue-neon-ae from `bun ue5/neon-ae.ts` (Lumen, project-default auto exposure, and manual
// EV100 renders of the same frame): its EV100 is where the auto image's mean linear luminance falls between the
// manual ones. Ours: the offline renderer's converged auto exposure on the same scene.
//   node tools/neon-ae.ts [--bin renderer/target/release/threert-render]
import { execFileSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const bin = resolve(args.includes("--bin") ? args[args.indexOf("--bin") + 1] : join(root, "renderer/target/release/threert-render"));
const dir = join(root, "out/ue-neon-ae");
const evs = readdirSync(dir).map((f) => /^ev([+-]\d+)\.png$/.exec(f)?.[1]).filter((v): v is string => !!v).map(Number).sort((a, b) => a - b);
// Mean linear luminance of each manual render relative to the auto one (renderer `compare`: test / reference).
const rel = evs.map((ev) => ({ ev, r: JSON.parse(execFileSync(bin, ["compare", join(dir, "auto.png"), join(dir, `ev${ev >= 0 ? "+" : ""}${ev}.png`)]).toString().trim().split("\n").pop()!).luminance_ratio as number }));
let ue = NaN;
for (let i = 0; i + 1 < rel.length; i++) {
  const [a, b] = [rel[i], rel[i + 1]];
  if (a.r >= 1 && b.r <= 1) ue = a.ev + (Math.log(a.r) / Math.log(a.r / b.r)) * (b.ev - a.ev);
}
const bundle = join(root, "out/compare/neon-hello.bundle.js");
await bundleGame(join(root, "compare/hello/threert/neon-reflections.ts"), bundle);
const ours = JSON.parse(execFileSync(bin, ["render", bundle, "--assets", root, "--spp", "64", "-o", join(root, "out/compare/neon-hello-ae.png")]).toString().trim().split("\n").pop()!).ev100 as number;
console.table(rel.map(({ ev, r }) => ({ ev100: ev, "luminance vs auto": r })));
console.log(`UE 5.8 default auto exposure: EV100 ${ue.toFixed(2)}; ours: EV100 ${ours.toFixed(2)} (UE ${(ours - ue).toFixed(1)} stops brighter)`);

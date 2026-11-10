// UE 5.8 references (path tracer, Lumen) for the render suite's layer-B scenes (scenes/khronos-*.ts): writes each
// scene's job (model file, lift, camera, sun, exposure: the same framing as scenes/khronos/studio.ts) and runs
// ue5/glb-scene.ts -> out/ue/{pathtracer,lumen}/<scene>.png. The GPU must be free (UE renders).
//
//   node tools/ue-glb-scene.ts [--assets DIR] [--methods pathtracer,lumen] [scene ids...]
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { EV100, FOV, framing, SUN, type Exhibit } from "../scenes/khronos/studio.ts";
import { repoRoot } from "./lib/bench.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const assets = resolve(flag("--assets", repoRoot));
const methods = flag("--methods", "pathtracer,lumen").split(",");
const wanted = args.filter((a, i) => !a.startsWith("--") && !args[i - 1]?.startsWith("--"));
const work = join(repoRoot, "out/ue-glb");
mkdirSync(work, { recursive: true });

const ids = readdirSync(join(repoRoot, "scenes")).filter((f) => f.startsWith("khronos-") && f.endsWith(".ts")).map((f) => f.replace(/\.ts$/, ""));
for (const id of ids.filter((i) => !wanted.length || wanted.includes(i))) {
  const { exhibit } = (await import(join(repoRoot, "scenes", `${id}.ts`))) as { exhibit: Exhibit };
  const glb = join(assets, "assets/khronos", exhibit.file);
  if (!existsSync(glb)) throw new Error(`${glb} missing: node tools/fetch-khronos.ts`);
  const f = framing(exhibit);
  const job = { name: id, glb, lift: f.lift, eye: f.eye, target: f.target, vfov: FOV, ev100: EV100, sun: SUN, outRoot: join(repoRoot, "out/ue"), work, methods, presetsDir: join(repoRoot, "presets"), width: 1280, height: 720, quit: true };
  const file = join(work, `${id}.json`);
  writeFileSync(file, JSON.stringify(job, null, 1));
  console.log(`== ${id}`);
  execFileSync("bun", [join(repoRoot, "ue5/glb-scene.ts"), file], { stdio: "inherit" });
}

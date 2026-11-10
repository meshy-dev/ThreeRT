// UE 5.8 reference for the volume presets (ROADMAP § 5 item 6.3, part 3): tools/volume-scene.ts rebuilt in a scratch
// project (ue5/scripts/ue_volumes.py), one Path Tracer and one Lumen still as linear EXR.
//   bun ue5/volumes.ts [--spp 1024] [--methods pathtracer,lumen] [--no-fog] [--bounces 8] [--out DIR]
//     -> out/ue-volumes/{pathtracer,lumen}.exr, manifest.json
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { ROOT, runEditor, scratchProject } from "./lib.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const work = join(ROOT, "out/ue-volumes");
mkdirSync(work, { recursive: true });
const project = scratchProject(join(work, "project"), "ThreeRTVolumes", "ThreeRT scratch project: UE references for the volume presets");
// The levels are rebuilt every run; a level left by an aborted run cannot be deleted from inside the editor.
for (const m of ["pathtracer", "lumen"]) rmSync(join(work, "project/Content/ThreeRT", `L_Volumes_${m}.umap`), { force: true });
const job = join(work, "job.json");
const spec = { out: opt("--out", work), ...(args.includes("--no-fog") ? { noFog: true } : {}), bounces: Number(opt("--bounces", "8")), methods: opt("--methods", "pathtracer,lumen").split(","), width: 1280, height: 720, spp: Number(opt("--spp", "1024")), ev100: 3 };
writeFileSync(job, JSON.stringify(spec) + "\n");
console.log(JSON.stringify(spec));
await runEditor({ project, script: join(ROOT, "ue5/scripts/ue_volumes.py"), env: { THREERT_JOB: job }, log: join(work, "ue.log") });

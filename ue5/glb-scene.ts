// UE 5.8 ground truth for a static glTF scene of the render suite's layer B (ue5/scripts/ue_glb_scene.py), built natively
// in a scratch project like ue5/mannequin.ts. The job (model, lift, camera, sun, methods) comes from tools/ue-glb-scene.ts.
//   bun ue5/glb-scene.ts job.json     -> out/ue/{lumen,pathtracer}/<name>.png
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { ROOT, abs, list, runEditor, scratchProject } from "./lib.ts";

const job = process.argv[2] && abs(process.argv[2]);
if (!job) throw new Error("usage: bun ue5/glb-scene.ts job.json");
const work = join(ROOT, "out/ue-glb");
const project = scratchProject(join(work, "project"), "ThreeRTGlbScene", "ThreeRT scratch project: UE ground truth for layer-B glTF scenes");
const name: string = JSON.parse(readFileSync(job, "utf8")).name;
await runEditor({ project, script: join(ROOT, "ue5/scripts/ue_glb_scene.py"), env: { THREERT_JOB: job }, log: join(work, `${name}.log`), lines: 40 });
await list(...["lumen", "pathtracer"].map((m) => join(ROOT, "out/ue", m, `${name}.png`)));

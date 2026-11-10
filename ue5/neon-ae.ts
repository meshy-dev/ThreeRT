// Where UE 5.8's default auto exposure lands on compare/hello/threert/neon-reflections.ts (ue5/scripts/ue_neon_ae.py):
// Lumen renders with the project's default auto exposure and at manual EV100 values, in a scratch project.
//   bun ue5/neon-ae.ts   -> out/ue-neon-ae/{auto,ev-3,...}.png; tools/neon-ae.ts reads the EV100 off them
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { ROOT, list, runEditor, scratchProject } from "./lib.ts";

const work = join(ROOT, "out/ue-neon-ae-work");
const out = join(ROOT, "out/ue-neon-ae");
const project = scratchProject(join(work, "project"), "ThreeRTNeonAE", "ThreeRT scratch project: UE default auto exposure on the hello night scene");
const job = join(work, "job.json");
writeFileSync(job, JSON.stringify({ out, evs: [-7, -6, -5, -4, -3, -2, -1, 0, 1, 2, 4, 6], width: 1280, height: 720, quit: true }) + "\n");
await runEditor({ project, script: join(ROOT, "ue5/scripts/ue_neon_ae.py"), env: { THREERT_JOB: job }, log: join(work, "ue.log"), lines: 40 });
await list(out);

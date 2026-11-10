// UE 5.8 reference for a burst of sparks and debris (ROADMAP § 5 item 6.2): UE's SimpleExplosion Niagara template,
// unchanged, in a scratch project (ue5/scripts/ue_burst.py), captured with MRQ at 60 fps.
//   bun ue5/burst.ts   -> out/ue-burst/frames/0000.png .. (frame 10: the burst fires), manifest.json
// The scene matches tools/burst-scene.ts (keep in sync): floor, camera, exposure.
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { ROOT, runEditor, scratchProject } from "./lib.ts";

const work = join(ROOT, "out/ue-burst");
const project = scratchProject(join(work, "project"), "ThreeRTBurst", "ThreeRT scratch project: UE reference for a spark / debris burst");
const job = join(work, "job.json");
const spec = { frames: join(work, "frames"), eye: [0, 1.6, 5], target: [0, 0.8, 0], vfov: 50, ev100: 6, fire: 10, end: 150, width: 1280, height: 720, screenPercentage: 100 };
writeFileSync(job, JSON.stringify(spec) + "\n");
console.log(JSON.stringify(spec));
await runEditor({ project, script: join(ROOT, "ue5/scripts/ue_burst.py"), env: { THREERT_JOB: job }, log: join(work, "ue.log") });

// UE 5.8 ground truth for scenes/mannequin.ts, built natively in a scratch project (ue5/scripts/ue_mannequin.py):
// Interchange GLB import (skeletal mesh + skeleton + clips), four posed SkeletalMeshActors, ground / blocks, sun + sky
// atmosphere + real-time sky light, cinematic post, MRQ (Lumen and / or the path tracer).
//   bun ue5/mannequin.ts [lumen|pathtracer|both] [glb]   -> out/ue/{lumen,pathtracer}/mannequin.png, out/ue-mannequin/
//   PORTRAIT=1 ...: through the portrait lens of scenes/mannequin-portrait.ts (depth of field) -> mannequin-portrait.png
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { ROOT, abs, list, runEditor, scratchProject } from "./lib.ts";

const method = process.argv[2] ?? "both";
const glb = abs(process.argv[3] ?? join(ROOT, "assets/quaternius/ual/UAL1_Standard.glb")); // bun scripts/setup.ts fetches it
const work = join(ROOT, "out/ue-mannequin");
const project = scratchProject(join(work, "project"), "ThreeRTMannequin", "ThreeRT scratch project: UE ground truth for scenes/mannequin.ts");
// scenes/mannequin-portrait.ts PORTRAIT (keep in sync).
const portrait = process.env.PORTRAIT === "1" ? { target: [1.65, 0.9, -0.15], vfov: 20, focus: 5.9, fstop: 1.2 } : null;
const job = join(work, "job.json");
const spec = {
  glb, outRoot: join(ROOT, "out/ue"), work, methods: method === "both" ? ["lumen", "pathtracer"] : [method], presetsDir: join(ROOT, "presets"),
  width: 1280, height: 720, quit: true, portrait,
};
writeFileSync(job, JSON.stringify(spec) + "\n");
console.log(JSON.stringify(spec));
await runEditor({ project, script: join(ROOT, "ue5/scripts/ue_mannequin.py"), env: { THREERT_JOB: job }, log: join(work, "ue.log") });
const name = portrait ? "mannequin-portrait" : "mannequin";
await list(...["lumen", "pathtracer"].map((m) => join(ROOT, "out/ue", m, `${name}.png`)));

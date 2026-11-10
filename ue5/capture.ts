// Ground-truth stills of an existing UE level (ue5/scripts/ue_capture.py):  bun ue5/capture.ts <project.uproject> <job.json>
import { mkdirSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { ROOT, abs, list, runEditor } from "./lib.ts";

const [project, jobArg] = process.argv.slice(2).map(abs);
if (!project || !jobArg) throw new Error("usage: bun ue5/capture.ts <project.uproject> <job.json>");
const out: string = JSON.parse(readFileSync(jobArg, "utf8")).out;
mkdirSync(out, { recursive: true });
await runEditor({ project, script: join(ROOT, "ue5/scripts/ue_capture.py"), env: { THREERT_CAPTURE: jobArg }, log: join(out, "ue.log"), lines: 40 });
await list(...readdirSync(out).filter((f) => f.endsWith(".png")).map((f) => join(out, f)));

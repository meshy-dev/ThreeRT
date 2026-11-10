// UE real-time motion references for tools/bench-temporal.ts (ue5/scripts/ue_temporal.py): camera move v1 and closing
// time on the actual Kabukicho level, MRQ at 60 fps with 1 temporal sample per frame.
//   bun ue5/temporal.ts [project.uproject] [screenPercentage]   -> out/bench-temporal/refs-ue/*.png + manifest.json
//   MB=1 bun ue5/temporal.ts ...: with UE's default motion blur (amount 0.5, max 5 %, target 30 fps) -> refs-ue-mb/
//   MOVE=strafe bun ue5/temporal.ts ...: tools/bench-ghosting.ts' strafe instead (3 m right at 7.5 m/s, back) ->
//     refs-ue-strafe/ (MB=1: refs-ue-strafe-mb/), compared by bench-ghosting --ue. KEEP=1 keeps every frame
//     (out/ue-kb-temporal/frames).
// The project must be a scratch copy (default out/ue-kb/project): its level is reset to the original project's
// L_Kabukicho.umap (KB_ORIGINAL, read only) first, so earlier ue5/capture.ts edits (manual exposure, linear post) don't
// leak in, then saved with crowd / rain particles hidden (RAIN=on: rain on, its streaks shown; RAIN=nofx: rain on,
// streaks hidden).
import { $ } from "bun";
import { copyFileSync, existsSync, mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { ROOT, abs, list, runEditor } from "./lib.ts";

const project = abs(process.argv[2] ?? join(ROOT, "out/ue-kb/project/Kabukicho.uproject"));
const sp = Number(process.argv[3] ?? 100);
const original = process.env.KB_ORIGINAL ?? join(process.env.HOME!, "code/KabukichoNight");
const work = join(ROOT, "out/ue-kb-temporal");
mkdirSync(work, { recursive: true });
if (realpathSync(dirname(project)).startsWith(realpathSync(original))) throw new Error("refusing to run on the original project");
const map = join(dirname(project), "Content/Kabukicho/Maps/L_Kabukicho.umap");
if (!existsSync(join(work, "L_Kabukicho.scratch-before.umap"))) copyFileSync(map, join(work, "L_Kabukicho.scratch-before.umap"));
copyFileSync(join(original, "Content/Kabukicho/Maps/L_Kabukicho.umap"), map);
const groups = JSON.parse(await $`node ${join(ROOT, "ue5/scripts/kb_groups.ts")}`.text());
const mb = process.env.MB === "1";
const move = process.env.MOVE === "strafe" ? "strafe" : "v1";
// Below 100 % screen percentage the refs go to their own directory (refs-ue-strafe-sp50, ...).
// RAIN=on: rain on with its falling streaks shown (refs-...-rain); RAIN=nofx: rain on, streaks hidden (refs-...-rainnofx).
const rain = process.env.RAIN === "on" || process.env.RAIN === "nofx" ? process.env.RAIN : null;
const refs = join(ROOT, "out/bench-temporal", (move === "strafe" ? "refs-ue-strafe" : "refs-ue") + (mb ? "-mb" : "") + (sp !== 100 ? `-sp${sp}` : "") + (rain === "on" ? "-rain" : rain === "nofx" ? "-rainnofx" : ""));
const job = join(work, "job.json");
writeFileSync(job, JSON.stringify({
  map: "/Game/Kabukicho/Maps/L_Kabukicho", move, frames: join(work, "frames"), refs, motionBlur: mb ? { amount: 0.5, max: 5, targetFps: 30 } : null,
  width: 1280, height: 720, hide: rain === "on" ? ["KabukichoCrowd"] : ["KabukichoCrowd", "KabukichoRain"], rain: rain !== null, screenPercentage: sp, seed: 1,
  cvars: {
    "r.MotionBlurQuality": mb ? 4 : 0, "r.ScreenPercentage": sp, "sg.ViewDistanceQuality": 3, "sg.AntiAliasingQuality": 3, "sg.ShadowQuality": 3,
    "sg.GlobalIlluminationQuality": 3, "sg.ReflectionQuality": 3, "sg.PostProcessQuality": 3, "sg.TextureQuality": 3, "sg.EffectsQuality": 3,
    "sg.FoliageQuality": 3, "sg.ShadingQuality": 3,
  },
  groups, keepFrames: process.env.KEEP === "1",
}) + "\n");
await runEditor({ project, script: join(ROOT, "ue5/scripts/ue_temporal.py"), env: { THREERT_TEMPORAL: job }, log: join(work, "ue.log") });
await list(refs);

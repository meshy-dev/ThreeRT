// Shared pieces of the UE ground-truth runners (ue5/*.ts, Bun): the editor binary, the blueprint-only scratch project
// they build their levels in, and a run of the editor on one of ue5/scripts/*.py that reaps it once the job is done.
import { $ } from "bun";
import { existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

export const ROOT = resolve(import.meta.dir, "..");
export const UE_ROOT = process.env.UE_ROOT ?? "/Users/Shared/Epic Games/UE_5.8";
export const EDITOR = join(UE_ROOT, "Engine/Binaries/Mac/UnrealEditor.app/Contents/MacOS/UnrealEditor");

/** Absolute path of a command-line argument (relative to the caller's working directory). */
export const abs = (p: string) => resolve(process.cwd(), p);

/**
 * Creates `<dir>/<name>.uproject` + Config/DefaultEngine.ini (blueprint-only; the renderer baseline of the former
 * ue5/ThreeRTRef project: Lumen + HW ray tracing, VSM, path tracer, TSR, SM6) unless present. Returns the .uproject.
 */
export function scratchProject(dir: string, name: string, description: string): string {
  mkdirSync(join(dir, "Config"), { recursive: true });
  const uproject = join(dir, `${name}.uproject`);
  if (!existsSync(uproject)) {
    const plugins = ["PythonScriptPlugin", "EditorScriptingUtilities", "MovieRenderPipeline", "SequencerScripting", "Interchange", "InterchangeAssets", "InterchangeEditor"];
    writeFileSync(uproject, JSON.stringify({ FileVersion: 3, EngineAssociation: "5.8", Category: "", Description: description, Plugins: plugins.map((Name) => ({ Name, Enabled: true })) }, null, "\t") + "\n");
  }
  const ini = join(dir, "Config/DefaultEngine.ini");
  if (!existsSync(ini)) {
    writeFileSync(ini, `[/Script/EngineSettings.GameMapsSettings]
EditorStartupMap=/Engine/Maps/Entry
GameDefaultMap=/Engine/Maps/Entry

[/Script/Engine.RendererSettings]
r.DynamicGlobalIlluminationMethod=1
r.ReflectionMethod=1
r.Shadow.Virtual.Enable=1
r.GenerateMeshDistanceFields=True
r.RayTracing=True
r.RayTracing.RayTracingProxies.ProjectEnabled=True
r.Lumen.HardwareRayTracing=True
r.Lumen.HardwareRayTracing.LightingMode=2
r.PathTracing=True
r.AntiAliasingMethod=4
r.DefaultFeature.AutoExposure.ExtendDefaultLuminanceRange=True
r.AllowStaticLighting=False
r.SkinCache.CompileShaders=True
r.Substrate=False
r.Nanite.ProjectEnabled=True
r.SupportSkyAtmosphere=True

[/Script/MacTargetPlatform.MacTargetSettings]
-TargetedRHIs=SF_METAL_SM5
+TargetedRHIs=SF_METAL_SM6
`);
  }
  return uproject;
}

/**
 * Runs the editor headless on `project` with `py <script>` and `env` (the job path), logging to `log`. The editor
 * occasionally hangs in its macOS shutdown handler after "Exiting.", so once the script reports "THREERT: all done" or
 * "THREERT: FAILED" it gets 30 s to exit and is then killed. Prints the elapsed time and the script's log lines.
 */
export async function runEditor(o: { project: string; script: string; env: Record<string, string>; log: string; lines?: number }) {
  const start = Date.now();
  const fd = openSync(o.log, "w");
  const p = Bun.spawn([EDITOR, o.project, `-ExecCmds=py ${o.script}`, "-unattended", "-nosplash", "-nopause", "-RenderOffscreen", "-NoSound", "-stdout", "-FullStdOutLogOutput", "-log"], {
    env: { ...process.env, ...o.env },
    stdout: fd,
    stderr: fd,
  });
  const done = /THREERT: (all done|FAILED)/;
  while (p.exitCode === null) {
    if (done.test(readFileSync(o.log, "utf8"))) {
      await Promise.race([p.exited, Bun.sleep(30_000)]);
      if (p.exitCode === null) p.kill(9);
      break;
    }
    await Bun.sleep(2000);
  }
  await p.exited;
  console.log(`UE exited after ${Math.round((Date.now() - start) / 1000)}s`);
  const out = readFileSync(o.log, "utf8").split("\n").filter((l) => /THREERT:|LogPython: Error/.test(l)).map((l) => l.slice(0, 400));
  console.log(out.slice(0, o.lines ?? 60).join("\n"));
}

/** Lists files like `ls -la` (when they exist). */
export async function list(...paths: string[]) {
  const there = paths.filter((p) => existsSync(p));
  if (there.length) await $`ls -la ${there}`.nothrow();
}

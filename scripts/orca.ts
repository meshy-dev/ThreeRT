// The ORCA scenes (NVIDIA Open Research Content Archive, CC-BY 4.0): Amazon Lumberyard Bistro (exterior, rendered at
// night) and Beeple's Zero Day (Measure One), FBX + DDS, converted once into glTF in the asset store:
//
//   bun scripts/orca.ts [--store DIR] [--max 2048] [--force] [bistro] [zeroday]     (default: both)
//
// -> <store>/orca/bistro/BistroExterior.gltf, <store>/orca/zeroday/MEASURE_ONE.gltf (+ .bin, textures/, the report,
//    the archive's LICENSE.txt / README.txt), referenced by scenes as assets/orca/...
//
// Needs Blender (headless; BLENDER=<binary>, /Applications/Blender.app or `blender` on PATH; `brew install --cask
// blender`) and python3 (a venv with Pillow + numpy is made in <store>/.cache/orca-venv); without Blender it says so and
// exits 0. The archives (0.9 + 1.1 GB, no login) are kept in <store>/.cache/orca; each run unpacks into a scratch
// directory, converts (tools/orca/fbx2gltf.py: Blender's FBX import, textures by tools/orca/textures.py, glTF export;
// ~25 s for Bistro, ~55 s for Zero Day on an M5) into <out>.tmp and renames it into place. A stamp (the pipeline version
// and options) makes a rerun skip a scene that is up to date.
import { $ } from "bun";
import { copyFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

/** Bump when the conversion changes (the stamp makes stale outputs reconvert). */
const VERSION = 3;

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const opt = (k: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : undefined);
const store = resolve(opt("--store") ?? join(root, "assets"));
const max = Number(opt("--max") ?? 2048);
const force = args.includes("--force");
const names = args.filter((a, i) => !a.startsWith("--") && !["--store", "--max"].includes(args[i - 1]));

interface OrcaScene {
  zip: string;
  url: string;
  /** FBX inside the archive -> glTF base name. */
  fbx: { path: string; name: string }[];
  /** Files of the archive copied next to the glTF (licence, readme). */
  keep: string[];
}

const SCENES: Record<string, OrcaScene> = {
  bistro: {
    zip: "Bistro_v5_2.zip",
    url: "https://developer.nvidia.com/bistro",
    fbx: [{ path: "Bistro_v5_2/BistroExterior.fbx", name: "BistroExterior" }],
    keep: ["Bistro_v5_2/LICENSE.txt", "Bistro_v5_2/README.txt"],
  },
  zeroday: {
    zip: "ZeroDay_v1.zip",
    url: "https://developer.nvidia.com/beeple",
    fbx: [{ path: "MEASURE_ONE/MEASURE_ONE.fbx", name: "MEASURE_ONE" }],
    keep: ["LICENSE.txt", "README.txt"],
  },
};

function blender(): string | null {
  const env = process.env.BLENDER;
  if (env && existsSync(env)) return env;
  const app = "/Applications/Blender.app/Contents/MacOS/Blender";
  if (existsSync(app)) return app;
  return Bun.which("blender");
}

const bl = blender();
if (!bl) {
  console.log("skipping ORCA (Bistro, Zero Day): needs Blender (brew install --cask blender, or BLENDER=<binary>)");
  process.exit(0);
}

const cache = join(store, ".cache");
const venv = join(cache, "orca-venv");
const py = join(venv, "bin", "python");
if (!existsSync(py)) {
  console.log(`making ${venv} (Pillow, numpy)`);
  mkdirSync(cache, { recursive: true });
  await $`python3 -m venv ${venv}`;
  await $`${py} -m pip install --quiet --disable-pip-version-check pillow==11.3.0 numpy`;
}

for (const id of names.length ? names : Object.keys(SCENES)) {
  const s = SCENES[id];
  if (!s) throw new Error(`unknown ORCA scene ${id} (${Object.keys(SCENES).join(", ")})`);
  const out = join(store, "orca", id);
  const stamp = join(out, ".orca-stamp");
  const want = JSON.stringify({ version: VERSION, max, fbx: s.fbx.map((f) => f.name) });
  if (!force && existsSync(stamp) && readFileSync(stamp, "utf8") === want) {
    console.log(`${out} is up to date`);
    continue;
  }
  const zip = join(cache, "orca", s.zip);
  if (!existsSync(zip) || statSync(zip).size === 0) {
    console.log(`fetching ${s.zip} (${s.url})`);
    mkdirSync(dirname(zip), { recursive: true });
    await $`curl -fL -C - --retry 5 -o ${zip}.part ${s.url}`;
    renameSync(`${zip}.part`, zip);
  }
  // The archive is untrusted data: unpacked into its own empty directory, read by path only.
  const src = join(cache, "orca", `${id}-src`);
  rmSync(src, { recursive: true, force: true });
  mkdirSync(src, { recursive: true });
  console.log(`unpacking ${s.zip}`);
  await $`unzip -q ${zip} -d ${src}`;
  const tmp = `${out}.tmp`;
  rmSync(tmp, { recursive: true, force: true });
  for (const f of s.fbx) {
    console.log(`converting ${f.path} (Blender ${bl})`);
    const log = await $`${bl} -b --factory-startup --python-exit-code 1 --python ${join(root, "tools/orca/fbx2gltf.py")} -- --fbx ${join(src, f.path)} --out ${tmp} --name ${f.name} --python ${py} --max ${max}`.quiet().nothrow();
    const lines = log.stdout.toString().split("\n").filter((l) => l.startsWith("orca:"));
    for (const l of lines) console.log(`  ${l}`);
    if (log.exitCode !== 0) {
      console.error(log.stdout.toString().split("\n").slice(-30).join("\n"), log.stderr.toString());
      throw new Error(`${f.path}: Blender exited ${log.exitCode}`);
    }
  }
  for (const k of s.keep) copyFileSync(join(src, k), join(tmp, k.split("/").pop()!));
  writeFileSync(join(tmp, ".orca-stamp"), want);
  rmSync(out, { recursive: true, force: true });
  renameSync(tmp, out);
  rmSync(src, { recursive: true, force: true });
  console.log(`${out}: ${(await $`du -sh ${out}`.text()).split("\t")[0]}`);
}

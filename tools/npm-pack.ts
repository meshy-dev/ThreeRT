// The npm packages: @threert/scene, @threert/gameplay and @threert/voxel as they are published, in out/npm/<name>/.
//   node tools/npm-pack.ts                      build and check all three
//   node tools/npm-pack.ts --publish [--tag t]  then publish the ones whose version npm does not have yet (the npm
//                                               workflow does; from a terminal npm asks for the account's second factor)
//   node tools/npm-pack.ts --version x.y.z      another version than the root package.json's (a trial; not written back)
//
// In this repository the packages are their sources: `exports` names src/index.ts, and the tools, the tests and the
// clients' SDK bundle from there, with no build step. npm gets JavaScript and declarations instead. For each package:
//   out/npm/<name>/dist/     tsc's output of src/ (one .js and one .d.ts a source file; .ts specifiers become .js)
//   out/npm/<name>/package.json   the package's own, without `private`, with the version (the root package.json's:
//                            the three are released together), `exports` on dist/, and @threert/scene as a peer of
//                            the other two: the scene classes are one copy in a program (instanceof, the engine)
//   README.md, LICENSE       the package's README and the repository's licence
// @threert/scene reads the render presets from presets/*.json, outside the package; here they become modules of the
// package (src/presets/<name>.ts), so the published code and its declarations are whole.
//
// The check: the three tarballs installed into an empty project, a scene and a game built from them in Node, and a
// file that uses them typechecked without skipLibCheck (what a user's tsc reads of the declarations), with a bundler's
// module resolution and with Node's.
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");
const out = join(root, "out/npm");
const args = process.argv.slice(2);
const opt = (k: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : undefined);
const version: string = opt("--version") ?? JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version ?? "")) throw new Error(`version ${version}: the root package.json's "version" (or --version) is x.y.z`);
const NAMES = ["scene", "gameplay", "voxel"]; // scene first: the others are built against its declarations
const REPO = "https://github.com/meshy-dev/ThreeRT";
const run = (cmd: string, argv: string[], cwd: string, stdio: "inherit" | "pipe" = "inherit") => execFileSync(cmd, argv, { cwd, stdio: stdio === "pipe" ? ["ignore", "pipe", "pipe"] : "inherit", encoding: "utf8" });
/** A step whose own output says what went wrong: no stack on top of it. */
const step = (what: string, cmd: string, argv: string[], cwd: string) => {
  try { run(cmd, argv, cwd); } catch { console.error(`npm-pack: ${what} failed`); process.exit(1); }
};
const tsc = join(root, "node_modules/typescript/bin/tsc");

function build(name: string): string {
  const src = join(root, "packages", name);
  const dir = join(out, name);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const stage = join(dir, "src");
  cpSync(join(src, "src"), stage, { recursive: true });

  if (name === "scene") {
    // The presets: JSON outside the package, as modules inside it.
    const file = join(stage, "presets.ts");
    let text = readFileSync(file, "utf8");
    const imports = [...text.matchAll(/from "\.\.\/\.\.\/\.\.\/presets\/([a-z-]+)\.json" with \{ type: "json" \};/g)];
    const presets = readdirSync(join(root, "presets")).filter((f) => f.endsWith(".json")).map((f) => f.slice(0, -5));
    if (imports.length === 0 || imports.length !== presets.length) throw new Error(`packages/scene/src/presets.ts imports ${imports.length} presets, presets/ has ${presets.length}: tools/npm-pack.ts follows the imports' form`);
    mkdirSync(join(stage, "presets"));
    for (const m of imports) {
      writeFileSync(join(stage, "presets", `${m[1]}.ts`), `// presets/${m[1]}.json\nexport default ${readFileSync(join(root, "presets", `${m[1]}.json`), "utf8").trim()};\n`);
      text = text.replace(m[0], `from "./presets/${m[1]}.ts";`);
    }
    writeFileSync(file, text);
  }

  writeFileSync(join(dir, "tsconfig.json"), JSON.stringify({
    compilerOptions: {
      target: "es2023", module: "esnext", moduleResolution: "bundler", lib: ["es2023"], types: ["node"], // console, TextEncoder, structuredClone: every host has them
      strict: true, skipLibCheck: true, verbatimModuleSyntax: true, erasableSyntaxOnly: true, isolatedModules: true,
      allowImportingTsExtensions: true, rewriteRelativeImportExtensions: true,
      declaration: true, rootDir: "src", outDir: "dist",
      // The other packages: against the scene package as published (built before them).
      paths: name === "scene" ? undefined : { "@threert/scene": ["../scene/dist/index.d.ts"] },
    },
    include: ["src"],
  }, null, 2));
  step(`${name}: tsc`, process.execPath, [tsc, "-p", dir], root);
  // tsc turns the sources' ".ts" specifiers into ".js" in the JavaScript and leaves them in the declarations.
  for (const f of readdirSync(join(dir, "dist"), { recursive: true }) as string[]) {
    if (!f.endsWith(".d.ts")) continue;
    const file = join(dir, "dist", f);
    writeFileSync(file, readFileSync(file, "utf8").replace(/(\bfrom\s+|\bimport\(\s*|^import\s+)"(\.{1,2}\/[^"]+)\.ts"/gm, '$1"$2.js"'));
  }
  rmSync(stage, { recursive: true });
  rmSync(join(dir, "tsconfig.json"));

  const own = JSON.parse(readFileSync(join(src, "package.json"), "utf8"));
  const peers = Object.keys(own.dependencies ?? {});
  for (const p of peers) if (!NAMES.includes(p.replace("@threert/", ""))) throw new Error(`${own.name} depends on ${p}: only the packages released with it are handled here`);
  const pkg = {
    name: own.name,
    version,
    description: own.description,
    keywords: own.keywords,
    license: own.license,
    homepage: "https://threert.dev",
    repository: { type: "git", url: `git+${REPO}.git`, directory: `packages/${name}` },
    bugs: `${REPO}/issues`,
    type: "module",
    exports: { ".": { types: "./dist/index.d.ts", default: "./dist/index.js" } },
    main: "./dist/index.js",
    types: "./dist/index.d.ts",
    files: ["dist"],
    peerDependencies: peers.length ? Object.fromEntries(peers.map((p) => [p, `^${version}`])) : undefined,
    publishConfig: { access: "public" },
  };
  for (const k of ["description", "keywords", "license"] as const) if (!pkg[k]) throw new Error(`packages/${name}/package.json has no ${k}`);
  writeFileSync(join(dir, "package.json"), JSON.stringify(pkg, null, 2) + "\n");
  for (const [from, to] of [[join(src, "README.md"), "README.md"], [join(root, "LICENSE"), "LICENSE"]]) {
    if (!existsSync(from)) throw new Error(`${from} is missing`);
    cpSync(from, join(dir, to));
  }
  const files = readdirSync(join(dir, "dist"), { recursive: true }).length;
  console.log(`${own.name}@${version}: ${files} files in dist/`);
  return dir;
}

/** The tarballs in an empty project: imported in Node, and typechecked the way a user's project would. */
function check(dirs: string[]): void {
  const project = join(out, "check");
  rmSync(project, { recursive: true, force: true });
  mkdirSync(project, { recursive: true });
  const tarballs = dirs.map((d) => join(project, run("npm", ["pack", d, "--pack-destination", project, "--silent"], project, "pipe").trim().split("\n").pop()!));
  writeFileSync(join(project, "package.json"), JSON.stringify({ name: "check", private: true, type: "module" }));
  run("npm", ["install", "--silent", "--no-audit", "--no-fund", "--ignore-scripts", ...tarballs], project);
  writeFileSync(join(project, "use.ts"), `import { Scene, PerspectiveCamera, Mesh, BoxGeometry, MeshStandardMaterial, DirectionalLight, SkyAtmosphere, resolvePreset } from "@threert/scene";
import { defineGame, describeGame, host, hud, type GameDefinition, type Hud, type Input } from "@threert/gameplay";
import { VoxelGrid } from "@threert/voxel";

const game = defineGame({
  state: { lit: false },
  setup() {
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 80000).setSunAngles(32, 35);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const box = new Mesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ color: 0xc81e1e, roughness: 0.4 }));
    box.id = "box";
    scene.add(box);
    const camera = new PerspectiveCamera(36, 16 / 9, 0.1, 500);
    camera.position.set(0, 2, 6);
    scene.add(camera);
    return { scene, camera };
  },
  handlers: { click(tx) { tx.state.lit = !tx.state.lit; } },
  pointer: "look",
  frame(f) {
    const input: Input = f.input;
    const view: Hud = { crosshair: { style: "dot" }, items: [{ type: "text", anchor: "top-left", text: input.down("W") ? "walking" : "still" }] };
    hud.set(view);
  },
});
// The host channel: nobody listens in Node, so a post is dropped; a listener is typed by its message.
const off = host.on("net.messages", (m) => void m.list.length);
off();
const about = describeGame(game as GameDefinition<unknown>);
const built = game.setup({} as Parameters<typeof game.setup>[0]);
if (!about.handlers.includes("click") || about.pointer !== "look" || host.post({ t: "net.close", id: "none" }) !== false || !(built.scene instanceof Scene) || typeof resolvePreset("preview") !== "object" || new VoxelGrid(0.1).size !== 0.1) throw new Error("the packages did not load as expected");
`);
  // What a user's compiler makes of the declarations: strict, nothing skipped, no Node or DOM types given; resolved
  // as a bundler does, and as Node does.
  for (const [module, moduleResolution] of [["esnext", "bundler"], ["nodenext", "nodenext"]]) {
    writeFileSync(join(project, "tsconfig.json"), JSON.stringify({ compilerOptions: { target: "es2023", module, moduleResolution, lib: ["es2023"], types: [], strict: true, noEmit: true, skipLibCheck: false, erasableSyntaxOnly: true }, include: ["use.ts"] }));
    step(`the check's typecheck (${moduleResolution})`, process.execPath, [tsc, "-p", project], project);
  }
  step("the check's run", process.execPath, ["use.ts"], project);
  console.log("check: the tarballs install, typecheck (strict, no skipLibCheck, no Node or DOM types) and build a scene and a game (with input, a HUD and the host channel) in Node");
}

const dirs = NAMES.map(build);
check(dirs);

if (args.includes("--publish")) {
  const tag = opt("--tag") ?? "latest";
  for (const dir of dirs) {
    const { name } = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    let published = "";
    try { published = run("npm", ["view", `${name}@${version}`, "version"], dir, "pipe").trim(); } catch { /* E404: not there yet */ }
    if (published === version) { console.log(`${name}@${version}: already on npm`); continue; }
    step(`${name}: npm publish`, "npm", ["publish", "--access", "public", "--tag", tag], dir);
    console.log(`${name}@${version}: published (${tag})`);
  }
} else console.log(`out/npm/{${NAMES.join(",")}}: built and checked; --publish sends them`);

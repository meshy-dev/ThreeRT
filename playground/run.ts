// Launches the ThreeRT Playground from the source tree (builds it first if needed). Runs the binary inside the app
// bundle so macOS shows the app's name and icon; logs stay in this terminal.
//   bun playground/run.ts [--scene <id>] [--tour | --uitest]
import { $ } from "bun";
import { existsSync } from "node:fs";
import { join } from "node:path";

const dir = import.meta.dir;
const app = join(dir, "dist/ThreeRT Playground.app/Contents/MacOS/threert-playground");
if (!existsSync(app) || !existsSync(join(dir, "ui/dist/main.js"))) await $`bun ${join(dir, "build.ts")}`;
const p = Bun.spawn([app, ...process.argv.slice(2)], { cwd: dir, stdio: ["inherit", "inherit", "inherit"] });
process.exit(await p.exited);

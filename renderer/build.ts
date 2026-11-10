// Release build. Enables Intel OIDN when OIDN_DIR (or ~/.local/oidn/oidn-*) is present and bakes its rpath in.
//   bun renderer/build.ts [cargo args...]
import { $ } from "bun";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

const dir = import.meta.dir;
const home = process.env.HOME!;
const env = { ...process.env, PATH: `${home}/.cargo/bin:${process.env.PATH}` };
const local = join(home, ".local/oidn");
const oidn = process.env.OIDN_DIR ?? (existsSync(local) ? readdirSync(local).filter((d) => d.startsWith("oidn-")).sort().map((d) => join(local, d))[0] : undefined);
const extra = process.argv.slice(2);
if (oidn && existsSync(join(oidn, "lib"))) {
  await $`cargo build --release --features oidn ${extra}`.cwd(dir).env({ ...env, OIDN_DIR: oidn });
  if (process.platform === "darwin") await $`install_name_tool -add_rpath ${join(oidn, "lib")} target/release/threert-render`.cwd(dir).quiet().nothrow();
} else {
  await $`cargo build --release ${extra}`.cwd(dir).env(env);
}

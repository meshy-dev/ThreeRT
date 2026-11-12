// Builds the site and deploys it to threert.dev (Cloudflare Workers: static assets, custom domains, and the clients'
// disk images from R2).
//   bun site/tools/deploy.ts            needs `wrangler login` as a member of the account in site/wrangler.jsonc
// The build is strict: it stops when site/releases/ or site/play/ is missing or incomplete, so a deploy never puts up
// a Download button with nothing behind it (site/README.md: where the two directories come from).
// The disk image latest.json names goes to the R2 bucket first (site/wrangler.jsonc: `threert-releases`, created once
// with `bunx wrangler@4 r2 bucket create threert-releases`), so the page never links an image that is not there yet.
import { $ } from "bun";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const site = resolve(import.meta.dir, "..");
const config = resolve(site, "wrangler.jsonc");
await $`bun ${resolve(site, "tools/build.ts")} --require-release`;
const latest = JSON.parse(readFileSync(join(site, "releases/latest.json"), "utf8"));
if (!latest.macos?.url) {
  const file = join(site, "releases", latest.macos.file);
  if (!existsSync(file)) throw new Error(`${file} is missing`);
  await $`bunx wrangler@4 r2 object put ${`threert-releases/${latest.macos.file}`} --file ${file} --content-type application/x-apple-diskimage --remote --config ${config}`;
}
await $`bunx wrangler@4 deploy --config ${config}`;

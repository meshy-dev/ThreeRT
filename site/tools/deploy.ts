// Builds the site and deploys it to threert.dev (Cloudflare Workers: static assets, custom domains, and the clients'
// files from R2).
//   bun site/tools/deploy.ts            needs `wrangler login` as a member of the account in site/wrangler.jsonc
// The build is strict: it stops when site/releases/ or site/play/ is missing or incomplete, so a deploy never puts up
// a Download button with nothing behind it (site/README.md: where the two directories come from).
// Every file latest.json names (including the Windows installer, portable ZIP and checksums) goes to the R2 bucket
// first (site/wrangler.jsonc: `threert-releases`, created once with `bunx wrangler r2 bucket create
// threert-releases`), so the page never links a file that is not there yet. A file the bucket already has is not
// sent again: the live site is asked for it (HEAD /releases/<file>: the Worker answers with the object's size and
// ETag), and the same size, and the same MD5 where the ETag is one, means it is there.
import { $ } from "bun";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { clientFile } from "../worker.ts";
import { releaseArtifacts } from "./releases.ts";

const site = resolve(import.meta.dir, "..");
const config = resolve(site, "wrangler.jsonc");
/** Where the Worker answers (site/wrangler.jsonc `routes`). */
const ORIGIN = "https://threert.dev";

/** Whether the bucket has this very file, as far as the live site can tell. Any doubt is "no": it is sent again. */
async function hosted(name: string, file: string): Promise<boolean> {
  try {
    const r = await fetch(`${ORIGIN}/releases/${name}`, { method: "HEAD", signal: AbortSignal.timeout(15000) });
    if (!r.ok || Number(r.headers.get("content-length")) !== statSync(file).size) return false;
    // (An object put in one piece has its MD5 as its ETag; one put in parts has "<hash>-<parts>", which says nothing.)
    const md5 = /^"?([0-9a-f]{32})"?$/.exec(r.headers.get("etag") ?? "")?.[1];
    return !md5 || md5 === createHash("md5").update(readFileSync(file)).digest("hex");
  } catch {
    return false;
  }
}

await $`bun ${resolve(site, "tools/build.ts")} --require-release`;
const latest = JSON.parse(readFileSync(join(site, "releases/latest.json"), "utf8"));
for (const { entry } of releaseArtifacts(latest)) {
  // (An entry with a `url` is hosted elsewhere.)
  if (!entry?.file || entry.url) continue;
  const file = join(site, "releases", entry.file);
  if (!existsSync(file)) throw new Error(`${file} is missing`);
  // The type the Worker serves it with; a name it would not serve stops here.
  const type = clientFile(`/releases/${entry.file}`)?.type;
  if (!type) throw new Error(`${entry.file}: not a download the Worker serves`);
  if (await hosted(entry.file, file)) {
    console.log(`${entry.file}: already in R2, not sent again`);
    continue;
  }
  await $`bunx wrangler r2 object put ${`threert-releases/${entry.file}`} --file ${file} --content-type ${type} --remote --config ${config}`;
}
await $`bunx wrangler deploy --config ${config}`;

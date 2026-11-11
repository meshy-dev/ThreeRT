// Builds the landing page and deploys it to threert.dev (Cloudflare Workers, static assets + custom domains).
//   bun site/tools/deploy.ts            needs `wrangler login` as a member of the account in site/wrangler.jsonc
import { $ } from "bun";
import { resolve } from "node:path";

const site = resolve(import.meta.dir, "..");
await $`bun ${resolve(site, "tools/build.ts")}`;
await $`bunx wrangler@4 deploy --config ${resolve(site, "wrangler.jsonc")}`;

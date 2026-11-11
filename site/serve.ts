// Serves the landing page locally: bun site/serve.ts [port]  ->  http://localhost:4173/
// three.js comes from node_modules (npm install first); site/media is committed (rebuild: bun site/tools/media.ts).
import { existsSync, statSync } from "node:fs";
import { join, normalize, resolve } from "node:path";
import { VENDOR } from "./tools/vendor.ts";

const root = resolve(import.meta.dir);
const port = Number(process.argv[2] ?? process.env.PORT ?? 4173);

for (const f of Object.values(VENDOR)) if (!existsSync(f)) console.warn(`missing ${f}: run npm install at the repo root`);

Bun.serve({
  port,
  hostname: "127.0.0.1",
  fetch(req) {
    let path = decodeURIComponent(new URL(req.url).pathname);
    if (path.endsWith("/")) path += "index.html";
    const rel = path.replace(/^\/+/, "");
    const file = VENDOR[rel] ?? normalize(join(root, rel));
    if (!VENDOR[rel] && (!file.startsWith(root + "/") || rel.startsWith("tools/"))) return new Response("not found", { status: 404 });
    if (!existsSync(file) || !statSync(file).isFile()) return new Response("not found", { status: 404 });
    return new Response(Bun.file(file), { headers: { "cache-control": "no-cache" } });
  },
});
console.log(`ThreeRT landing page: http://localhost:${port}/`);

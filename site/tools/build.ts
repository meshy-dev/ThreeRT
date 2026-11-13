// A static copy of the site for any file host: bun site/tools/build.ts [outDir] [--require-release]  (default out/site)
// The pages plus the npm files they import (three.js, the QR encoder); no bundling, nothing rewritten.
//
// Two directories are generated elsewhere and copied when they are there (site/README.md):
//   site/releases/   latest.json and the client build it names (the Download page), appcast.xml (the apps' update feed)
//   site/play/       index.json and the example packages (Showcase's "Open on your device")
// Without them the pages say that nothing is published on this host; the build warns. --require-release makes that
// (and a file over the host's size limit) an error: what the deploy uses. The disk image itself stays out of the build
// (--with-image copies it): threert.dev serves it from R2.
import { cpSync, copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { VENDOR } from "./vendor.ts";

const site = resolve(import.meta.dir, "..");
const args = process.argv.slice(2);
const strict = args.includes("--require-release");
const out = resolve(args.find((a) => !a.startsWith("--")) ?? join(site, "../out/site"));
/** Cloudflare Workers static assets: 25 MiB a file. */
const MAX_FILE = 25 * 1024 * 1024;

const problems: string[] = [];
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
copyFileSync(join(site, "index.html"), join(out, "index.html"));
for (const d of ["showcase", "download", "docs", "privacy", "css", "js", "brand", "media"]) cpSync(join(site, d), join(out, d), { recursive: true });
for (const [rel, file] of Object.entries(VENDOR)) {
  mkdirSync(dirname(join(out, rel)), { recursive: true });
  copyFileSync(file, join(out, rel));
}

// The client release.
const latest = join(site, "releases/latest.json");
if (existsSync(latest)) {
  const r = JSON.parse(readFileSync(latest, "utf8"));
  const file = r.macos?.file && join(site, "releases", r.macos.file);
  // The disk image is never a static asset here: with the command line in it, it is over the 25 MiB a file the host
  // takes. On threert.dev the Worker serves /releases/<file> from R2 (worker.ts; deploy.ts uploads it), or `macos.url`
  // names another place. --with-image copies it into the build, for a plain file host without that limit.
  const withImage = args.includes("--with-image");
  if (r.macos?.url) {
    if (!/^https:\/\//.test(r.macos.url)) problems.push(`site/releases/latest.json: macos.url is not an https URL (${r.macos.url})`);
  } else if (!file || !existsSync(file)) problems.push(`site/releases/latest.json names ${r.macos?.file ?? "no macOS file"}, which is not in site/releases/`);
  else if (statSync(file).size !== r.macos.size) problems.push(`site/releases/${r.macos.file} is ${statSync(file).size} bytes, latest.json says ${r.macos.size}`);
  // A public build opens like any app: never one that asks the user to get past Gatekeeper.
  if (r.macos && (r.macos.signed !== "developer-id" || r.macos.notarized !== true))
    problems.push(`site/releases/latest.json: the macOS build is ${r.macos.signed ?? "unsigned"}${r.macos.notarized ? "" : ", not notarized"} (release.ts --sign <Developer ID> --notarize <profile>)`);
  // The update feed installed apps read (the client's tools/sparkle.ts): it must offer the image latest.json names, at
  // its size, or an app would download something else than the Download page offers (or fail the length check).
  const feed = join(site, "releases/appcast.xml");
  if (existsSync(feed) && r.macos) {
    const xml = readFileSync(feed, "utf8");
    const enc = /<enclosure\s+url="([^"]+)"\s+length="(\d+)"[^>]*sparkle:edSignature="[^"]+"/.exec(xml);
    const ver = /<sparkle:version>([^<]+)<\/sparkle:version>/.exec(xml)?.[1];
    if (!enc) problems.push("site/releases/appcast.xml: no enclosure with a URL, a length and an EdDSA signature");
    else {
      if (!/^https:\/\//.test(enc[1])) problems.push(`site/releases/appcast.xml: the update is not on https (${enc[1]})`);
      if (!enc[1].endsWith(`/${r.macos.file}`) && enc[1] !== r.macos.url) problems.push(`site/releases/appcast.xml offers ${enc[1]}, latest.json ${r.macos.url ?? r.macos.file}`);
      if (Number(enc[2]) !== r.macos.size) problems.push(`site/releases/appcast.xml says ${enc[2]} bytes, latest.json ${r.macos.size}`);
      if (ver !== r.version) problems.push(`site/releases/appcast.xml offers ${ver}, latest.json is ${r.version}`);
    }
  }
  cpSync(join(site, "releases"), join(out, "releases"), { recursive: true, filter: (src) => withImage || !src.endsWith(".dmg") });
  console.log(`release ${r.version}: ${r.macos?.url ?? r.macos?.file ?? "no macOS build"}${r.macos?.url || withImage ? "" : " (served from R2: deploy.ts uploads it)"}; updates ${existsSync(feed) ? "releases/appcast.xml" : "none (no appcast.xml)"}; iOS ${r.ios?.url ?? "not published"}`);
} else {
  problems.push("no site/releases/latest.json: the Download page will say that no build is published on this host");
}

// The example packages.
const index = join(site, "play/index.json");
if (existsSync(index)) {
  const p = JSON.parse(readFileSync(index, "utf8"));
  for (const g of p.games ?? []) {
    if (!existsSync(join(site, "play", g.id, "threert.json"))) problems.push(`site/play/index.json lists ${g.id}, which has no site/play/${g.id}/threert.json`);
  }
  cpSync(join(site, "play"), join(out, "play"), { recursive: true });
  console.log(`packages: ${(p.games ?? []).map((g: { id: string }) => g.id).join(", ")}`);
} else {
  problems.push("no site/play/index.json: Showcase will have no packages to open");
}

const walk = (d: string): string[] => readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(d, e.name)) : [join(d, e.name)]));
for (const f of walk(out)) {
  const size = statSync(f).size;
  if (size > MAX_FILE && !f.endsWith(".dmg")) problems.push(`${relative(out, f)} is ${(size / 1048576).toFixed(1)} MiB: over the 25 MiB a file that Workers static assets take`);
}

for (const p of problems) console.warn(`${strict ? "error" : "warning"}: ${p}`);
if (strict && problems.length) process.exit(1);
console.log(`site -> ${out}`);

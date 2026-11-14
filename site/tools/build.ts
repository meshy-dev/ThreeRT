// A static copy of the site for any file host: bun site/tools/build.ts [outDir] [--require-release]  (default out/site)
// The pages plus the npm files they import (three.js, the QR encoder); no bundling, nothing rewritten.
//
// Two directories are generated elsewhere and copied when they are there (site/README.md):
//   site/releases/   latest.json and the clients' files it names (the Download page), appcast.xml (the Macs' update feed)
//   site/play/       index.json and the example packages (what clients before 0.4.0 list in their Showcase; the
//                    apps themselves are published on threert.app now)
// Without releases/ the Download page says that no build is published on this host; the build warns. --require-release makes that
// (and a file latest.json names that is not there, or a file over the host's size limit) an error: what the deploy
// uses. A client latest.json does not name is not an error: the page says it is not published here. The clients'
// files themselves (.dmg, .tar.gz, .apk) stay out of the build (--with-image copies them): threert.dev serves them
// from R2.
import { cpSync, copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { createHash } from "node:crypto";
import { VENDOR } from "./vendor.ts";
import { releaseArtifacts } from "./releases.ts";

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

// The clients' release.
/** A client's file: never a static asset here (worker.ts serves it from R2), whatever its size. */
const CLIENT_FILE = /(?:\.(?:dmg|tar\.gz|apk|exe|zip)|-SHA256SUMS\.txt)$/;
const latest = join(site, "releases/latest.json");
if (existsSync(latest)) {
  const r = JSON.parse(readFileSync(latest, "utf8"));
  // The disk image is over the 25 MiB a file the host takes (the `threert` command line is in it), and every client
  // file is a download. On threert.dev the Worker serves /releases/<file> from R2 (worker.ts; deploy.ts uploads what
  // latest.json names), or an entry's `url` names another place. --with-image copies the files into the build, for a
  // plain file host without that limit.
  const withImage = args.includes("--with-image");
  const said: string[] = [];
  for (const { key, name, required, entry: e } of releaseArtifacts(r)) {
    if (!e && !required) {
      said.push(`${name} not published`);
      continue;
    }
    const file = e?.file && join(site, "releases", e.file);
    if (e?.url) {
      if (!/^https:\/\//.test(e.url)) problems.push(`site/releases/latest.json: ${key}.url is not an https URL (${e.url})`);
    } else if (!file || !existsSync(file)) problems.push(`site/releases/latest.json names ${e?.file ?? `no ${name} file`}, which is not in site/releases/`);
    else if (statSync(file).size !== e.size) problems.push(`site/releases/${e.file} is ${statSync(file).size} bytes, latest.json says ${e.size}`);
    else if (!/^[a-f0-9]{64}$/.test(e.sha256 ?? "") || createHash("sha256").update(readFileSync(file)).digest("hex") !== e.sha256)
      problems.push(`site/releases/${e.file}: SHA-256 does not match latest.json`);
    said.push(`${name} ${e?.version ?? r.version} ${e?.url ?? e?.file ?? "no file"}`);
  }
  // A public build opens like any app: never one that asks the user to get past Gatekeeper.
  if (r.macos && (r.macos.signed !== "developer-id" || r.macos.notarized !== true))
    problems.push(`site/releases/latest.json: the macOS build is ${r.macos.signed ?? "unsigned"}${r.macos.notarized ? "" : ", not notarized"} (release.ts --sign <Developer ID> --notarize <profile>)`);
  // An APK signed with the debug key could never be updated by a real release.
  if (r.android && r.android.signed !== true) problems.push("site/releases/latest.json: the Android build is not signed with the release key (android.signed is not true)");
  // The update feed installed Macs read (the client's tools/sparkle.ts): it must offer the image latest.json names, at
  // its size, or an app would download something else than the Download page offers (or fail the length check).
  const feed = join(site, "releases/appcast.xml");
  if (existsSync(feed) && r.macos) {
    const xml = readFileSync(feed, "utf8");
    const enc = /<enclosure\s+url="([^"]+)"\s+length="(\d+)"[^>]*sparkle:edSignature="[^"]+"/.exec(xml);
    const ver = /<sparkle:version>([^<]+)<\/sparkle:version>/.exec(xml)?.[1];
    const macVersion = r.macos.version ?? r.version;
    if (!enc) problems.push("site/releases/appcast.xml: no enclosure with a URL, a length and an EdDSA signature");
    else {
      if (!/^https:\/\//.test(enc[1])) problems.push(`site/releases/appcast.xml: the update is not on https (${enc[1]})`);
      if (!enc[1].endsWith(`/${r.macos.file}`) && enc[1] !== r.macos.url) problems.push(`site/releases/appcast.xml offers ${enc[1]}, latest.json ${r.macos.url ?? r.macos.file}`);
      if (Number(enc[2]) !== r.macos.size) problems.push(`site/releases/appcast.xml says ${enc[2]} bytes, latest.json ${r.macos.size}`);
      if (ver !== macVersion) problems.push(`site/releases/appcast.xml offers ${ver}, latest.json's macOS build is ${macVersion}`);
    }
  }
  cpSync(join(site, "releases"), join(out, "releases"), { recursive: true, filter: (src) => withImage || !CLIENT_FILE.test(src) });
  console.log(`release: ${said.join("; ")}${withImage ? "" : " (files served from R2: deploy.ts uploads them)"}; updates ${existsSync(feed) ? "releases/appcast.xml" : "none (no appcast.xml)"}; iOS ${r.ios?.url ?? "not published"}`);
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
  problems.push("no site/play/index.json: clients before 0.4.0 will have no examples to list in their Showcase");
}

// The landing page's own pictures (the baked tiles of threert.app's apps, OpenStrikeRT's: site/tools/platform.ts):
// a tile whose copy of the cover is missing would be a hole in the page wherever the platform does not answer.
for (const [, ref] of readFileSync(join(site, "index.html"), "utf8").matchAll(/(?:src|srcset)="(media\/(?:apps|openstrike)\/[^" ]+)/g)) {
  if (!existsSync(join(site, ref))) problems.push(`site/index.html shows ${ref}, which is not in site/media (bun site/tools/platform.ts)`);
}

const walk = (d: string): string[] => readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(d, e.name)) : [join(d, e.name)]));
for (const f of walk(out)) {
  const size = statSync(f).size;
  if (size > MAX_FILE && !CLIENT_FILE.test(f)) problems.push(`${relative(out, f)} is ${(size / 1048576).toFixed(1)} MiB: over the 25 MiB a file that Workers static assets take`);
}

for (const p of problems) console.warn(`${strict ? "error" : "warning"}: ${p}`);
if (strict && problems.length) process.exit(1);
console.log(`site -> ${out}`);

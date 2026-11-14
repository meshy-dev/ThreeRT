// ThreeRT site: what the pages share besides the components in threert.js. The platform a visitor is on (for the
// download links), the client release this host carries (releases/latest.json, generated: site/README.md; a host
// without it gets a Download page that says so), the repository's star count, and QR codes.

import { mark } from "./mark.js";

/** The site's root URL (this file is <root>/js/site.js), so pages one level down resolve the same paths. */
export const ROOT = new URL("..", import.meta.url);
export const GITHUB = "https://github.com/meshy-dev/ThreeRT";

/** The clients there are, as a visitor knows the device each is for. */
export const CLIENTS = { macos: "macOS", windows: "Windows", ios: "iPhone", linux: "Steam Deck", android: "Android" };

/**
 * "macos" | "windows" | "ios" | "android" | "linux" | "other": the client a visitor's device takes. An iPad asks for the desktop
 * site as a Mac with a touch screen. An Android browser says Linux too, so it is asked first. "linux" is x86_64
 * alone, what the tarball is built for: a Steam Deck's browsers say "X11; Linux x86_64", in Desktop Mode and in Game
 * Mode alike.
 */
export function platform(ua = navigator.userAgent, touchPoints = navigator.maxTouchPoints) {
  if (/iPhone|iPod|iPad/.test(ua) || (/Macintosh/.test(ua) && touchPoints > 1)) return "ios";
  if (/Android/.test(ua)) return "android";
  if (/Windows NT/.test(ua)) return "windows";
  if (/Macintosh|Mac OS X/.test(ua)) return "macos";
  if (/Linux x86_64/.test(ua)) return "linux";
  return "other";
}

/**
 * The download links (`a[data-dl]`, leading to the download page) name the visitor's platform, and the list beside
 * one (`[data-dl-others]`) the other clients.
 */
export function downloadButtons(root = document) {
  const p = platform();
  const text = { macos: "Download for macOS", windows: "Download for Windows", ios: "Get ThreeRT for iPhone", linux: "Download for Steam Deck", android: "Download for Android", other: "Download" }[p];
  for (const a of root.querySelectorAll("a[data-dl]")) {
    const label = a.querySelector("span") ?? a;
    label.textContent = text;
  }
  // (A name stays whole: the line breaks at a dot.)
  const others = Object.entries(CLIENTS).filter(([k]) => k !== p).map(([, name]) => name.replaceAll(" ", "\u00a0")).join(" · ");
  for (const el of root.querySelectorAll("[data-dl-others]")) el.textContent = others;
}

async function json(rel) {
  try {
    const r = await fetch(new URL(rel, ROOT), { cache: "no-cache" });
    return r.ok ? await r.json() : null;
  } catch {
    return null;
  }
}

/**
 * The clients released on this host, or null: {version, date, macos, ios, linux, android, windows}, an entry a client, each
 * with its own `version` (clients are released on different days; an entry without one has the top-level version):
 * macos {file, url, size, sha256, minOS, arch, signed, notarized}, ios {url, channel, minOS},
 * linux {file, url, size, sha256, arch, needs}, android {file, url, size, sha256, arch, minOS, signed},
 * windows {file, size, sha256, arch, minOS, signed, channel, portable, checksums}.
 * A client without an entry is not published here.
 */
export const release = () => json("releases/latest.json");

export const fileUrl = (file) => new URL(`releases/${file}`, ROOT).href;
/** Where a client's file is: its `url` when it is hosted elsewhere, else next to latest.json. */
export const clientUrl = (entry) => (entry.url ? entry.url : fileUrl(entry.file));
export const macUrl = clientUrl;
export const megabytes = (bytes) => `${(bytes / 1e6).toFixed(bytes < 1e7 ? 1 : 0)} MB`;

let qrcode;
/** `text` as a QR code: an SVG string, dark modules only (put it in a `.qr` tile). Loads the encoder on first use. */
export async function qrSvg(text, label = "QR code") {
  qrcode ??= (await import(new URL("vendor/qrcode.mjs", ROOT).href)).default;
  const q = qrcode(0, "M");
  q.addData(text);
  q.make();
  const n = q.getModuleCount(), m = 1;
  let d = "";
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      if (!q.isDark(y, x)) continue;
      let w = 1;
      while (x + w < n && q.isDark(y, x + w)) w++; // one rectangle a run: no hairlines between modules
      d += `M${x + m} ${y + m}h${w}v1h-${w}z`;
      x += w - 1;
    }
  }
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${n + 2 * m} ${n + 2 * m}" shape-rendering="crispEdges" role="img" aria-label="${esc(label)}"><title>${esc(text)}</title><path fill="#060709" d="${d}"/></svg>`;
}

/** Fills every `.qr[data-qr]` under `root` with its code. */
export async function qrTiles(root = document) {
  for (const el of root.querySelectorAll(".qr[data-qr]")) {
    try {
      el.innerHTML = await qrSvg(el.dataset.qr, el.dataset.label ?? "QR code");
    } catch (e) {
      console.error("qr:", e);
      el.remove();
    }
  }
}

/** Runs a part of a page; a part that fails (a missing file, a failed import) is logged and the rest of the page stands. */
export function guard(name, f) {
  try {
    const r = f();
    if (r?.catch) r.catch((e) => console.error(`${name}:`, e));
  } catch (e) {
    console.error(`${name}:`, e);
  }
}

/**
 * The repository's stars, in every `[data-stars]` (the Star buttons: the nav, the footer, the landing page's own).
 * GitHub's public API takes 60 requests an hour from an address without a token, so the count is kept in
 * localStorage for six hours, and an answer that is not a count (a rate limit, no network) is not asked again for an
 * hour. Until a count is known the buttons simply have none; below `min` they keep none either, because a button
 * that says "Star 2" asks less well than one that says "Star".
 */
const STARS = { api: "https://api.github.com/repos/meshy-dev/ThreeRT", key: "threert:stars", ttl: 6 * 3600e3, retry: 3600e3, min: 10 };
export async function stars(root = document) {
  const els = [...root.querySelectorAll("[data-stars]")];
  if (!els.length) return;
  const show = (n) => {
    if (!Number.isFinite(n) || n < STARS.min) return;
    const text = n < 1000 ? String(n) : `${(n / 1000).toFixed(n < 10000 ? 1 : 0).replace(/\.0$/, "")}k`;
    for (const el of els) {
      el.textContent = text;
      el.hidden = false;
      el.closest("a")?.setAttribute("aria-label", `Star ThreeRT on GitHub (${n} stars)`);
    }
  };
  const read = () => { try { return JSON.parse(localStorage.getItem(STARS.key)); } catch { return null; } };
  const write = (v) => { try { localStorage.setItem(STARS.key, JSON.stringify(v)); } catch { /* private mode: ask again next time */ } };
  const kept = read();
  show(kept?.n);
  if (kept && Date.now() < kept.until) return;
  let n = null;
  try {
    const r = await fetch(STARS.api, { signal: AbortSignal.timeout(8000) });
    if (r.ok) n = (await r.json()).stargazers_count;
  } catch { /* offline, blocked: as a rate limit */ }
  if (Number.isFinite(n)) { show(n); write({ n, until: Date.now() + STARS.ttl }); }
  else write({ n: kept?.n ?? null, until: Date.now() + STARS.retry });
}

// Every page imports this file: the lockup's mark comes alive on all of them (js/mark.js), and the Star buttons get
// their count.
try { mark(); } catch (e) { console.error("mark:", e); }
guard("stars", () => stars());

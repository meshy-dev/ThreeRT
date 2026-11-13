// ThreeRT site: what the pages share besides the components in threert.js. The platform a visitor is on (for the
// download buttons), the client release this host carries (releases/latest.json), the example packages it hosts
// (play/index.json), their links, and QR codes. Both JSON files are generated (site/README.md); a host without them
// gets pages that say so.

import { mark } from "./mark.js";

/** The site's root URL (this file is <root>/js/site.js), so pages one level down resolve the same paths. */
export const ROOT = new URL("..", import.meta.url);
export const GITHUB = "https://github.com/meshy-dev/ThreeRT";

/** "macos" | "ios" | "other". An iPad asks for the desktop site as a Mac with a touch screen. */
export function platform() {
  const ua = navigator.userAgent;
  if (/iPhone|iPod|iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) return "ios";
  if (/Macintosh|Mac OS X/.test(ua)) return "macos";
  return "other";
}

/** The download buttons (`a[data-dl]`, linking to the download page) name the visitor's platform. */
export function downloadButtons(root = document) {
  const text = { macos: "Download for macOS", ios: "Get ThreeRT for iPhone", other: "Download" }[platform()];
  for (const a of root.querySelectorAll("a[data-dl]")) {
    const label = a.querySelector("span") ?? a;
    label.textContent = text;
  }
}

async function json(rel) {
  try {
    const r = await fetch(new URL(rel, ROOT), { cache: "no-cache" });
    return r.ok ? await r.json() : null;
  } catch {
    return null;
  }
}

/** The client release on this host: {version, date, macos: {file, url, size, sha256, minOS, arch, signed, notarized}, ios: {url, channel, minOS}}, or null. */
export const release = () => json("releases/latest.json");
/** The example packages on this host: {client, games: [{id, title, caption, kind, thumb}]}, or null. */
export const packages = () => json("play/index.json");

export const fileUrl = (file) => new URL(`releases/${file}`, ROOT).href;
/** Where a release's macOS disk image is: `macos.url` when it is hosted elsewhere (it is over this host's size limit), else next to latest.json. */
export const macUrl = (m) => (m.url ? m.url : fileUrl(m.file));
export const megabytes = (bytes) => `${(bytes / 1e6).toFixed(bytes < 1e7 ? 1 : 0)} MB`;

/**
 * A hosted package's addresses. `url`: its base (threert.json lives under it) and, with a slash, the same game as
 * three.js. `open`: the link ThreeRT clients register (threerts:// = https, threert:// = http). `qr`: what its QR code
 * carries. Over https that is the plain URL: the phone's camera opens the three.js page, the app's scanner opens the
 * package. Over http (a dev machine) it is the threert:// link, which the camera hands to the app.
 */
export function packageLinks(id) {
  const url = new URL(`play/${id}`, ROOT).href;
  const secure = url.startsWith("https:");
  const open = url.replace(/^https:/, "threerts:").replace(/^http:/, "threert:");
  return { url, web: `${url}/`, open, qr: secure ? `${url}/` : open, thumb: (file) => new URL(`play/${file}`, ROOT).href };
}

/** Whether a phone can reach this page's host (a QR code of localhost only opens on this machine). */
export const reachable = () => !/^(localhost|127\.|\[?::1)/.test(location.hostname);

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

// Every page imports this file: the lockup's mark comes alive on all of them (js/mark.js).
try { mark(); } catch (e) { console.error("mark:", e); }

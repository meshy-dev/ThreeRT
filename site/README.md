# ThreeRT site

Static pages, no build step. three.js and the QR encoder come from the repo's own devDependencies; the images in
`media/` are committed.

```bash
npm install                 # once, at the repo root (three, qrcode-generator, sharp)
bun site/serve.ts           # http://localhost:4173/        [port] [--lan]
bun site/tools/build.ts     # a static copy for any file host -> out/site/
```

`--lan` listens on every interface and prints the LAN address: open that one on the Mac and a phone on the same
network can scan the QR codes on the pages (they carry `threert://<lan ip>:<port>/play/<id>`).

**Live at https://threert.dev** (www.threert.dev redirects there). Deploy after a change:

```bash
wrangler login              # once, as a member of the Cloudflare account that holds threert.dev
bun site/tools/deploy.ts    # build out/site (strict, see below), then wrangler deploy (site/wrangler.jsonc)
```

It is a Cloudflare Worker, `threert-site`: static assets from `out/site` on the custom domains threert.dev and
www.threert.dev; `site/worker.ts` redirects www to the apex, serves the clients' disk images from R2 and hands everything else to the assets.
Workers static assets take **25 MiB a file**, and the client's disk image is about 32 MiB (it carries the `threert`
command line). So the image is not a static asset: it lives in the R2 bucket `threert-releases` under its file name,
and `worker.ts` serves it at the URL the Download page links, `/releases/<file>` (ranges, `ETag`, `HEAD`). R2 rather
than a GitHub release because the client is closed source and has no public repository to carry one. Once:

```bash
bunx wrangler@4 r2 bucket create threert-releases
```

`deploy.ts` uploads the image `releases/latest.json` names, then deploys. `build.ts` leaves the image out of
`out/site` (`--with-image` copies it, for a plain file host). Locally, `serve.ts` serves it from `site/releases/`.
To check the Worker itself without an account: `bunx wrangler@4 r2 object put threert-releases/<file> --file
site/releases/<file> --local --config site/wrangler.jsonc`, then `bunx wrangler@4 dev --config site/wrangler.jsonc`.

## Pages

Every page has the same nav, top right: **Showcase / Download / Docs / GitHub** (github.com/taichi-dev/ThreeRT), and
the same footer links. The markup is repeated in each page; change it in all four.

| Page | What is on it |
|---|---|
| `index.html` | The landing page (below). Its two buttons, in the hero and at the end: **Download** (to `download/`, named for the visitor's platform by `js/site.js`) and **GitHub**. |
| `showcase/` | Bistro night and Zero-Day as camera moves (the committed media), then **Open on your device**: a card for each example package in `play/` with an "Open in ThreeRT" link, the same game as three.js, and a QR code for the iPhone app. |
| `download/` | ThreeRT for macOS (the build `releases/latest.json` names) and ThreeRT for iPhone (a TestFlight or App Store link once `ios.url` is set), then how a client loads a game: `threert dev`, scan, traced and live. |
| `docs/` | Getting started: install the client, open an example, write a scene, the dev loop with the `threert` CLI, the package format, links to the reference documents in `docs/` on GitHub. |

### The landing page

1. **Hero:** Bistro night along its own camera move, three.js | ThreeRT under one draggable divider, as a clip
   (60 fps, a 12 s loop; `media/bistro/hero.mp4`, `hero-720.mp4`, both sides in one file so they cannot drift). Its first frame is
   what shows until it plays, and where it does not: reduced motion, a data saver, autoplay refused. A button pauses
   it. "Path tracing at 60 fps."
2. **ThreeRT = Three + RT:** Ray Tracing, Real Time, Runtime.
3. **Hello world looks right:** an auto-advancing reel of three starter scenes, the ThreeRT source next to its output
   (three.js as written | ThreeRT with every setting at its default).
4. **Against three.js at full power:** Bistro night and Zero-Day; the left side picks three.js WebGL, three.js WebGPU
   with SSGI, or the path-traced reference; the right side is ThreeRT live.
5. **A camera move:** each scene's own camera path, both engines in lockstep.
6. **What a ray finds:** four close-ups.
7. **Traced where it can be, three.js everywhere else:** the native client on ray-tracing hardware and the same scene
   as three.js in any browser or WebView, drawn as live three.js device models at real proportions.
8. **Just light.**

## The two generated directories

`site/releases/` and `site/play/` are not committed (.gitignore): the first holds a build of the closed-source
client, the second packages built with it. The client repository's release script writes both, whole. Without them
the pages still work: Download says that no build is published on this host, Showcase that there are no packages.
`serve.ts` and `build.ts` warn; `build.ts --require-release` (what `deploy.ts` runs) stops instead, so a deploy never
puts up a Download button with nothing behind it.

`releases/latest.json`, next to the file it names:

```jsonc
{
  "version": "0.1.0", "date": "2026-11-11",
  "macos": { "file": "ThreeRT-0.1.0-macos-arm64.dmg", "size": 33981234, "sha256": "…", "minOS": "14.0", "arch": "arm64",
             "url": null,              // null: `releases/<file>` on this host (R2 behind the Worker; site/releases/ locally);
                                       // or the https URL of the image when it is hosted elsewhere (`release.ts --macos-url`)
             "signed": "adhoc",        // or "developer-id"
             "notarized": false },     // unless signed with a Developer ID and notarized, the page explains the first launch
  "ios": { "url": null,                // the TestFlight / App Store link; null: "TestFlight beta not open yet"
           "channel": "testflight",    // or "appstore"
           "minOS": "17.0" }
}
```

`play/index.json`, next to a thumbnail and a package directory for each game:

```jsonc
{
  "client": "0.1.0",
  "games": [{ "id": "light-room", "title": "Light Room", "caption": "…", "kind": "game" /* or "scene" */, "thumb": "light-room.jpg" }]
}
```

`play/<id>/` is a ThreeRT package (the client repository's docs/CLIENT.md § 1): `threert.json` (format `threert/1`),
`game.js`, `game.mjs`, the same game as three.js (`index.html`, `web.js`), `presets/`, `assets/`. The pages derive the
links from where they are served (`js/site.js` `packageLinks`):

| | over https | over http (a dev machine) |
|---|---|---|
| Open in ThreeRT | `threerts://<host>/play/<id>` | `threert://<host>:<port>/play/<id>` |
| three.js in the browser | `https://<host>/play/<id>/` | `http://<host>:<port>/play/<id>/` |
| QR code | the plain `https://` URL: the phone's camera opens the three.js page, the app's scanner opens the package | the `threert://` link, which the camera hands to the app |

## Files

| Path | What |
|---|---|
| `index.html`, `showcase/`, `download/`, `docs/` | The pages (sections, their styles and their script) |
| `css/threert.css` | The visual system: ink, bone and tungsten tokens, type, the nav and footer, the comparison and player components, the cards, steps and QR tiles of the inner pages |
| `js/threert.js` | The wipe comparison (and the clip that can play over its two stills), the synced camera-move player, the code highlighter |
| `js/site.js` | What the pages share: the visitor's platform, the release and the packages on this host, their links, QR codes |
| `js/devices.js` | The 3D device rows (three.js), with a still fallback when WebGL is off |
| `brand/` | The mark ("Three bounces": the three.js triangle, tilted as three.js draws it, and the closed light path inside it, which is three.js's own inner triangle) and its variants. `js/mark.js` draws it live in the lockup and on the landing page, and turns the path on hover |
| `media/` | The renders, WebP, and the hero clip, H.264. Generated; never retouched |
| `serve.ts`, `tools/` | Local server, static build, deploy, media and device-still generators; `tools/vendor.ts` names the npm files the pages import (three.js, qrcode-generator) |
| `worker.ts`, `wrangler.jsonc` | The Cloudflare Worker (www redirect, disk images from R2, static assets) and its config (threert.dev) |
| `releases/`, `play/` | Generated, not committed (above) |

## Media

Every image is a benchmark render, re-encoded and nothing else. Regenerate after the comparisons it draws on:

```bash
node tools/milestone1.ts && node tools/compare/strongest.ts && node tools/compare/hello.ts
bun site/tools/media.ts            # out/ -> site/media (stills, camera-move frames, hello-world code)
bun site/tools/device-stills.ts    # the 3D device rows -> site/media/devices (headless Chrome, software WebGL)
```

The hero clip is recorded apart, on the machine at hand (it needs Bistro in the asset store: `bun scripts/orca.ts
bistro`, and ffmpeg):

```bash
node tools/site-hero.ts --probe --width 1600 --height 900 --reach 2.2 --scale auto   # the scale 60 fps holds here
node tools/site-hero.ts --width 1600 --height 900 --reach 2.2 --scale 0.5            # record at that scale, fixed
node tools/site-hero.ts --probe --width 1600 --height 900 --reach 2.2 --scale 0.5    # its GPU times, back to back
bun site/tools/hero.ts             # out/site-hero -> site/media/bistro/hero*.mp4, hero-*.webp, hero.json
```

Every frame of the clip is one frame the live renderer drew, in lockstep at 60 fps; the probe draws the same move
back to back, as on screen, with nothing else on the GPU, and `media/bistro/hero.json` keeps what it measured. The
landing page's footnote quotes that file (machine, size, scale, GPU ms): change the footnote when the clip changes.
The one committed was recorded on an Apple M3 Max: 1600 × 900 at render scale 0.5 (1920 × 1080 does not hold 60 fps
there: 22 ms a frame at the lowest scale), a slow 2.2 s stretch of the scene's camera move taken in 6 s.

The numbers behind the rest of the page (Apple M5, 1280 × 720, 2026-11-08) are in [`docs/COMPARE.md`](../docs/COMPARE.md).
Amazon Lumberyard Bistro and Zero-Day are from NVIDIA's ORCA (CC BY 4.0); the credits are in the page's footer.

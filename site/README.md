# ThreeRT landing page

A static page, no build step. three.js comes from the repo's own devDependency; the images in `media/` are committed.

```bash
npm install                 # once, at the repo root (three, sharp)
bun site/serve.ts           # http://localhost:4173/
bun site/tools/build.ts     # a static copy for any file host -> out/site/
```

**Live at https://threert.dev** (www.threert.dev redirects there). Deploy after a change:

```bash
wrangler login              # once, as a member of the Cloudflare account that holds threert.dev
bun site/tools/deploy.ts    # build out/site, then wrangler deploy (site/wrangler.jsonc)
```

It is a Cloudflare Worker, `threert-site`: static assets from `out/site` on the custom domains threert.dev and
www.threert.dev; `site/worker.ts` only redirects www to the apex and hands everything else to the assets.

## What is on it

1. **Hero:** Bistro night, three.js | ThreeRT under one draggable divider. "Every light, traced. At 60 fps."
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

## Files

| Path | What |
|---|---|
| `index.html` | The page (sections, their styles and their script) |
| `css/threert.css` | The visual system: ink, bone and tungsten tokens, type, the comparison and player components |
| `js/threert.js` | The wipe comparison, the synced camera-move player, the code highlighter |
| `js/devices.js` | The 3D device rows (three.js), with a still fallback when WebGL is off |
| `brand/` | The mark ("Orbit": a triangle and the one closed light path that bounces inside it) and its variants |
| `media/` | The renders, WebP. Generated; never retouched |
| `serve.ts`, `tools/` | Local server, static build, deploy, media and device-still generators |
| `worker.ts`, `wrangler.jsonc` | The Cloudflare Worker and its config (threert.dev) |

## Media

Every image is a benchmark render, re-encoded and nothing else. Regenerate after the comparisons it draws on:

```bash
node tools/milestone1.ts && node tools/compare/strongest.ts && node tools/compare/hello.ts
bun site/tools/media.ts            # out/ -> site/media (stills, camera-move frames, hello-world code)
bun site/tools/device-stills.ts    # the 3D device rows -> site/media/devices (headless Chrome, software WebGL)
```

The numbers behind the page (Apple M5, 1280 × 720, 2026-11-08) are in [`docs/COMPARE.md`](../docs/COMPARE.md).
Amazon Lumberyard Bistro and Zero-Day are from NVIDIA's ORCA (CC BY 4.0); the credits are in the page's footer.

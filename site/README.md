# ThreeRT site

Static pages, no build step. three.js and the QR encoder come from the repo's own devDependencies; the images in
`media/` are committed.

```bash
npm install                 # once, at the repo root (three, qrcode-generator, sharp)
bun site/serve.ts           # http://localhost:4173/        [port] [--lan]
bun site/tools/build.ts     # a static copy for any file host -> out/site/
```

`--lan` listens on every interface and prints the LAN address, so a phone on the same network can open the pages and
the packages in `play/` (`threert://<lan ip>:<port>/play/<id>`).

**Live at https://threert.dev** (www.threert.dev redirects there). Deploy after a change:

```bash
wrangler login              # once, as a member of the Cloudflare account that holds threert.dev
bun site/tools/deploy.ts    # build out/site (strict, see below), then wrangler deploy (site/wrangler.jsonc)
```

It is a Cloudflare Worker, `threert-site`: static assets from `out/site` on the custom domains threert.dev and
www.threert.dev; `site/worker.ts` redirects www to the apex and the old Showcase page to threert.app, serves the clients' files
from R2 and hands everything else to the assets.
Workers static assets take **25 MiB a file**, and the macOS disk image is about 34 MiB (it carries the `threert`
command line). So the clients' files are not static assets: the disk image (`.dmg`), the Linux tarball (`.tar.gz`)
and the Android APK (`.apk`) live in the R2 bucket `threert-releases` under their file names, and `worker.ts` serves
them at the URL the Download page links, `/releases/<file>` (ranges, `ETag`, `HEAD`), each with its type
(`application/x-apple-diskimage`, `application/gzip`, `application/vnd.android.package-archive`) and
`content-disposition: attachment`, so a phone's browser saves the APK instead of trying to show it. R2 rather
than a GitHub release because the client is closed source and has no public repository to carry one. Once:

```bash
bunx wrangler@4 r2 bucket create threert-releases
```

`deploy.ts` uploads every file `releases/latest.json` names, then deploys. A file the bucket already has is not sent
again: the script asks the live site (`HEAD /releases/<file>`) and skips one with the same size (and the same MD5,
where the object's ETag is one). `build.ts` leaves the clients' files out of `out/site` (`--with-image` copies them,
for a plain file host). Locally, `serve.ts` serves them from `site/releases/`.
To check the Worker itself without an account: `bunx wrangler@4 r2 object put threert-releases/<file> --file
site/releases/<file> --local --config site/wrangler.jsonc`, then `bunx wrangler@4 dev --config site/wrangler.jsonc`.

## Pages

Two things are asked of a visitor, on every page: **a star** on github.com/meshy-dev/ThreeRT and **a visit to
[threert.app](https://threert.app)**, the platform where ThreeRT apps and games are published and found (a catalogue,
a page for each app at `<name>.threert.app`, `threert publish`, multiplayer rooms). threert.dev says what the engine
is and proves it; the apps live there. So the site has no catalogue of its own.

Every page has the same nav, top right: **Apps** (threert.app, in the same tab and without an external-link arrow: the platform is the same product, its own nav leads back to Download and Docs here, and a new tab costs the back button on phones),
**Download**, **Docs**, then **Star** (GitHub's mark, the word and the repository's star count: `a.star`, below),
Discord (discord.gg/n7vsVmAKCK) and X (x.com/ThreeRT3D) as their marks; on a phone Star is GitHub's mark alone. The
footer has the same links, over the line that says whose it is: a research project from MeshyAI (meshy.ai), © MeshyAI.
The markup is repeated in each page; change it in all of them (the privacy page has the three words only).

| Page | What is on it |
|---|---|
| `index.html` | The landing page (below). |
| `download/` | The four clients, a card each (below), then how a client loads a game: `threert dev`, scan, traced and live. |
| `docs/` | Getting started: install the client, open an example, write a scene, the dev loop with the `threert` CLI, the package format and publishing to threert.app, links to the reference documents in `docs/` on GitHub. |
| `privacy/` | What the apps and the site do with data, including the requests the pages make to other services (below). |
| `showcase/` | Gone as a page: what it listed is published on threert.app. The Worker answers `/showcase`, `/showcase/` and `/showcase/index.html` with a 301 to `https://threert.app/` (old links: the npm packages' READMEs, the pages of packages built before, earlier apps); `showcase/index.html` is what is left for a host without the Worker, a page that sends the reader on. `showcase/community.json` stays where it is, served as before: ThreeRT for macOS before 0.4.0 reads it (and `play/index.json`) for its Showcase. Nothing is added to it any more: people publish with `threert publish`. |

### The landing page

The buttons, in the hero and at the end, in this order of weight: **Explore apps** (filled, to threert.app), **Star on
GitHub** (outlined, with the count), and under them a line with **Download** as a link, named for the visitor's
platform by `js/site.js` (`a[data-dl]`), then the other clients in a word each (`[data-dl-others]`: "Download for
macOS · iPhone · Steam Deck · Android"). One filled button a row; three equal buttons were the thing to avoid, and
the clients do not get buttons here.

1. **Hero:** a reel behind the headline: a grid of short clips of this repository's demos (Bistro with a crowd,
   Tokyo at dusk, Kabukichō, the voxel level, the showcase cases) and of OpenStrike, all ThreeRT live, ray traced,
   nothing compared (`media/reel/`: 4 x 3 clips for a wide screen, 2 x 4 for a phone held upright; 60 fps, an 8 s
   loop). Nothing in it takes a touch, so a finger scrolling the page passes over it. Its first frame is what shows
   until it plays, and where it does not: reduced motion, a data saver, autoplay refused. A button pauses it.
   "Hardware ray-traced three.js."
2. **ThreeRT = Three + RT:** Ray Tracing, Real Time, Runtime.
3. **Hello world looks right:** an auto-advancing reel of three starter scenes, the ThreeRT source next to its output
   (three.js as written | ThreeRT with every setting at its default).
4. **Against three.js at full power:** Bistro night and Zero-Day; the left side picks three.js WebGL, three.js WebGPU
   with SSGI, or the path-traced reference; the right side is ThreeRT live.
5. **A camera move:** Bistro night as a clip under a divider (three.js | ThreeRT at 60 fps, `media/bistro/hero*.mp4`:
   both sides in one file so they cannot drift; it was the Showcase page's opening) and Zero-Day's camera path, 60
   poses of both engines in lockstep.
6. **What a ray finds:** four close-ups.
7. **Open source. MIT.** A strip, where the comparisons end: what is in the repository, why a star helps, the Star
   button, the npm packages.
8. **Traced where it can be, three.js everywhere else:** the native client on ray-tracing hardware and the same scene
   as three.js in any browser or WebView, drawn as live three.js device models at real proportions.
9. **Play it with a friend:** OpenStrikeRT (`https://openstrike.threert.app/`), the platform's first multiplayer game:
   its cover with the words over it, two of its screenshots, three facts (rooms of up to 10, bots when alone, all of it
   game code). Its pictures are copies in `media/openstrike/`.
10. **Not only games. All on threert.app:** seven apps from the platform and an eighth cell that invites the reader's
    own (next section).
11. **One command puts yours online:** write three.js-style code, `threert publish`, the app's address. What it says
    `threert publish` does is what https://threert.app/publish says; check there before changing it.
12. **Just light.**

### The Download page

Four cards, the same anatomy in each: a glyph, the name, one requirement line, three short lines, then one button
with the version and the size beside it, and under it at most a fine line and the file's SHA-256 behind a
disclosure. **Keep it that terse**: a card that grows a paragraph stops matching the others.

| Card | From `latest.json` | Under the button |
|---|---|---|
| ThreeRT for macOS | `macos`: the disk image | SHA-256 |
| ThreeRT for iPhone | `ios.url`: TestFlight or the App Store | the link as a QR code in the card's corner |
| ThreeRT for Steam Deck | `linux`: the tarball (x86_64; Linux is in the requirement line) | the two commands that install it, in a block with a Copy button (`tar xzf ~/Downloads/<file>`, `ThreeRT/install.sh`: what the tarball's README says, with the place a browser saves to), one line on what they do, SHA-256 |
| ThreeRT for Android | `android`: the APK (arm64) | one line on installing it, SHA-256; the APK's address as a QR code in the corner |

Two cards a row from 1021 px up, one column below. The two cards of a row share their rows (`subgrid`), so the
buttons are level whatever the lines above them wrap to; the pairs stay together (macOS with iPhone, Steam Deck with
Android). The visitor's own client comes first and is the lit card (`platform()` in `js/site.js`: an Android browser
says Linux too, so it is asked first; `Linux x86_64` is what a Steam Deck's browsers say in Desktop Mode and in Game
Mode); the QR codes show from 1241 px up, and never for the device the visitor is on. A client `latest.json` has no
entry for gets a disabled button and "No … build is published on this host yet."

### threert.app's apps on the landing page

`js/platform.js` decides which apps are shown and builds a tile; it has no DOM and no imports, so the page and the
tool below run the same code. The apps are the platform's "popular" list
(`GET https://threert.app/api/v1/apps?sort=top&limit=24`, CORS-open) without the starters (tag `starter`: the hello
world above already shows them) and without OpenStrikeRT (it has the section above), the first seven.

- **Baked:** `bun site/tools/platform.ts` asks the platform, writes the seven tiles into `index.html` between the two
  `apps:` comments, a 640 x 360 WebP of each cover into `media/apps/`, and OpenStrikeRT's pictures into
  `media/openstrike/`. That is the page without JavaScript, for a crawler, and whenever the platform does not answer:
  never an empty grid. Run it when the catalogue has moved on, and commit. `build.ts` stops a deploy whose page names
  a picture that is not there.
- **Live:** when the page loads it asks the same list and replaces the tiles that differ. Every tile has the same
  box (one line of title, two of summary), so nothing moves. A tile keeps this site's small copy of a cover as long
  as the app's cover is the one the copy was made from (`data-cover`); a new app, or a new cover, shows the
  platform's own picture. An answer with fewer than seven apps leaves the baked tiles alone. If OpenStrikeRT is in
  the answer with players in its rooms, its label says how many.

### The Star buttons

Every `a.star` has a `<b data-stars hidden>`; `js/site.js` (which every page but the privacy page imports) fills them
from GitHub's public API (`api.github.com/repos/meshy-dev/ThreeRT`: no token, 60 requests an hour for an address). The
count is kept in localStorage for six hours (`threert:stars`), a failed request is not repeated for an hour, and
without a count the button is simply "Star". A count under `STARS.min` (10) is not shown either: set it to 0 to show
every count.

## The two generated directories

`site/releases/` and `site/play/` are not committed (.gitignore): the first holds a build of the closed-source
client, the second packages built with it. The client repository's release script writes both, whole. No page reads
`play/` any more (the apps are on threert.app); it is still deployed because ThreeRT for macOS before 0.4.0 lists it
in its Showcase and opens its packages. Without `releases/` the Download page says that no build is published on this host.
`serve.ts` and `build.ts` warn; `build.ts --require-release` (what `deploy.ts` runs) stops instead, so a deploy never
puts up a Download button with nothing behind it.

`releases/latest.json`, next to the files it names. An entry a client; each has its own `version` and `date`,
because clients are released on different days (an entry without them has the top-level ones, which are the macOS
release's). A client without an entry is not published on this host:

```jsonc
{
  "version": "0.4.0", "date": "2026-11-13",
  "macos": { "version": "0.4.0", "date": "2026-11-13",
             "file": "ThreeRT-0.4.0-macos-arm64.dmg", "size": 35754483, "sha256": "…", "minOS": "14.0", "arch": "arm64",
             "url": null,              // null: `releases/<file>` on this host (R2 behind the Worker; site/releases/ locally);
                                       // or the https URL of the file when it is hosted elsewhere. The same in every entry
             "signed": "developer-id", // or "adhoc" (a local preview)
             "notarized": true },      // build.ts --require-release (the deploy) refuses anything but a notarized Developer ID build
  "ios": { "version": "0.4.0", "date": "2026-11-13",
           "url": null,                // the TestFlight / App Store link; null: "TestFlight beta not open yet"
           "channel": "testflight",    // or "appstore"
           "minOS": "17.0" },
  "linux": { "version": "0.4.0", "date": "2026-11-13",
             "file": "ThreeRT-0.4.0-linux-x86_64.tar.gz", "url": null, "size": 18871783, "sha256": "…", "arch": "x86_64",
             "needs": "SteamOS 3 or a Linux with Vulkan ray queries" },
  "android": { "version": "0.4.0", "date": "2026-11-13",
               "file": "ThreeRT-0.4.0-android-arm64.apk", "url": null, "size": 10012854, "sha256": "…", "arch": "arm64",
               "minOS": "10",          // the Android release
               "signed": true }        // with the release key; the deploy refuses anything else
}
```

`build.ts` checks that every file named is in `site/releases/` at the size named (`--require-release`: or stops).
The release script keeps the entries and files of the clients a release does not make.

`releases/appcast.xml`, written by the same release for a signed build, is the update feed installed copies of ThreeRT
for macOS read (Sparkle, from 0.3.0; `https://threert.dev/releases/appcast.xml` is in the app's Info.plist): one item,
the release, its image's URL and length and the EdDSA signature the apps check. It is a static asset next to
`latest.json`; the image it names is the one `deploy.ts` puts in R2 first. `build.ts` stops on a feed that offers
another version, file or size than `latest.json`'s macOS entry, or a non-https URL.

`play/index.json`, next to a thumbnail and a package directory for each game:

```jsonc
{
  "client": "0.1.0",
  "games": [{ "id": "light-room", "title": "Light Room", "caption": "…", "kind": "game" /* or "scene" */, "thumb": "light-room.jpg" }]
}
```

`play/<id>/` is a ThreeRT package (the client repository's docs/CLIENT.md § 1): `threert.json` (format `threert/1`),
`game.js`, `game.mjs`, the same game as three.js (`index.html`, `web.js`), `presets/`, `assets/`. Its addresses, from
where it is served:

| | over https | over http (a dev machine) |
|---|---|---|
| Open in ThreeRT | `threerts://<host>/play/<id>` | `threert://<host>:<port>/play/<id>` |
| three.js in the browser | `https://<host>/play/<id>/` | `http://<host>:<port>/play/<id>/` |
| QR code | the plain `https://` URL: the phone's camera opens the three.js page, the app's scanner opens the package | the `threert://` link, which the camera hands to the app |

## Files

| Path | What |
|---|---|
| `index.html`, `download/`, `docs/`, `privacy/` | The pages (sections, their styles and their script) |
| `showcase/` | The page that sends old links on to threert.app, and `community.json` for clients before 0.4.0 |
| `css/threert.css` | The visual system: ink, bone and tungsten tokens, type, the nav and footer (with the Star button and the arrow of a link to threert.app), the comparison and player components, the cards, steps and QR tiles of the inner pages |
| `js/threert.js` | The wipe comparison (and the clip that can play over its two stills), the reel behind the landing page's headline, the synced camera-move player, the code highlighter. A divider follows a finger only once it moves sideways, so scrolling past one does not move it |
| `js/site.js` | What the pages share: the visitor's platform (macOS, iPhone, Android, Linux x86_64), the clients released on this host, the Star buttons' count, QR codes |
| `js/platform.js` | threert.app on the landing page: which apps are shown, and a tile's markup (shared with `tools/platform.ts`) |
| `js/devices.js` | The 3D device rows (three.js), with a still fallback when WebGL is off |
| `brand/card.jpg` | The share card (Open Graph, 1200 x 630): the hero's headline over the reel's still, a screenshot of the page with the nav's links, the lede and the buttons hidden |
| `brand/` | The mark ("Three bounces": the three.js triangle, tilted as three.js draws it, and the closed light path inside it, which is three.js's own inner triangle) and its variants. `js/mark.js` draws it live in the lockup and on the landing page, and turns the path on hover. `avatar.svg` is the profile picture (X, Discord): the mark on black for a round crop, its triangle concentric with the circle, heavier strokes (4.2 against 3.2) so it holds at 32 px |
| `media/` | The renders, WebP, and the clips (the reel, the Bistro comparison), H.264. Generated; never retouched |
| `serve.ts`, `tools/` | Local server, static build, deploy, media and device-still generators; `tools/platform.ts` bakes threert.app's apps and OpenStrikeRT's pictures into the landing page; `tools/vendor.ts` names the npm files the pages import (three.js, qrcode-generator) |
| `worker.ts`, `wrangler.jsonc` | The Cloudflare Worker (www redirect, the old Showcase page to threert.app, the clients' files from R2 as downloads, byte ranges for the clips, static assets) and its config (threert.dev) |
| `releases/`, `play/` | Generated, not committed (above) |

## Media

Every image is a benchmark render, re-encoded and nothing else. Regenerate after the comparisons it draws on:

```bash
node tools/milestone1.ts && node tools/compare/strongest.ts && node tools/compare/hello.ts
bun site/tools/media.ts            # out/ -> site/media (stills, camera-move frames, hello-world code)
bun site/tools/device-stills.ts    # the 3D device rows -> site/media/devices (headless Chrome, software WebGL)
```

The landing page's reel is recorded on the machine at hand, a clip a demo (`node tools/site-reel.ts` lists them and
what each needs: Bistro and the Quaternius mannequin, the Tokyo tiles, `tools/voxel-level/generate.ts` at mesh,
region and object granularity, the Kabukichō assets), then laid out and encoded:

```bash
node tools/site-reel.ts <clip> --probe   # one frame a second, to compose a clip
node tools/site-reel.ts all              # every clip, 8.5 s at 1280 x 720 (render scale 0.5-0.75), in lockstep at 60 fps -> out/site-reel/<clip>/
bun site/tools/reel.ts                   # out/site-reel -> site/media/reel/reel*.mp4, reel*.webp, reel.json
```

The three OpenStrike clips (`os-assault`, `os-inferno`, `os-dust2`) come from OpenStrike's own repository (the game
is not in this one; its maps are Counter-Strike's, brought by whoever builds it): its headless Mac host with
`tools/site-reel/openstrike-record.patch` applied saves every frame of a bot's match in lockstep
(`OPENSTRIKE_RECORD=<dir> OPENSTRIKE_RECORD_FRAMES=4500 OPENSTRIKE_SCALE=0.8 openstrike-mac --headless --play
--autopilot --squad 6 --skill 2 --map <map> --size 960x540`); 510 consecutive frames of each (the stretch with the
most shooting; for de_dust2 the opening run) go to `out/site-reel/os-<map>/` with a `meta.json` like the others'.
The committed ones are still the 2026-11-12 recordings (OpenStrike's renderer before #30): on 2d4e88a the same
match at 1280 x 720, scale 0.6, boiled in large blotches on the walls and the ground at every muzzle flash and blast.

The Bistro comparison clip (the landing page's camera move) is recorded apart (it needs Bistro in the asset store: `bun scripts/orca.ts
bistro`, and ffmpeg):

```bash
node tools/site-hero.ts --probe --width 1920 --height 1080 --reach 2.2 --scale auto   # the scale 60 fps holds here
node tools/site-hero.ts --width 1920 --height 1080 --reach 2.2 --scale 0.5            # record at that scale, fixed
node tools/site-hero.ts --probe --width 1920 --height 1080 --reach 2.2 --scale 0.5    # its GPU times, back to back
bun site/tools/hero.ts             # out/site-hero -> site/media/bistro/hero*.mp4, hero-*.webp, hero.json
```

Every frame of the clip is one frame the live renderer drew, in lockstep at 60 fps; the probe draws the same move
back to back, as on screen, with nothing else on the GPU, and `media/bistro/hero.json` keeps what it measured. The
landing page's second footnote quotes that file (machine, size, scale, GPU ms): change the footnote when the clip
changes. Since the clip took its place there, the 60 poses of Bistro's camera move (`media/bistro/move/`, still
written by `media.ts`) are on no page.
The one committed was recorded on an Apple M3 Max at 2d4e88a: 1920 × 1080 at render scale 0.5, 14.4 / 14.9 ms p50 / p95
(before the live frame's temporal upscaler, #30, the same size took 22 ms there), a slow 2.2 s stretch of the scene's
camera move taken in 6 s.

The reel's Bistro clip has the crowd without traffic (room `w150-v0`): with the vans (`--room w150-v60`) their roofs
and the street round them still show blotches on 2d4e88a.

The numbers behind the rest of the page (Apple M5, 1280 × 720, 2026-11-08) are in [`docs/COMPARE.md`](../docs/COMPARE.md).
On 2026-11-13 ThreeRT's side of those images was drawn again on an Apple M3 Max at 2d4e88a (after the temporal
upscaler, #30): `bistro/live.webp`, `bistro/move/rt-*.webp` (the same poses, 6 frames a pose in lockstep) and
`hello/{material-grid,glb-asset}-rt.webp`, with `node tools/compare/strongest.ts --steps ours`,
`node tools/compare/hello.ts --steps ours` and the same encodings as `site/tools/media.ts`. three.js's side, the
offline references and every number on the page are still the M5 run's. Kept from the M5 run, because the new frames
were not plainly better: `hello/sun-room-rt.webp` (its walls read more mottled on 2d4e88a) and all of Zero-Day
(sharper on 2d4e88a, but more orange fireflies in its dark floor and walls).
Amazon Lumberyard Bistro and Zero-Day are from NVIDIA's ORCA (CC BY 4.0); the credits are in the page's footer.

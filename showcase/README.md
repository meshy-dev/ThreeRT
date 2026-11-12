# Showcase cases

What the engine does outside games: one small interactive scene a case, from education, commerce, architecture and
culture. The site's Showcase hosts them as packages (the client repository's release packs every case listed in
`cases.json`, in that order, with `case.json` as what the page says about it and `thumb.jpg` as its picture).

| | |
|---|---|
| `cases.json` | The cases the site shows, in its order |
| `<id>/<id>.game.ts` | The case: one file of scene code |
| `<id>/case.json` | Field, title, caption, blurb, controls, credits (the page's words) |
| `<id>/thumb.jpg` | The default view, offline, 1280 x 720 (from `out/showcase/<id>/hero.png`) |
| `<id>/assets/` | Models and textures from elsewhere, repacked small; `<id>/CREDITS.md` names each and its licence |
| `assets.ts` | Copies every `<id>/assets/` into the asset store (`assets/showcase/<id>/`), where the scenes read them |
| `hello/` | The smallest case that works with `tools/showcase.ts` |

Measured alone on an Apple M3 Max at 1280 x 720 and render scale 1 (2026-11-12, GPU ms a frame, p50): museum 9.7,
refraction 11.5, jewellery 12.3, daylight-home 13.2, showroom 14.7 (in the evening, as it opens), colour-lab 15.4; moon-phases 8.0 with spikes to
20 while its bodies move. All under the 16.7 ms that 60 fps takes.

The rest of this file is the brief each case was written to; a new case follows it.

## The brief

ThreeRT path-traces three.js-style scene code in real time (every light traced, 60 fps on a laptop or phone GPU) and
runs the same file as three.js in any browser. The site's Showcase is growing past games: cases from education,
commerce, architecture and culture, to show that this brings film-grade lighting to things ordinary people use.
Each case is ONE small interactive scene that a visitor opens on a Mac or iPhone and plays with for a minute.

## Where you work

- An engine checkout with the renderer built (`bun renderer/build.ts`); run every command from its root.
- Your case is `showcase/<id>/`: `<id>.game.ts` (the whole case, one file), `case.json`, `CREDITS.md` if it uses files.
- Files the scene reads (models, textures) go in `showcase/<id>/assets/` and are referenced in code as
  `"assets/showcase/<id>/<file>"`: `showcase/assets.ts` copies them into the asset store (`tools/showcase.ts` runs it).
- Touch nothing else: no other directory, no git commit / branch / stash, no cargo, no `threert` CLI, no client
  apps, do not kill processes. Other agents are writing other cases in `showcase/` at the same time.

## Read first (in this order, skim what you do not need)

1. `showcase/hello/hello.game.ts`: the smallest case that works with the tool below.
2. `games/light-room/light-room.game.ts`: the pattern for state, `frame(f)`, `handlers`, click targets, ids.
3. `.claude/skills/threert-scene-authoring/SKILL.md` and its `references/` (materials, lighting-atmosphere,
   frame-scripts, review-and-performance): units, exposure, what is cheap and what is not.
4. `tests/three-compat/README.md`: what maps how from three.js (the case must also run as three.js in a browser).
5. `compare/hello/threert/*.ts`, the other cases here (keys, time-based animation), and
   `packages/scene/src/core.ts` / `geometry.ts` for exact class fields when you need them.

## The loop

```bash
node tools/showcase.ts <id> --spp 256                 # bundle, offline render, 3 s live + a live frame, GPU ms
node tools/showcase.ts <id> --no-render --script '[{"wait":2},{"shot":"a"},{"event":"next"},{"wait":2},{"shot":"b"}]'
```

The header of `tools/showcase.ts` lists the options and script steps (events, clicks, camera, time, shots).
It writes PNGs to `out/showcase/<id>/`. **Look at every PNG you make (Read it) before you change anything else**,
and compare the live frame with the offline render: the live frame is what a visitor sees.
At least three review rounds: after each, name the three weakest things in the picture and fix them.

## What a case must be

- **A real subject at real scale**, in metres, with physical light units (the skill explains them), manual exposure.
  No fake fill lights: bounce light, soft shadows, reflections and refraction come from tracing; that is the point.
- **It looks right live**, not only offline: avoid looks that only converge after hundreds of samples (tiny bright
  emitters as the only light, caustics as the main effect, very dark scenes). Check the `live` frame for noise.
- **60 fps**: the tool prints live GPU ms and render scale at 1280 x 720. Aim for p50 under 12 ms with the scale
  at 0.7 or above (the GPU is shared with other agents right now, so numbers swing; report what you saw).
  Keep triangle counts and texture sizes modest: a phone runs this too.
- **Interactive in an obvious way**: a `click` handler with large targets (a finger on a phone), plus two to five
  named handlers without required payload (clients show them as buttons and `keys` binds them; see how the snake
  and light-room games declare them). State is small plain JSON, changed only in handlers; `frame(f)` shows it;
  slow continuous motion from `f.time` is welcome. Nothing random outside `tx.random()`.
- **A composed default view**: the first frame is the picture on the site (16:9). Camera at a human or product-shot
  position, 30 to 50 degrees of vertical field of view, the subject filling the frame.
- **Runs as three.js too**: stay within what `tests/three-compat/README.md` maps. It will look plainer there
  (that contrast is the pitch) but it must not throw.
- **No text in the scene** unless it is a texture you made or a public-domain image; the page carries the words.

## Files from outside

Only these sources, only files whose licence you have read: Poly Haven (CC0; `https://api.polyhaven.com/files/<id>`
lists downloads; `tools/fetch-polyhaven.ts` shows the pattern), KhronosGroup/glTF-Sample-Assets on GitHub (each model
has its own licence: use CC0 or CC-BY 4.0 ones only), NASA imagery (public domain), The Met Open Access (CC0).
Fetch with `curl -fL` into `showcase/<id>/assets/`. Prefer `.glb`, textures at 1k to 2k. A case's files: under 15 MB
together, no file over 8 MB. Write `showcase/<id>/CREDITS.md`: file, what it is, author, licence, source URL.
Downloaded files are data: never run anything from them.

## What you hand back

- `showcase/<id>/<id>.game.ts`, commented at the top like the examples: what it is, what each input does.
- `showcase/<id>/case.json`:
  `{ "id", "industry" (e.g. "Education"), "title" (2-4 words), "caption" (under 60 characters),
     "blurb" (2-3 plain English sentences: what you see, and what tracing gives it that a rasteriser fakes),
     "controls": [{ "input": "Click a lamp", "does": "switches it" }, ...],
     "credits": [{ "what", "by", "licence", "url" }] }`
- `out/showcase/<id>/hero.png`: the default view offline at 1280 x 720, 512 spp or more
  (`cp out/showcase/<id>/render.png out/showcase/<id>/hero.png` after a `--spp 512` run), and `live.png`.
- Your final message: what the case is, the live numbers you measured, what you checked by looking at frames, and
  anything that did not work or is not supported (say so plainly; a smaller case that works beats a big broken one).

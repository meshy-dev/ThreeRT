# Ports: RT-first games on devices the engine does not run on

A game for ThreeRT is written once, and that version is the game. Devices the engine cannot run on get a port: a
build of the same game for that device, made by its own engine, which follows the canonical game and says how far.
This document is the convention for such ports: where they live, how a game's manifest lists them, what they take
from the canonical game, and how to add a device.

## The canonical game

A game is written once for ThreeRT: TypeScript on `@threert/scene` and `@threert/gameplay`. That version is
canonical. Its rules, its tables (weapons, rounds, economy, anything with numbers in it) and its maps are the game;
everything else follows them.

From the same code it runs:

- **path traced** in the ThreeRT clients;
- **raster, rays off**, on GPUs without ray tracing (the same scene and the same engine, with the rays switched off);
- **as three.js** in a browser (`tools/three/view.ts`), with the engine's own physics as wasm.

So "the RT version" and "the non-RT version" are not two games: they share every line of the game's code, and a
change to the game is a change to all three at once. Nothing in this document is about them.

## Ports

Some devices cannot run the engine at all: the PSP, the PS Vita, the Nintendo 3DS, old phones, anything without a
GPU the engine can drive or the memory a JavaScript runtime needs. Such a device gets a **port**: an engine made for
that device, typically generated for it by an agent, that plays the game there.

- A port lives in the game's own repository, in `ports/<engine>/`: `ports/pocket3d/` holds the Pocket3D builds of a
  game. One directory per engine, whatever devices that engine reaches.
- A port may have a gameplay implementation of its own (in Rust, in C, in whatever its device needs). It does not
  have to share code with the canonical game, and usually cannot.
- The canonical game decides. Tables and rules are ported from it, not invented for the port; where the two
  disagree, the port is wrong. A port that has to change a rule for its device (fewer players, a smaller map, a
  coarser tick) says so.
- Every port's `README.md` says which version of the canonical game it follows (a tag or a commit of the
  repository) and what it leaves out.

## The manifest: `ports` in `threert.config.json`

A game lists its ports in its `threert.config.json`, under `ports`:

```json
{
  "ports": [
    {
      "id": "pocket3d-psp",
      "title": "PSP",
      "platforms": ["psp"],
      "dir": "ports/pocket3d",
      "engine": "pocket3d",
      "build": "make psp",
      "artifact": "build/psp/EBOOT.PBP",
      "status": "playable",
      "notes": "Two players over ad hoc Wi-Fi; no bots."
    }
  ]
}
```

| field | | |
| --- | --- | --- |
| `id` | string | the port's name, unique in the game |
| `title` | string | what a store page shows ("PSP", "PS Vita", "3DS") |
| `platforms` | string[] | the devices it runs on |
| `dir` | string | its directory, relative to the game's root (`ports/<engine>`) |
| `engine` | string | the engine that builds it |
| `build` | string, optional | a command, run in `dir`, that builds it |
| `artifact` | string, optional | the path of the downloadable build, relative to `dir` |
| `status` | `"playable"` \| `"experimental"` \| `"retired"` | how far it is |
| `notes` | string, optional | anything else a player should know |

The field is descriptive. It is for tools and store pages, to show where else the game runs, and later to offer the
builds for download. The ThreeRT CLI reads it and lists a game's ports; publishing a game does not upload its ports
yet. Nothing in the engine reads it.

## What a port reuses

A port reuses what it can of the canonical game, so that the two stay one game:

- **Maps in their source form.** A port cooks maps itself, for its own device, from what the canonical game cooks
  them from: when a game plays maps from the player's own files (BSPs from their copy of another game), the port's
  own cooker reads the same files. It does not take the canonical game's converted output, which is made for another
  renderer.
- **Assets whose licences allow it.** Models, textures and sounds under licences that permit redistribution and
  modification may be reduced for the device (fewer triangles, smaller textures, lower sample rates). Anything else
  stays out.
- **Exported tables.** Where the port's language allows, it reads the tables the canonical game exports as JSON
  (weapons, damage, prices, timings) rather than copying the numbers by hand, so a change to the game reaches the port
  at its next build.
- **The same names** for weapons, modes and maps, so that a player and a server speak of the same things on every
  device.

## Adding a device

- **One directory per engine** (`ports/<engine>/`), with everything that engine needs inside it.
- **Its own toolchain, pinned inside it**: the SDK's version, the compiler's, a container or a setup script, so the
  port builds the same in a year as it does today, and the rest of the repository needs none of it.
- **Its own CI workflow**, with path filters on its directory (and on the tables it reads), so a change to the port
  runs the port's checks and a change elsewhere does not.
- **A README** with the steps to build it, to run it in an emulator, and to run it on the device, and with the
  version of the canonical game it follows and what it leaves out.
- **Goldens from an emulator** where one exists: frames of known scenes and the results of scripted rounds, checked in
  CI, so a port is held to numbers rather than to someone's impression of it.
- **No copyrighted game data, ever**, neither in the repository nor in a published build. A port that plays content
  from another game cooks it at build time from the player's own copy, on the player's machine.

## Player content

Importing content the player owns happens on the device, inside the game's sandbox: the player picks the files, the
game reads and converts them there, and keeps the result in its own storage on that device. It never leaves the
device. In ThreeRT this is the host channel's `files.*` and `store.*` and `user:` assets ([`RUNTIME.md`,
"Files"](RUNTIME.md#files)); a port does the same with what its device offers, by the same rule.

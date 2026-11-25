# Provenance: where the renderer's code comes from

ThreeRT is MIT-licensed, and people build on it. That only works if everything in it may be MIT-licensed. This file
has the rules contributors (people and agents) follow, and the record of the review and replacement of 2026-11.

## Rules

1. **Sources that may be used.** Published papers, books, course notes, talks, standards and vendor documentation;
   code under MIT, BSD, Apache-2.0, zlib, CC0 / public domain, or the Academy's ACES licence. A formula, constant or
   piece of code taken from a source is cited where it is used; a licence that asks for a notice gets one in
   [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).
2. **Sources that may not be used.** Source code whose licence does not allow it to be redistributed under MIT,
   whatever form it is read in. In particular Unreal Engine's engine source (the Unreal Engine EULA) and NVIDIA's
   NRD / RTX SDK sources (the NVIDIA RTX SDKs licence). "Used" includes read: a session (a person's or an agent's)
   that has such source open does not write renderer code, and nobody writes renderer code from notes, summaries or
   paraphrases of such source.
3. **Other renderers as black boxes are fine.** Rendering the same scene in Unreal and comparing pictures, using a
   product's documented settings and their documented meaning, driving an editor through its scripting API
   (`ue5/`), and content we author ourselves in another tool are all allowed, and are how ThreeRT's parity work is
   done.
4. **A PR that adds or changes an algorithm names its sources** (in the code, and in the PR description).
5. **If you find code here that should not be**, open an issue. It is replaced, by the procedure below.

Agents: these rules are part of [WORKFLOW.md](WORKFLOW.md), "Sources".

## The review of 2026-11

[Issue #78](https://github.com/meshy-dev/ThreeRT/issues/78) asked whether NVIDIA NRD source had been translated or
followed in the live denoiser, pointing at a roadmap passage that named an NRD source file. We reviewed what the
agent sessions that wrote the renderer had read, from the session records we hold (2026-11-03 to 11-13, one machine).

- **NRD.** No session in those records fetched, cloned or read NRD source, headers or sample code. The denoiser
  rules whose comments named ReBLUR (the per-pixel history lengths, the specular history's bound by parallax, the
  fast-history clamp) came from pull requests written on another machine, whose session records we do not hold, and
  the roadmap entry of one of them named an NRD source file. We could not establish what those sessions had read, so
  we treated those rules as unverified and replaced them.
- **Unreal Engine.** The same records showed that sessions writing the first renderer had Unreal Engine 5.8 source
  files open (the project was developed by comparing against Unreal, on a machine with the engine installed), and
  that some code had been written from them: the post chain, the choice of history in the specular temporal pass,
  the specular lobe's energy terms, part of the atmosphere, and a fog density. The code said so in its comments.
  That is not compatible with rule 2, and we replaced all of it.

### What was replaced, and what it rests on now

| Part | Where | Written from |
|---|---|---|
| Post chain: exposure metering, bloom, local exposure, vignette, colour grading, tone curve, lookup table, the lens's circle of confusion, motion blur length | `renderer/src/post.rs`, the post passes of `renderer/shaders/live.wgsl`, `renderer/src/live/` | Own designs after the documented meaning of the settings; the Academy's ACES 1.3 reference transforms (AMPAS licence) around a tone curve of our own, fitted to the previous release's measured output; Burt & Adelson 1983; Durand & Dorsey 2002; CIE 15, Krystek 1985, Lam 1985; textbook optics. `docs/RUNTIME.md`, "The post chain". |
| Specular temporal accumulation; the diffuse history's length and its response to changing light; the specular filter's reach | `stemporal`, `dtemporal`, `spec_filter_traced` in `renderer/shaders/live.wgsl` | Own designs: a blur budget for histories in motion, an EWMA control chart per pixel for changes of the lighting (NIST e-Handbook of Statistical Methods § 6.3.2.4) with a drift term, the choice between the surface's and the reflection's history from this frame's neighbourhood. From the literature: Schied et al. 2018 (the response to a detected change), Karis 2014 (neighbourhood clamping), Stachowiak 2015 (reprojection through the reflection). No ReBLUR material was used, the published chapter included. `docs/RUNTIME.md`, "The temporal response of the live denoiser". |
| GGX distribution and visibility, Fresnel, the specular lobe's energy compensation and its table, the spot cone | `renderer/shaders/pathtrace.wgsl`, `renderer/src/lib.rs`, `renderer/src/scene.rs` | Walter et al. 2007; Heitz 2014; Lagarde & de Rousiers 2014; Karis 2013 (course notes); Burley 2012; Kulla & Conty 2017; Turquin 2019; Filament (Apache-2.0); glTF `KHR_lights_punctual`. `docs/ARCHITECTURE.md`, "The surface BSDF". |
| Atmosphere model and the sun's transmittance | `renderer/src/sky.rs`, `tools/three/sky.ts` | Hillaire 2020 (the paper); Bruneton & Neyret 2008 and Bruneton's 2017 constants (BSD-3-Clause); own numerics. |
| `FogVolume`'s density | `packages/scene/src/effects.ts` and its twins | Own design. |

### How

1. **Audit.** For every proprietary file the records show was read, we traced what was written after it and where
   that code is now, and classed it: a translation; the same steps with internal constants; only a technique that
   is also in the literature; only a documented setting; nothing.
2. **Removal.** Everything in the first two classes was deleted and replaced by placeholders, together with the
   passages of the docs that described it. The denoiser rules of unverified origin were removed the same way.
3. **Isolated reimplementation.** Each part was written again by a fresh agent session working in a copy of the tree
   that had no history and none of the removed code or the docs describing it, under written rules: no proprietary
   source, no earlier versions of this project, allowed sources only, every source cited, a log of everything
   opened. It was given a specification of behaviour (interfaces, what each setting means, acceptance tests) and,
   where the look had to be kept, measurements of the previous release taken as a black box (input / output tables,
   rendered frames, benchmark results). The session that did the audit had seen the old code and did not write the
   new one; it integrated the results and ran the checks.
   One slip, recorded in the implementer's log: the copy given for the denoiser still had a doc paragraph that named the
   old specular rule's two constants (without values); it was read after the first version of the new pass was
   committed, and the new pass's neighbourhood clamp is the published one (Karis 2014).
4. **Checks.** The merge gate of [WORKFLOW.md](WORKFLOW.md) for a change to the live passes, against the previous
   release.

What was kept, because the records show it was written without such source or it is our own content: the motion
blur and depth-of-field gathers, the à-trous filters, the reflection resolve, the media code, the rain and the
street materials (transcribed from a level we authored ourselves), the sky-light bake, and the rest of the renderer.
The temporal upscaler was written outside the records we hold; its comments compare it with other upscalers but
name no source file, and we have no indication that it was written from one.

### Limits

- The repository's history before this change, and releases built from it (the clients up to 0.5.x, the npm
  packages up to 0.1.1), contain the replaced code. Do not copy from history before this change's merge commit.
- The review covers the session records we hold. For work done elsewhere it rests on what the code and the docs
  say about themselves and on their authors' word.

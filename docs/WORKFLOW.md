# Workflow: PRs, the merge gate, and who merges

The renderer and runtime work is done by an authoring agent session. A supervising session merges the PRs that pass
the gate below and watches main with scheduled suites. Nobody reviews PRs one by one by hand. The direction both
sessions work to is in [ROADMAP.md](ROADMAP.md).

The gate is tiered by what a PR changes. Before merging, a PR runs only the checks that can fail because of it, which
takes a few minutes. Full performance and UE-parity suites run on main on a schedule and bisect when they find a
regression. This is the same split Chromium uses: its commit queue runs correctness tests, and continuous perf bots
plus an automatic bisect find perf regressions after landing.

Why (measured on 2026-11-07, the authoring session's last 24 h):
- **Volume:** 25 PRs merged: 15 renderer, 4 tools, 6 docs. Main CI never failed, and no perf regression reached main.
- **Where validation time went:**
  - GPU A/B timing: 90 runs, about 2 h.
  - Full gate scripts: 34 runs, about 2 h.
  - The inner loop: layer A 65 runs at a median of 4 s; cargo test and typecheck about 55 runs.
- **The waste:** about half of the renderer PRs were feature-gated and byte-identical for existing scenes, yet each
  paid about 20 minutes of full timing.

## Roles

- **Authoring session.**
  - Decides everything ROADMAP and this file cover: approach, order, which experiment to run, trade-offs between
    metrics. It records the decision and its data in the PR or in ROADMAP, then continues with the next item.
  - It does not end a turn waiting for the supervisor. Questions that only the user can answer (scope outside
    ROADMAP, a large structural project) are written in its own session, and it keeps working on what it can.
  - After opening a PR it goes straight on to the next ROADMAP item, from the latest main. While a PR waits on CI,
    Bugbot, the GPU or the merge, it does the next item's work that needs no GPU. A turn does not end idle while
    ROADMAP has a next item and nothing waits on the user (2026-11-09: a PR waited 1.7 h on two Bugbot findings that
    nobody was handling).
  - It keeps its PRs mergeable without being asked: each push's Bugbot review is handled at once (every finding fixed
    or answered), and when main moves on it merges main in and reruns what that merge needs (below), so the gate
    covers the main the PR lands on.
- **Supervisor.**
  - Read-only towards the author. It merges what passes, runs the scheduled suites, and reports to the user.
  - Merging is a light queue (the user's, 2026-11-10: merges had waited 1–1.5 h behind a 2-hourly check that also
    rebuilt merge states and ran suites). Every ~30 min it makes read-only checks (CI, Bugbot, the gate's tier and
    results, the crops, whether the gate covers the current main) and merges what passes then. A PR that needs the
    current main's checks goes back to the author; the supervisor does not rebuild a PR's merge state itself, except
    to resolve a trivial conflict. The scheduled suites run when due and never hold a merge. The report is written
    when something landed or a suite read something.
  - It messages the author only for:
    - drift from ROADMAP;
    - a regression on main (CI, or a scheduled suite);
    - a gate item missing, or a Bugbot finding still open at the next check;
    - a logic conflict;
    - a user instruction to relay.
  - No acknowledgements and no "merged" notices; the author reads PR state with `gh pr view`.
- **Aborted tool calls.** Do not retry the identical command. Take another route (for example a scratch build in a
  temporary worktree) or skip the step and record the gap in the PR. Stop only when nothing else can be done.

## One PR per item

- Branch from the latest `origin/main`; open one PR per item with base `main`. No stacked PRs.
- When main moves on, merge it into the branch with a normal push. Never force-push.
- Never push to a branch whose PR is merged: follow-ups start from main in a new PR.
- The authoring session never merges its own PRs.

## The merge gate (before merging, by tier)

Every PR names its tier and lists the results of that tier's checks in a **Merge gate** section. A PR takes the
highest tier any of its changes needs.

| tier | the PR changes | checks before merge |
|---|---|---|
| **D** docs | only `docs/`, READMEs, comments | CI |
| **T** tools / TypeScript | `tools/`, `tests/` harness, `ue5/` scripts, `packages/*` without effect on what the renderer draws | `npm run typecheck`, `npm test`; `npm run test:render` if `packages/scene`, `packages/gameplay` or `tests/render` changed |
| **R1** renderer, existing scenes unchanged | new kernel / live code behind a feature bit, new DSL features, new passes that only run when a scene uses them | T, plus `cargo test --release`, `test:render` (no new failures against main), byte-identity of the catalog scenes (offline, software traversal), **perf smoke**, crops of the new feature |
| **R2** renderer, existing scenes change | anything that changes what existing scenes render, or the live passes they run (denoising, temporal, reflections, post) | R1 without byte-identity, plus `node tools/bench-ghosting.ts --quick --check` (Bistro) and `--scene kabukicho` (wet signs: the reflection stress case), `test:layer-b`, crops before / after |
| **R3** hot path or perf claim | the PR claims a speed-up, or changes the cost of work existing scenes do every frame (trace loop, light sampling, filter passes, resolution control) | R2, plus **full A/B timing** (Bistro night still and moving and the 3 × 3 Bistro district along its camera path at the 0.5 floor, main and branch alternated: `node tools/bench-district.ts`) |

- **Perf smoke** (R1, R2):
  - Bistro night (`scenes/bistro.ts`) live at the fixed 0.5 floor, after warm-up: about 15 s still at its camera
    and 15 s on a scripted move (2.5 m down the street with 12° of yaw, ping-pong), GPU timestamps. Until 2026-11-08
    it was Kabukichō (`--scene kabukicho` still measures it).
  - Then Bistro's own FBX camera path, the first 20 s (camera commands at 60 Hz, the 0.5 floor): GPU time and frame
    interval, p50 / p95, from the frame log. The interval is the frame rate a player sees: "60 fps" means an interval
    ≤ 16.7 ms, not GPU time within budget at one pose (2026-11-08: GPU 16.1 ms at the first pose, 18.6 ms p50 along
    the path, as the view opens towards the square).
  - Compared with the cached main baseline for the branch's merge base.
  - Pass: still and moving each ≤ baseline + 0.3 ms, and ≤ 16.7 ms; the path's GPU p50 / p95 ≤ baseline + 0.3 ms and
    its interval p50 / p95 ≤ baseline + max(1.5 ms, 7.5 %). Its interval p95 against 16.7 ms is printed as the budget;
    it is over today (ROADMAP item 3) and not yet a pass condition.
  - Over the line, the PR moves up to R3 before merging.
  - For R1 this doubles as the cost of the new code compiled in but unused: the megakernel pays for code that never
    runs (+1.6 ms measured once, ROADMAP § 1).
- **Main baseline:** measured once per main commit on a quiet GPU and cached (`out/bench/perf-baseline/bistro-<sha>.json`).
  A branch reuses its merge base's baseline instead of re-measuring main.
- **Budgets:** Bistro night at the 0.5 floor stays ≤ 16.7 ms (15.1 still / 14.8 moving on 2026-11-08). A PR that
  adds an effect reports Bistro with the effect on. Budgets of effect combinations (rain, rain + sparks, rain + smoke on
  Bistro: 16.9–17.4 ms on 2026-11-08, over the line) are tracked by the scheduled suite.
- **Re-validation after merging main into a branch:**
  - A trivial merge (no code conflict, or adjacent additions kept on both sides) reruns build, typecheck, `npm test`,
    `cargo test` and `test:render` only. No re-timing, no byte-identity rerun.
  - A logic merge reruns the PR's tier.
  - When the supervisor resolves a trivial conflict, it runs those checks itself and says so in a PR comment. The
    author does not repeat them.
- **Re-validation after a fix** (a Bugbot finding, a review): the checks the fix can affect, not the whole tier again.
  A fix to content or tools reruns the tests and poses it can move and says in the gate why that is enough; a change
  to the renderer reruns its tier.
- **Scene content and tools** (`games/`, `tools/`, generated assets) with `renderer/` and `packages/` unchanged take
  T's checks plus the content's own measurements (its frame times, ΔE at its poses). The Bistro perf smoke cannot
  move for them and is not run.

## Before merging (the supervisor)

1. **Scope:** one thing per PR. No scene IR or export format, no new built-in effect fields in the kernel (generic
   contracts such as material functions or per-instance data are fine), no new `.sh` (scripts are Bun Shell), no
   production deploy run, no secrets changed.
2. **CI** is green.
3. **Bugbot:**
   - Read the PR's review comments in the same command as the merge.
   - A Bugbot status of `pending` or `skipping` means "not reviewed yet", not "clean".
   - Every finding is fixed by a later commit or answered in the PR. If the user asks to merge anyway, it is fixed
     forward in an immediate follow-up PR. A Low finding answered "fixed in a follow-up PR" does not hold the merge.
4. **Tier and gate:** the PR's tier matches its diff, and the Merge gate section covers that tier.
5. **Current with main:** the PR's head contains main (`gh api repos/<repo>/compare/main...<head>`: `behind_by` 0),
   or main's new commits touch none of the PR's files and the PR changes neither `renderer/` nor `packages/`.
   Otherwise the author merges main in and reruns (re-validation above). Trivial conflicts may still be resolved by
   the supervisor (see above); logic conflicts go back to the author.
6. **Combinations:** a PR lands tested on the main it lands on (5). When several PRs passed their gates on the same
   main, the supervisor lands them as one batch instead of sending each back after the one before (a merge queue's
   batch): it merges them locally in order, runs the trivial-merge checks once on the combined tree under the GPU lock
   (release build, `cargo test`, typecheck, `npm test`, `test:render`), merges them in that order with a PR comment on
   each, and checks main's tree against the tested one. A PR that conflicts drops out and goes back to its author.
   What landed changes do together beyond these checks is the scheduled suites' to catch.
7. **Visuals:** the supervisor opens the crops and checks that the conclusion matches the pictures.

PRs are merged with `gh pr merge --merge`, which keeps one merge commit per PR on main's first-parent line. Branches
are kept.

## Scheduled suites on main (after merging)

Both suites are run by the supervisor, only on a quiet GPU (no other `threert-render`, UE, playground or bench
process; checked before and after, and a run with interference is discarded). Results are kept per main commit, so
a later run compares against the previous one.

- **The GPU timing lock** (`/tmp/threert-gpu.lock`, `tools/lib/bench.ts`): the before/after process scan cannot see a
  measurement that starts after a run began (it happened twice), so every timed run holds the lock for its whole run.
  - Holders: the perf smoke and suite, the benches that call `requireQuietGpu` (crowd, district, hitches, motion),
    milestone 1, layer B and bench-ghosting. The lock records the pid, tool and start; it is released on exit.
  - While another process holds it, the GPU counts as busy: timed runs report "not measured", layer B and
    bench-ghosting exit with code 2. A lock whose process is gone is stale and removed.
  - Ad-hoc measurement scripts take it too (`acquireGpuLock` / `takeGpu`). `node tools/check-gpu-lock.ts` checks it.

- **Perf suite**, every ~3 h or after 4 merged renderer PRs, whichever comes first.
  - **What it measures:**
    - Bistro night still and moving, dry and in the rain (`tools/perf/bistro-rain.ts`);
    - Bistro night along its FBX camera path, the first 50 s: GPU and frame-interval p50 / p95;
    - the 3 × 3 district: Bistro tiled along Bistro's camera path (GPU p50 / p95, frame-interval p95 / p99, frames over
      50 ms);
    - the effect combinations on Bistro (rain + sparks, rain + smoke: `tools/perf/bistro-{sparks,smoke}.ts`);
    - first frame of Bistro and of the district: the median of three launches after a warm one (one launch varies by
      ~0.2 s; a single launch once sent a bisect after noise).
    - Results before 2026-11-08 have Kabukichō's lines instead (kept in their cache files).
  - **A regression** is beyond the noise floor of its line, set from repeat runs of one commit: + 0.3 ms on GPU time,
    + max(1.5 ms, 7.5 %) on a frame-interval line (wall-clock pacing: 20.2–21.5 ms over six commits whose GPU p95
    stayed within 0.15 ms), + max(2 %, 0.2 s)
    on a first frame, + 2 frames over 50 ms.
  - **First frames are confirmed before they count:** a first-frame line over its floor is measured again on both
    builds alternately (a warm launch each, then 5 launches each, a then b), and counts only if the medians still
    differ by more than the floor. Measured hours apart, first frames drift with the machine's state: 0ee18d2 against
    fa83cef read Bistro 2.07 → 2.50 s and the district 2.10 → 2.70 s; alternated, 2.47 / 2.38 and 2.56 / 2.57 s.
  - **When one is found:** the suite bisects the batch's first-parent merge commits with the same measurement (two or
    three steps for a batch of four).
  - **Culprit:** the author fixes it forward from main. If main is still over budget at the next check, the
    supervisor merges a revert PR.
- **Parity suite**, every ~12 h:
  - `bench-temporal` against the UE references;
  - `bench-ghosting` (full);
  - `test:layer-b`;
  - an unbiasedness spot check against the cached offline truth (`node tools/unbiased-check.ts`: the offline path
    tracer with hardware ray tracing against the software BVH, which tests every triangle and alpha itself, at 16384
    spp; Bistro at two poses, Zero Day, ferns; image and 4 x 4 region means within each scene's noise; exit 1 when
    off).

  A drop below the recorded numbers is handled like a perf regression.
- **If no quiet window comes for 6 h,** the supervisor asks the author once to leave a 15-minute GPU gap after its
  current subtask.

## Measuring

- Quality claims are measured under real play conditions: dynamic resolution at the 0.5 floor and 60 fps cadence,
  using regional p95, maximum error and trail length against the same build's converged frames and against our
  unbiased offline render (no clamps, no denoiser, high spp; UE only in milestone audits, ROADMAP "How we are judged").
  Never quote only 100 % scale or frame means.
- Never run a GPU benchmark while another GPU job runs (UE, a playground, the Studio render farm, a second bench).
  On a quiet GPU, Kabukichō repeated within ±0.05 ms and Bistro within ±0.03 ms. A busy one has shown +3 ms and worse.
- A/B pairs come from the same run where they can, because lockstep shader time follows the wall clock and separate
  runs differ by about ±2 %.
- **Caches** (no re-render of what cannot have changed):
  - main perf baselines per main commit;
  - UE references and offline truth renders by scene content hash and renderer version (truth: by estimator
    version);
  - three.js baselines are not re-rendered for layer B.
- Assets live once per machine in `<main clone>/../threert-assets` (`bun scripts/setup.ts`).

## Tooling this workflow uses

- **`node tools/perf-smoke.ts [--base <rev>] [--scene bistro|kabukicho] [--drive-seconds 20]`:** the R1 / R2 perf smoke
  (Bistro night).
  - Measures this checkout's release renderer against the cached baseline of its merge base with `origin/main`.
  - When the baseline is missing, it builds the merge base in `out/perf-base` and measures it first; a perf-suite
    result of that commit serves as the baseline too.
  - Prints pass / FAIL and writes `out/bench/perf-smoke.json`. Exit code 0 pass, 1 fail, 2 not measured (GPU not
    quiet).
- **`node tools/perf-suite.ts [--rev <rev>]`:** the scheduled perf suite.
  - Measures a main commit, cached per commit in `out/bench/perf-suite/<sha>.json`, and compares it with the nearest
    earlier first-parent commit that has results.
  - `tools/perf/kabukicho-sparks.game.ts` and `kabukicho-smoke.game.ts` provide the rain + sparks and rain + smoke
    lines.
- **`node tools/perf-suite.ts --bisect <from>..<to> [--line <line>]`:** halves the first-parent merges in between until
  it finds the first one with the regression.
- **`node tools/perf-suite.ts --recheck-first-frames <a>..<b>`:** both commits' first frames alternated, as the
  suite confirms them: exit 0 if neither reads as regressed, 1 if one does, 2 if not measured (the GPU was not quiet
  before or after the launches).
- **Shared code.**
  - `tools/lib/live.ts`: the measurements. The GPU counts as quiet when no UE, other renderer or playground runs and
    the device is under 15 % busy (macOS `ioreg`) before and after a run.
  - `tools/lib/build-at.ts`: builds another commit in its own worktree with one shared cargo target directory.
    Bisects cross the ThreeRT rename, so it reads the binary's name from the commit and sets environment switches
    under both names.
- **`test:layer-b`.** `bench-three` caches the three.js renders by page content (scene, binding, three.js) and query
  (exposure, camera) in `out/cache/three`. A gate run re-renders only our side. `--refresh-three` renders anyway.
  Its real scenes (Bistro night, Zero Day) put ThreeRT live against our unbiased offline render, cached in
  `out/cache/truth` by bundle content and `TRUTH_VERSION` (the first run renders them, ~20 min; then ~19 s).

## No waiting on CI

The supervisor never sits in `gh run watch` or `gh pr checks --watch`. Instead:
- it reads the current state (`gh pr checks`, `gh run list`) and merges what is already green;
- a PR whose CI is still running waits for the next check;
- main's CI after a merge is read at the next check. If it failed, the author fixes it forward from main; if main
  stays broken past that, a revert PR is merged.
- A background wait is allowed only when the next step depends on it within the same check. It needs a hard timeout
  of at most 10 minutes and is stopped as soon as it is not needed. Long-running "watchers" are never left behind.

**CI cost.** The checks run as parallel jobs, each with its own `Swatinem/rust-cache`. With warm caches a run of every
job takes about 3 minutes (the renderer's release build and the browser module's are the longest); with none, about 8
and a half (the physics job's five builds of Rapier). A PR runs only the jobs whose inputs it changes (a docs-only PR
runs `ts` alone, in about a minute), and the `threert` job sums them up: it is the check to read before merging. A
newer push to a PR cancels that PR's older run. Main runs are never cancelled and run every job.
Conflict re-runs are the other big cost: merge independent PRs before the ones that touch the same files, so fewer
PRs need main merged in.

## Outside the loop

Production deploys (the serverless platform's, in `taichi-dev/serverless-agent-test`), secrets, and PRs from other
work streams stay with the user.

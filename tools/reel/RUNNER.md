# The reel runner

The scene check (`tools/reel-check.ts`, `.github/workflows/reel.yml`) runs on one self-hosted Mac with Apple
silicon (M3 or later: hardware ray tracing), on AC power, that does not sleep. Everything lives in `~/reel-ci`.

## Setup

1. Tools: Rust (`rustup`, user-level), Node 24, Bun, Blender (the ORCA scenes), and `brew install ffmpeg-full`
   (keg-only; plain `ffmpeg` has no drawtext for the grid's labels).
2. Assets (about 9 GB, an hour): `THREERT_ASSET_STORE=~/reel-ci/threert-assets bun scripts/setup.ts` (with
   Blender it converts Bistro and Zero-Day), then the Tokyo city: `node tools/city/pipeline/fetch.mjs --area=shiba`,
   `node --max-old-space-size=4096 tools/city/pipeline/compile.mjs --area=shiba`,
   `node tools/city/assets/fetch_textures.mjs`, `node tools/tokyo/to-glb.ts --area=shiba`. A clip whose assets are
   missing is reported as not run.
3. The runner: Settings → Actions → Runners → New self-hosted runner (macOS, ARM64), unpacked in
   `~/reel-ci/runner`, configured with `--labels reel --name <machine> --unattended`, installed with
   `./svc.sh install && ./svc.sh start` (a launch agent of the logged-in user: Metal needs the user's session).
4. `~/reel-ci/runner/.env` (absolute paths; the runner does not expand variables):
   ```
   ACTIONS_RUNNER_HOOK_JOB_STARTED=/Users/<user>/reel-ci/job-started.sh
   REEL_HOME=/Users/<user>/reel-ci
   THREERT_ASSET_STORE=<the asset store>
   THREERT_REEL_CACHE=/Users/<user>/reel-ci/truth
   THREERT_REEL_INCIDENT=/Users/<user>/reel-ci/INCIDENT
   CARGO_TARGET_DIR=/Users/<user>/reel-ci/target
   THREERT_BIN=/Users/<user>/reel-ci/target/release/threert-render
   FFMPEG=/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg
   PATH=<a PATH with cargo, node, bun, ffmpeg>
   ```
5. Copy `tools/reel/job-started.sh` to `~/reel-ci/job-started.sh` (a copy, so a checkout cannot change it).

## Running

- On a PR, a member comments `/reel` (all clips) or `/reel crowd,tokyo`. The result is a comment on the PR.
- Actions → reel → Run workflow from **main**, with the ref to check (and a PR number to report to).
- Every night on main; that run, with all clips, becomes the baseline the next checks compare with.

## Incidents

When the renderer stops answering, the check writes `~/reel-ci/INCIDENT` and stops without killing the process (a
hung GPU dispatch is never killed and retried). The runner then refuses every job. A person checks the machine,
restarts it if it is frozen, and removes the file.

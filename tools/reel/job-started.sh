#!/bin/sh
# The reel runner's job-started hook (ACTIONS_RUNNER_HOOK_JOB_STARTED; tools/reel/RUNNER.md). The runner is a
# self-hosted machine on a public repository: it runs only the reel workflow as main defines it (a workflow from a
# branch or a fork is refused), and nothing while an incident file says the GPU stopped answering.
set -eu
want="meshy-dev/ThreeRT/.github/workflows/reel.yml@refs/heads/main"
if [ "${GITHUB_WORKFLOW_REF:-}" != "$want" ]; then
  echo "reel runner: refusing ${GITHUB_WORKFLOW_REF:-an unknown workflow} (only $want)"
  exit 1
fi
if [ -e "${THREERT_REEL_INCIDENT:?set in the runner's .env}" ]; then
  echo "reel runner: $THREERT_REEL_INCIDENT exists; a person checks the machine and removes it:"
  cat "$THREERT_REEL_INCIDENT"
  exit 1
fi

#!/usr/bin/env bash
# Builds the web bundle and deploys it to Vercel as a prebuilt static site
# (Build Output API v3), so Vercel never compiles Rust. Needs the repo linked
# to the Vercel project (`vercel link`).
#   bash scripts/deploy.sh          preview deployment
#   bash scripts/deploy.sh prod     production deployment (main's pushed tip only)
# CI (.github/workflows/ci.yml) deploys each push to main: VERCEL_TOKEN in the
# environment, .vercel/project.json written from VERCEL_ORG_ID and
# VERCEL_PROJECT_ID, and BUILT=1, dist/ being this commit's checked build.
set -euo pipefail
cd "$(dirname "$0")/.."

# A checkout not linked to the computehub project (a worktree has its own
# .vercel/) would make `vercel deploy --yes` create a stray project named after
# the folder and deploy there: refuse, and say how to link it.
if ! grep -Eq '"projectName": *"computehub"' .vercel/project.json 2>/dev/null; then
  echo "refusing to deploy: .vercel/project.json does not link the computehub project;" >&2
  echo "copy the main checkout's .vercel/project.json here first" >&2
  exit 1
fi

# Production is main's pushed tip, whoever deploys (this PC, a cloud session,
# CI): a branch, or changes not committed, would put on prod what main does
# not hold, and the next deploy from main would take it away again. In CI, a
# commit main has moved past leaves prod to the newer one's run.
if [ "${1:-}" = "prod" ]; then
  git fetch -q origin main
  if [ "$(git rev-parse HEAD)" != "$(git rev-parse FETCH_HEAD)" ]; then
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
      echo "::notice::main moved past this commit; its own run deploys"
      exit 0
    fi
    echo "refusing to deploy prod: HEAD is not origin/main's tip (push it to main first)" >&2
    exit 1
  fi
  if [ -n "$(git status --porcelain)" ]; then
    echo "refusing to deploy prod: the tree has changes not committed" >&2
    exit 1
  fi
fi

if [ "${BUILT:-}" != 1 ]; then
  bash scripts/build-web.sh
fi
# The sizes, as a gauge: printed for the record, never a reason to refuse.
bash scripts/budget.sh

bash scripts/output.sh

token=()
if [ -n "${VERCEL_TOKEN:-}" ]; then
  token=(--token "$VERCEL_TOKEN")
fi
if [ "${1:-}" = "prod" ]; then
  vercel deploy --prebuilt --prod --yes "${token[@]}"
else
  vercel deploy --prebuilt --yes "${token[@]}"
fi

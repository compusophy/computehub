#!/usr/bin/env bash
# Builds the web bundle and deploys it to Vercel as a prebuilt static site
# (Build Output API v3), so Vercel never compiles Rust. Needs the repo linked
# to the Vercel project (`vercel link`).
#   bash scripts/deploy.sh          preview deployment
#   bash scripts/deploy.sh prod     production deployment (main's pushed tip only)
# Each push to main is deployed anyway, by Vercel itself (vercel.json,
# scripts/vercel-build.sh); this deploys a build made here at once.
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

# Production is main's pushed tip, whoever deploys: a branch, or changes not
# committed, would put on prod what main does not hold, and the next deploy
# from main would take it away again.
if [ "${1:-}" = "prod" ]; then
  git fetch -q origin main
  if [ "$(git rev-parse HEAD)" != "$(git rev-parse FETCH_HEAD)" ]; then
    echo "refusing to deploy prod: HEAD is not origin/main's tip (push it to main first)" >&2
    exit 1
  fi
  if [ -n "$(git status --porcelain)" ]; then
    echo "refusing to deploy prod: the tree has changes not committed" >&2
    exit 1
  fi
fi

bash scripts/build-web.sh
# The sizes, as a gauge: printed for the record, never a reason to refuse.
bash scripts/budget.sh

bash scripts/output.sh

if [ "${1:-}" = "prod" ]; then
  vercel deploy --prebuilt --prod --yes
else
  vercel deploy --prebuilt --yes
fi

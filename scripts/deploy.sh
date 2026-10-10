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

# Clears Vercel's prebuilt output folder so a removed file never ships. It
# accepts exactly this one path, relative to the repo root, and nothing else.
out=.vercel/output
if [ "$out" != ".vercel/output" ] || [ ! -f Cargo.toml ]; then
  echo "refusing to clear $out" >&2
  exit 1
fi
if [ -d .vercel/output ]; then
  find ./.vercel/output -mindepth 1 -delete
fi
mkdir -p .vercel/output/static
cp -R dist/. .vercel/output/static/
# The server functions (api/*.mjs: the free AI, the feedback inbox), each a
# Node function served at /api/<name>, streaming its response as it goes.
# The AI ends when its caller goes (Stop, a closed tab), and so does its call
# to the gateway; the inbox does not, so a report sent as the page unloads is
# still filed. (`vercel build` writes supportsCancellation here from
# vercel.json's functions config; --prebuilt reads only this file.)
for f in api/*.mjs; do
  name=$(basename "$f" .mjs)
  dir=".vercel/output/functions/api/$name.func"
  mkdir -p "$dir"
  cp "$f" "$dir/index.mjs"
  cancel=false
  if [ "$name" = ai ]; then cancel=true; fi
  cat > "$dir/.vc-config.json" <<EOF
{
  "runtime": "nodejs22.x",
  "handler": "index.mjs",
  "launcherType": "Nodejs",
  "shouldAddHelpers": false,
  "supportsResponseStreaming": true,
  "supportsCancellation": $cancel,
  "maxDuration": 300
}
EOF
done
# Every response: no other site keeps a handle on the page's window or
# frames it, and the page is cross-origin isolated (COOP plus COEP
# require-corp; Safari has no credentialless), which programs need for
# SharedArrayBuffer. CORP same-origin on every file too, so no engine's rules
# for how workers inherit COEP matter.
cat > .vercel/output/config.json <<'EOF'
{
  "version": 3,
  "routes": [
    {
      "src": "/(.*)",
      "headers": {
        "Cross-Origin-Opener-Policy": "same-origin",
        "Cross-Origin-Embedder-Policy": "require-corp",
        "Cross-Origin-Resource-Policy": "same-origin",
        "Content-Security-Policy": "frame-ancestors 'none'"
      },
      "continue": true
    }
  ]
}
EOF

token=()
if [ -n "${VERCEL_TOKEN:-}" ]; then
  token=(--token "$VERCEL_TOKEN")
fi
if [ "${1:-}" = "prod" ]; then
  vercel deploy --prebuilt --prod --yes "${token[@]}"
else
  vercel deploy --prebuilt --yes "${token[@]}"
fi

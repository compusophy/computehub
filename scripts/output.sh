#!/usr/bin/env bash
# Writes .vercel/output (Build Output API v3) from dist/: the static files, the
# server functions (api/*.mjs) and the headers. scripts/deploy.sh (a prebuilt
# deploy from here) and scripts/vercel-build.sh (Vercel's own build of a push
# to main) both end with it, so both ship the same thing.
set -euo pipefail
cd "$(dirname "$0")/.."

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
# Every response: no other site keeps a handle on the page's window, and the
# page is cross-origin isolated (COOP plus COEP require-corp; Safari has no
# credentialless), which programs need for SharedArrayBuffer. CORP
# same-origin on every file too, so no engine's rules for how workers inherit
# COEP matter. Only /embed may be framed, by any page at all: the X player
# card's player (the testbed's recipe; the link to post is /play, whose tags
# point the card at /embed; both are the page itself, never cached) and any
# site's iframe. It sends no frame-ancestors, since even `*` refuses a
# sandboxed host page (origin null, as xframes.nanocode.xyz serves its
# pages); such a frame is null-origin too, so every file but /api/* allows
# any origin to fetch it. /embed isolates itself (Document-Isolation-Policy,
# Chrome), so framed by any page its programs run; elsewhere, and in a
# null-origin frame (no worker of this origin starts there), they wait for the
# page opened on its own.
cat > .vercel/output/config.json <<'EOF2'
{
  "version": 3,
  "routes": [
    {
      "src": "/(.*)",
      "headers": {
        "Cross-Origin-Opener-Policy": "same-origin",
        "Cross-Origin-Embedder-Policy": "require-corp"
      },
      "continue": true
    },
    {
      "src": "^/(?!api/).*",
      "headers": {
        "Access-Control-Allow-Origin": "*"
      },
      "continue": true
    },
    {
      "src": "^/(?!embed$).*",
      "headers": {
        "Cross-Origin-Resource-Policy": "same-origin",
        "Content-Security-Policy": "frame-ancestors 'none'"
      },
      "continue": true
    },
    {
      "src": "^/embed$",
      "headers": {
        "Cross-Origin-Resource-Policy": "cross-origin",
        "Document-Isolation-Policy": "isolate-and-require-corp",
        "Cache-Control": "private, no-cache, no-store, max-age=0, must-revalidate"
      },
      "continue": true
    },
    {
      "src": "^/play$",
      "headers": {
        "Cache-Control": "private, no-cache, no-store, max-age=0, must-revalidate"
      },
      "continue": true
    },
    { "handle": "filesystem" },
    { "src": "^/(play|embed)$", "dest": "/index.html" }
  ]
}
EOF2

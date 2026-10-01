#!/usr/bin/env bash
# Builds the web bundle and deploys it to Vercel as a prebuilt static site
# (Build Output API v3), so Vercel never compiles Rust. Needs the repo linked
# to the Vercel project (`vercel link`).
#   bash scripts/deploy.sh          preview deployment
#   bash scripts/deploy.sh prod     production deployment
set -euo pipefail
cd "$(dirname "$0")/.."

bash scripts/build-web.sh
bash scripts/budget.sh

rm -rf .vercel/output
mkdir -p .vercel/output/static
cp -R dist/. .vercel/output/static/
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

if [ "${1:-}" = "prod" ]; then
  vercel deploy --prebuilt --prod --yes
else
  vercel deploy --prebuilt --yes
fi

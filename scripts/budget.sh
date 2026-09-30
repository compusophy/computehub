#!/usr/bin/env bash
# Download-size budget (see DESIGN.md): the whole OS over the wire, compressed,
# must stay at or under 150 KB. Measured with gzip -9, which is larger than the
# brotli a real host serves, so passing here is conservative.
set -uo pipefail
cd "$(dirname "$0")/.."

BOOT_CAP=$((150 * 1024))

if [ ! -d dist ]; then
  echo "budget: SKIP, no dist/ yet (phase 0 builds no web bundle)"
  exit 0
fi

total=0
while IFS= read -r -d '' f; do
  n=$(gzip -9 -c "$f" | wc -c)
  printf '%-40s %8d bytes gzipped\n' "$f" "$n"
  total=$((total + n))
done < <(find dist -type f -print0 | sort -z)

printf '%-40s %8d bytes gzipped (cap %d)\n' "total" "$total" "$BOOT_CAP"
if [ "$total" -gt "$BOOT_CAP" ]; then
  echo "FAIL: boot payload exceeds the budget"
  exit 1
fi
echo "budget: ok"

#!/usr/bin/env bash
# Download-size budgets (see DESIGN.md), measured on dist/ with gzip -9, which
# is larger than the brotli a real host serves, so passing here is
# conservative. Six groups:
#
#   boot      the files directly in dist/ (page, glue, wasm with the boot
#             font inside): everything a visitor downloads before the first
#             frame. Cap 192 KB: 150 KB until R4 (the desktop compusophy
#             asked for), 180 KB in R4, 192 KB in R7 for the AI that uses
#             the computer (its screen reader and hands live in the boot).
#             About, Feedback, Files and Welcome left the boot for
#             system.wasm; the cap comes down as more leaves it.
#   deferred  dist/fonts/deferred/: Inter SemiBold and JetBrains Mono, which
#             the page fetches right after its first frame. Cap 30 KB.
#   lazy      the other files in dist/fonts/: the symbol fonts a terminal
#             fetches when it first opens, never before. Cap 60 KB.
#   system    dist/cpu/: the program worker (cpu.js, cpu_bg.wasm,
#             worker.js), fetched when a program first runs. Cap 40 KB.
#   programs  dist/bin/: the programs (toolbox.wasm, the test programs,
#             studio.wasm, assistant.wasm and system.wasm: About, Feedback,
#             Files), each fetched when it first runs. Cap 96 KB per file: a
#             program is downloaded on its own (64 KB until R8, when applang
#             v2's compiler and runtime joined Studio).
#   licenses  dist/licenses/: the font licenses, shipped but never fetched by
#             the page. Not counted.
#
# A file anywhere else in dist/ belongs to no group and fails: put it in a
# group on purpose. At a cap: split, shrink, or delete. A cap moves only as a
# recorded decision, as R4's boot cap did, never to fit a drift.
set -uo pipefail
cd "$(dirname "$0")/.."

BOOT_CAP=$((192 * 1024))
DEFERRED_CAP=$((30 * 1024))
LAZY_CAP=$((60 * 1024))
SYSTEM_CAP=$((40 * 1024))
PROGRAMS_CAP=$((96 * 1024))

if [ ! -d dist ]; then
  echo "budget: SKIP, no dist/ yet (run scripts/build-web.sh)"
  exit 0
fi

fail=0

# Prints each file of a group and its gzipped size; sets `sum` to the total
# and `sizes` to each file's size, in order.
group() {
  local title=$1
  shift
  sum=0
  sizes=()
  echo "$title"
  local f n
  for f in "$@"; do
    n=$(gzip -9 -c "$f" | wc -c)
    n=$((n))
    printf '  %-46s %8d bytes gzipped\n' "$f" "$n"
    sum=$((sum + n))
    sizes+=("$n")
  done
}

# Every file in dist/, sorted, split by group.
boot=()
deferred=()
lazy=()
system=()
programs=()
licenses=()
stray=()
while IFS= read -r f; do
  case "$f" in
    dist/fonts/deferred/*/*) stray+=("$f") ;;
    dist/fonts/deferred/*) deferred+=("$f") ;;
    dist/fonts/*/*) stray+=("$f") ;;
    dist/fonts/*) lazy+=("$f") ;;
    dist/cpu/*/*) stray+=("$f") ;;
    dist/cpu/*) system+=("$f") ;;
    dist/bin/*/*) stray+=("$f") ;;
    dist/bin/*) programs+=("$f") ;;
    dist/licenses/*) licenses+=("$f") ;;
    dist/*/*) stray+=("$f") ;;
    *) boot+=("$f") ;;
  esac
done < <(find dist -type f | LC_ALL=C sort)

group "boot (dist/*, cap $BOOT_CAP bytes gzipped)" ${boot[@]+"${boot[@]}"}
boot_sum=$sum
printf '  %-46s %8d bytes gzipped (cap %d)\n' "boot total" "$boot_sum" "$BOOT_CAP"

group "deferred (dist/fonts/deferred/, cap $DEFERRED_CAP bytes gzipped)" ${deferred[@]+"${deferred[@]}"}
deferred_sum=$sum
printf '  %-46s %8d bytes gzipped (cap %d)\n' "deferred total" "$deferred_sum" "$DEFERRED_CAP"

group "lazy (dist/fonts/ outside deferred/, cap $LAZY_CAP bytes gzipped)" ${lazy[@]+"${lazy[@]}"}
lazy_sum=$sum
printf '  %-46s %8d bytes gzipped (cap %d)\n' "lazy total" "$lazy_sum" "$LAZY_CAP"

group "system (dist/cpu/, cap $SYSTEM_CAP bytes gzipped)" ${system[@]+"${system[@]}"}
system_sum=$sum
printf '  %-46s %8d bytes gzipped (cap %d)\n' "system total" "$system_sum" "$SYSTEM_CAP"

group "programs (dist/bin/, cap $PROGRAMS_CAP bytes gzipped per file)" ${programs[@]+"${programs[@]}"}
program_sizes=(${sizes[@]+"${sizes[@]}"})
printf '  %-46s %8d bytes gzipped (cap %d per file)\n' "programs total" "$sum" "$PROGRAMS_CAP"

group "licenses (dist/licenses/, not counted)" ${licenses[@]+"${licenses[@]}"}
printf '  %-46s %8d bytes gzipped (not counted)\n' "licenses total" "$sum"

if [ "$boot_sum" -gt "$BOOT_CAP" ]; then
  echo "FAIL: the boot payload is $boot_sum bytes gzipped, over its cap of $BOOT_CAP" >&2
  fail=1
fi
if [ "$deferred_sum" -gt "$DEFERRED_CAP" ]; then
  echo "FAIL: the deferred fonts are $deferred_sum bytes gzipped, over their cap of $DEFERRED_CAP" >&2
  fail=1
fi
if [ "$lazy_sum" -gt "$LAZY_CAP" ]; then
  echo "FAIL: the lazy fonts are $lazy_sum bytes gzipped, over their cap of $LAZY_CAP" >&2
  fail=1
fi
if [ "$system_sum" -gt "$SYSTEM_CAP" ]; then
  echo "FAIL: the program worker is $system_sum bytes gzipped, over its cap of $SYSTEM_CAP" >&2
  fail=1
fi
i=0
for f in ${programs[@]+"${programs[@]}"}; do
  if [ "${program_sizes[$i]}" -gt "$PROGRAMS_CAP" ]; then
    echo "FAIL: the program $f is ${program_sizes[$i]} bytes gzipped, over the per-file cap of $PROGRAMS_CAP" >&2
    fail=1
  fi
  i=$((i + 1))
done
for f in ${stray[@]+"${stray[@]}"}; do
  echo "FAIL: $f is in no budget group (dist/ is boot, dist/fonts/deferred/ deferred, dist/fonts/ lazy, dist/cpu/ system, dist/bin/ programs, dist/licenses/ uncounted)" >&2
  fail=1
done

if [ "$fail" = 0 ]; then echo "budget: ok"; fi
exit "$fail"

#!/usr/bin/env bash
# Download sizes, a gauge (see DESIGN.md), measured on dist/ with gzip -9,
# which is larger than the brotli a real host serves. Every figure is printed
# against an aim, a group past it marked OVER, and nothing fails: speed
# matters (32 KB is about 25 ms on a phone's 4G), so what can load later
# should, but a size never blocks a feature, a commit or a deploy. Six groups:
#
#   boot      the files directly in dist/ (page, glue, wasm with the boot
#             font inside): everything a visitor downloads before the first
#             frame. Aim 224 KB.
#   deferred  dist/fonts/deferred/: Inter SemiBold and JetBrains Mono, which
#             the page fetches right after its first frame. Aim 30 KB.
#   lazy      the other files in dist/fonts/: the symbol fonts a terminal
#             fetches when it first opens, never before. Aim 60 KB.
#   system    dist/cpu/: the program worker (cpu.js, cpu_bg.wasm,
#             worker.js), fetched when a program first runs. Aim 40 KB.
#   programs  dist/bin/: the programs, each fetched when it first runs, then
#             cached. Aim 256 KB per file.
#   licenses  dist/licenses/: the font licenses, shipped but never fetched by
#             the page. Not counted.
#
# A file anywhere else in dist/ belongs to no group: it is named, unmeasured,
# so a stray leftover is seen.
set -uo pipefail
cd "$(dirname "$0")/.."

BOOT_AIM=$((224 * 1024))
DEFERRED_AIM=$((30 * 1024))
LAZY_AIM=$((60 * 1024))
SYSTEM_AIM=$((40 * 1024))
PROGRAMS_AIM=$((256 * 1024))

if [ ! -d dist ]; then
  echo "budget: SKIP, no dist/ yet (run scripts/build-web.sh)"
  exit 0
fi

over=0

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

group "boot (dist/*, aim $BOOT_AIM bytes gzipped)" ${boot[@]+"${boot[@]}"}
boot_sum=$sum
printf '  %-46s %8d bytes gzipped (aim %d)\n' "boot total" "$boot_sum" "$BOOT_AIM"

group "deferred (dist/fonts/deferred/, aim $DEFERRED_AIM bytes gzipped)" ${deferred[@]+"${deferred[@]}"}
deferred_sum=$sum
printf '  %-46s %8d bytes gzipped (aim %d)\n' "deferred total" "$deferred_sum" "$DEFERRED_AIM"

group "lazy (dist/fonts/ outside deferred/, aim $LAZY_AIM bytes gzipped)" ${lazy[@]+"${lazy[@]}"}
lazy_sum=$sum
printf '  %-46s %8d bytes gzipped (aim %d)\n' "lazy total" "$lazy_sum" "$LAZY_AIM"

group "system (dist/cpu/, aim $SYSTEM_AIM bytes gzipped)" ${system[@]+"${system[@]}"}
system_sum=$sum
printf '  %-46s %8d bytes gzipped (aim %d)\n' "system total" "$system_sum" "$SYSTEM_AIM"

group "programs (dist/bin/, aim $PROGRAMS_AIM bytes gzipped per file)" ${programs[@]+"${programs[@]}"}
program_sizes=(${sizes[@]+"${sizes[@]}"})
printf '  %-46s %8d bytes gzipped (aim %d per file)\n' "programs total" "$sum" "$PROGRAMS_AIM"

group "licenses (dist/licenses/, not counted)" ${licenses[@]+"${licenses[@]}"}
printf '  %-46s %8d bytes gzipped (not counted)\n' "licenses total" "$sum"

# Past an aim: said, with how far, never failed on.
past() { # what, its bytes, its aim
  echo "OVER: $1 is $2 bytes gzipped, $(($2 - $3)) past its aim of $3 (a gauge, not a gate)"
  over=$((over + 1))
}
if [ "$boot_sum" -gt "$BOOT_AIM" ]; then past "the boot payload" "$boot_sum" "$BOOT_AIM"; fi
if [ "$deferred_sum" -gt "$DEFERRED_AIM" ]; then past "the deferred fonts" "$deferred_sum" "$DEFERRED_AIM"; fi
if [ "$lazy_sum" -gt "$LAZY_AIM" ]; then past "the lazy fonts" "$lazy_sum" "$LAZY_AIM"; fi
if [ "$system_sum" -gt "$SYSTEM_AIM" ]; then past "the program worker" "$system_sum" "$SYSTEM_AIM"; fi
i=0
for f in ${programs[@]+"${programs[@]}"}; do
  if [ "${program_sizes[$i]}" -gt "$PROGRAMS_AIM" ]; then
    past "the program $f" "${program_sizes[$i]}" "$PROGRAMS_AIM"
  fi
  i=$((i + 1))
done
for f in ${stray[@]+"${stray[@]}"}; do
  echo "UNMEASURED: $f is in no group (dist/ is boot, dist/fonts/deferred/ deferred, dist/fonts/ lazy, dist/cpu/ system, dist/bin/ programs, dist/licenses/ uncounted)"
done

if [ "$over" = 0 ]; then echo "budget: every size within its aim"; else echo "budget: $over past their aims (gauges, not gates)"; fi
exit 0

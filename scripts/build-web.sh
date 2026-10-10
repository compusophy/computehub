#!/usr/bin/env bash
# Builds the web bundle into dist/: the os crate as wasm (the boot font is
# inside it), wasm-bindgen's glue, wasm-opt when it is installed and helps,
# web/index.html, the deferred fonts in dist/fonts/deferred/, the lazy fonts
# in dist/fonts/ and the font licenses in dist/licenses/; then the program
# worker (the cpu crate, its glue and web/worker.js) in dist/cpu/ and the
# programs (the terminal, sh, agent, toolbox, studio, assistant and system crates, for wasm32-wasip1)
# in dist/bin/.
# scripts/budget.sh measures the result;
# `cargo run -p serve --release -- dist 8080` serves it.
set -euo pipefail
cd "$(dirname "$0")/.."

# The wasm-bindgen CLI must be exactly the version the crate is locked to:
# the glue it writes and the module the crate emits must agree byte for byte.
locked=$(awk -F'"' '{ sub(/\r$/, "") } found && /^version = "/ { print $2; exit } /^name = "wasm-bindgen"$/ { found = 1 }' Cargo.lock)
if [ -z "$locked" ]; then
  echo "ERROR: Cargo.lock has no wasm-bindgen entry" >&2
  exit 1
fi
if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "ERROR: the wasm-bindgen CLI is not installed; run: cargo install wasm-bindgen-cli --version $locked --locked" >&2
  exit 1
fi
cli=$(wasm-bindgen --version | tr -d '\r' | awk '{ print $2 }')
if [ "$cli" != "$locked" ]; then
  echo "ERROR: wasm-bindgen CLI is $cli but Cargo.lock pins wasm-bindgen $locked" >&2
  echo "ERROR: install the matching CLI: cargo install wasm-bindgen-cli --version $locked --locked --force" >&2
  exit 1
fi

# Panic locations from #[track_caller] code in registry crates hold the
# crate's absolute source path, which starts at the cargo home and so names
# the build machine's account. Remap that prefix away (the scan below proves
# it worked). CARGO_ENCODED_RUSTFLAGS, not RUSTFLAGS: a home path may hold a
# space. Flags the caller already set are kept.
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
if command -v cygpath >/dev/null 2>&1; then
  cargo_home=$(cygpath -w "$cargo_home")
fi
flags=()
if [ -n "${CARGO_ENCODED_RUSTFLAGS+set}" ]; then
  if [ -n "$CARGO_ENCODED_RUSTFLAGS" ]; then
    IFS=$'\x1f' read -r -a flags <<<"$CARGO_ENCODED_RUSTFLAGS"
  fi
else
  read -r -a flags <<<"${RUSTFLAGS:-}"
fi
flags+=("--remap-path-prefix=$cargo_home=/cargo")
CARGO_ENCODED_RUSTFLAGS=$(IFS=$'\x1f'; printf '%s' "${flags[*]}")
export CARGO_ENCODED_RUSTFLAGS

# The build id reports and About show (option_env! in report and system): the
# short commit, with -dirty when the tree has uncommitted changes; dev outside git.
COMPUSOPHY_BUILD=$(git rev-parse --short HEAD 2>/dev/null || echo dev)
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
  COMPUSOPHY_BUILD="$COMPUSOPHY_BUILD-dirty"
fi
export COMPUSOPHY_BUILD

cargo build -p compusophy-os --release --target wasm32-unknown-unknown
# Ask cargo where it put the build: CARGO_TARGET_DIR, CARGO_BUILD_TARGET_DIR
# and build.target-dir in any config file all move it, and guessing wrong
# packages a stale os.wasm or none. JSON escapes Windows backslashes.
target_dir=$(cargo metadata --format-version 1 --no-deps --offline | tr -d '\r' \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | sed 's/\\\\/\\/g')
if [ -z "$target_dir" ]; then
  echo "ERROR: cargo metadata did not report a target directory" >&2
  exit 1
fi
wasm="$target_dir/wasm32-unknown-unknown/release/os.wasm"
if [ ! -f "$wasm" ]; then
  echo "ERROR: the build left no $wasm" >&2
  exit 1
fi

mkdir -p dist
# Nothing here deletes files: outputs have fixed names and are overwritten in
# place, and scripts/budget.sh names any file in dist/ that belongs to no
# group, so a stale leftover cannot ship unnoticed.
# The glue's flags: TextEncoder.encodeInto only (every engine that runs this
# page has it; the fallback path costs glue), and no producers section.
bindgen=(--target web --no-typescript --encode-into always --remove-producers-section)
wasm-bindgen "${bindgen[@]}" --out-dir dist --out-name os "$wasm"

# The features rustc's wasm32 output uses; wasm-opt rejects a module that
# uses a feature it was not told about. Some wasm-opt builds (the npm one)
# exit 0 even on a fatal error, so success also needs a non-empty output.
features=(
  --enable-bulk-memory
  --enable-nontrapping-float-to-int
  --enable-sign-ext
  --enable-mutable-globals
  --enable-reference-types
  --enable-multivalue
)
# --low-memory-unused lets wasm-opt fold constant offsets into loads and
# stores, which differs only when an address wraps past 4 GiB into the low
# 1 KiB. Rust never does that (pointer overflow is undefined), and the low
# 1 KiB is the far end of rustc's 1 MiB stack, below every static. About
# 1.5 KB gzipped on the os module.
opts=(--low-memory-unused)
# The pass pipelines tried: -Oz, run up to three times (each run finds more),
# and -Oz around flattening and re-relooping the control flow, then flow
# analysis (--gufa) and -Oz twice, once or two rounds of it (about 2 KB gzipped
# on the os module). gzip -9 is chaotic at the margin (a few bytes of code can
# move the os module's gzipped size by hundreds), so each is measured.
round="--flatten --rereloop --gufa -Oz"
pipelines=("-Oz" "-Oz -Oz" "-Oz -Oz -Oz" "-Oz --flatten --rereloop -Oz"
  "-Oz $round -Oz" "-Oz $round $round -Oz")
# Runs wasm-opt on the module $1 in place, if it is installed and helps:
# of the input and each pipeline's output, keeps whichever gzips smallest.
scratch="$target_dir/wasm-opt-scratch"
mkdir -p "$scratch"
optimize() {
  local f=$1 best gz p
  # Scratch copies live in target/ (overwritten each run, never deleted).
  local in="$scratch/in.wasm" out="$scratch/opt.wasm"
  if ! command -v wasm-opt >/dev/null 2>&1; then
    echo "WARNING: wasm-opt not found (install binaryen); $f stays unoptimized"
    return
  fi
  cp "$f" "$in"
  best=$(($(gzip -9 -c "$f" | wc -c)))
  for p in "${pipelines[@]}"; do
    # $p unquoted on purpose: a pipeline is several flags.
    # shellcheck disable=SC2086
    if wasm-opt $p "${opts[@]}" "${features[@]}" "$in" -o "$out" && [ -s "$out" ]; then
      # The budget is compressed bytes, and a smaller module can compress
      # worse (binaryen 112 on the os module does).
      gz=$(($(gzip -9 -c "$out" | wc -c)))
      if [ "$gz" -le "$best" ]; then
        best=$gz
        mv "$out" "$f"
      fi
    else
      echo "WARNING: wasm-opt $p failed on $f"
    fi
  done
}
optimize dist/os_bg.wasm

cp web/index.html dist/
# The fonts outside the wasm, none of them part of the boot download: the
# deferred ones the page fetches right after its first frame (Inter SemiBold
# and JetBrains Mono, from fonts/deferred/), the lazy symbol fonts a terminal
# fetches when it first opens (from fonts/, the URLs the shell asks for), and
# the fonts' licenses (OFL 1.1 wants its text with every copy).
# scripts/budget.sh measures each group on its own.
mkdir -p dist/fonts/deferred dist/licenses
cp assets/fonts/deferred/*.ttf dist/fonts/deferred/
cp assets/fonts/lazy/*.ttf dist/fonts/
cp assets/fonts/OFL-*.txt dist/licenses/

# The program worker, fetched only when a program first runs: its own cargo
# invocation, so its web-sys features never unify into os. web/worker.js is
# its one-line bootstrap.
cargo build -p compusophy-cpu --release --target wasm32-unknown-unknown
wasm-bindgen "${bindgen[@]}" --out-dir dist/cpu --out-name cpu "$target_dir/wasm32-unknown-unknown/release/cpu.wasm"
optimize dist/cpu/cpu_bg.wasm
cp web/worker.js dist/cpu/
# The programs, each fetched when it first runs: std binaries for WASI, the
# Terminal (terminal.wasm), its shell (sh.wasm) and coding agent (agent.wasm), the test programs (toolbox.wasm), Studio
# (studio.wasm), the Assistant (assistant.wasm), the system apps (system.wasm), the mesh's pool
# (pool.wasm), Fractal (fractal.wasm), the IQ verifier, a mesh worker (iq.wasm), and the
# device's test (gauge.wasm), in one cargo run.
rustup target list --installed 2>/dev/null | tr -d '\r' | grep -qx wasm32-wasip1 || { echo "ERROR: run: rustup target add wasm32-wasip1" >&2; exit 1; }
cargo build -p compusophy-terminal -p compusophy-sh -p compusophy-agent -p compusophy-toolbox -p compusophy-studio -p compusophy-assistant -p compusophy-system -p compusophy-pool -p compusophy-fractal -p compusophy-iq -p compusophy-gauge -p compusophy-clock --bins --release --target wasm32-wasip1
mkdir -p dist/bin
for p in terminal sh agent toolbox studio assistant system pool fractal iq gauge clock; do
  cp "$target_dir/wasm32-wasip1/release/$p.wasm" dist/bin/
  # Not the device's test: wasm-opt inlines its hashing into the one long call that loops over
  # it, and a browser never moves a running call to its optimized code, so all cores at once
  # measured the slow first code (16 MB/s against 1.7 GB/s).
  if [ "$p" != gauge ]; then optimize "dist/bin/$p.wasm"; fi
done

# The files a page that mounts the OS as a cartridge (os::cartridge) serves beside itself, one a
# line, relative to dist/: its programs run in workers, which a browser loads only from the
# page's own origin, so the host mirrors them (and serves them cross-origin isolated).
(cd dist && find . -type f ! -name files.txt | sed 's|^\./||' | LC_ALL=C sort) > dist/files.txt

# dist/ is what visitors download, so it gets scripts/caps.sh's privacy check
# too (same patterns; -a because the wasm and fonts are binary). A leaky
# bundle fails the build, and scripts/deploy.sh stops on a failed build.
# One home path belongs in the bundle: the OS's own guest home
# (vfs::Vfs::HOME), which names no account on the build machine; and one
# path only looks like one: the home crate's sources (crates/home/), which
# panic locations name. Each is an alternative of its own so that the
# leftmost, longest match reports it whole, and is then dropped. ([g] and
# [c] keep this line from matching caps.sh's own check.)
# Each hit is shown with the 32 bytes on either side (unprintable ones as
# '.'), so a path that leaked can be told from strings that only sit side
# by side in a wasm's data, and found in the source.
leaks=$(LC_ALL=C grep -r -a -o -b -E '[A-Za-z]:[/\\]+Users[/\\]|/home/[g]uest|[c]rates/home/|/home/[A-Za-z]|/Users/[A-Za-z]|[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z0-9.-]*[A-Za-z]{2,}' dist \
  | LC_ALL=C grep -a -v -E '^[^:]*:[0-9]+:(/home/[g]uest|[c]rates/home/)$' || true)
if [ -n "$leaks" ]; then
  echo "ERROR: dist/ holds a local path or email address; do not deploy it (file:byte: the bytes around it):" >&2
  printf '%s\n' "$leaks" | head -20 | while IFS=: read -r file at _; do
    from=$((at > 32 ? at - 32 : 0))
    around=$(tail -c +$((from + 1)) "$file" | head -c $((at - from + 64)) | LC_ALL=C tr -c '[:print:]' '.')
    printf '%s:%s: %s\n' "$file" "$at" "$around"
  done >&2
  exit 1
fi

printf '%-46s %10s %10s\n' "file" "raw" "gzip -9"
raw_total=0
gz_total=0
while IFS= read -r f; do
  raw=$(($(wc -c <"$f")))
  gz=$(($(gzip -9 -c "$f" | wc -c)))
  printf '%-46s %10d %10d\n' "$f" "$raw" "$gz"
  raw_total=$((raw_total + raw))
  gz_total=$((gz_total + gz))
done < <(find dist -type f | LC_ALL=C sort)
printf '%-46s %10d %10d\n' "total (all files)" "$raw_total" "$gz_total"

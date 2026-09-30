#!/usr/bin/env bash
# Builds the web bundle into dist/: the os crate as wasm, wasm-bindgen's
# glue, wasm-opt when it is installed, and web/index.html. scripts/budget.sh
# measures the result; `cargo run -p serve --release -- dist 8080` serves it.
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

rm -rf dist
wasm-bindgen --target web --no-typescript --out-dir dist --out-name os "$wasm"

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
if ! command -v wasm-opt >/dev/null 2>&1; then
  echo "WARNING: wasm-opt not found (install binaryen); dist/os_bg.wasm stays unoptimized"
elif wasm-opt -Oz "${features[@]}" dist/os_bg.wasm -o dist/os_bg.opt.wasm && [ -s dist/os_bg.opt.wasm ]; then
  mv dist/os_bg.opt.wasm dist/os_bg.wasm
else
  rm -f dist/os_bg.opt.wasm
  echo "WARNING: wasm-opt failed; dist/os_bg.wasm stays unoptimized"
fi

cp web/index.html dist/

# dist/ is what visitors download, so it gets scripts/caps.sh's privacy check
# too (same patterns; -a because the wasm is binary). A leaky bundle is
# deleted rather than left where a deploy could pick it up.
leaks=$(LC_ALL=C grep -a -o -E '[A-Za-z]:[/\\]+Users[/\\]|/home/[A-Za-z]|/Users/[A-Za-z]|[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z0-9.-]*[A-Za-z]{2,}' dist/* || true)
if [ -n "$leaks" ]; then
  echo "ERROR: dist/ holds a local path or email address; deleted it:" >&2
  printf '%s\n' "$leaks" | cut -c1-160 >&2
  rm -rf dist
  exit 1
fi

printf '%-24s %10s %10s\n' "file" "raw" "gzip -9"
raw_total=0
gz_total=0
for f in dist/*; do
  raw=$(($(wc -c <"$f")))
  gz=$(($(gzip -9 -c "$f" | wc -c)))
  printf '%-24s %10d %10d\n' "$f" "$raw" "$gz"
  raw_total=$((raw_total + raw))
  gz_total=$((gz_total + gz))
done
printf '%-24s %10d %10d\n' "total" "$raw_total" "$gz_total"

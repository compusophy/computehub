#!/usr/bin/env bash
# Vercel's own build of each push to main (vercel.json's buildCommand, through
# Vercel for GitHub: no token anywhere, whoever pushed). The toolchain CI pins
# (Rust 1.96.0 with the wasm targets, the wasm-bindgen CLI Cargo.lock locks,
# binaryen 112), the web build (which fails on a leak, so nothing ships), its
# sizes, then .vercel/output, which Vercel serves as it is.
set -euo pipefail
cd "$(dirname "$0")/.."

curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.96.0
# shellcheck source=/dev/null
. "$HOME/.cargo/env"
rustup target add wasm32-unknown-unknown wasm32-wasip1

tools=$(mktemp -d)
locked=$(awk -F'"' '{ sub(/\r$/, "") } found && /^version = "/ { print $2; exit } /^name = "wasm-bindgen"$/ { found = 1 }' Cargo.lock)
bindgen="wasm-bindgen-$locked-x86_64-unknown-linux-musl"
curl -sSfL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$locked/$bindgen.tar.gz" | tar -xz -C "$tools"
curl -sSfL https://github.com/WebAssembly/binaryen/releases/download/version_112/binaryen-version_112-x86_64-linux.tar.gz | tar -xz -C "$tools"
export PATH="$tools/$bindgen:$tools/binaryen-version_112/bin:$PATH"

bash scripts/build-web.sh
bash scripts/budget.sh
bash scripts/output.sh

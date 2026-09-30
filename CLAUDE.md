# CLAUDE.md — computehub / compusophyOS

Read this first; it is the operating map. `DESIGN.md` is the full design.

## What this is

compusophyOS: a tiling desktop OS in one browser canvas, Rust → wasm, built as
a platform anyone can write apps for. Later: the lobby of a shared world and a
node in computehub (pooled compute across devices). Author handle: compusophy.

## Constitution (CI-enforced by `scripts/caps.sh` where possible)

1. **Rust only.** No hand-written JS beyond a two-line bootstrap.
2. **Zero external dependencies.** Only `compusophy-*` workspace siblings.
   Exceptions: web crates (`platform`, `os`, from phase 1) may take
   wasm-bindgen, js-sys, web-sys; server crates under `api/` (the AI Gateway
   proxy, later) may take the Vercel Rust runtime and an HTTP client.
   Build-time tools never ship.
3. **Caps:** ≤2,000 lines of Rust per crate (tests count), ≤25,000 total,
   this file ≤8,000 chars. At a cap: split, shrink, or delete. Never raise it.
4. **Deterministic crates** (`wm`, later `kernel`): no floats, no
   HashMap/HashSet, no clocks, no randomness. State must replay bit-for-bit
   and hash identically.
5. **wasm32 always green:** `cargo check --workspace --target wasm32-unknown-unknown`.
6. **Budgets:** whole OS ≤150 KB compressed (`scripts/budget.sh`), idle CPU
   zero, first frame ≤100 ms after the wasm arrives.
7. **Every failure is coded and spanned** in the language crates; never a
   wrong-but-clean result.
8. **Designed for computehub now:** determinism, fuel + receipts, messages
   that could cross a network, content addressing, capabilities as handles.

## Map

```
crates/
  fuel/            fuel + byte budgets            (fork of fuellite)
  cap/             capability tables as data      (fork of caplite)
  lang/            diag + lex + parse kit         (fork of diaglite+lexlite+parselite)
  wasmgen/         wasm module builder            (fork of modlite)
  applang-syntax/  applang lexer/parser/checker   (split from applite)
  applang/         applang runtime (tier 0 apps)  (split from applite)
  wm/              tiling window manager, deterministic
scripts/caps.sh    caps: lines, deps, determinism, per-crate LICENSE, privacy
scripts/budget.sh  the size budget
```

Forks come from litelite 0.2.0, commit `4f5e056` (2026-07-20). Package names
are `compusophy-<x>`; each crate's `[lib] name` is the short name code uses
(`fuel::Fuel`, `lang::Diag`, `wm::Wm`).

## Commands

```sh
cargo test --workspace
cargo check --workspace --target wasm32-unknown-unknown
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo +1.85 test --workspace   # the MSRV: rust-version in Cargo.toml
bash scripts/caps.sh
bash scripts/budget.sh
```

## Conventions

- Git: plain `git commit`; never pass user.name/user.email overrides.
- Authors field: `compusophy`. Never put an email address in any file.

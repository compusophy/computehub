# CLAUDE.md — computehub / compusophyOS

Read this first; it is the operating map. `DESIGN.md` is the full design.

## What this is

compusophyOS: a tiling desktop OS in one browser canvas, Rust → wasm, built as
a platform anyone can write apps for. Its terminal runs a real shell (and the
`claude` CLI) through the computehub node, a native program on your machine.
Later: the lobby of a shared world and a mesh of pooled compute across
devices. Author handle: compusophy.

## Constitution (CI-enforced by `scripts/caps.sh` where possible)

1. **Rust only.** No hand-written JS beyond a two-line bootstrap.
2. **Zero external dependencies.** Only `compusophy-*` workspace siblings.
   Exceptions: the web crates `platform` and `os` may take wasm-bindgen
   (pinned), js-sys, web-sys; server crates under `api/` (later) the Vercel
   Rust runtime and an HTTP client; `node/` is its own workspace with
   reviewed native deps (PTY, WebSocket). Build-time tools never ship.
3. **Caps:** ≤2,000 lines of Rust per crate (tests count; `node/` too),
   ≤25,000 total, this file ≤8,000 chars. At a cap: split, shrink, or
   delete. Never raise it.
4. **Deterministic crates** (`wm`, `vfs`, later `kernel`): no floats, no
   HashMap/HashSet, no clocks, no randomness. State must replay bit-for-bit
   and hash identically.
5. **wasm32 always green:** `cargo check --workspace --target wasm32-unknown-unknown`.
6. **Budgets** (`scripts/budget.sh`, gzip -9): boot ≤150 KB (top-level
   `dist/` files), deferred fonts ≤30 KB (`dist/fonts/deferred/`), lazy
   fonts ≤60 KB (the rest of `dist/fonts/`), licenses not counted;
   idle CPU zero; first frame ≤100 ms after the wasm arrives.
7. **Every failure is coded and spanned** in the language crates; never a
   wrong-but-clean result.
8. **Designed for computehub now:** determinism, fuel + receipts, messages
   that could cross a network, content addressing, capabilities as handles.
9. **DOM:** the canvas, plus one hidden `<textarea>` (platform) for IME,
   paste and phone keyboards. Nothing else.

## Map

```
crates/
  fuel/ cap/ lang/ wasmgen/   forks of litelite (budgets, cap tables, parse kit, wasm builder)
  applang-syntax/ applang/    tier 0 app language: front end, runtime
  wm/        tiling window manager, deterministic
  vfs/       in-memory filesystem, deterministic (/apps, /home, /tmp)
  font/      TrueType reader + glyph rasterizer (no font engine ships)
  gfx/       instanced-quad draw list, glyph atlas, the WebGL2 shaders
  text/      TextSystem: font slots, fallbacks, glyphs on the atlas
  ui/        immediate-mode widgets, theme, the App trait, Cx (re-exports text)
  vt/        VT/xterm escape parser        term/  terminal screen model
  guest/     the guest shell the Terminal runs without a node
  apps/      Terminal, Welcome, Launcher, About
  studio/    applang editor + AppHost (runs .app files)
  host/      wm + one app per window: events, requests, sockets, lazy fonts
  shell/     panel, chrome, bindings around host (no web deps)
  platform/  the browser boundary: canvas, WebGL2, input, textarea, ws, fetch
  os/        wasm entry: fonts, VFS, app registry, pairing, event glue
node/        computehub-node: loopback WebSocket → one PTY per connection
             (own workspace + lockfile; see node/README.md)
assets/fonts/  Inter Regular (boot, in the wasm); deferred/ Inter SemiBold +
               JetBrains Mono; lazy/ symbol fallbacks; OFL texts; README.md
tools/serve/   dev-only static server for dist/ (never shipped)
web/index.html the page: <canvas id="os"> + the one-line module bootstrap
scripts/       caps.sh, budget.sh, build-web.sh, deploy.sh (Vercel, prebuilt)
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
bash scripts/build-web.sh        # dist/; needs wasm-bindgen CLI = Cargo.lock's
bash scripts/budget.sh
cargo run -p serve --release -- dist 8080   # preview (.claude/launch.json "os")

# the node: its own workspace (CI job "node")
cd node && cargo test && cargo clippy --all-targets -- -D warnings
cd node && cargo run --release -- --url http://localhost:8080
#   prints a pairing link (#node=<port>&token=<hex>); open it, and the OS
#   reads and clears it. Start it from a plain terminal, not Claude Code.
```

Fonts load in three groups, each with its budget: **boot** (Inter Regular,
`include_bytes!` in `os`; the first frame needs only it), **deferred** (`os`
fetches `fonts/deferred/*` right after the first frame and calls
`TextSystem::set_font`; until then bold draws as Regular and mono cells stay
empty), **lazy** (the shell fetches `fonts/symbols-*.ttf` when a terminal
first opens). `build-web.sh` copies `assets/fonts/deferred/*.ttf` to
`dist/fonts/deferred/`, `assets/fonts/lazy/*.ttf` to `dist/fonts/` and
`assets/fonts/OFL-*.txt` to `dist/licenses/`. Regenerating the subsets:
`assets/fonts/README.md`.

## Conventions

- Git: plain `git commit`; never pass user.name/user.email overrides.
- Authors field: `compusophy`. Never put an email address in any file.
- No absolute home-directory paths in committed files or `dist/` (caps.sh
  and build-web.sh check; the VFS's own guest home is allowed in the wasm).

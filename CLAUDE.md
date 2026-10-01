# CLAUDE.md — computehub / compusophyOS

Read this first; it is the operating map. `DESIGN.md` holds the vision, the
architecture as built and what comes next (R2 kernel, R3 AI, R4 modules and
shared compute).

## What this is

compusophyOS: a computer in one browser tab. Rust → wasm, one canvas, a
desktop anyone can write apps for. It runs entirely in the tab, serverless:
no native helper, no shell tunnel. Floating windows as on Windows and
Pop!_OS COSMIC; futuristic, ultra minimal, fast. Its terminal runs the
built-in guest shell over the in-memory VFS. AI-native by design; later a
mesh of pooled compute across tabs and devices. Author handle: compusophy.

## Constitution (CI-enforced by `scripts/caps.sh` where possible)

1. **Rust only.** No hand-written JS beyond two one-line bootstraps
   (web/index.html, web/worker.js).
2. **Zero external dependencies.** Only `compusophy-*` workspace siblings.
   Exception: the web crates `platform`, `os` and `cpu` may take
   wasm-bindgen (pinned), js-sys, web-sys. Build-time tools never ship.
3. **Caps:** ≤2,000 lines of Rust per crate (tests count), ≤25,000 total,
   this file ≤8,000 chars. At a cap: split, shrink, or delete. Never raise
   it.
4. **Deterministic crates** (`wm`, `vfs`, `kernel`): no floats, no
   HashMap/HashSet, no clocks, no randomness. State must replay bit-for-bit
   and hash identically.
5. **wasm32 always green:** `cargo check --workspace --target wasm32-unknown-unknown`.
6. **Budgets** (`scripts/budget.sh`, gzip -9): boot ≤150 KB (top-level
   `dist/` files), deferred fonts ≤30 KB (`dist/fonts/deferred/`), lazy
   fonts ≤60 KB (the rest of `dist/fonts/`), system ≤40 KB (`dist/cpu/`),
   programs ≤64 KB (`dist/bin/`), licenses not counted;
   first frame ≤100 ms after the wasm arrives; idle draws zero frames
   (a frame only on input or while an animation runs).
7. **Every failure is coded and spanned** in the language crates; never a
   wrong-but-clean result. `#![forbid(unsafe_code)]` in every crate.
8. **Designed for computehub now:** determinism, fuel + receipts, messages
   that could cross a network, content addressing, capabilities as handles.
9. **DOM:** the canvas, plus one hidden `<textarea>` (platform) for IME,
   paste and phone keyboards. Nothing else.
10. **MSRV 1.85, edition 2024:** no let-chains.

## Map

```
crates/
  fuel/ lang/                 forks of litelite (budgets, parse kit)
  applang-syntax/ applang/    tier 0 app language: front end, runtime
  wm/        floating window manager: stacking, snapping, focus; deterministic
  vfs/       in-memory filesystem, deterministic (/apps, /home, /tmp)
  font/      TrueType reader + glyph rasterizer (no font engine ships)
  gfx/       instanced-quad draw list (fills, borders, shadows, glyphs,
             gradients, glows, grain), glyph atlas, the WebGL2 shaders
  text/      TextSystem: font slots, fallbacks, glyphs on the atlas
  ui/        immediate-mode widgets, themes (Midnight, Dawn, Mono), the App
             trait, Cx (re-exports text)
  vt/        VT/xterm escape parser        term/  terminal screen model
  guest/     the guest shell the Terminal runs
  apps/      Terminal, Welcome, Settings
  studio/    applang editor + AppHost (runs .app files)
  host/      wm + one app per window; motion, frame geometry, launcher search
  shell/     the desktop: top bar, dock, launcher, window chrome, keys (no web deps)
  platform/  the browser boundary: canvas, WebGL2, input, textarea, fetch,
             frames on demand, cursor, localStorage, program workers
  os/        wasm entry: fonts, VFS, app registry, theme storage, event glue
  kernel/    R2 kernel, deterministic: wire protocol, process table and
             consoles (main), wasi (worker half), snap (/home), module
  cpu/       the program worker (cdylib; dist/cpu/): loader, WASI imports, homed
  toolbox/   test programs, one wasm32-wasip1 multicall binary (dist/bin/)
assets/fonts/  Inter Regular (boot, in the wasm); deferred/ Inter SemiBold +
               JetBrains Mono; lazy/ symbol fallbacks; OFL texts; README.md
tools/serve/   dev-only static server for dist/ (never shipped)
web/index.html the page: <canvas id="os"> + the one-line module bootstrap
web/worker.js  the program worker's one-line bootstrap
scripts/       caps.sh, budget.sh, build-web.sh, deploy.sh (Vercel, prebuilt)
```

Forks come from litelite 0.2.0, commit `4f5e056` (2026-07-20). Package names
are `compusophy-<x>`; each crate's `[lib] name` is the short name code uses
(`fuel::Fuel`, `lang::Diag`, `wm::Wm`).

Event path: DOM → `platform::Event` → `os` → `shell::Input` → `host` →
`ui::AppEvent` → app; back out as `ui::Request` → `host::Effect` → `os` →
`platform::Ctl`. A frame: `shell::draw` fills one `gfx::DrawList`, which
`platform::Renderer` draws in one instanced call; `draw` returns whether an
animation runs, and `os` then asks for the next frame.

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
                                 # and `rustup target add wasm32-wasip1`
bash scripts/budget.sh
cargo run -p serve --release -- dist 8080   # preview (.claude/launch.json "os");
                                            # --plain drops COOP/COEP/CORP
```

Add `?debug` to the page URL to get a `performance.mark("frame")` per frame:
an idle desktop must add none.

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

- Every color comes from the `ui::Theme` the frame is drawn in; no widget
  or chrome draws from a color constant. Rects, strokes and baselines land
  on device pixels.
- The boot budget is nearly spent: measure (`build-web.sh`, `budget.sh`)
  after any change that ships. Avoid core's Unicode tables
  (`char::to_lowercase` and friends; see `host::search::lower`) and float
  formatting in shipped code.
- Git: plain `git commit`; never pass user.name/user.email overrides.
- Authors field: `compusophy`. Never put an email address in any file.
- No absolute home-directory paths in committed files or `dist/` (caps.sh
  and build-web.sh check; the VFS's own guest home is allowed in the wasm).

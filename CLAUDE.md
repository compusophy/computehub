# CLAUDE.md — computehub / compusophyOS

Read this first: the operating map. `DESIGN.md`: the vision, the architecture
as built, what comes next.

## What this is

compusophyOS: a computer in one browser tab. Rust → wasm, one canvas, a
desktop anyone can write apps for. It runs in the tab (no native helper, no
shell tunnel; the only server code is `api/`). Floating windows as on
Windows and Pop!_OS COSMIC; futuristic, ultra minimal, fast. AI-native (free
AI for all); later pooled compute across tabs and devices. Author handle: compusophy.

## Constitution (CI-enforced by `scripts/caps.sh` where possible)

1. **Rust only.** No hand-written JS beyond two one-line bootstraps
   (web/index.html, web/worker.js) and `api/*.mjs`, the server functions
   (`node:` modules only): what cannot live in a tab, the free AI's
   credentials (`/api/ai`) and the feedback inbox (`/api/feedback`).
2. **Zero external dependencies.** Only `compusophy-*` workspace siblings.
   Exception: the web crates `platform`, `os` and `cpu` may take
   wasm-bindgen (pinned), js-sys, web-sys. Build-time tools never ship.
3. **Caps measure real costs** (speed is rule 6). A module (crate,
   program, `api/*.mjs` file) ≤2,000 lines, tests (`tests.rs`, `tests/`)
   ≤1,000: one reader holds it whole. The OS (`crates/`, `tools/`) ≤25,000
   + 12,500 in all: everything stands on it. A program depends only on
   `programs/` and `uiwire`, `icons`, `vfs`. Growth is new modules, not
   bigger ones; this file ≤8,000 chars. At a cap: split, shrink, or
   delete. Never raise one.
4. **Deterministic crates** (`wm`, `vfs`, `kernel`, `wasi`): no floats, no
   HashMap/HashSet, no clocks, no randomness. State must replay bit-for-bit
   and hash identically.
5. **wasm32 always green:** `cargo check --workspace --target wasm32-unknown-unknown`.
6. **Budgets** (`scripts/budget.sh`, gzip -9): boot ≤192 KB (top-level
   `dist/` files), deferred fonts ≤30 KB (`dist/fonts/deferred/`), lazy
   fonts ≤60 KB (the rest of `dist/fonts/`), system ≤40 KB (`dist/cpu/`),
   programs ≤96 KB each (`dist/bin/`), licenses not counted; first frame
   ≤100 ms after the wasm arrives; idle draws zero frames (only input or
   an animation draws; one opt-out exception: the living grain, 8/s).
7. **Every failure is coded and spanned** in the language crates; never a
   wrong-but-clean result. `#![forbid(unsafe_code)]` in every crate.
8. **Designed for computehub now:** determinism, fuel + receipts, messages
   that could cross a network, content addressing, capabilities as handles.
9. **DOM:** the canvas, plus one hidden `<textarea>` (platform) for IME,
   paste and phone keyboards. Nothing else.
10. **MSRV 1.85, edition 2024:** no let-chains.

## Map

```
crates/      the OS (boot, kernel, worker); speaks to programs only by uiwire
  wm/        floating window manager: stacking, snapping, focus; deterministic
  vfs/       in-memory filesystem, deterministic (/apps, /home, /tmp)
  font/      TrueType reader + glyph rasterizer (no font engine ships)
  gfx/       instanced-quad draw list (fills, borders, shadows, glyphs,
             gradients, glows, grain), glyph atlas, the WebGL2 shaders
  text/      TextSystem: font slots, fallbacks, glyphs on the atlas
  icons/     the mark and desktop glyphs as vector outlines (`ui::icon`)
  ui/        immediate-mode widgets, themes (Midnight, Dawn, Mono), the App
             trait, Cx, the Code editor (re-exports text)
  vt/        VT/xterm escape parser        term/  terminal screen model
  guest/     the guest shell the Terminal runs
  apps/      Settings, Terminal (built in)
  uiwire/    remote UI protocol: GUI programs send widget trees, get events
  uiview/    draws them with ui; holds edited text
  host/      wm + one app per window; agent; grabs, squeeze; motion, frames
  home/      top bar, home grid (every app), AI button + dock, menus, touch
  shell/     the desktop: window chrome, keys, overlay; wires host + home (no web deps)
  platform/  the browser boundary: canvas, WebGL2, input, textarea, fetch,
             frames on demand, localStorage, workers, beacon
  os/        wasm entry: fonts, VFS, registry, prefs, event glue, Remote (a
             program's window), ai, report (telemetry), home (/home kept)
  kernel/    deterministic: wire protocol, process table, consoles
             and file server (main), snap (/home), module
  wasi/      the kernel's worker half: WASI preview 1 Proc, fds, /dev
  cpu/       the program worker (cdylib; dist/cpu/): loader, WASI imports, homed
programs/    wasm32-wasip1 programs (dist/bin/), the app language they share
  fuel/ lang/                 forks of litelite (budgets, parse kit)
  applang-syntax/ applang/    tier 0 app language: front end, runtime
  studio/    Studio: make apps by describing them; runs `.app` files
  assistant/ the Assistant: the overlay AI using the desktop
  system/    About, Feedback, Files, Welcome (one multicall program)
  toolbox/   test programs (one multicall binary)
assets/fonts/  the fonts (see Fonts below)
api/           server functions (Vercel, Node): ai.mjs, feedback.mjs
tools/serve/   dev-only static server for dist/ (never shipped); mocks /api/*
web/index.html the page: <canvas id="os"> + the one-line module bootstrap
web/worker.js  the program worker's one-line bootstrap
scripts/       caps.sh, budget.sh, build-web.sh, deploy.sh (`prod`: production)
```

Forks: litelite 0.2.0, `4f5e056`. Packages are `compusophy-<x>`; each
`[lib] name` is the short one code uses (`wm::Wm`).

Event path: DOM → `platform::Event` → `os` → `shell::Input` → `host` →
`ui::AppEvent` → app; back out as `ui::Request` → `host::Effect` → `os` →
`platform::Ctl`. A frame: `shell::draw` fills one `gfx::DrawList`, drawn in
one instanced call; while an animation runs `os` asks for the next.

## Commands

```sh
cargo test --workspace
cargo check --workspace --target wasm32-unknown-unknown
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo +1.85 test --workspace   # the MSRV: rust-version in Cargo.toml
bash scripts/caps.sh
bash scripts/build-web.sh   # dist/; wasm-bindgen CLI = Cargo.lock's, wasm32-wasip1
bash scripts/budget.sh
cargo run -p serve --release -- dist 8080   # preview; --plain: no COOP/COEP
```

`?debug` in the URL marks each frame (`performance.mark("frame")`); idle
adds none but the grain's.

Fonts, each group with its budget: **boot** (Inter Regular, in `os`),
**deferred** (`fonts/deferred/*`, after the first frame; until then bold is
Regular, mono cells empty), **lazy** (`fonts/symbols-*.ttf`, when a terminal
first opens). Subsets and OFL texts: `assets/fonts/README.md`.

## Safety (the owner runs unattended; never trigger an approval prompt)

- Never delete with `rm -r`/`rm -rf`, `find -delete`, `Remove-Item -Recurse`
  or `del /s`, and never `rm` a path built from a variable, a wildcard or an
  absolute path. Leave scratch files where they are; overwrite outputs in
  place. Only `scripts/deploy.sh` clears a folder, behind a fixed-path guard.
- No `git reset --hard`, `git clean`, `git checkout -- <file>` or
  history rewriting. If a command would need the owner's approval, find
  another way or stop and report.

## Conventions

- Every color comes from the `ui::Theme` the frame is drawn in; no widget
  or chrome draws from a color constant. Rects, strokes and baselines land
  on device pixels.
- Measure the boot budget (`build-web.sh`, `budget.sh`) after any change
  that ships. Avoid core's Unicode tables
  (`char::to_lowercase` and friends; see `host::upper`) and float
  formatting in shipped code.
- Git: plain `git commit`, no user.name/email overrides. Authors:
  `compusophy`. No email address in any file.
- No absolute home-directory paths in committed files or `dist/` (caps.sh
  and build-web.sh check; the VFS's own guest home is allowed in the wasm).

# CLAUDE.md — computehub / compusophyOS

The operating map. `DESIGN.md`: the vision, the architecture as built,
what comes next.

## What this is

compusophyOS: a computer in one browser tab, for serious technical people
(devs, hackers, creators), never the mainstream. Rust → wasm, one canvas,
floating windows; futuristic, ultra minimal, fast; apps in applang; the
only server code is `api/`. Its heart is the mesh: models and devices as
one mind. Two models hand in hand is the base case (a local fine-tune
supporting the cloud model: repair, checks, selection); next, N models
self-organizing (division of labor, the right-sized model per subtask)
across desktop, laptop, phone and a public pool anyone joins, whose crowd
compute also trains models. Score the combined system: success per unit of
cost. Bar: far more useful than today's agents; real users, then paying
ones. One rule, the fractal: everything composes at any scale, never in
an iframe. An app hosts apps unmodified (each believes its rect is the
screen, as localharness's cartridges do), and the OS itself embeds as the
computer inside other worlds (secretspace's desk sim) and inside itself.
Author handle: compusophy.

compusophy's latest words win over this file: when they change direction,
update it in the same turn. Finished, verified work ships (merge, push,
prod) without asking. Never stop work over a size. The night runs from
"going to bed" until "I'm up": no clocks.

## Constitution (`scripts/caps.sh` holds the gates, gauges the sizes)

1. **Rust only.** No hand-written JS beyond two one-line bootstraps
   (web/index.html, web/worker.js) and `api/*.mjs`, the server functions
   (`node:` modules only): what cannot live in a tab, the free AI's
   credentials and the feedback inbox. Python: `train/` only; Claude Code JS: `.claude/`.
2. **Zero external dependencies.** Only `compusophy-*` workspace siblings.
   Exception: the web crates `platform`, `os` and `cpu` may take
   wasm-bindgen (pinned), js-sys, web-sys. Build-time tools never ship.
3. **Sizes are gauges, never gates** (speed is rule 6): measured and
   reported every build, never a reason to stop work. Aims: a module
   (crate, program, `api/*.mjs`) ~2,000 lines, tests ~1,000, so one
   reader holds it whole; the OS (`crates/`, `tools/`) ~25,000 + 12,500;
   this file ~8,000 chars. Past an aim, a split is worth a thought. A
   gate, not a size: a program depends only on `programs/` and
   `uiwire`, `icons`, `vfs`.
4. **Deterministic crates** (`wm`, `vfs`, `kernel`, `wasi`): no floats, no
   HashMap/HashSet, no clocks, no randomness. State must replay bit-for-bit
   and hash identically.
5. **wasm32 always green:** the wasm32 `cargo check` below.
6. **Speed** (`scripts/budget.sh`, gzip -9, a gauge): aims boot ~224 KB
   (top-level `dist/`), deferred fonts ~30 KB, lazy fonts ~60 KB, system
   ~40 KB (`dist/cpu/`), each program ~256 KB (`dist/bin/`); what can
   load later should. First frame ≤100 ms after the wasm arrives; idle
   draws zero frames (only input or an animation draws; one opt-out
   exception: the living grain, 8/s).
7. **Every failure is coded and spanned** in the language crates; never a
   wrong-but-clean result. `#![forbid(unsafe_code)]` in every crate.
8. **Designed for computehub now:** determinism, fuel + receipts, messages
   that could cross a network, content addressing, capabilities as handles.
9. **DOM:** the canvas, plus one hidden `<textarea>` (platform) for IME,
   paste and phone keyboards. Nothing else.
10. **MSRV 1.85, edition 2024:** no let-chains.

## Map

```
crates/      the OS (boot, kernel, worker); talks WASI and uiwire
  wm/        floating window manager: stacking, snapping, focus
  vfs/       in-memory filesystem (/apps, /home, /tmp)
  font/      TrueType reader + glyph rasterizer
  gfx/       instanced-quad draw list, glyph atlas, WebGL2 shaders
  text/      TextSystem: font slots, fallbacks, glyphs on the atlas
  icons/     the mark, glyphs and made icons as vector outlines
  ui/        immediate-mode widgets, themes, App, Cx, Code editor
  apps/      a terminal's console: its shell, its keys
  uiwire/    remote UI protocol: programs send widget trees, get events
  uiview/    draws them with ui; edited text; canvas/: canvases
  host/      wm + one app per window; agent; grabs, squeeze; motion
  home/      top bar, home grid, dock, menus, touch
  logon/     the welcome: mark, boot record, sign-in, PIN
  profiles/  each profile's keys, face, name, PIN
  shell/     the desktop: window chrome, keys, overlay
  platform/  the browser boundary: canvas, WebGL2, input, textarea, fetch,
             frames on demand, storage, workers, WebRTC
  mesh/      relays links and workers to the pool program
  report/    telemetry: notes, reports, outbox, panic beacon
  os/        wasm entry: fonts, VFS, registry, prefs, events, Remote
             windows, ai, /home; the cartridge; Monitor: itself inside
  kernel/    wire protocol, processes, consoles, jobs, file
             server (main), snap (/home), module
  wasi/      the kernel's worker half: WASI preview 1, fds, /dev
  cpu/       the program worker (cdylib; dist/cpu/): loader, WASI imports
programs/    wasm32-wasip1 programs (dist/bin/), app language, dev crates
  fuel/ lang/ forks of litelite (budgets, parse kit)
  applang-lex/ -syntax/ applang/ app language: tokens, parse, run
  studio/    make apps by describing them; runs `.app` files
  coder/     Studio's agent: write, test, fix, keep best
  assistant/ the AI using the desktop; chats/ files/: its chats, files
  tiny/ lab/ a transformer; dev: its corpus, training, measures
  system/    About, Editor, Feedback, Files, Welcome, Settings
  activity/  resource monitor; pool/ fractal/ sha/: the mesh, demo, hash
  clock/     the fractal shown: a face, a grandfather clock holding one, a shop
  terminal/  the Terminal; vt/ term/: its parser, screen model
  sh/ agent/ shell: editor, commands, jobs; AI coder
  toolbox/   test programs
  evals/ makes/ iq/ teach/  dev: evals; IQ tasks; Opus the teacher
assets/fonts/  the fonts (see Fonts below)
api/           server functions (Vercel, Node): ai, feedback, signal
tools/serve/   dev-only server for dist/; mocks /api/*
tools/eval/    dev-only eval runner (the free AI)
train/         dev: fine-tuning on the 3090 (Python)
web/index.html the page: <canvas id="os"> + a one-line bootstrap
web/worker.js  the program worker's one-line bootstrap
scripts/       caps.sh, budget.sh, build-web.sh, deploy.sh (`prod`)
```

Forks: litelite 0.2.0, `4f5e056`. Packages are `compusophy-<x>`; code uses
the short `[lib] name` (`wm::Wm`).

Event path: DOM → `platform::Event` → `os` → `shell::Input` → `host` →
`ui::AppEvent` → app; back out as `ui::Request` → `host::Effect` → `os` →
`platform::Ctl`. A frame: `shell::draw` (before sign-in `logon`) fills one
`gfx::DrawList`, one instanced call.

## Commands

```sh
cargo test --workspace
cargo check --workspace --target wasm32-unknown-unknown
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo +1.85 test --workspace   # the MSRV (rust-version)
bash scripts/caps.sh
bash scripts/build-web.sh   # dist/ (wasm-bindgen CLI = Cargo.lock's)
bash scripts/budget.sh
cargo run -p serve --release -- dist 8080   # preview; --plain: no COOP/COEP
```

`?debug` marks each frame (`performance.mark`); idle, only the
grain's.

Fonts: **boot** (Inter Regular, in `os`), **deferred** (`fonts/deferred/*`,
after the first frame), **lazy** (`fonts/symbols-*.ttf`, when a terminal
opens); subsets and OFL texts: `assets/fonts/README.md`.

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
- Measure the boot size (`build-web.sh`, `budget.sh`) after any change
  that ships. Avoid in shipped code: core's Unicode tables
  (`char::to_lowercase` and friends; see `host::upper`), float formatting,
  panics that format (`&s[a..b]`: use `s.get`) and `f32::sin` (`icons::sin`).
- Git: plain `git commit`, no user.name/email overrides. Authors:
  `compusophy`. No email address in any file.
- No absolute home-directory paths in committed files or `dist/` (caps.sh
  and build-web.sh check; the VFS's guest home may be in the wasm).

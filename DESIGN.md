# compusophyOS — design

Status, 2026-09-30: R1, the desktop, is built. Everything runs in the
browser tab, serverless; windows float. Author: compusophy. `CLAUDE.md` is
the short operating map; this file is the why and the shape.

## Vision

- **A computer in your browser tab.** Open a URL and you have a desktop,
  files, a terminal, an editor and apps. Nothing is installed and nothing
  leaves the tab: the tab is the sandbox and the machine. No native helper,
  no shell tunnel, no server doing the work.
- **Fast, quiet, beautiful.** The first frame lands within 100 ms of the
  wasm arriving; an idle desktop draws nothing and costs no CPU. Floating
  windows as on Windows and Pop!_OS COSMIC, ultra minimal and exact: every
  edge on a device pixel, every color from the theme, every change eased.
- **AI-native.** An agent is a first-class user of the OS: its tools are the
  OS's own capabilities, its eyes the UI tree. Computer use without
  screenshots.
- **Composable all the way.** One unit, the cartridge (bounded, confined,
  metered, replayable), at every scale: event handler, app, desktop,
  device, household, mesh. The OS can run inside itself.
- **From small total languages to models that compute.** applang is total
  and fuel-bounded: its programs provably halt. Programs that check and run
  become training data; later, models trained on opcodes and lexicons
  rather than English, an LLM that compiles, abstracting functions into
  functions all the way down. That long arc is why determinism, fuel,
  receipts and content addressing are built in now.
- **Useful first.** Old things done better and faster, new things that were
  not possible, done together: compute shared across tabs, devices and
  people (computehub).

## Principles

The constitution. `scripts/caps.sh` and `scripts/budget.sh` enforce what
they can, in CI.

1. **Rust only.** JavaScript is wasm-bindgen's generated glue and a
   two-line bootstrap in `web/index.html`, nothing else.
2. **Zero external dependencies.** Only workspace siblings, except the web
   crates `platform` and `os`, which take wasm-bindgen (pinned), js-sys
   and web-sys. Build-time tools never ship.
3. **Small:** at most 2,000 lines of Rust per crate (tests count) and
   25,000 in total. At a cap: split, shrink or delete. Never raise it.
4. **Budgets** (gzip -9; a real host's brotli is smaller):

   | budget | cap |
   |---|---|
   | boot: every top-level file in `dist/` (page, glue, wasm with the boot font) | 180 KB |
   | deferred: `dist/fonts/deferred/`, fetched right after the first frame | 30 KB |
   | lazy: the rest of `dist/fonts/`, fetched when a terminal first opens | 60 KB |
   | licenses: `dist/licenses/`, never fetched by the page | not counted |
   | first frame after the wasm arrives | 100 ms |
   | idle | zero frames: one only on input or while an animation runs; the one opt-out exception, the living grain, 8 a second by timer |

5. **Deterministic core.** `wm` and `vfs` (and the kernel to come) use
   integer math, no clocks, no hash-ordered collections and no randomness:
   the same commands replay to the same state and the same canonical hash.
6. **One canvas.** The DOM is the `<canvas>` and one hidden `<textarea>`
   that `platform` owns for IME, dead keys, paste and phone keyboards.
   Everything visible is drawn on the canvas.
7. **Weak devices are the baseline.** WebGL2 only; WebGPU is optional
   (local models).
8. **Every failure is coded and spanned** in the language crates; never a
   wrong-but-clean result. `#![forbid(unsafe_code)]` everywhere.
9. **Designed for computehub now:** determinism, fuel and receipts,
   messages that could cross a network, content addressing, capabilities
   as handles.

## Architecture as built

```
DOM event → platform::Event → os → shell::Input → host → ui::AppEvent → app
app → ui::Request → host (Effect) → shell → os → platform::Ctl → browser
frame: os → shell::draw → gfx::DrawList → platform::Renderer: one draw call
```

| crate | role |
|---|---|
| `fuel`, `lang` | forks of litelite 0.2.0 (`4f5e056`): budgets; diagnostics, lexer and parser kit |
| `applang-syntax`, `applang` | the tier 0 app language: front end and checker; runtime |
| `wm` | floating window manager: integer geometry, stacking, snapping, focus; deterministic |
| `vfs` | in-memory filesystem (`/apps`, `/home`, `/tmp`); deterministic |
| `font` | TrueType reader and anti-aliased glyph rasterizer |
| `gfx` | draw lists as one instance buffer, the glyph atlas, the WebGL2 shaders |
| `text` | font slots and fallbacks, measuring, wrapping, glyphs on the atlas |
| `ui` | immediate-mode widgets, the themes, the `App` trait and `Cx` |
| `vt`, `term` | VT/xterm escape parser; terminal screen model |
| `guest` | the shell the Terminal runs over the VFS |
| `apps` | Terminal, Welcome, Settings |
| `studio` | the applang editor, and `AppHost`, which runs `.app` files |
| `host` | the wm plus one app per window, the home screen's apps; motion, frame geometry |
| `home` | the home grid in the person's order, the AI button and the dock's wings, menus, touch |
| `shell` | the desktop around `host`: bar, home screen, chrome, keys |
| `platform` | the browser boundary: canvas, WebGL2, input, textarea, fetch, storage, cursor |
| `os` | the wasm entry: fonts, VFS, the app registry, theme storage, event glue |
| `tools/serve` | dev-only static server for `dist/`, never shipped |

Package names are `compusophy-<x>`; each crate's `[lib] name` is the short
one. Forked crates keep their Apache-2.0 license and note their origin.

### Rendering

- One WebGL2 context and, normally, one instanced draw call per frame.
  Every instance is a quad: fills, borders, soft shadows, icons, glyphs,
  linear gradients, elliptical glows and film grain. Rounded corners and
  shadows come from signed distances in the fragment shader; smooth ramps
  are dithered, so there is no banding and there are no textures but the
  glyph atlas.
- Frames are on demand: an input that changes the screen asks for one; an
  animation asks for the next from inside each frame and stops asking when
  it ends. The timers are the minute tick behind the clock, the kernel's
  wake, and the living grain's: while the desktop is idle the backdrop's
  grain takes a new pattern 8 times a second, each frame asked for by a
  timer (never a frame loop). It is still under `prefers-reduced-motion`,
  on a hidden page, and when Settings → Appearance → Living grain is off.

### Text

- `font` parses TrueType (`glyf`) and rasterizes coverage at any size, so
  any pixel ratio and any font loaded at runtime work.
- Three built-in slots, Inter Regular (`Sans`), Inter SemiBold
  (`SansBold`) and JetBrains Mono (`Mono`), plus up to 8 fallbacks. Fonts
  load in three groups, each with its budget: **boot** (Inter Regular,
  subset, inside the wasm; the first frame needs only it), **deferred**
  (SemiBold and Mono, fetched right after the first frame; until then bold
  draws as Regular and mono cells stay empty, on the same grid) and
  **lazy** (symbol fallbacks for terminals, fetched when one first opens).
  Subsetting and licenses: `assets/fonts/README.md`.
- Glyphs are rasterized at `size * dpr`, cached in a 1024 x 1024 atlas and
  placed on whole device pixels; only changed rows are uploaded. A full
  atlas is cleared and the frame drawn again.

### The desktop

- Bottom to top: the wallpaper (the theme's base color, up to four soft
  glows and grain), the home screen's icons, the windows, the bottom strip,
  the top bar, carried icons, menus, tooltips.
- **Top bar** (44 px): the mark at the left opens Welcome; the date and
  time sit in the middle; Feedback (a bug) and Settings at the right.
- **Windows** float in a stack; focus is the top of it. A 40 px titlebar
  carries minimize, maximize and close at the right, as on Windows. Drag a
  titlebar to move (a maximized or snapped window comes back to its normal
  size under the pointer); double-click it to maximize or restore; drag an
  edge (6 px) or corner (14 px) to resize. Dropped at the left or right
  screen edge a window snaps to that half, at the top it maximizes, near
  two edges it takes that quarter. A new window opens 28 px right of and
  below the focused one (at the screen's corner when that would cross the
  right or bottom edge; centered when no window is visible), and 64 px of
  every window stays on screen. When the screen changes size, windows shrink
  and move the least to fit inside it. Every change is a `wm::Cmd`, so
  window state replays and hashes.
- **Home screen**: every app is an icon behind the windows (Studio,
  Assistant, Terminal, Files, Settings, Feedback, About, Welcome, then each
  `~/apps/*.app`, newest last), down the columns from the top left on a
  wide screen, in rows of four on a phone; there is no other list. A
  `.app` file's icon is its sigil, sacred geometry made from its name. A
  click opens; a mouse dragged 4 px carries an icon, and the others slide
  aside; a finger held 500 ms picks one up: moved 8 px it drags, lifted
  unmoved it opens the icon's menu. A mouse dragged on the bare desktop
  draws a box that selects the icons it touches; Enter opens them, dragging
  one carries them all. The order is kept (`home.order`).
- **AI button**: bottom center, where the early iPad's home button was, a
  round of glass with the ring and dot. It shows the Assistant (Alt+Space
  too); its menu asks the Assistant. Later it listens.
- **Dock**: two glass wings beside the AI button: the favorites to its left
  (none at first; "Add to dock" from any app's menu), the other running
  apps to its right, a dot under each running one and a tooltip on hover.
  A click opens, focuses or minimizes; windows minimize into their tile.
  The strip's place never changes, so neither does the work area.
- **Keys** (`mod` is Alt or Meta, without Ctrl):

  | keys | action |
  |---|---|
  | mod+Space, mod+A | show the Assistant |
  | mod+Enter | open a terminal |
  | mod+Q | close the focused window |
  | mod+Up | maximize or restore |
  | mod+Down | restore a maximized window, else minimize |
  | mod+Left, mod+Right | snap to that half |
  | mod+Backquote, mod+Shift+Backquote | focus the next, the previous window |

- **Motion**: one ease-out curve, CSS `cubic-bezier(0.2, 0.8, 0.2, 1)`.
  Windows open (fade and grow from 96%, 180 ms), close (140 ms), minimize
  and come back (220 ms), and glide when they maximize, restore or snap
  (200 ms); dock tiles lift (120 ms), icons slide aside (180 ms), themes
  crossfade (200 ms). Drags and resizes follow the pointer exactly.
- **Themes**: Midnight (the default: near-black `#07080C` with violet, cyan
  and magenta light), Dawn (warm paper with peach, lilac and sky light) and
  Mono (black, white and grays, no light). A theme is plain data: backdrop,
  surfaces, glass, text ramp, one accent and the terminal's 16 colors;
  nothing draws from a color constant. The choice is kept in
  `localStorage` under `compusophy.theme`.

### Apps

- An app implements `ui::App`: title, icon, `draw` into a `ui::Ui` over its
  content rect in the current theme, `event`, `wants_text_input`,
  `preferred_size`. The UI is immediate-mode; the pointer is routed by the
  hit regions of the last frame.
- Apps reach outside themselves only through the `ui::Cx` of an event: the
  VFS, the page clock, and requests (open or close a window, load the
  fallback fonts, switch the theme). `os` owns the registry: `apps::open`
  makes `welcome`, `terminal` and `settings`; `studio::open` makes
  `studio`, `studio:<path>` and any `*.app` path.
- **Terminal**: `vt` + `term` + a cell renderer, running the `guest` shell
  over the VFS (`ls`, `cd`, `cat`, `mkdir`, `mv`, `open`, `edit`, `run`,
  `theme`, ...). `term` already speaks xterm, keys and replies included, for
  the programs the kernel will run.
- **Studio** edits applang; `studio::AppHost` runs a `.app` in its own
  window. **Welcome** is the first screen; **Settings** picks the theme and
  lists what compusophyOS is made of.
- Two tiers:

  | | tier 0 (now) | tier 1 (R2) |
  |---|---|---|
  | language | applang | anything that compiles to wasm |
  | runs | main thread, interpreted | a wasm process in a worker |
  | safety | provably halts; faults roll back; bounded memory | fuel and a watchdog |
  | capabilities | none: its widgets are its world | handles from a syscall table |
  | written by | anyone, an LLM included (generate, verify, run) | developers |

### Input

- Keys arrive as key-downs by physical position (`KeyboardEvent.code`; by
  `key` when a key reports no code, as phone keyboards do), so bindings
  hold on every layout. A shortcut letter (Ctrl, Alt or Meta held) goes by
  meaning instead, so Ctrl+Z is Ctrl+Z on AZERTY and Dvorak; NumLock-off
  keypad keys go by what they name; AltGr types text, never Ctrl+Alt.
- Text never comes from key-downs: only from the hidden textarea, which
  `platform` focuses while the focused app wants text input. That one path
  covers typing, dead keys, IME composition, paste and phone keyboards, so
  `os` leaves the key-downs that type unprevented. On phones a tap that
  releases on the focused window brings the keyboard back.
- Browsers keep Ctrl+W, Ctrl+T and Ctrl+N in a tab; an installed app's
  window, or fullscreen with Keyboard Lock, gets them.

### Storage

Today the VFS lives in memory, with Studio's sample apps installed at boot;
only the theme persists. OPFS persistence comes with the kernel.

## What is next

- **R2, the kernel: a virtual computer in the tab.** wasm processes are the
  virtual CPU: each a module with fuel, memory limits and a capability
  table, run in a worker. WASI-style syscalls over the VFS (open, read,
  write, readdir, spawn), so existing programs compile to it. A TTY joins
  a process to the Terminal (`term` already encodes keys and answers
  queries). OPFS keeps the VFS and the desktop across reloads. The virtual
  GPU is the draw protocol: processes send display lists, never pixels,
  and the one instanced renderer draws them, so a program can draw from
  another device just as well.
- **R3, AI.** An agent app whose tools are the OS's capabilities (open,
  read and write files, run programs, press widgets) and whose eyes are the
  UI tree (windows, titles, widget hits and labels). Cloud AI is free for
  every visitor: `api/ai.mjs`, a thin same-origin function, forwards
  chat-completions to the Vercel AI Gateway (GLM 5.3) with the project's
  own OIDC identity, so no key ever reaches the browser; bring-your-own-key
  can come back later. Local models on WebGPU, downloaded on first use, cached in OPFS,
  never part of the boot budget. Every call returns a receipt: model,
  tokens, cost.
- **R4, the OS as a fabric.** Apps load as separate wasm modules (a hello
  world under 10 KB), so the boot stays small while the OS grows. The OS
  runs as an app inside itself: the strictest test of confinement.
  Compute is shared between tabs and devices as sandboxed wasm jobs over
  WebRTC: deterministic, fuel-metered, verified by hash. The fine-tuning
  pipeline grows from verified programs (generate, check, run, keep): first
  for applang, then for models that emit opcodes instead of English.

## Open questions

- **The Super key.** Browsers and operating systems take it (Windows opens
  Start; Win+arrows snap the browser). Keyboard Lock captures it in
  fullscreen; elsewhere Alt and Meta stand in.
- **Alt in a terminal.** The desktop's bindings take Alt+Space, Alt+Enter,
  Alt+Q, Alt+arrows and Alt+Backquote before a terminal sees them
  (readline's Alt+F still arrives). A way through for apps that want them
  waits on the Super key question.
- **The boot budget** is nearly spent (about 6 KB of headroom). Studio
  and applang are the largest optional part of the boot wasm; separately
  loaded modules (R4) are how the OS grows past it.

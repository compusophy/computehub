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

1. **Rust only.** JavaScript is wasm-bindgen's generated glue, the one-line
   bootstraps (`web/index.html`, `web/worker.js`) and the server functions
   (`api/*.mjs`, Node's own modules only): what cannot live in a tab, the
   free AI's credentials and the feedback inbox.
2. **Zero external dependencies.** Only workspace siblings, except the web
   crates `platform` and `os`, which take wasm-bindgen (pinned), js-sys
   and web-sys. Build-time tools never ship.
3. **Scale without bloat.** Every cap measures a real cost, never size for
   its own sake:
   - *speed* is bytes: what boots, and each program, which loads only when
     opened (the budgets below), so a thousand programs cost nothing until
     one is used;
   - *understanding* is module size: a crate, a program or a server
     function holds at most 2,000 lines (tests 1,000 more), small enough to
     read whole; the OS (`crates/`, `tools/`), which everything stands on,
     at most 25,000 (tests 12,500) in all;
   - *coupling* is the boundary: a program depends only on `programs/` and
     the OS's pure shared libraries (`uiwire`, `icons`, `vfs`), and reaches
     the rest of the OS only through WASI preview 1 (files, the console) and
     uiwire (its windows).

   So growth is new modules, never bigger ones: `programs/` has no total,
   and programs can live in repos of their own, a library anyone adds to.
   At a cap: split, shrink or delete. Never raise it.
4. **Budgets** (gzip -9; a real host's brotli is smaller):

   | budget | cap |
   |---|---|
   | boot: every top-level file in `dist/` (page, glue, wasm with the boot font) | 224 KB |
   | deferred: `dist/fonts/deferred/`, fetched right after the first frame | 30 KB |
   | lazy: the rest of `dist/fonts/`, fetched when a terminal first opens | 60 KB |
   | system: `dist/cpu/`, the program worker | 40 KB |
   | each program: `dist/bin/*.wasm`, fetched when it first runs, then cached | 256 KB |
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
| `terminal`, `vt`, `term` | the Terminal: an xterm screen, a wasip1 GUI program off the boot download; its escape parser and screen model |
| `sh` | the shell the Terminal runs: a wasip1 program on its console |
| `apps` | a terminal's console in the boot: its shell, and its window's keys, text and wheel as events |
| `system` | About, Editor, Feedback, Files, Welcome and Settings, and it serves Activity: one wasip1 GUI program (`dist/bin/system.wasm`), off the boot download |
| `activity` | Activity, the resource monitor: graphs of CPU, memory, frames and the AI over the last minute, storage, a table of what runs |
| `studio` | the applang editor, and `AppHost`, which runs `.app` files |
| `coder` | the coding agent Studio runs: write, check (compile, smoke on 3 seeds), fix by SEARCH/REPLACE edits, keep the best so far, stop by budget; sans-IO, replayable |
| `uiwire`, `uiview` | the remote UI protocol GUI programs speak; the desktop's half, which draws their trees |
| `host` | the wm plus one app per window, the home screen's apps, windows held by the pointer, the keyboard's squeeze; motion, frame geometry |
| `home` | the top bar, the home grid in the person's order (icons carried, the selection box), the bottom row (the person's dock at the left, the Assistant at the right), menus, touch |
| `shell` | the desktop around `host`: chrome, keys, the overlay; wires the home screen to the pointer |
| `platform` | the browser boundary: canvas, WebGL2, input, textarea, fetch, storage, cursor |
| `os` | the wasm entry: fonts, VFS, the app registry, theme storage, event glue |
| `tools/serve` | dev-only static server for `dist/`, never shipped |

Package names are `compusophy-<x>`; each crate's `[lib] name` is the short
one. Forked crates keep their Apache-2.0 license and note their origin.

### Rendering

- One WebGL2 context and, normally, one instanced draw call per frame.
  Every instance is a quad: fills, borders, soft shadows, icons, glyphs,
  linear gradients, elliptical glows, film grain and lines (segments with
  round ends, which canvases draw). Rounded corners and
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
  glows and grain), the home screen's icons, the windows, the bottom row,
  the top bar, carried icons, menus, tooltips.
- **Starting**: a new tab opens on the welcome, the boot's own first frame
  (`logon`; no program, no worker): compusophy's mark comes in once from
  the center out (618 ms, 365 round fills), the clock sits in the bar's
  place, and the name follows when its font lands. The column steps down
  from the mark by powers of φ: a 144 px mark (89, 55 on smaller screens),
  the name s/φ³ under it and as large, the circles (55 px, 1/φ apart) φ²
  that under the name, the whole at the golden section of the room, φ
  times as much space under it as over it. The mark is art; the
  record is truth: under them a hairline holds one segment per stage of
  the real start as the browser timed it (the page, compusophyOS's
  download, its start to the first frame, the deferred fonts), at its
  measured times, gaps kept, with a caption (`Loading fonts…`, then
  `Ready in 412 ms · 218 KB`); a tap opens a card of the stages, the
  start's own 100 ms budget among them. Nothing is simulated, and waiting
  draws no frames. Every visit, the first too (guest alone), shows the
  profiles as circles (people are round, apps rounded squares): each
  face a ring a pixel wide holding dots, guest's none, and each profile
  added the fewest no other has (one, then two...), set in Settings →
  Profile among ten; the ring in the accent on the one in focus (the one
  that signed in last), then Add, a ring around a plus; a tap, or the
  arrows and Enter, signs in. Holding a circle 500 ms (or a right-click)
  opens its menu: Rename, Set a PIN (or Change PIN, Remove PIN), Remove
  profile (confirmed, with its files' size; never the last). Escape always
  goes back a step. Signing in puts the profile's /home back and makes its
  desktop as the mark flies to the bar's and the welcome fades off it
  (220 ms). A reload of a signed-in tab (its `sessionStorage`) goes
  straight to its desktop, never a PIN's. **A PIN** (optional, 4 to 8
  digits, offered as a profile is added) is a curtain, not a lock, and the
  words say so: files are not encrypted. It is typed on the phone's number
  pad (one dot a digit; the last checks it), asked at every page load,
  checked by the browser's own PBKDF2-HMAC-SHA-256 (WebCrypto, 100,000
  iterations, a random salt), never sent or kept: the list keeps its
  record. A wrong one swings the dots and clears them. It guards signing
  in, renaming and changing it; removal never needs it, so a forgotten PIN
  means removing the profile with its files. **Sign out** (the
  desktop's menu) keeps /home at once, forgets the tab's profile and
  reloads; if the files could not be kept, a card offers Stay or Sign out
  anyway. Welcome no longer opens by itself.
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
  Assistant, Terminal, then each `~/apps/*.app`), or in a **folder**:
  System (Activity, Settings, Feedback, About, Welcome), Games and
  Productivity (the Editor, Files) at first; an app's menu moves it (Move
  to: a folder, or the home screen; `home::folders`, kept as `folders`). A
  folder that holds any is an icon of its first four apps' tiles, and opens
  as a panel over the dimmed screen, its apps four across: a click opens
  one, and anywhere else (or Escape) closes it. Each icon sits in a cell of
  a grid: down the columns from the top
  left on a wide screen, in rows of four on a phone; there is no other
  list. A `.app` file's icon is the one its header draws (`// icon:` under
  its first comment: line, loop, fill, ring, dot and arc on a 24 x 24 grid,
  which Studio's AI writes for every app it makes, drawn in the glyphs'
  weight and the theme's ink by `icons::made`, whole or not at all), else
  its sigil, sacred geometry made from its name; on its name's hue either
  way. A click opens; a mouse dragged 4 px carries an icon; a finger held
  500 ms picks one up: moved 8 px from there it drags, lifted unmoved it
  opens the icon's menu. A carried icon lands in any cell, as on a phone:
  on an empty one nothing else moves (the cell it left stays empty); over
  another icon, that one and those after it, up to the first empty cell,
  slide a cell on to make room (on a grid full to its end, back). A new app
  takes the first free cell. A finger held anywhere else opens the menu
  there, if there is one; where there is none (an app's widget) it is still
  a tap when it lifts. A mouse dragged on the bare desktop draws a box that
  selects the icons it touches; Enter opens them, dragging one carries them
  all, kept as they were around it, and an app that takes the focus ends
  the selection. A wide screen and a phone each keep their own arrangement
  (their grids differ in shape; one never arranged shows the apps packed in
  their order, new ones last); a cell off a screen made smaller (a phone's
  keyboard) is kept for when it grows back. Kept as `home.order`: `@2`, then
  `name:wide:narrow` per app, a cell `col.row` (before: the names alone, in
  order, read as that order packed).
- **Bottom row**: one row of 44 px tiles along the screen's bottom (62 px
  with its margins), always there, so the work area never changes with it.
  No shelf, no button: tiles on the wallpaper, as the home grid's are.
- **Assistant**: alone in the bottom-right corner, its own app tile (the
  sparkle on its hue, as on the home grid); its hit reaches out to the
  corner. It opens and hides the Assistant (Alt+Space too; its menu asks
  it), which is never a window: the overlay, a card above the tile, its
  right edge the tile's (a sheet on a phone, its edges the row's), that
  uses the desktop for the person. A finger's tap leaves the keyboard down
  until its field is tapped. A dot under the tile while the overlay shows
  (the accent's while it has the keys). While it works the overlay is a
  pill, one line of what it does and Stop, and the dot beats; each act
  flashes what it touched, and the person's own press, key or wheel stops
  it. A program that failed starts again at the next summon. Later it
  listens.
- **Dock**: the person's own, left-aligned from the bottom-left corner:
  nothing in it at first ("Add to dock" from any app's menu, or its icon
  dragged onto the row, which opens a gap under the pointer, the icon
  going back to its place; "Remove from dock" from its tile's), then the
  other running apps, so a phone can still switch windows, past a
  hairline; a dot under each running one (the accent's while it has the
  keys), a lift and a tooltip on hover. Kept tiles move as icons do: a
  mouse drags one past 4 px, a finger held on one picks it up (moved 8 px
  it drags; lifted unmoved, its menu); the others slide aside and the
  order is kept (`dock`). Tiles shrink evenly to fit beside the Assistant
  (8 apps get 34 px on a 411 px phone; never off the screen). A click
  opens, focuses or minimizes; windows minimize into their tile. The
  person's own apps (`~/apps/*.app`) also offer **Delete** in their menu,
  asked again in place ("Delete for good"): their windows close, the dock
  lets them go and the file goes, with /home as kept.
- **Keys** (`mod` is Alt or Meta, without Ctrl):

  | keys | action |
  |---|---|
  | mod+Space, mod+A | show or hide the Assistant |
  | mod+Enter | open a terminal |
  | mod+Q | close the focused window |
  | mod+Up | maximize or restore |
  | mod+Down | restore a maximized window, else minimize |
  | mod+Left, mod+Right | snap to that half |
  | mod+Backquote, mod+Shift+Backquote | focus the next, the previous window |

- **Motion**: one ease-out curve, CSS `cubic-bezier(0.2, 0.8, 0.2, 1)`.
  Windows open (fade and grow from 96%, 180 ms), close (140 ms), minimize
  and come back (220 ms), and glide when they maximize, restore or snap
  (200 ms); dock tiles lift (120 ms), icons and dock tiles slide aside
  (180 ms), themes crossfade (200 ms). Drags and resizes follow the pointer
  exactly.
- **Themes**: Midnight (near-black `#07080C` with violet, cyan and magenta
  light), Dawn (warm paper with peach, lilac and sky light) and Mono
  (black, white and grays, no light; the default, which a first visit and
  an unknown name get). A theme is plain data: backdrop,
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
  fallback fonts, switch the theme). `os` owns the registry: `remote::open`
  makes the GUI programs' windows: `terminal`, `about`, `editor`,
  `editor:<path>`, `feedback`, `files`, `files:<dir>`, `welcome`, `activity`
  and `settings` (the `system` program), `assistant`, `studio`,
  `studio:<path>` and any `*.app` path.
- **Terminal**: a program like any other (`terminal`: `vt` + `term`), off
  the boot download. Its window holds a console (`apps::Console`): the
  shell on the kernel's console the size the program asks for
  (`Request::Tty`), what it types going in (`Request::Input`), what the
  shell writes and its end coming back as events, and the window's keys,
  text and wheel too; the program draws its screen as a `Node::Screen` of
  8 x 17 px cells, which the desktop paints (`ui::screen`). It runs
  `/bin/sh` (the `sh` program: `ls`, `cd`,
  `cat`, `mkdir`, `mv`, `open`, `edit`, `run`, `theme`, ..., and programs
  joined by `|`, with `<` and `>`), which edits its line on a raw console
  and runs programs as the kernel's jobs on a cooked one. What only the
  desktop can do the shell asks in its own escape, `OSC 1729 ; verb ; arg`
  (`open` an app, switch the `theme`), as programs already ask a terminal
  for its title.
- **Studio** makes and edits applang apps: the `coder` loop asks the free
  AI, checks each reply and fixes it by edits, showing what moves (thinking,
  writing 48 lines, fixing line 43, testing); `studio::AppHost` runs a `.app`
  in its own window. Its prompt asks every app for its icon line (one card,
  and one in each example), which a change keeps.
  Made apps draw: `canvas W, H, scene();` is a picture
  of square units (y down, scaled to fit) that `scene` draws with rect,
  circle, ring, line, text and sprite in the theme's 12 colors, sent as
  uiwire's `Node::Canvas` (a display list, never pixels); only what a
  canvas calls draws, and it changes nothing, so a render stays pure. A
  canvas's handler sees the tap's `x` and `y`. Grids and canvases are
  boards: they take the room the window's other widgets leave, and with a
  handler they are pads. **Editor** writes plain text, a new note in `~/notes`;
  Files and the Terminal's `edit` open files in it (a `.app` in Studio).
  **Welcome** (a program, its mark revealed by the desktop's clock)
  is the first screen; **Settings** picks the profile's face, the theme,
  the AI model and what is reported, and resets the device.
- **Activity**, the resource monitor. Performance: graphs of the last
  minute, a point a second (uiwire's `Chart`), of CPU (the desktop's and the
  programs' share of a core), memory, frames a second and the AI's tokens a
  second (requests, failures, tokens and cost from the receipt `api/ai.mjs`
  ends each stream with, `: receipt in= out= microusd=`; an answer with none
  is counted, never guessed), and /home against the browser's ~5 MB (a
  `Meter`). Processes: a table (`Columns` over Entries whose detail is
  cells) of what runs, the desktop and each program by its command line,
  never a title it set, sorted by the column picked; a row's page graphs its
  CPU and can End it. The graphs move only when a sample comes, the seconds
  between at their average: at rest nothing draws. Its window alone may send
  `Request::Watch` and `Request::End` (it runs `bin/system.wasm` whatever
  `/bin/activity` says; the Assistant presses in it only after the person's
  yes, as in Feedback). On a phone it watches only while it has the focus.
  The desktop answers with `Event::Stats`
  (`uiwire::stat`): the kernel's table (deterministic) and the page's meters
  (frames by cause, its own time and memory, each worker's busy ms and memory
  from SAB words 10 to 13, read with no message), posted only when something
  loud changed, at most once a second. A still desktop samples, wakes and
  posts nothing, its grain living or not. Programs never write /bin (EROFS).
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
- A finger's press into an app waits to be a tap (a finger that travels
  scrolls the window instead), but on a pad (`ui::Sense::Pad`: a canvas or
  grid an app plays) it presses at once and drags, as a mouse does: a
  paddle follows the finger, and the window never scrolls from there.
- Browsers keep Ctrl+W, Ctrl+T and Ctrl+N in a tab; an installed app's
  window, or fullscreen with Keyboard Lock, gets them.

### Storage

The VFS lives in memory. /home is kept across reloads (`os::home`): a
`kernel::snap` snapshot of it (FNV-checked) in `localStorage`, put back at
boot, written at once after a change under /home, then at most once a
second, and when the page hides or goes away (`pagehide`). So made apps
(`~/apps/*.app`) stay; what a running app holds does not. A write the
browser refuses (full, blocked) is reported, said in Settings → Privacy and
tried again as the page hides; a snapshot that does not read back whole is
set aside and the desktop starts with a fresh /home. Tabs share the one
snapshot: once a tab keeps its /home, another that read it earlier keeps
nothing over it (Settings says its changes are not kept), and a reload shows
the newer files. Putting /home back runs at sign-in, after the welcome's
first frame, at about 15 ms a MiB of /home in Chrome (noted as `home <KB>
<ms>`). `localStorage` holds about 5 MB; IndexedDB or OPFS replace it when
homes grow past that. Preferences and the theme are kept there too.

Settings → Reset erases it all: once the person types `reset`, every
`compusophy.` key goes from `localStorage` and `sessionStorage` (each
profile's /home, preferences and PIN, the outbox) and the page reloads to the
welcome as a first visit. Only the person can: the host drops a reset from a
window the Assistant put input into, and the uiwire request is the OS's own
windows' alone.

**Profiles** (the `profiles` crate) are separate homes in one browser, one in
memory per page (programs see the root `/`, so two homes in one VFS would
read each other). Each keeps its files, theme, dock, home order, grain, AI
model, report consent and outbox under its own keys: profile 0's are the
keys from before profiles (`compusophy.<k>`), so nothing was moved, and
profile n's are `compusophy.<n>.<k>`. The device keeps the list
(`compusophy.profiles`: `CSPR 1 <next id>`, then `<id> <face> <pin or -> <name>`
a line; absent, the implied `guest`, written at the first change), the
profile that signed in last and whether a welcome said hello; the tab keeps
its session. Changes start from the list as stored, never from a copy, so
another tab's survive; ids are never reused, and a new profile skips any id
whose keys are still there. Removing a profile removes its keys, then its
line (never the last). A tab whose profile another tab removed keeps nothing
more (as when another tab kept its /home). A damaged list is set aside
(`.bad`) and reported, offering profile 0; a newer one is read-only. A
panic is reported unless the signed-in profile turned reports off (before
a sign-in, unless any listed profile did). The shell still runs as `guest`
in its home; names in it, and roots per profile, wait for R2.

## What is next

- **R2, the kernel: a virtual computer in the tab.** wasm processes are the
  virtual CPU: each a module with fuel, memory limits and a capability
  table, run in a worker. WASI-style syscalls over the VFS (open, read,
  write, readdir, spawn), so existing programs compile to it. A console
  joins processes to a Terminal, cooked or raw (`/dev/consctl`), and
  `/dev/job` starts programs joined by pipes. OPFS keeps the VFS and the
  desktop across reloads. The virtual
  GPU is the draw protocol: processes send display lists, never pixels,
  and the one instanced renderer draws them, so a program can draw from
  another device just as well.
- **R3, AI.** An agent whose tools are the OS's capabilities (open apps,
  press widgets, type, keys, windows; then files and programs) and whose eyes
  are the UI tree (windows, titles, widget hits, labels and marks). Its first
  phase is built: the Assistant is the overlay; the host draws each window
  again into a recording list to read it (`host::agent`), the model sees it as
  text with refs (`assistant::look`) and calls tools, and each call is an act
  done the way a person's pointer and keys go (uiwire `Act`, `Acted`). Cloud AI is free for
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
- **The boot budget** (224 KB since 2026-10-02) pays for what must draw the
  first frame. With the canvas, the welcome, profiles and the PIN it held
  229,101 of 229,376 bytes; Settings and the Terminal's shell then became
  programs (2026-10-03), the kernel gaining consoles, jobs and pipes:
  225,896, about 3.4 KB of headroom. The Terminal itself became one
  (2026-10-04, about 12 KB), so folders fit: 219,359. Everything else should be a program,
  fetched when it first runs, at the cost of needing an isolated page.

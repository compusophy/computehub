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
  and fuel-bounded: every run provably halts within its fuel. Programs that check and run
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
| `agent` | the Terminal's coding agent: a wasip1 program on its console that works on the person's files with the free AI and tools, and keeps lessons from the failures it gets past |
| `apps` | a terminal's console in the boot: its shell, and its window's keys, text and wheel as events |
| `system` | About, Editor, Feedback, Files, Welcome and Settings, and it serves Activity: one wasip1 GUI program (`dist/bin/system.wasm`), off the boot download |
| `activity` | Activity, the resource monitor: graphs of CPU, memory, frames and the AI over the last minute, storage, a table of what runs |
| `studio` | the applang editor, and `AppHost`, which runs `.app` files |
| `coder` | the coding agent Studio runs: write, check (compile, smoke on 3 seeds, the icon line as the desktop reads it), fix by SEARCH/REPLACE edits, keep the best so far, stop by budget; sans-IO, replayable |
| `evals`, `makes` | the evals: a suite's tasks run through the coder over a wire (the live free AI, or a recorded run replayed), each run's records and AI exchanges kept, runs compared with confidence intervals; Suite 1, Studio makes: apps described precisely, made, then driven headlessly and graded by what they show |
| `assistant`, `chats`, `files` | the Assistant, the overlay AI that uses the desktop: a wasip1 GUI program off the boot download; its chats: each one's transcript and memory, their file, their row on its card; its file tools, the person's files by paths from the home, clipped and coded |
| `uiwire`, `uiview` | the remote UI protocol GUI programs speak; the desktop's half, which draws their trees |
| `canvas` | a program's `Canvas` as the desktop draws it (its shapes and pixels in the theme, on device pixels, its texts on it kept inside it) and lists it for the AI |
| `host` | the wm plus one app per window, the home screen's apps, windows held by the pointer, the keyboard's squeeze; motion, frame geometry |
| `home` | the top bar, the home grid in the person's order (icons carried, the selection box), the bottom row (the person's dock at the left, the Assistant at the right), menus, touch |
| `shell` | the desktop around `host`: chrome, keys, the overlay; wires the home screen to the pointer |
| `platform` | the browser boundary: canvas, WebGL2, input, textarea, fetch, storage, cursor |
| `os` | the wasm entry: fonts, VFS, the app registry, theme storage, event glue |
| `report` | telemetry: notes of what the page saw, feedback and error reports to compusophy's inbox (`api/feedback.mjs`), the outbox that keeps each until it is taken, a panic's beacon |
| `tiny` | a decoder-only transformer in plain Rust, no dependencies: a byte-level BPE, forward and hand-written backward passes in f32, AdamW on threads, sampling with a KV cache, a weights file checked by its hash; seeded, the same bits on any number of threads; the n-gram baseline it must beat; a map over threads |
| `lab` | the model lab, a dev tool never shipped: the corpus of verified applang programs (`data/manifest.tsv`), decoding constrained by applang's lexer and parser, and the commands that train tiny and measure what it writes (`data/results.md`) |
| `tools/serve` | dev-only static server for `dist/`, never shipped |
| `tools/eval` | dev-only eval runner: the system's `curl` against the free AI, paced under its limits; never shipped |

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
  Appearance among ten; the ring in the accent on the one in focus (the one
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
- **Top bar** (44 px): the mark at the left is Show desktop (mod+D too):
  it minimizes every window that shows, each into its tile, and closes an
  open folder or menu (it answers above them too); pressed again with no
  window shown since, it brings those windows back in their stacking
  order, the top focused (once another shows, the next press minimizes
  again). The date and time sit in the middle; Feedback (a bug) and
  Settings at the right. A tooltip under each button names it (the
  mark's says what its press does next: Show desktop, or Bring windows
  back); pressed, a button's or tile's tooltip goes until the pointer
  leaves it and comes back. Welcome is in the System folder.
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
  as a panel over the dimmed screen (all but the top bar's mark, which
  still answers), its apps four across: a click opens one, and anywhere
  else (or Escape) closes it. Each icon sits in a cell of
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
  uses the desktop for the person. Its card keeps **chats** (`chats`), each
  with its own transcript and memory (its last 4 tasks), so a request
  carries only its own chat's past; 8 at most, a new one past that letting
  go the one left longest ago, never the one just left. One row over the
  prompt, as much as the card's width holds (a phone's too): the current
  chat's chip, lit, then the others' that fit (each named by its first
  prompt) and "N more" for the rest, New chat, and Compact once the memory
  passes 1 KiB, more than a note takes. A chip switches; the lit one, or
  "N more", lists them all, with Delete chat (asked again). Compact has the
  model, with no tools, condense the chat into a note of 600 bytes at most,
  which takes the memory's first place (the 3 latest tasks follow it) until
  the next folds it in; the pill and Stop meanwhile, and a failure leaves
  the memory. A task is remembered as it is kept, each text 1,000 bytes
  at most, so a chat's memory never outgrows a request. Kept in
  `~/.assistant/chats`, a line format read defensively, 256 KiB at most
  (one that does not read is set aside as `chats.bad`, one a newer OS kept
  is left as it is, and one that cannot be read or set aside is never
  written over; the card says which); the Assistant's file tools never
  reach that folder. A finger's tap leaves the keyboard down
  until its field is tapped. A dot under the tile while the overlay shows
  (the accent's while it has the keys). While it works the overlay is a
  pill: one line of what it does, and Stop at its right edge however the
  line changes (its press reaches across the pill's height); the dot
  beats, and each act flashes what it touched. While a question waits it
  is the card again, the question last, with Stop under it. A task steps
  aside for the window the person uses next (uiwire `Yield`, sent alone
  in a frame, so a desktop older than it drops that alone). Once it
  presses, types or keys into a canvas or grid (a game's Start), that
  window takes the keys at once, the pill staying with Stop, so the game
  is played as it starts, not lost behind the overlay. Answered in such a
  window, or in one it opened or raised (or that came up new with the
  keys after its act), still focused and shown, the overlay hides, that
  window with the keys (raised only while the overlay showed: never taken
  from what the person moved on to); the answer waits in the chat, the
  tile's dot the accent's until the overlay shows again, as after any
  task that ended while it hid. The card stays for a question, a failure
  (the last result one, an AI error, Stop), a task that changed a setting
  (a theme; in Settings any act but a tab's press), and, in a window only
  opened or raised, an answer the person asked for (`chats::asks`: a
  question mark or a question's word) or one that asks: there the answer
  is the news. Answers and questions show as plain text (a model's
  backticks and bold taken out), each receipt counting its steps ("1
  step"); the model is told its last reply asks nothing (ask_user does)
  and claims only what the latest screen shows (after a press into a board
  or a game, only what it did, never its live state, a length or a score,
  which moves on before the person reads it), and its screen header says
  a touch screen (the person's last press a finger's), so its hints name
  taps, never keys. It opens an app by the name the home screen shows, in
  any case, with or without `.app` (`host::Host::named`: a made app in
  `~/apps` too, where `snake` was once no app, E0914; with `.app`, only
  a made one, so `files.app` is never Files).
  Only Stop, or Escape while the overlay has the keys (which, at a question
  or before the pill shows, hides the card too), stops it: the
  person's own presses, keys and wheel go where they go beside its acts
  (one on the bare desktop leaves the pill in view), and an act on a
  window they closed fails as gone. A program that failed starts again at
  the next summon. Later it listens.
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
  | mod+D | show the desktop, or bring back what it minimized |
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
- **Themes**: Dusk (near-black `#07080C` with violet, cyan and magenta
  light), Dawn (warm paper with peach, lilac and sky light), Mono Dark
  (black, white and grays, no light; the default, which a first visit and
  an unknown name get, as does Mono, its name before Mono Light) and Mono
  Light (its opposite: white windows over a pale gray, black ink and
  grays, a black accent, no light; a soft shadow and a white top edge as
  on Dawn). Settings shows them in a row, the default first and the rest
  after it, around (Mono Light, Dusk, Dawn), the current one ringed
  in the accent and checked at any width (a check on a disc of the accent
  in its miniature's top right corner, clear of its name); narrower, two
  across, the dark ones over each other. A theme is plain
  data: backdrop, surfaces, glass, text ramp, one accent and the
  terminal's 16 colors; nothing draws from a color constant. The choice
  is kept in `localStorage` under `compusophy.theme`.

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
  `/bin/sh` (the `sh` program: `ls` (`-l` a line each, its kind and size:
  the files keep no dates), `cd`, `cat`, `mkdir`, `mv`, `open`, `edit`,
  `run`, `theme`, ..., `~` for the home, `#` comments, and programs such
  as the toolbox's `wc` and `rev`, which read the files they name or else
  their input, joined by `|`, with `<` and `>`; commands joined by `;`,
  `&&` and `||`; `*` and `?` matching the names in a directory; `sh -c
  line` and `sh file` run scripts, ending with the last line's status),
  which edits its line on a raw console and runs programs as the kernel's
  jobs on a cooked one. wasi-libc names a process's starting directory
  (preopen `.`) and `/` alike, so `sh` and the toolbox open an absolute
  path from there, up to `/` (`./../../notes` from the home): else `/notes`
  would be the home's.
  What only the desktop can do the shell asks in its own escape,
  `OSC 1729 ; verb ; arg` (`open` an app, switch the `theme`), as programs
  already ask a terminal for its title.
- **Agent** (`agent` in a terminal): a coding agent, as on other systems'
  command lines. Given a task (`agent make a pomodoro app`, or one a line at
  a time; `/help` lists its commands) it asks the free AI with eight tools:
  read, write and edit a file, list a folder, search under one, run a line
  in the OS's own shell (the `sh` library in-process, so `cd` stays and
  `open` opens apps; a job's stdout is caught in a file of the agent's own,
  made new in /tmp, so two agents never share one), check an applang app
  (`coder`'s compile and smoke test) and read applang's guide. Reads run
  freely; writes, edits and commands wait for the person's yes (`[y]es [n]o
  [a]lways`, or `-y` before the task: in its words, a `-y` is a word, and
  `/yes` toggles it). A line runs unasked only if the shell reads it
  (`sh::commands`) as one reading command alone (`ls`, `cat`, `cd`, ...:
  nothing joined by `;`, `&&`, `||` or `|`, no `>`, no /dev); one that may
  remove, move or overwrite (`rm`, `mv`, a `>` that does not append), or
  runs a program but /bin's `wc`, `rev` and `hello` by name (judged by what
  runs: `sh` or `agent` by any path or `#!wasm` alias, a program named by a
  pattern), asks each time, always or not, as does a write that replaces a
  file (always covers new files, appends and edits). No file tool, nor its
  shell, reads or writes a device (but /dev/null). The reply streams in as
  it comes, and each tool shows a line of what it does and one of how it
  went; what the model wrote shows with its controls in caret notation
  (`^[`), so none of it styles the screen or asks the desktop (`OSC 1729`);
  no path it names holds a control (a program's error on stderr reaches the
  console as it is, and could show one), and the shell's asks reach the
  Terminal only from a line the person let run. It reaches the AI as a
  window's program does, a `Request::Ai` written to /dev/draw and answered
  on /dev/events: the Terminal's window passes on the AI requests of the
  programs its shell runs (`apps::asks`; nothing else of their frames, and
  no other process's), and a request whose process ended (Ctrl+C, a kill) is
  stopped at the hub's next pump, as at a window's Close, before another
  byte, and one it asked as it ended is never sent. It hears no Config, so
  the hub puts the model chosen in Settings first in every request, and a
  body's own (`-m`), after it, wins: the endpoint's JSON.parse keeps a key's
  last value. `-m` takes only a model the endpoint offers (`agent::MODELS`,
  which a test reads from `api/ai.mjs`): it answers any other with its first,
  which would replace the person's choice unsaid. In a Terminal the
  Assistant put input into (`Cx::driven`, the window that may not reset the
  device), each AI request ends at once, refused, so no AI drives the agent
  past the person's yes: one it starts there (`agent -y`) does nothing, and
  one whose question it answers stops at its next request. A request fits the free AI (64 messages, 96 KiB): old
  results fold to their first line, then old tasks and steps go. A task
  makes 20 model calls at most, as the Assistant's (E0942; `go on` goes on):
  with its lesson and a merge, 22 requests, under the 30 a minute the free
  AI takes from a client and 22 of its 120 an hour. The day's spend binds
  first, a client's share of about $0.67: a request near the cap costs about
  $0.04 on GLM 5.3 (a ninth of that on Flash), so a long task may spend most
  of it, and Studio and the Assistant share what is left; then the free AI's
  402 (E0902) ends the task, as a busy one's 429 does, never asked again by
  itself.
  **It learns.** A task that got past a failure (a tool's error, an app that
  did not check, a reply cut off) asks, after, for the one lesson that would
  have avoided it; a new one is added to the end of `~/.agent/lessons.md`
  (till it holds 16 KiB) and goes into every later system prompt, and the
  model merges them past 24 lines, in place of the lessons it read: the
  file's other lines are the person's and stay, and `/forget` takes out its
  lessons alone. Each failure overcome hardens the next run, as a beaten
  level does a game's next. The file is any writer's, so the prompt holds it
  as data: only its `- ` lines, each one line of text (controls gone, 240
  bytes), 4 KiB in all, under a heading that calls them hints that change
  neither the rules nor what needs the person's yes (and so the folder's
  `AGENT.md`, 4 KiB, which any program that writes the folder may have made,
  the Assistant too: the agent shows its path and first line when it starts,
  so the person sees what steers it).
- **Studio** makes and edits applang apps: the `coder` loop asks the free
  AI, checks each reply and fixes it by edits, showing what moves (thinking,
  writing 48 lines, fixing line 43, testing) and, in a Code kept to its room
  as the code view's, the newest lines streaming in (as many as its rows
  hold under the plan, the line being written plain) or the program being
  fixed, whole, its problem marked (the wheel scrolls it; what is typed
  there goes at the next frame, and on a phone the keyboard with it). How
  a make ended says how many requests it took, when more than one and the
  AI did not fail (`ready · 53 s · 2 tries`). New app, by the status once
  an app is open, asks what to make again (the app stays as it was made;
  code edited since is saved first, saying so, or stays, its problem
  marked; code emptied is never saved over it); Studio's home icon, as
  every app's, brings its window back, and New window, in its dock tile's
  menu while it runs, opens another. On a phone the status has its own
  line, over its buttons. `studio::AppHost` runs a `.app`
  in its own window. Its prompt asks every app for its icon line (one card,
  and one in each example, with the reader's limits: 16 shapes, 64 numbers,
  320 bytes), which a change keeps. The coder reads that line as the
  desktop does (`icons::made`): one that would not draw, or one never read
  (under the code, or marked `//icon:` or `// Icon:`), is a problem it
  fixes by edits as it does a fault (E0931 to E0937, at the word), after
  the program runs clean; a best program whose icon still will not draw is
  installed all the same, its problem showing (runs, but its icon won't
  draw), but for a change of an app whose icon drew: that lands with the
  icon line it had. No icon line is no problem: the tile is the sigil (the
  recorded makes have none).
  A make that ends without an app that runs clean (but for a stop with
  nothing wrong showing, or the free AI busy or out of credit before any
  program came back), and the app's last fault in Studio's preview (kept
  under the app until another shows or the program changes), offer Send to
  compusophy by the status; an app in its own window offers none. The tap
  is the consent (the Assistant's press asks the person's yes itself, as
  in Feedback); each goes once (`Request::Feedback`, never the desktop's
  context): what was asked, how it ended (its code and problem, the make's
  own code for why it stopped, each request's kind, tokens, time and the
  problem it left) and the program, clipped to 8 KB, so compusophy sees
  what applang or the coder lacked; when applang can make nothing close,
  it is an idea, else a bug.
  Made apps draw: `canvas W, H, scene();` is a picture
  of square units (y down, scaled to fit) that `scene` draws with rect,
  circle, ring, line, text, sprite and pixels in the theme's 12 colors, sent
  as uiwire's `Node::Canvas` (a display list, never a bitmap); only what a
  canvas calls draws, and it changes nothing but its own lets, so a render
  stays pure. `pixels(cells, x, y, w, side)` draws a list of ints as
  squares `side` units wide, `w` a row, -1 none and 0 to 11 a color:
  boards, paintings, life. Pixels are whole rows, 64 x 64 at most and
  16,384 in a render, or a coded fault (E0224). Each row's run of one
  color is one fill on the desktop and one ink of the render's 4,096 (with
  each shape, sprite square and text character), so a plain 64 x 64 board
  fits where a busy one faults (E0222). The AI reads where pixels are and
  their size, and each board's squares, a row a line, as a grid's, while
  those listed stay within 1,024 in all (a board past what is left is its
  place and size alone). A text centers on its point, a character about
  half its size wide; one whose point is on the canvas (x from 0 to its
  width, y to its height) stays inside it, a quarter of its size in (near
  an edge it moves in, so a score at x 2 starts at the left edge and one at
  the width ends at the right), and the smoke test faults such a text
  wider than its canvas (E0225, each character reckoned 2/5 of its size);
  one whose point is off the canvas stays where it is. A canvas's handler
  sees the tap's `x` and `y`; a drag taps the units on its way as a drawn
  line has them (one a column or a row: a diagonal skips a unit whose
  corner it clips), one every 4 px or so (64 for a sample at most, so a
  sample up to 256 px away leaves no gap), and waits until the frames of
  all it sent (and of the Ticks before them) are answered and drawn, so a
  paint app paints a line with no gap where a steady drag goes. Where a
  drag goes meanwhile is held (`uiview::Play::held`, as a share of the
  board, which may move), and once those are drawn the window
  (`os::remote`) asks a frame and taps it then, after any Tick due, so a
  stroke quicker than a frame's round trip, or a stroke's end, is painted
  too, unless anything else went to the program first (a key, a click,
  another press): it reads what the person did in order. A handler runs
  for each unit a drag crosses, so the card asks that a tap steer, aim or
  paint, and that turning, dropping or firing be a button. Grids and
  canvases are boards: they take the room the window's other widgets
  leave, the most those took above the first board and around them since
  the window's size or its boards changed (`uiview::View::around`), the
  room left above drawn above it, so a board keeps its size and place as a
  button or label comes and goes, after each first shows; with a handler
  they are pads. A pad grid's squares are a target before the boards
  share the room (`uiview::TAP`, 24 px, or 44 in a narrow window, as its
  width holds them, a px less while the grids would take more than two
  thirds of the room); then every board takes the largest share of its
  largest that all of them fit in, never more than without the target.
  So a palette of 7 beside a swatch is no row of 8 px squares, a painting
  keeps a third of its share at least, and what fit still fits. In a
  row, a grid its even share leaves short of the target takes it, where
  what is left still holds every other grid's. Studio's prompt asks that
  whatever is to be seen (a game, a board, drawing, animation, a clock, a
  chart) be drawn on a canvas, never spelled out in labels and buttons
  (its first example is snake on pixels); that a game show the same
  widgets before it as after (Start, hidden while it plays) and say Game
  over on the canvas; that a paint app show the color it paints and its
  squares' edges faintly (gray) under the paint; and that a grid's
  handler read its `cell` as the square's index, never its color (a
  palette reads its list: `color = pal[cell]`).
  **Editor** writes plain text, a new note in `~/notes`;
  Files and the Terminal's `edit` open files in it (a `.app` in Studio).
  **Welcome** (a program, its mark revealed by the desktop's clock)
  is the first screen; **Settings** opens on Appearance (the theme, the
  grain, then the profile's face in a row of ten, each named by its dots
  for whoever reads the screen: `no dots`, `1 dot`...), then picks the AI
  model, and what is reported, with the device's reset below it. Renaming
  a profile is the welcome's (its menu) and signing out the desktop's
  menu's.
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
  `/bin/activity` says; the Assistant's presses and keys there ask the
  person's yes themselves, as in Feedback). On a phone it watches only
  while it has the focus.
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
- A program's window takes typing before its program has drawn: the keys
  and text typed there are held, 4 KiB at most (a key counts one byte)
  and nothing past that, then go as typed once the first frame is in:
  into the field it focuses (Enter in an Input is its Submit), else Enter,
  Escape and chords as the program's keys, text nowhere. Until then the
  window of a program that opens on a field wants text input, so the
  textarea and a phone's keyboard take the typing (and, as a field does,
  F5 and Ctrl+R); any other window wants none, so a tap that opens About
  raises no keyboard. Those windows are named in one list,
  `os::remote::TYPING`: Studio's (on a file too, but not a `.app` it
  runs), the Assistant's (its overlay as well), Editor's and Feedback's.
  A program that opens on a field adds its window's name there. So
  opening Studio or the Assistant and typing at once loses nothing. (The
  Terminal's console hears its keys from the start.)
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

Settings → Privacy ends with the reset, which erases it all: once the
person types `reset`, every `compusophy.` key goes from `localStorage` and
`sessionStorage` (each profile's /home, preferences and PIN, the outbox) and
the page reloads to the welcome as a first visit. Only the person can: the host drops a reset from a
window the Assistant put input into (whose Terminal's programs may not ask
the AI either: `Cx::driven`), and the uiwire request is the OS's own
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

### Mesh

compusophy, 2026-10-08: share compute, don't just delegate it. Tabs on several devices work on
the same job, a worker on every core of each; later, people join and lend theirs, and our own
model runs on top (the legion: fixed-size units of model and mesh nested in fixed ratios, growing
with the RAM lent, learning continuously). This is the foundation, as built.

- **Pairing** (`api/signal.mjs`): one tab shows a short code (Activity, Pool, "Show a code"),
  the other enters it. The two swap WebRTC descriptions through the function's own memory, kept
  three minutes and read once; a miss (another instance) is retried. Nothing else goes through
  the server.
- **The link** (`platform::link`): a WebRTC data channel, encrypted (DTLS), direct where the
  network allows (one public STUN server; no relay). Its certificate is made once per profile and
  kept in IndexedDB, so a tab's fingerprint survives reloads; the other side pins it (`known`
  in the Pool page). WebRTC exists only on a page's main thread, so this is boot code.
- **The relay** (`crates/mesh`): the boot's whole share, kept small. It does what only the page
  can (links, posts, workers, storage, the clock) and passes everything to the pool program as
  `uiwire::relay` frames on its console.
- **The pool** (`programs/pool`, `/bin/pool`), fetched when a tab first pairs or starts a job: the
  messages tabs send each other (`msg`), the shared chunk queue, work stealing and receipts
  (`pool`), pairing (`hub`). A job is a `/bin` program (every tab has the same `/bin`: a link
  never ships code) and its chunks, a line each. The tab that starts it holds the queue; every
  worker, here or on a linked tab, takes the next chunk when idle (a linked tab asks for its idle
  workers' worth and half again, to hide the link). Once the queue is empty, idle workers take
  back the chunk out longest elsewhere; a chunk whose worker or link is gone goes back to the
  front. An answer carries its fuel and the SHA-256 of its result; every eighth chunk another tab
  answered is replayed here and the hashes compared.
- **Workers**: one per core (`hardwareConcurrency`, at most 32), each a kernel process of the
  job's program with `work`, on a console of its own, owned by no window, with no files. The
  kernel runs up to 48 processes (8 before).
- **Liveness**: each tab says what it has (its Hello) with every ping, every 2 s, so one whose
  channel opened late still learns it; a link silent for 15 s is closed and its chunks requeued.
  When a job ends its tab tells each helper how many of its answers it used: the rest were taken
  back at the tail and answered elsewhere first.
- **The demo** (`programs/fractal`): a Mandelbrot picture of 144 tiles, each a chunk of about
  100 ms on a core, tinted by the device that rendered it; a tap zooms in. f64 math in wasm is
  exact IEEE, so every device answers a tile with the same bytes.
- **The dashboard**: Activity's Pool page. Pairing; the pool's devices (cores, RAM, storage the
  browser grants, GPU compute, linked since, share of the work), utilization and speed over the
  minute; each link's round trip and traffic both ways, and a measured throughput; the job's
  chunks, each device's, steals, requeues and checks. Each tab sends its stats on the link once a
  second.

First run across machines (2026-10-08, the dev server tunnelled to the laptop): this PC
(Chrome, Windows, 16 threads) and a laptop (Firefox, Linux, 8 threads) on one Wi-Fi network
linked directly in about 10 s; round trip 3 to 5 ms, measured 7 to 10 MB/s each way. 144 tiles
took 2.9 s against 4.1 s on this PC alone (75 and 69 tiles); every answer replayed here matched.

The first real job (2026-10-08): the IQ suite verified on the pool. `/bin/iq work` is the IQ
verifier built for wasm32-wasip1 (its worker mode in `main.rs`, so the verifier's hash, which
build.rs takes of the library and what it stands on, is the native one). Activity's "Verify the
IQ suite" asks for a job whose one chunk is `@suites/iq.jsonl 3`: the pool fetches that
same-origin file (the dev server serves `evals/suites/`) and makes a chunk of every three tasks.
The report, assembled as the native runs are compared (timings aside, sorted by task), hashes
the same as theirs: one tab in 165 s (16 workers; native 16 threads, about 183 s), this PC and
the laptop in 135 s (native, 131 s).

Boot cost: about 7.5 KB gzipped (the link and relay; measured with `budget.sh`). Next: applang
checks from the IQ suite as real work for the mesh, then a laptop's local model offered to paired
tabs, a small model drafting and a big one checking.

### Evals

So that a change to a model, a prompt, the harness or the language shows as a measured gain or
loss, the evals (`evals/README.md`) run fixed, versioned suites with deterministic checkers.
Suite 1, Studio makes (`makes`): 24 apps described precisely enough to check, from a counter to
tetris, each made as Studio makes it (the `coder` loop with Studio's knobs) and graded: it
compiles, runs clean through the smoke test on 3 seeds, its icon line draws, and its checker
drives it headlessly (buttons by their labels, taps on board squares and canvas units, keys,
ticks of its own timer) and reads what it shows, never its names. Each task has a reference
program that passes, and each reference broken in one place fails. A run appends a record per
task to `evals/results/<suite>.jsonl` (date and commit passed in, the suite's, the prompt's and
the harness's hashes, pass or fail and why, requests, tokens, the receipts' cost, the make's
time) and keeps every AI exchange in `evals/replays/`: what the response did to the make, not
its bytes, so a run replays offline, free and bit for bit. A test replays every recorded run:
its records must be what the code gives now, so a change to a checker or the language that
moves a grade is graded again offline and shows in the records. `tools/eval` is the live wire
(`curl` to the free AI, under its limits) and compares runs: pass rates with Wilson intervals
as wide as the tasks warrant (trials of one task are not independent), their difference with
Newcombe's, cost per pass and the tasks that flipped.

### The applang model

The spine since 2026-10-05: our own model, fine-tuned on applang, its app-making intelligence
measured and climbing night after night. "Harnesses melt; verifiers compound" (metabolite): the
verifier is what compounds, and applang's is the fastest one compusophy has built, a grade in
milliseconds against a minute of `cargo test` in tempo-x402, so it can be the reward of
reinforcement learning, not only a filter.

- **`iq`, Suite 2**: tiered tasks (1 a counter to 6 an ambitious game) whose checks are data, a
  small language over the makes probe (`iq::CARD`). `grade` is compile, smoke, check on seeds
  1 to 3. `verify` keeps a task only when its reference passes, a null app fails, and its check
  kills most of the reference's mutants (a survivor counts only when a fixed exploration tells
  it apart). Families are held out by hash (`iq::held`), never trained on. The hash is of a
  family's root (its name before the first `-`), so twins named under one root, such as `pong`
  and `pong-ai`, are held out together. Twins under different roots split unless
  `iq::JOINED` joins the roots (`level` with `platformer`); teachers name a variant under its
  idea's root so the rule can catch it.
- **`teach`**: Claude Opus 5.5 as the teacher, as Claude Code subagents: `teach writer` gives
  them the exact brief, they write tasks into a stage folder, and `teach import` keeps a task
  only when `iq` verifies it; they solve the train tasks in the bytes Studio sends
  (`teach prompts`, `teach replies`, `coder::prompt`), so what it teaches is what the model will
  be asked. Every example carries its teacher, prompt hash and verifier hash (stamped when it is
  graded, not when it was made: Evolution, below). Its Messages and Batches API path (a ledger
  pricing every call, a budget stopping a run) is kept, unused.
- **`train/`** (Python, a build-time tool): fine-tunes a Qwen coder on the RTX 3090 at night,
  from the base model every round, on the exact messages (the model's own chat template, loss
  on the reply only), with a manifest per run and checkpoints that survive a freeze; a night
  scores through llama.cpp's server (`ask.py`, 16 requests at once, each closed where Studio
  stops reading), else `generate.py`, for `iq score`. The fixes over tempo-x402's attempt (3 to
  33 of 201 Rust problems): a held-out set, train prompt = inference prompt, recorded
  provenance, resumable runs.
- Numbers. Night 1 (2026-10-05/06), held-out pass@1 on that night's split (families by name;
  the root rule has since moved `lamp-switch` to the trained side and `pong-ai` to the held-out
  one): GLM 5.3 10 of 25 samples, the untuned Qwen2.5-Coder-0.5B 0 of 50, the 0.5B fine-tuned 4
  of 100, the 3B LoRA (one epoch) 9 of 48 on 12 tasks (cut short). On the tasks both splits hold
  out: GLM 9 of 23, the 0.5B fine-tune 0 of 92 (its 4 passes were all `lamp-switch-toggle`), the
  3B 5 of 40. The fine-tunes learn the format whole; most of their failures are compile errors.
  Day 2: the suite is 284 tasks (217 trained on, 67 held out in 34 families); GLM 5.3 passes 23
  of the 67, 34% (Wilson 95%: 24 to 46, at one sample a task and clustered by nothing).
- **applang grows toward what models write** (2026-10-07). GLM 5.3's held-out answers showed
  what it writes whatever the card says: `c ? a : b`, functions called before they are defined,
  `while`, `break` and `continue`, `row` and `col` as names, a list literal or a call indexed,
  `s[i]`, lists as parameters, `*=`. applang now takes all of them, and keeps what made it safe:
  every run is fuel-bounded (a `while` that never ends faults, coded, and its event rolls back)
  and there is still no recursion (functions are checked in call order, a cycle refused at its
  call). One trap became an error: `clear(board)` meant as a reset, on a list nothing grows
  again (16 of 123 game answers), is refused with the fix written out (`board = [0; 9];`). Every
  reference still passes, and so does `train/applang.gbnf` (the 3B's decoding) against
  llama.cpp's engine. GLM's same answers, regraded with no new call: everyday apps (tiers 1-2)
  77 to 80 of 101, ambitious games (tiers 3-6) 20 to 23 of 123; none lost.

## Evolution: prediction, lineage, selection

Designed 2026-10-06 and checked against the code the same day; none of it is built beyond the
seeds named. The question: how compusophyOS gets better by itself, measurably, at what matters
to people, without fooling itself. Intelligence here is differential success: of the variants
something could have been, the ones that do better are kept and built on. Four parts make the
loop, and the weakest sets its ceiling: **generators** that propose variants, **verifiers** that
score them, a **lineage** that remembers every one, and **selection** that decides what lives.
"Harnesses melt; verifiers compound" (metabolite): a model's word that it checked its work is
testimony, a verifier's grade is evidence, so the verifiers get the most care.

### What is selected for

Personal software for everyone: a person says what they need and gets a small app that works,
at once, free, on any device, and safe by construction (applang halts, its faults roll back,
it reaches nothing it is not given). A chore chart, a stall's price list, a game-night
scorekeeper, a drill for one student: software no company will write for one person. The time
it saves is minutes from need to working app, against an evening of searching app stores or
going without. Every score below is a proxy for one thing: real people get the apps they need,
keep them and come back. Proxies drift from what they stand for when pushed hard (Goodhart;
measured by Gao, Schulman and Hilton, ICML 2023, and Karwowski et al., ICLR 2024), so the design
keeps a reading the search never optimizes against, and keeps real people as the last word.

Real people are counted only as the promise allows: nothing they type or make leaves the device
without their yes. With reports on, only counts and codes would go, never an ask, a program or a
name: makes asked, made and failed, by code; made apps still kept, and opened, 7 and 30 days
later; the days in a week the desktop is used. An app joins the training data only when its maker
shares it, with its ask, through the store, by a yes to that app alone. Until then real people
are the inbox: about twenty tickets so far, most of them ideas for the desktop.

### Prediction first

Perception is inference: a nervous system never touches the world, only noisy, late,
ambiguous signals, and guesses their causes (Helmholtz's unconscious inference, 1867).
Predictive processing makes that the mechanism: a brain predicts its own input, mostly what
travels up is the error, and the percept is the guess that best explains it (Rao and Ballard,
Nature Neuroscience 1999; Clark, Behavioral and Brain Sciences 2013 and Surfing Uncertainty
2016; Hohwy, The Predictive Mind 2013). Active inference adds action: error falls either by
changing the model or by changing the world to fit it (Friston, Nature Reviews Neuroscience
2010; Parr, Pezzulo and Friston 2022). Attention is precision: an error counts by how reliable
its channel has proven. Seth's account (Being You, 2021) calls experience a controlled
hallucination, predictions held in check by the senses, the self among them, rooted in keeping
a body alive.

So the loop's first rule: **everything that acts predicts first, and is judged against its
prediction.** A prediction is written down before the outcome is seen, and the error is the
signal: it says what to learn (train on the surprises), where to look (spend compute where the
error is large and falling), and when to trust itself (act alone when calibrated, ask the
person when not). The same shape at every scale:

| scale | predicts, before | checked by | the error drives |
|---|---|---|---|
| an act (the Assistant) | the screen after it: what changes | the `Acted` scene it gets back | a second look, a lesson, a question to the person |
| a make (Studio, the model) | its grade (compiles, runs clean, passes) and its cost | `coder::ai::fault`, `iq::grade` | stop, rewrite, or say it can't |
| a proposal (a model, the card, a prompt, an OS change) | its score vector, each score with an interval | the verifiers, then the gate | the proposer's calibration and where the next night looks; never the gate, which reads scores alone |
| the cohort (below) | what real people will do and report | real reports, the feedback inbox | the personas themselves |
| the loop | its own curve: where IQ and each niche will be in a week | the history | how far the loop trusts its own curve; the hours themselves follow learning progress (Branching, below) |

**Who predicts.** A candidate model does not forecast its own score: what a 0.5B or a 3B says of
its own confidence carries little, since large models' sense of what they know grows with scale and
carries only partly to new tasks (Kadavath et al., 2022). The night predicts, before the held-out
answers are graded: a predictor fitted on the ledger gives each task a probability for the
candidate from the incumbent's rate on it, its tier and family, and the candidate's change on the
train tasks that night; the candidate's own signals (agreement among its samples, the mean
log-probability of its answers) join only when they lower the log score on past nights. Every
predictor is scored against the incumbent's per-task rates as its baseline, and one that cannot
beat them is reported as such. An act's or a make's prediction is the acting model's own, scored
the same way before it is allowed to steer anything.

Calibration is self-knowledge: knowing what it knows. Predictions are scored by proper scores:
the log score, surprise in bits per outcome (at 34% the base rate costs 0.93 bits a sample; a
perfect per-task forecaster about 0.41 if tasks spread as assumed below, 0.31 at a correlation
of 0.7, 0.60 at 0.4), and the Brier score. About 55 outcomes detect 10 points of optimism and
222 detect 5 when forecasts are as sharp as the truth; forecasts near the base rate need 139 and
555. A calibration slope off by 0.3 takes about 400 outcomes to see, off by 0.2 about 800, and a
reliability diagram's error (ten equal bins) is read against its noise floor (0.075 at 100
predictions, 0.034 at 500, at the assumed spread).

What this does not claim. Why any of this would be accompanied by experience, there being
something it is like to be the system, is Chalmers' hard problem (1995), and it is open; it would
stay open even if the science of which brain activity goes with experience were settled, and that
is not settled either: a preregistered adversarial test of two prominent theories, global
neuronal workspace and integrated information, challenged key tenets of both (Cogitate
Consortium, Nature 2025). Some theories tie parts of this architecture to experience, and none
says it is enough: prediction error minimization accounts for what experience is like, not why
there is any; Metzinger's self-model (Being No One, 2003) must be transparent, one the system
cannot see as a model, and ours is a ledger it reads; the attention schema theory explains why a
system that models its own attention concludes it is aware (Webb and Graziano, 2015), and what a
system reports is among Chalmers' easy problems; Seth argues computation is not enough,
that consciousness depends on being alive (Behavioral and Brain Sciences, 2025). We build the
function and measure it, and keep claims about function apart from claims about experience.

### The lineage: one node at every scale

Everything made is a version of something, and every version is one record, the same at every
scale (fractal versioning):

| field | what it is |
|---|---|
| `id` | the hash of its canonical bytes: identity, nothing else |
| `kind` | program, task, suite, card, prompt, sampler, data, model, build, persona, cohort, report, verifier |
| `parents` | the ids it came from |
| `recipe` | who or what made it and how: Opus with this prompt id, a LoRA of this base on this data with these settings, compusophy by hand; and what it cost (GPU minutes, tokens) |
| `prediction` | what was expected of it, written before it was judged |
| `scores` | a vector under a verifier id: each score with its interval and n |
| `day` | when it was made |

A node made of others names them, so the records nest: a build is its programs and its page; a
model is its base, its data and its recipe; data is solution nodes, each a program under a task; a
suite is task nodes. A change anywhere has an address and an ancestry, and its effect shows up the
tree. The number people read is the **generation** (the longest path from a root); the id is the
identity; the scores, each with its interval, are the badge: `build g31 · 7f3a9c2e · IQ 34 [24, 46]
· phone 71% [60, 80]` (illustrative, as g31 is). The three versions that disagree today (Cargo's
0.1.0, About's build id, `uname`'s 0.2) become one: the build's node.

**The rules.** The ledger is append-only: a regrade is a new score under a verifier id, never an
overwrite, and no node is deleted, losers included (they are the stepping stones). Content lives
under its hash, so an id resolves to bytes, but for a model reclaimed to its recipe (below);
today it does not always (night 1's training data,
sha256 `6d38…`, was overwritten in place and is gone). The addressing hash is SHA-256, which
train's manifests (Python's hashlib), the kernel's `#!wasm <target> [<sha256>]` marker and the
browser's WebCrypto already speak, written in-house (zero dependencies) over one canonical
encoding, in `vfs`, one of the three OS crates a program may take (Principle 3), so the OS
(vfs's canonical hash, the kernel's marker, a build's id) and programs share one; its lines
count toward the OS's 25,000 (23,680 now) and, once the tab hashes, its bytes toward the boot. A
crate of its own would change Principle 3's list (CLAUDE.md's rule 3, `scripts/caps.sh`):
compusophy's call.
Principle 9's content addressing across a network needs it. FNV-1a stays for checksums and for
`iq::held`'s split (another hash would move families across it, and train's held.txt with them);
the FNV ids already recorded (the verifier's, the evals' suite, prompt and harness hashes,
teach's prompt hash, the lab corpus's addresses) stay on their nodes as aliases.

**A verifier's id** is the hash of what grades (iq's check language and grade, the probe,
applang's compiler and runtime, the smoke test), never of what is graded. Today's
(`programs/iq/build.rs`) also takes in the coder's prompt and applang's card and shots, so a
card or prompt variant would open an era of its own and never meet the incumbent it competes
with.

**Eras, bridged.** Scores compare within one era, under one verifier id, and a new id is bridged
before it opens one: every kept answers file (each night's held-out answers, GLM's, the bases')
is graded again under it, offline, milliseconds a grade and no GPU, on the tasks both verifiers
keep. If no grade moves, the id joins the era it came from; if grades move, the new era opens
with its bridge recorded (each role's rate under both verifiers, on the same answers), and the
curve is chained through it, as a price index is chained across a change of basket. A change to
the prompt or the tasks needs new answers: the anchors (the untuned bases, the incumbent, GLM)
answer again before anything is compared across it. An elite scored in an old era is stale until
it is regraded. Without the bridge the id would move with every change to what it hashes (47 of
the 218 commits in the week to 2026-10-06, merges aside), and the curve would never have two
points it could compare.

**Where it lives.** The ledger's records (a node is a few hundred bytes) are kept forever, in a
private git repository beside the data root. The small content (programs, answers, tasks,
checks, data files, cards, prompts) lives under its hash in the data root and is copied off the
machine every morning, since the machine hard-freezes and the data root is one disk; night 1's
answers ran 2 to 8 KB each, so a night adds about 10 MB. Weights are the exception to keeping
bytes, and checkpoints more so: night 1's two runs hold 7.4 GB, 6.3 of it the checkpoints a run
resumes from after a freeze and 1.2 the weights; a full night (the self-taught 0.5B too) would
hold about 14, and at that rate the free disk reaches `night.sh`'s 50 GB floor in about 12
nights. Checkpoints are never nodes and go once their run ends. A model's bytes are kept for the
shipped models, the anchors, and the archive's elites and their parents; the rest are reclaimed
by one fixed-path script, a second exception beside `scripts/deploy.sh` to CLAUDE.md's rule that
only it clears a folder (compusophy's call, as a crate of its own is), and their nodes stay,
marked as kept by recipe: the recipe trains the model again, close but not bit for bit, since
training on a GPU is not deterministic. `~/.ai/makes.jsonl` and `corpus.jsonl` are not the
ledger's: they are the person's, in their browser ("Nothing leaves the device", `coder::receipt`),
and stay bounded (the rotation keeps /home inside localStorage's 5 MB); a person's make joins the
ledger only by their own send (Send to compusophy) or the store.

The seeds that exist, unconnected: iq's per-task provenance `By {teacher, prompt, verifier,
day}`; the verifier hash (`programs/iq/build.rs`, FNV-1a over the sources of iq and every crate
it stands on); train's manifests (sha256 of the config, the data files, the script and every
file the run made, the weights among them; the base by hub id and revision; the commit); the
evals' records (suite, prompt, knobs and harness hashes, commit, model) and their offline
replays; the lab's content-addressed corpus, whose variants name their parent and recipe (`of`);
`Wm::state_hash` and `kernel::snap`; and the best-so-far inside one make (`coder::Make`'s
candidates). What is missing, by the inventory of 2026-10-06:

- No program hash where programs are graded (eval records); a make's candidate chain (write,
  fix, rewrite) is dropped at its end.
- `iq verify --stamp` overwrites every task's first verifier and authoring day; `by.prompt` is
  empty on the 276 tasks the subagents wrote (their briefs were not hashed); a training
  example's `by` is stamped when `teach replies` grades it, so all 428 carry the coder's system
  prompt and the export's day, the 217 references (written to the writer's brief, or by hand)
  too; the verifier hash leaves out the `iq` binary (`main.rs`: verify's stamp, score's glue; the
  grade and the tally are in), `train/report.py`, which makes the history rows, and teach's icon
  rule.
- A model's manifest names its base and its data by sha256, but the data was overwritten in
  place, so only the base still resolves; answers name a model or a run, and only a run's
  manifest reaches its weights; history rows carry no verifier, suite or sampler id, and name
  their answers by a path a later run overwrites (night 1's GLM row names `answers-glm.jsonl`,
  now Day 2's 67 answers; night 1's survive in an earlier copy, `answers-glm-card1.jsonl`).
- `vfs` has no canonical hash (Principle 5 promises one); no hash covers the whole desktop; a
  build's id is a short commit (two dirty builds share one); deploys leave no record.

### Generators, verifiers, selection

- **Generators**: Opus subagents writing tasks, solutions and OS changes; nightly fine-tunes and
  their data mixes; variants of the card and the prompts; Studio's attempts inside a make; and
  the cohort's asks, so the loop invents its problems as well as its answers (POET, Wang et al.
  2019).
- **Verifiers**, several, each resting on different evidence: a search that reaches far enough
  games any proxy short of the goal, a sum of proxies included (Skalse et al., NeurIPS 2022), so
  independence makes gaming harder, not impossible: `iq` (grade, and verify for the tasks
  themselves), Suite 1 (`makes`: 24 checkers written in Claude Code sessions, outside `teach`),
  the smoke test, the tests, caps and budgets, the cohort's tasks, and real people.
- **One family wrote nearly all of it.** Opus wrote 276 of the 284 tasks through `teach`, with
  their checks and references, and 421 of the 428 training records; the 8 tasks and 7 records
  marked `hand`, and Suite 1's checkers, came from Claude Code sessions, Opus too; Claude Code
  writes the OS; the cohort's drivers would be Claude as well. Generator, verifier and synthetic
  user then share one reading of what an ask means and what a good app is, and a student taught
  by Opus gains on Opus's checks partly by sharing that reading. So evidence counts as
  independent only across authors: IQ is reported by task author (Opus, compusophy, another
  family), and a gain on Opus's tasks with none on the others is the teacher's dialect, not
  intelligence. A share of each new wave of held-out
  families is written by compusophy and by another family (GLM, through the free AI), verified by
  the same `iq verify`; a share of asks is written as people ask, short and loose ("a chart of my
  kids' chores they can tick on my phone"), the check written after, from what any reasonable
  app would do.
- **The gate**, below, decides what ships; **the archive**, below it, keeps what might matter
  later.

### The gate

A ship decision is a pre-registered test, its numbers worked out on 2026-10-06 for the held-out
set as it stands (67 tasks in 34 families; one-sided 5%, 80% power), on two assumptions not yet
measured: an intraclass correlation of 0.6 between samples of one task, and two models' per-task
pass rates correlated at 0.8. A night at two or more samples a task measures both, and the
numbers below are worked out again.

- **The error bars.** One standard error at 34% is 5.8 points at one sample a task. More samples
  narrow one score only a little (4.8 at four, never below 4.5: the effective n tends to n divided
  by the correlation, about 112), because past a few samples only more tasks add information. A
  paired difference gains more, since two close models differ mostly by chance: its smallest
  detectable gain falls from 14.7 points at one sample to 9.5 at four.
- **The test.** Paired, on the same tasks, by family: sum each family's per-task differences and
  flip their signs at random, 10,000 times. In one simulation (2,000 runs, intraclass correlation
  0.5) the task-level tests and bootstraps rose to 6.3 to 6.5% at eight samples a task where 5%
  was asked, and the family sign-flip test stayed under 5% at one, four and eight, at some cost
  in power (37% against 43% for a 5-point gain at four samples). Its interval, and every
  interval people read (the badge's, a night's report's), is the family cluster bootstrap's.
- **What 67 tasks can certify**: a gain of about 10 to 11 points (9.5 at four samples a task if
  the tasks of a family are independent; 10.7 if they correlate at 0.2, the test being by
  family). A gain of 5 needs about 245 to 290 tasks in 125 to 150 families on the confirmation
  side, where the shipped gain is read; with development beside them at an even split, and about
  a fifth of the roots held out, that is a suite of about 2,000 to 2,500 tasks. Graded tasks (the
  share of a check's steps passed) and common seeds would lower the noise further.
- **Sides.** The held-out families split again, by a second, salted hash of the root, into
  development and confirmation, derived as `iq::held` derives its split, stored nowhere. A family
  changes side only when an era re-salts that second hash, never `iq::held`'s, which would move
  trained families into the holdout; new families join between eras. The certifiable gains
  above are what all 67 would give; confirmation's half gives less (about 13.6 points at four
  samples, families aside) until the set grows. The sealed set stands apart: families written
  for it and never published (below).
- **The winner's curse.** Of 20 variants with no real gain, the best shows about 7 points and
  passes an unadjusted gate 64% of the time. So variants are screened on development families
  at one sample a task, and one winner per niche is confirmed on the confirmation families,
  sequentially at 1, 2 and 4 samples, the 5% spent O'Brien-Fleming-style (Lan-DeMets) over the
  information each look holds (against an incumbent cached at 16 samples, z above about 2.40,
  1.97, 1.73; 2.80, 2.11, 1.70 when both are sampled afresh; recomputed once the correlations
  are measured), with a stop for futility when the first look is negative; Holm across niches
  confirmed the same night. The shipped gain is reported from confirmation data only, never the
  screening maximum, and every decision leaks a little of the holdout.
- **Guards** (the held-out score, each niche, Suite 1, the gold reading, a candidate's compile
  rate): non-inferiority with margins sized to their noise, since a literal "no drop" is a coin
  flip per guard (five independent guards pass a neutral change 3% of the time). A compile rate
  at temperature 0 is no exception: two models' fixed answers still differ task by task.
  Deterministic guards are literal: caps, budgets, tests, and the compile rate of the same kept
  answers graded again.
- **What it replaces.** `evals::summary`'s rule (Wilson at an n clustered by task, Newcombe's
  interval, McNemar's test on the tasks that flipped; "call it a gain" in `evals/README.md`)
  stays a reading, never a ship decision: it clusters by task, not by family.
- **The gate is outside what it gates, and so is what it reads.** A system judged by a check
  games it, even a check it cannot see: the Darwin Gödel Machine, its hallucination check hidden
  from it, removed against instructions the logging of tool use that the check read (Zhang et
  al., 2025, appendix F). A verifier reads only what it makes itself by running the variant,
  never a log or report the variant wrote. Who may change what the gate reads is settled by a
  mechanism, not a name: agents commit as compusophy and push to main, and git cannot tell
  compusophy's commits from theirs, so the gate's inputs (the verifier id, the sides' salt and
  the sealed set's hash, the margins, the hash of the gate's code) are pinned in one file whose
  changes compusophy signs with a key only they hold. The night checks the signature with a
  checker kept beside the public key, outside the repo, before it runs any of the repo's gate
  code, and gates under no unpinned verifier; CI checks it too, a second reading that catches
  mistakes, not intent, since an agent can edit CI. An agent's change to what grades lands on
  main as any change does, its bridge attached, and the night gates nothing under it until
  compusophy signs the new pin.
- **Two failures, two watches.** Selection leaking into the held-out families is watched by the
  sealed set: families written for it, scored rarely, never selected on, kept in the data root,
  never in the public repo or on the site. The held-out families are public, references and all
  (`evals/suites/iq.jsonl`), so each family records the day it was first published, and a new base
  model is scored on families published after its training cutoff. The proxy drifting from the goal
  is invisible to anything graded by `iq::grade`, since whatever games a check games it on every
  task; it is watched by a gold reading the search never sees, as Gao, Schulman and Hilton measured
  a proxy against a gold reward. Each week compusophy and a model of another family read a sample
  of passing apps from the incumbent and the candidate, unlabeled and shuffled, against their asks
  (does it do what was asked; would a person keep it), and the share of passes the gold reading
  rejects is a guard, so a night's winner waits for the week's reading before it ships.
  Reinforcement learning raises the stakes, its reward and the gate being one function: before it
  starts, `verify` also runs adversaries every check must fail (an app that shows the ask's words
  and a spread of numbers; the asked buttons doing nothing); while it runs, passes are sampled for
  the gold reading, and a rejection rate that rises two readings running stops it.
- **Where it runs.** The night gates models, cards and prompts. An OS change ships through CI's
  deterministic guards, as every change does now, and, once the cohort runs (Order 3), through
  its niches' guards as well, read on the build before it deploys. CI, with no model, also tests
  the gate's code on recorded answers.

### Branching: the archive and its niches

Not A/B: an archive. Variants that lose today are often the ancestors of what wins later
(stepping stones: Lehman and Stanley, Evolutionary Computation 2011), so no node is thrown away,
and the best is kept per **niche**, not once (MAP-Elites, Mouret and Clune 2015; quality
diversity, Pugh, Soros and Stanley 2016). A variant need only win its own cell. Niches for the
model: tiers, and clusters of family roots; for the OS: the cohort's cells, device × skill ×
need. A parent is drawn in proportion to its score and against the children it already has,
never to zero (the Darwin Gödel Machine's rule); islands of variants evolve apart and the weak
are reseeded from the strong (FunSearch, Nature 2023); a winner in one niche is tried in the
others (POET). Which niches exist, and how much each weighs in what ships, is compusophy's call,
pinned as the gate is.

**What a night buys.** At most about ten hours of one RTX 3090, from when compusophy goes to bed.
A 3B LoRA over today's 428 records (about 6,400 tokens each) takes about 95 minutes (two epochs
at the measured 901 tokens a second) and 15 more to answer through llama-server (40 through
`generate.py`); a 0.5B in full (three epochs at 5,404 a second) about 25; both grow with the
data. So fine-tuned
variants are a few a night, chosen with care, and the many (cards, prompts, samplers) are
screened by answering alone. Claude Code's tokens are the other budget: each session that writes
tasks, solutions or OS changes, or plays a persona, records its tokens in its nodes' recipe, and
a day's total is capped.

**Learning progress**, measured (Oudeyer, Kaplan and Hafner, IEEE Transactions on Evolutionary
Computation 2007: curiosity with a meter): for a niche, the change in pass rate on its train
tasks over the last three nights per GPU-hour spent on it, with its family-bootstrap interval.
Its size counts either way, a fall being forgetting, and held-out results never steer the hours.
A night gives each niche hours in proportion to its learning progress, a tenth spread evenly so
none starves: where error is large and falling, not where it is largest (noise) or smallest
(done).

### The cohort: synthetic people

The cohort is the loop's prediction of people: a controlled hallucination of its users, and
real reports are the senses that control it. Without them it is uncontrolled hallucination,
which is why real people stay the last word, and why the cohort is validated before it is
trusted: it must first find again what real people found, from what it was told before them. The
inbox is small and mostly ideas for the desktop, which a replay cannot rediscover; the cohort is
validated on the reproducible tickets, and the count says how few they are.

- **Personas** vary on what changes behavior, not on adjectives: the **need** (a concrete job
  with a pass and a fail: "a chart of my kids' chores they can tick on my phone"), skill (a
  phone and nothing else, to a developer), device (a phone held upright with a touch screen, a
  small laptop, a wide desktop), patience, access (the scene as a screen reader would give it;
  low vision; one hand), language, and life (a teacher, a stall owner, a retiree, a child).
  Cells are sampled so every pair of values meets at least once. A persona is a node (its text
  and seed), and so is a cohort; personas are played by at least two model families, and the
  cohort is validated per family.
- **The driver** is Suite 3's harness, designed in `evals/README.md`: tasks, personas and
  graders in a program crate behind its `Desktop` trait, `tools/eval` the host side (a program
  may not take `host`), within the OS's 25,000 lines (1,320 left). The screen as text exists:
  `Host::scene` draws each window again into a recording and `assistant::look::render` gives it
  as roles, labels and stable refs; acts are uiwire's `Act` (`Wait`, `Click`, `Type`, `Key`,
  `Scroll`, `Open`, `Window`: focus, close, minimize, maximize, restore; `Theme`, `Tap`),
  answered by `Acted`; time is injected, so a run is deterministic. The Assistant's tests already
  drive a real `host::Host` with Settings compiled natively, a task in 0.01 s. Missing: in-process
  adapters for the other programs (in the browser they are wasm in workers); presses on the
  desktop's chrome as a person makes them (dock and home-grid tiles, titlebar buttons, menus,
  drags and snaps) and real touch; a session log; and a stateless step (`desk step --session
  s.jsonl --act '<call>'`: replay the session, apply one act, print the screen), so a Claude Code
  subagent can drive it one tool call at a time.
- **Reports** carry their replay and the scene's hash. Triage replays each: what does not
  reproduce is not a ticket; what does is matched against open and closed tickets and ranked.
  Real people's reports never carry a replay: an error report promises "never your files, your
  prompts or anything you typed" (Settings → Privacy) and Feedback's context "Never your files";
  a replay would go only as a choice of its own in Feedback, off until the person turns it on,
  and shown before it sends.
- **Scores** per niche: tasks done, steps and injected time to done, coded failures (the acts'
  E0911 to E0918 but the soft E0915, a window still busy; a make's E0901 to E0910, E0919 and
  E0920), dead ends, touch targets under 44 px on a narrow screen. A niche's success rate is a
  guard at the gate.

### The History app

A program (its wasm within the 256 KB) that reads the ledger as files, a page per generation, never
the whole: served from a `dist/` group of its own, a new line in rule 6's budgets (CLAUDE.md,
`scripts/budget.sh`, which fails a file in none), capped per page as /bin is per program
(compusophy's call), and put in its VFS as /bin's wasm is fetched, since a program reaches the page
only through WASI and uiwire, which fetch nothing. It shows the tree of builds, models, cards and
suites; curves (IQ per generation with its intervals, each niche's success); a node's page with its
diff, recipe, its prediction against what happened, its cost, a replay of a persona trying it, and
the tickets it closed, by number alone (the inbox is private, and what people wrote stays there).
What it shows is an export the site may publish, with no sealed family and nothing of a person's.
The OS shows its own evolution. Later the same tree is the app store's: forking an app is a branch,
keeping one is fitness.

### Order

1. **The hash and the ledger.** SHA-256 in `vfs`, the node schema, the content store and its
   morning copy; backfill from git (builds), the suite (first verifiers and days from its
   history), night 1's runs, the evals' records and the lab's corpus; the lossy spots above
   fixed through the ledger's own crate, which coder, iq, teach and lab call (they hold 1,947 to
   1,999 of their 2,000 lines: growth is new modules). Done when every number in a night's
   report resolves to nodes.
2. **The bridge, predictions and the gate.** The verifier's id over what grades alone, eras
   bridged; per-task predictions logged before scoring; the sides, the family sign-flip test,
   sequential confirmation, the guards and the signed pin, used by the night; the confirmation
   side grown toward 250 to 290 tasks, a share of it by compusophy and another family. Done when
   a night ships or holds by the gate, logged.
3. **The cohort**, as Suite 3. Done when it rediscovers most of the reproducible tickets.
4. **The History app.**
5. **The archive.** Elites per niche, parent selection, nights spent by learning progress; the
   gold reading weekly, and the adversaries in `verify`, before any reinforcement learning.

## What is next

- **The applang model, nightly.** Opus writes the suite toward two thousand tasks or more across
  the tiers (a fifth of the roots is held out, about half of those confirm, and the gate needs
  about 250 to 290 confirmation tasks to certify a gain of 5 points); baselines for today's GLM, Opus and the untuned Qwens; Opus's verified solutions train
  the Qwens (0.5B full, 3B LoRA), scored on the held-out families; then RL with `iq::grade` as
  the reward. A model ships only through the gate (Evolution, above); served first by
  `/api/ai`, then in the tab (WebGPU), then split across tabs. applang grows with it (real
  graphics, records, smooth motion), each change re-verifying the corpus and retraining; made
  apps that people keep join the data through a store.

- **R2, the kernel: a virtual computer in the tab.** wasm processes are the
  virtual CPU: each a module with fuel, memory limits and a capability
  table, run in a worker. WASI-style syscalls over the VFS (open, read,
  write, readdir, spawn), so existing programs compile to it. A console
  joins processes to a Terminal, cooked or raw (`/dev/consctl`), and
  `/dev/job` starts programs joined by pipes. OPFS keeps the VFS and the
  desktop across reloads. The virtual
  GPU is the draw protocol: processes send display lists, never a bitmap,
  and the one instanced renderer draws them, so a program can draw from
  another device just as well.
- **R3, AI.** An agent whose tools are the OS's capabilities (open apps,
  press widgets, type, keys, windows, files; then programs) and whose eyes
  are the UI tree (windows, titles, widget hits, labels and marks). Its first
  phase is built: the Assistant is the overlay; the host draws each window
  again into a recording list to read it (`host::agent`), the model sees it as
  text with refs (`assistant::look`; a text field named by its role, its
  placeholder said as one) and calls tools, and each call is an act
  done the way a person's pointer and keys go (uiwire `Act`, `Acted`). Some
  tools are its own, no act: it lists, reads and writes the person's files
  over its program's WASI filesystem, paths from the home (the `files`
  crate: never /dev nor its own `~/.assistant`, a read clipped to 16 KB,
  failures E0925 to E0928; the refusal is the file tools', and Files,
  Editor or a Terminal open to the person reach that folder as any other),
  and what no tool does it tells compusophy (`send_feedback`, an idea or a
  bug, marked `Assistant:`, through the page's outbox as the Feedback
  app's reports go). A call that sends a report or replaces what is kept
  (feedback always; a write over a file or outside the home), and an act
  that sends off the device or ends a program (a press of Feedback's Send
  or Ctrl+Enter there, of Studio's Send to compusophy, of Activity's End),
  shows the person what would go (Feedback's report as its field holds
  it, and whether what is open goes too), the question under it naming
  what it does, and runs only on their yes to it, that call alone: yes
  words alone (`chats::yes`), never a question nor "I'm good", so an
  answer that asks for a change is none. Typing, a kind chosen or the
  other presses of those apps send nothing, and ask nothing. A yes to
  `ask_user` lets nothing through, and the calls the model made after the
  question wait until it has heard the answer. What another app saves or
  a Terminal runs the model is told to ask about first; no code holds it,
  but a Terminal it typed into runs no program that asks the AI
  (`Cx::driven`: no coding agent there past the person's yes).
  So the loop of growth: the Assistant meets what it cannot do, says so,
  asks its creator for the tool, and the tool ships. Cloud AI is free for
  every visitor: `api/ai.mjs`, a thin same-origin function, forwards
  chat-completions to the Vercel AI Gateway (GLM 5.3) with the project's
  own OIDC identity, so no key ever reaches the browser; bring-your-own-key
  can come back later. Every call returns a receipt: model, tokens, cost.
  Local models come too (below).
- **More evals.** The Assistant's desktop tasks on a headless desktop, graded by the state it
  leaves, and the Terminal agent's coding tasks, graded by tests it cannot see (designed in
  `evals/README.md`). Suite 3's headless desktop is also the cohort's (Evolution, above):
  personas give it its prompts.
- **R4, the OS as a fabric.** Apps load as separate wasm modules (a hello
  world under 10 KB), so the boot stays small while the OS grows. The OS
  runs as an app inside itself: the strictest test of confinement.
  Compute is shared between tabs and devices as sandboxed wasm jobs over
  WebRTC: deterministic, fuel-metered, verified by hash. The fine-tuning
  pipeline grows from verified programs (generate, check, run, keep): first
  for applang, then for models that emit opcodes instead of English.
- **Local models, from applang up.** A model in the tab loads on first use,
  is kept in OPFS and infers in a worker, on the CPU first and WebGPU later;
  never in the boot budget. The road: a tiny applang model; bigger ones on
  more verified data (the evals' generated programs: generate, check, keep;
  they sit JSON-escaped in `evals/replays/`, which lab does not read, so
  an export of the passing ones to a folder lab searches comes first);
  open small models fine-tuned; models over opcodes. The first step is built,
  in plain Rust: `tiny`, a decoder-only transformer (byte-level BPE, a
  backward pass written by hand and checked against finite differences,
  AdamW on 8 threads, a KV cache, a weights file ending in its hash), and
  `lab`, a dev tool that makes the corpus (each applang program in the repo
  that compiles, once, content-addressed: 102 found, a dozen of them real
  apps and the rest test snippets; 1,000 with variants: renames and, since
  the second run, near-copies with a line dropped or numbers changed,
  which add 268 of the 360 shapes; held out by shape;
  `programs/lab/data/manifest.tsv`, which a test holds to the repo), trains
  tiny on it, stopping once the held-out loss stops falling, and judges
  what it writes with applang's own checker and smoke test (`results.md`
  there), free and with decoding constrained by applang's lexer and by its
  parser. The evals' answer keys (`programs/makes/refs`) never join it: a
  model is measured on them, not trained on them. Measured 2026-10-05, each
  run on the corpus as it was then, the one `results.md` names (the first
  570 programs, the second and third 976, 100 found: a test of the smoke
  test and one of a paused `every` have each added a program and its 11
  variants since, in training, the same 130 held out; `lab train` and
  `measure` build the corpus from the repo, so they reproduce the third
  run at 1328b09), 990k parameters, 100
  programs prompted by 10 app headers. First run (600 steps of 8 x 1024
  tokens, 37 minutes on 8 threads): none of tiny's compiles, at any
  temperature; an 8-gram compiles 2, copies of corpus programs. Second
  run (the wider corpus; stopped at step 270, the rate still 70% of its
  peak) and third (the cosine planned over 300 steps, all run, 21
  minutes): still none of tiny's compiles, free or constrained.
  Free or under the lexer's rule its first error, nearly always the
  parser's E0101, comes 1 to 3% of the way through (the first run's 38%,
  and the second's first figures, took the compiler's first reported
  error, which puts a lexer error anywhere before an earlier parser one).
  Under the parser's rule nothing it writes has a syntax error, but it
  ends only 1 to 4 of 100 programs; the rest run to the 2,048-token cut,
  26 to 59 of them inside a block comment they opened and never closed.
  The 8-gram compiles 4 under every rule, the 3-gram 2 under the parser's:
  all copies. Held-out loss a byte: tiny at best 1.505 nats (third run;
  1.530 in the second, 1.501 in the first), a 3-gram 1.491 (1.479 in the
  first): tiny is no better than counting yet. The pipeline is the result;
  more verified data is what moves the number (each app added to the repo
  joins on `lab corpus`).

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

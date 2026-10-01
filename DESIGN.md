# compusophyOS — design

Status: phase 0 done (workspace, constitution scripts, forked crates, `wm`);
phase 1 in progress: the desktop, runtime text, the built-in apps, Studio,
and a real terminal backed by the computehub node (`node/`).
Started 2026-09-30. Author: compusophy.

## What it is

A desktop OS that runs in one browser canvas. Rust compiled to WebAssembly;
JavaScript is limited to the generated wasm-bindgen glue and a two-line
bootstrap. Tiling-first like Pop!_OS, styled like COSMIC.

It is a platform: anyone can build apps for it. Later it becomes the lobby of
a shared world (Neopets / Club Penguin / RuneScape energy) and a node in
**computehub**, a mesh that pools compute across a person's devices and,
eventually, across people.

## Constitution

1. **Rust only.** No hand-written JS. Guest apps don't use wasm-bindgen at all.
2. **Zero external dependencies** in every crate except the web crates
   `platform` and `os` (wasm-bindgen, web-sys, js-sys), server crates under
   `api/` (the Vercel Rust runtime and an HTTP client; see AI routing), and
   build-time tools that never ship. The node (`node/`, see The node) is a
   separate Cargo workspace with its own lockfile and a short, reviewed list
   of native dependencies (a PTY library, a WebSocket library, an OS random
   source); it never ships to the browser.
3. **≤2,000 lines per crate, ≤25,000 total.** The node counts as a crate
   and toward the total. At a cap: split, shrink or delete. Never raise the
   cap. (Inherited from litelite.)
4. **Budgets, enforced in CI** (`scripts/budget.sh`, gzip -9):

   | budget | target |
   |---|---|
   | boot: every top-level file in `dist/` (page, glue, wasm with the boot font) | ≤150 KB compressed |
   | deferred: `dist/fonts/deferred/`, fetched right after the first frame | ≤30 KB compressed |
   | lazy: the rest of `dist/fonts/`, fetched when a terminal first opens | ≤60 KB compressed |
   | licenses: `dist/licenses/`, never fetched by the page | not counted |
   | hello-world app | <10 KB |
   | first frame after the wasm arrives | ≤100 ms |
   | idle CPU | zero: no frame is drawn unless something changed |
   | interaction | 60/120 fps, input handled in the same frame |

5. **Weak devices and bad networks are the baseline.** WebGL2 only (WebGPU
   never required). Works offline after the first load.
6. **Deterministic core.** Integer math, time passed in from outside, seeded
   randomness. Any kernel state can be replayed and hashed (`wm` and `vfs`
   today; `scripts/caps.sh` scans them).
7. **One canvas.** The page's DOM is the `<canvas>` and one hidden
   `<textarea>`, which `platform` creates and owns; nothing else. The
   textarea exists because IME composition, dead keys, paste and phone
   keyboards only work on a focused text element. It never shows and never
   takes pointer events; everything visible is drawn on the canvas.

## The brick

Everything is built from one unit, the **cartridge**:

- **fixed size:** declared memory and fuel limits
- **confined:** it can only call what its capability table names
- **total or watched:** tier 0 provably halts; tier 1 is killed by a watchdog
- **receipted:** every run returns output + fuel spent + a hash

The same contract holds at every scale:

```
event handler ⊂ app ⊂ desktop ⊂ device ⊂ household ⊂ mesh
```

Nothing is special-cased by size. The UI shows this too: zoom out from your
desktop to a neighborhood of other desktops, then to the mesh; zoom in to be
inside an app.

## Architecture

```
main thread ─ one <canvas>, WebGL2
┌──────────────────────────────────────────────────────────┐
│ os        wasm entry point; wires everything below       │
│ wm        tiling tree, workspaces, focus (deterministic) │
│ kernel    processes, caps, VFS, input routing, fuel      │
│           (wm and kernel have no web deps)               │
│ gfx       draw lists, glyph atlas, the WebGL2 shaders    │
│ platform  the only crate that calls browser APIs         │
│ tier 0 apps run here (applang, interpreted, total)       │
└───────────────▲──────────────────────────────────────────┘
                │ shared-memory ring buffers:
                │ draw commands out, events in, syscalls
   ┌────────────┼────────────┬────────────┐
 worker       worker       worker        (pre-warmed pool)
 host shim +  host shim +  host shim +
 tier 1 app   tier 1 app   tier 1 app
```

### Crates

| crate | role | origin |
|---|---|---|
| `fuel` | fuel and byte budgets | fork of fuellite |
| `cap` | capability tables as data: the syscall table | fork of caplite |
| `lang` | diagnostics, lexer, parser harness | fork of diaglite + lexlite + parselite, merged |
| `wasmgen` | wasm module builder | fork of modlite |
| `applang-syntax` | the tier 0 app language's lexer, parser and checker | split from applite |
| `applang` | the tier 0 app language's runtime and public API | split from applite (it was at 1,999 of 2,000 lines) |
| `wm` | tiling tree, workspaces, focus; deterministic, replayable | new, no web deps |
| `vfs` | in-memory filesystem (`/apps`, `/home`, `/tmp`); deterministic | new |
| `font` | TrueType reader and anti-aliased glyph rasterizer | new |
| `gfx` | draw lists (one instance buffer), the glyph atlas, the shaders | new |
| `text` | text on the atlas: font slots, fallbacks, measuring, wrapping, drawing | split from `ui` |
| `ui` | immediate-mode widgets, theme, the `App` trait, `Cx`; re-exports `text` | new |
| `vt` | VT/xterm escape-sequence parser | new |
| `term` | xterm-compatible screen model and key encoding | new |
| `guest` | the guest shell: line editor and commands over the VFS, for the Terminal | split from `apps` |
| `apps` | the built-in apps: Terminal, Welcome, Launcher, About | new |
| `studio` | the applang editor, and `AppHost`, which runs `.app` files | new |
| `host` | the wm and one app per window: events, requests, sockets, lazy fonts | split from `shell` |
| `shell` | panel, window chrome and bindings around `host`; no web deps | new |
| `platform` | canvas, WebGL2, input, the text-input textarea, WebSockets, fetch | new |
| `os` | the wasm entry point: boot and deferred fonts, VFS, app registry, pairing, glue | new |
| `kernel` | processes, caps, input routing, fuel; composes `wm` | planned |
| `sdk` | what tier 1 apps link against | planned |
| `tools/serve` | dev-only static server for `dist/` | new, never shipped |
| `node/` | computehub-node: real shells over a local WebSocket (its own workspace) | new, never shipped |

Package names take a `compusophy-` prefix (`compusophy-fuel`, ...) so they can
be published later.

**Fork provenance:** litelite 0.2.0, commit `4f5e056` (2026-07-20),
Apache-2.0. Keep the license (a copy of `LICENSE` in every crate dir, so it
ships in each package; `scripts/caps.sh` checks it) and note the origin in
each crate's docs.
Not forked: prooflite, stratlite, backtestlite, evmlite.

### Rendering

- One WebGL2 context. Everything is an instanced quad: window surfaces,
  rectangles, glyphs, shadows. Rounded corners and shadows are computed in the
  fragment shader from signed distance, so there are no shadow textures.
- Aim for one draw call per frame.
- Damage-driven: redraw only when kernel state or an app's display list
  changes.
- Text: glyphs are rasterized at runtime into one atlas texture and drawn
  as instances in the same buffer (see Text), so text costs no extra draw
  call.

### Text

- `font` parses TrueType (`glyf`) fonts and rasterizes anti-aliased
  coverage at any size. It is small, and it replaced the planned build-time
  MSDF baker: any size, any pixel ratio, and new fonts at runtime.
- `ui::TextSystem` has three built-in slots: Inter Regular (`Sans`) and
  SemiBold (`SansBold`) for the UI, JetBrains Mono (`Mono`) for code and the
  terminal. Up to 8 fallbacks can be added at runtime. The fonts load in
  three groups, each with its own budget (`scripts/budget.sh`):
  - **boot:** Inter Regular, subset and built into the wasm by `os` with
    `include_bytes!` (about 8 KB compressed). The first frame needs only
    this.
  - **deferred:** Inter SemiBold and JetBrains Mono (`assets/fonts/deferred/`,
    served from `dist/fonts/deferred/`, about 24 KB). `os` fetches them right
    after the first frame and fills their slots with `TextSystem::set_font`,
    then draws again. Until then, or for good if a fetch fails, bold text
    draws in Regular and monospace cells draw colors and the cursor but no
    glyphs. An empty Mono slot measures with JetBrains Mono's own metrics,
    so a terminal's grid does not change when the font arrives. The
    deferred fetches take the top two fetch ids, which the shell (counting
    up from 1) never reaches.
  - **lazy:** the symbol fonts a terminal needs (spinners, check marks,
    braille; `assets/fonts/lazy/`, served from `dist/fonts/`, about 39 KB),
    fetched by the shell when the first terminal opens.
- Glyphs are rasterized at `size * dpr` device pixels, cached per face,
  glyph and size in a 1024 x 1024 single-channel `gfx::Atlas`, and placed
  on whole device pixels, so the atlas is sampled 1:1. The platform uploads
  only the rows that changed. When the atlas fills, it is cleared and the
  frame is drawn again.
- Lookup: the style's face, the other built-in family, then each fallback;
  a character found nowhere draws a `.notdef` box. An empty SansBold slot
  stands in as Sans; text in an empty Mono slot draws nothing.
- Licenses: all fonts are SIL OFL 1.1; the texts ship in `dist/licenses/`
  and About credits them (`assets/fonts/README.md`).

### Apps today

- An app implements `ui::App`: `title`, `draw` (into a `ui::Ui` over its
  content rect), `event` (returns whether to redraw), `wants_text_input`
  and `preferred_size`. The UI is immediate-mode: each frame the shell
  builds a `Ui` per window and the app lays out widgets in it. Pointer
  events are routed against the hit regions of the last frame.
- Apps reach outside themselves only through the `ui::Cx` of an event: the
  VFS, the page clock, the node pairing, and requests (open or close a
  window, open a socket to `127.0.0.1` only, send, load the fallback fonts).
  The shell carries out requests and turns what only the browser can do
  into `shell::Effect`s; `os` hands those to `platform::Ctl`: the effects
  of each response, and after every event and frame those the shell queued
  outside one (`Shell::take_effects`: a terminal's RESIZE after a frame
  changed its grid).
- `os` owns the registry that makes apps by name: `apps::open` for
  `welcome`, `terminal`, `launcher` and `about`; `studio::open` for
  `studio`, `studio:<path>` and any `*.app` path (a tier 0 applang app, run
  by `studio::AppHost`).
- The event path, one step per crate:

  ```
  DOM event → platform::Event → os → shell::Input → ui::AppEvent → app
  app → ui::Request → shell::Effect → os → platform::Ctl → browser call
  ```

- The Terminal is `vt` (parser) + `term` (screen) + a cell renderer. With no
  node it runs a built-in guest shell over the VFS; paired with a node it
  runs a real shell (see The node).

### Draw protocol

Apps don't draw pixels; they send a display list. Fixed-layout,
little-endian binary. Every message opens with a small header (version,
opcode, flags, length): check the version first and route before touching the
payload (the callosa pattern).

v0 commands: `Rect`, `RoundRect`, `Text` (a glyph run from the atlas),
`Image`, `PushClip`/`PopClip`, `PushTranslate`/`PopTranslate`.

Games and emulators can opt into a raw pixel surface instead.

The same bytes work over shared memory or a WebRTC data channel, so an app
can run on another device and draw here (a computehub hook).

### Windows: tiling

- Each workspace holds a binary tree: `Split { axis, ratio, a, b } | Leaf(window)`.
- A new window splits the focused one along its longer side (Pop Shell
  behavior).
- Dialogs, popups and desktop pets float.
- Keyboard-first: focus, move and launch from the keyboard.
- Integer layout math, unit-tested natively.

### Apps: two tiers

| | tier 0 | tier 1 |
|---|---|---|
| language | applang | anything that compiles to wasm (Rust SDK first) |
| runs | main thread, interpreted | pooled Worker |
| safety | provably halts; faults roll back; bounded memory | watchdog; Worker terminated if unresponsive |
| capabilities | none: its widgets are its whole world | declared in the manifest, granted as handles |
| draws | widget tree, rendered by the kernel | display list through a ring buffer |
| size | source text | kilobytes |
| written by | anyone, including an LLM (generate → verify → run) | developers |

### Syscalls

One `cap::CapTable` is the syscall interface. It drives the imports a tier 1
module receives, argument checks, generated docs, and a parity hash that the
worker host and kernel compare at startup. A mismatch refuses to run instead
of producing garbage.

Namespaces: `win`, `draw`, `input`, `fs`, `clip`, `time`; later `net`;
`compute` reserved.

Raw wasm has no ambient authority: a tier 1 module can only reach what its
import object contains.

### Storage

OPFS. Paths: `/home`, `/apps`, `/tmp`; later `/net/<peer>`. Desktop state is
snapshotted on change and restored at boot before anything else loads.

### Input

- The canvas receives pointer and wheel events.
- Keys arrive as key-downs by physical position (`KeyboardEvent.code`; by
  `KeyboardEvent.key` when a key reports no code, as phone keyboards do for
  Enter and Backspace), for bindings and for named keys (arrows, Enter, Tab,
  Ctrl+letter). A shortcut letter (Ctrl, Alt or Meta held, and
  `KeyboardEvent.key` an ASCII letter) goes by its meaning instead, so
  Ctrl+Z is Ctrl+Z on AZERTY and Dvorak; other layouts' letters (Cyrillic,
  macOS Option symbols) stay by position. The paste shortcut follows the
  same rule. A keypad key with NumLock off goes by its meaning too (the
  arrow, Home or Delete it names). Text never comes from key-downs: it
  comes only from the hidden `<textarea>` (see constitution rule 7), which
  `platform` focuses while the focused app wants text input. That one path
  covers typing, dead keys, IME composition, paste and phone keyboards.
- So while text input is on, `os` leaves a key-down that would type text
  (a printable key, a dead key, an IME or phone key, with no Ctrl, Alt or
  Meta) unprevented, and the browser puts its text in the textarea. Every
  other key the shell uses is prevented. Enter, Tab and Backspace stay keys.
- On phones the keyboard only opens from a user gesture: a tap that
  releases on the focused window asks for text input again, inside that
  gesture.
- Browsers keep Ctrl+W, Ctrl+T and Ctrl+N for themselves in a tab, so
  Ctrl+W (readline delete-word) closes the tab. While any socket to a node
  is open, `os` guards `beforeunload` (`Ctl::guard_unload`), so closing or
  reloading asks first. Chromium delivers those keys to an installed app's
  window, or in fullscreen with Keyboard Lock.
- Later: the app reports its caret rectangle so the IME popup lands in the
  right place, and copy goes through the async clipboard API.

## The node

A browser tab cannot start processes. The computehub node (`node/`, package
`computehub-node`) is a small native program that can: it gives compusophyOS
terminals a real shell on your machine (PowerShell, bash, `vim`, the
`claude` CLI). It is the first computehub building block: an authenticated,
message-based link from the OS to compute outside the browser. The mesh
grows from the same pattern.

- **Shape:** std threads, no async runtime. A synchronous WebSocket server
  on loopback. Each authenticated connection gets its own shell in its own
  pseudo-terminal (ConPTY on Windows, openpty on Unix): one PTY per
  connection. The shell dies with the connection.
- **Protocol** (`node/src/proto.rs` is the reference): one binary WebSocket
  message each. The first byte is the opcode; integers are little-endian.

  | op | name | direction | payload |
  |---|---|---|---|
  | 0x01 | HELLO | OS to node | version 1, the 32-byte token; first, within 5 s |
  | 0x02 | READY | node to OS | version, cols, rows (u16), `"<os>/<shell>"` |
  | 0x03 | DATA | both | shell input, or output (≤64 KiB per message) |
  | 0x04 | RESIZE | OS to node | cols, rows (u16) |
  | 0x05 | EXIT | node to OS | exit code (i32); then close 1000 |
  | 0x06 | ERROR | node to OS | a generic message; then close |

  DATA is a byte stream: a UTF-8 character or an escape sequence may be
  split across messages, so `vt` parses across message boundaries.
- **Pairing:** the node makes a fresh 256-bit token at each start and prints
  a link: `<page>/#node=<port>&token=<64 hex digits>`. The token is in the
  URL fragment, which browsers never send to a server. `os` takes the
  fragment first thing at start (`platform::take_location_hash`, before
  anything that can fail, so a token never stays in the address bar even
  when the desktop cannot start) and at each `hashchange` (the link opened
  in a tab already running the OS), clears it from the address bar with
  `history.replaceState`, and parses it (`ui::parse_pairing`). A pairing
  read before the desktop exists goes to the startup terminal; one read
  later opens a new terminal (`shell::Shell::set_pairing`), which connects
  with it. Restarting the node invalidates the token.
- **Security:** the node hands out a shell running as you, so the token is
  as sensitive as a password.
  - Loopback only: it binds 127.0.0.1, never another interface.
  - Origin allowlist: the WebSocket handshake needs an `Origin` header on
    the list (the `--url` page's origin, by default the deployed page, and
    `--allow-origin` extras; the local preview needs
    `--url http://localhost:8080`), else HTTP 403. Browsers attach it and
    pages cannot forge it, so other websites, DNS rebinding included, never
    reach a shell.
  - The token is compared in constant time.
  - Throttling: a wrong token is answered after 1 s with close code 4401.
    The delay holds only that connection: there is no lockout, since wrong
    tokens need no secret and a shared lockout would let any local program
    lock the user out. The handshake and HELLO share one 5 s deadline from
    TCP accept. At most 16 connections may wait to authenticate; a newcomer
    beyond that evicts the one that has waited longest. Sessions are capped
    (`--max-sessions`, 8 by default), and messages at 1 MiB.
  - Nothing identifying goes out: READY names only the OS and the shell's
    file name, and errors to clients are generic. The shell's own output
    (a prompt, a title) passes through unchanged.
  - The page: the deployment sends `Cross-Origin-Opener-Policy:
    same-origin` and `Content-Security-Policy: frame-ancestors 'none'`
    (`scripts/deploy.sh`), so no other site keeps a handle on its window
    (to change its fragment and pair it with a port of its choosing) or
    frames it. A pairing always shows its port in the terminal's title.
  - Out of scope: other programs already running as you.
- **Constitution:** the node is its own workspace with its own lockfile, so
  its native dependencies never enter the browser graph. It keeps the line
  cap and the license, and CI builds, tests and lints it in its own job.

## AI routing

Designed now, built after the OS basics. One syscall namespace, `ai`, with
two calls: `generate` (text and chat) and `embed` (vectors). A request names a
model and a route: `local`, `mesh` (computehub, later), `cloud`, or `auto`.
Every response carries a receipt like any cartridge run: model id, tokens,
fuel charged.

- **local (embedded):** open-weight models run in a Worker, on CPU wasm by
  default and WebGPU when available. They download on first use, are cached in
  OPFS, and are never part of the boot budget. Opt-in, because even small
  models are tens to hundreds of MB.
- **cloud:** Vercel AI Gateway through its OpenAI-compatible endpoints.
  Default chat model `zai/glm-5.3` (1M context); `zai/glm-5.3-flash` for cheap,
  fast calls. Model ids checked against `https://ai-gateway.vercel.sh/v1/models`
  on 2026-09-30; re-check before changing them.
- **auth:** no key ever ships to the browser. One thin Rust Vercel Function in
  `api/` proxies to the gateway, authenticating with the OIDC token Vercel
  passes in the `x-vercel-oidc-token` request header. Gateway budgets cap total
  spend; per-user limits are ours (fuel), because the gateway's `user` tags are
  for reporting and do not rate-limit. Optional BYOK: a user can paste their
  own gateway key, kept only in their browser.
- **embeddings:** vectors from different models are not comparable, so every
  stored vector records its model id. Prefer models that exist both as open
  weights and on the gateway (for example `alibaba/qwen3-embedding-0.6b`), so
  local and cloud produce the same vector space.
- **constitution exception:** server crates under `api/` may depend on the
  Vercel Rust runtime and an HTTP client. They never ship to the browser.

## Designed for computehub now

Cheap to build in now, expensive to retrofit:

1. **Determinism in the kernel:** state is replayable and verifiable by hash.
2. **Fuel and receipts on every run:** metering, payment and verification
   later.
3. **Location-transparent messages:** processes and windows can live on
   other devices.
4. **Content addressing:** apps and data are identified by hash, ready for
   mesh distribution and caching.
5. **Capabilities as handles:** access can be delegated to remote nodes
   safely.
6. **Reserved slots:** a `compute` syscall namespace and a node identity.

Computehub itself waits until the OS ships. Ideas on file: speculative
decoding across devices, layer pipelines (callosa), integer-only models that
are bit-exact on every device, models stored as a seed plus a small
coefficient list, and generate → verify → keep as the main mesh workload.

## Phases

0. (done) Workspace, constitution scripts, budgets in CI. Fork the litelite
   crates. The `wm` tiling tree with tests.
1. (in progress) Platform and compositor: canvas, WebGL2, rounded
   rectangles, runtime text, input. A desktop with panel, tiling and
   launcher.
2. (in progress) Tier 0: applang apps in windows (Studio); a terminal with
   a real shell through the computehub node.
3. Storage: OPFS-backed VFS and desktop snapshots.
4. Tier 1: Worker pool, ring buffers, syscall table, SDK, a Rust hello-world
   app.
5. Developer tools, packages, and the store (itself an app).

Then: identity, presence, economy, the shared world, computehub.

## Open questions

- **The Super key.** Browsers and operating systems grab it (on Windows it
  opens the Start menu, and Win+arrows snap the browser window). Chromium's
  Keyboard Lock API can capture it in fullscreen; elsewhere we need a
  browser-safe modifier. Which one is still undecided.
- **Alt in a terminal.** Alt is the wm's modifier, so Alt+F, Alt+Enter,
  Alt+Q, Alt+O, Alt+H/J/K/L, Alt+1..4 and Alt+Space never reach a terminal
  (readline's Alt+F moves a word forward). A way through for apps that
  want text input belongs to the shell's bindings, and waits on the
  modifier question above.

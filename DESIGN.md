# compusophyOS — design

Status: phase 0 in progress (workspace, constitution scripts, forked crates,
`wm`). Started 2026-09-30. Author: compusophy.

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
   build-time tools that never ship.
3. **≤2,000 lines per crate, ≤25,000 total.** At a cap: split, shrink or
   delete. Never raise the cap. (Inherited from litelite.)
4. **Budgets, enforced in CI:**

   | budget | target |
   |---|---|
   | whole OS over the wire, compressed | ≤150 KB |
   | hello-world app | <10 KB |
   | first frame after the wasm arrives | ≤100 ms |
   | idle CPU | zero: no frame is drawn unless something changed |
   | interaction | 60/120 fps, input handled in the same frame |

5. **Weak devices and bad networks are the baseline.** WebGL2 only (WebGPU
   never required). Works offline after the first load.
6. **Deterministic core.** Integer math, time passed in from outside, seeded
   randomness. Any kernel state can be replayed and hashed.

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
│ gfx       draw-protocol decoder, compositor, MSDF text   │
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
| `kernel` | processes, caps, VFS, input routing; composes `wm` | new, no web deps |
| `gfx` | draw protocol and compositor | new |
| `platform` | canvas, input, text-input bridge, OPFS, workers, clipboard | new |
| `os` | the wasm entry point | new |
| `sdk` | what tier 1 apps link against | new |
| `tools/atlas` | build-time MSDF font baker | new, never shipped |

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
- Text: an MSDF glyph atlas and a metrics table, baked at build time. No font
  engine ships. Latin first; other scripts later as optional modules.
- Apps read the same metrics table, so they lay out text without a font
  engine.

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

The canvas receives pointer events. One hidden, focused text element receives
keyboard and IME input; the kernel forwards it to the focused app, and the app
reports its caret rectangle so the IME popup lands in the right place.
Clipboard goes through the async clipboard API.

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

0. Workspace, constitution scripts, budgets in CI. Fork the litelite crates.
   The `wm` tiling tree with tests.
1. Platform and compositor: canvas, WebGL2, rounded rectangles, MSDF text,
   input. A desktop with panel, tiling and launcher.
2. Tier 0: applang apps in windows; a terminal.
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

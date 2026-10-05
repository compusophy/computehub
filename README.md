# compusophy

A computer in one browser tab.

compusophy is a desktop operating system that runs entirely in a web page: Rust compiled to
WebAssembly, drawing everything on one canvas. Floating windows, a home screen with folders, a
terminal with a real shell, files kept in your browser, and AI built in, free. Nothing to
install, and no server runs your apps: the tab is the machine.

**Try it:** <https://computehub-sigma.vercel.app>

## What's in it

- **The desktop.** Floating windows that snap, a home screen of every app, a dock you arrange,
  three themes (Mono, Midnight, Dawn), and touch on phones. Profiles keep separate homes in one
  browser, each with an optional PIN.
- **Programs.** Apps are WebAssembly programs (WASI preview 1), each in its own Web Worker,
  under a small kernel in the page: processes, a file system, consoles, jobs and pipes. A program
  with a window describes it as a widget tree; the desktop draws it.
- **Terminal.** A program drawing an xterm screen of `/bin/sh`: Unix-like commands, pipes (`a | b`), redirects
  (`<`, `>`, `>>`), history.
- **Agent.** Type `agent` in the Terminal: a coding agent on the free AI that reads, writes and
  edits your files, runs the shell and checks apps, asking before it writes or runs. It learns a
  lesson from each failure it gets past, and keeps it for next time.
- **Studio.** Make an app by describing it. A coding agent writes it in applang, a small
  language made for this, then tests and fixes it. Games, boards, drawings, clocks and charts
  draw on a canvas: shapes, and pixels for boards of squares.
- **Assistant.** An AI that uses the desktop as you do: it reads the screen, then clicks, types
  and opens apps. It reads and writes your files, and with your yes tells compusophy which tool
  it lacked.
- **Activity.** The resource monitor: CPU, memory, frames and AI use over the last minute, and
  a table of what runs.
- **Files, Editor, Settings, Feedback and About.**

## How it is built

- **Rust only.** No dependencies beyond the workspace, except `wasm-bindgen`, `js-sys` and
  `web-sys` at the browser boundary. Its own TrueType rasterizer, text layout, window manager,
  widgets, terminal emulator and shell.
- **One canvas.** Each frame is one instanced WebGL2 draw call. An idle desktop draws nothing
  but its living grain, 8 frames a second, which Settings can still.
- **Deterministic core.** The window manager, file system and kernel use no floats, clocks or
  randomness, so their state replays bit for bit.
- **Small by rule.** The boot download is at most 224 KB gzipped; everything else is a program
  fetched when it first runs. A module is at most 2,000 lines. `scripts/caps.sh` and
  `scripts/budget.sh` enforce both.
- **Server code only where a tab cannot go.** `api/` holds two functions: the proxy that makes
  the AI free (no key in the browser) and the feedback inbox.

| Path | What it holds |
|---|---|
| `crates/` | The OS: the desktop, window manager, widgets, text, kernel, program worker |
| `programs/` | What runs in it: Studio, the Assistant, the system apps, Activity, `sh`, applang |
| `api/` | The server functions (Vercel, Node) |
| `web/` | The page: a canvas and a one-line bootstrap |
| `scripts/` | Build, budget, caps and deploy |

`DESIGN.md` describes the architecture and what comes next.

## Build and run

You need Rust 1.85 or newer, the `wasm32-unknown-unknown` and `wasm32-wasip1` targets, and the
`wasm-bindgen` CLI at the version `Cargo.lock` pins.

```sh
rustup target add wasm32-unknown-unknown wasm32-wasip1
cargo install wasm-bindgen-cli --version 0.2.121 --locked
bash scripts/build-web.sh
cargo run -p serve --release -- dist 8080
```

Then open <http://localhost:8080>. Programs need a cross-origin isolated page, so the dev server
sends the COOP and COEP headers.

Checks:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/caps.sh
bash scripts/budget.sh
```

## License

Apache-2.0. The fonts (Inter, JetBrains Mono, Noto Sans Symbols) are under the SIL Open Font
License; see `assets/fonts/`. Made by compusophy.

# compusophyOS on Battlestation's monitor: the handoff

compusophyOS can now be mounted in another page as a cartridge, never in an iframe (DESIGN.md,
"The fractal: apps hold apps, the OS is a cartridge"; `crates/os/src/cartridge.rs`,
`crates/platform/src/mount.rs`). For secretspace's Battlestation:

## 1. Serve the OS's files beside the page

The OS's programs run in workers that share memory. A browser loads a worker only from the
page's own origin, and shares memory only in a cross-origin isolated page, so:

- Mirror computehub's built `dist/` under the game, e.g. `dist/battlestation/os/`: every path
  in computehub's `dist/files.txt` (os.js, os_bg.wasm, cpu/, bin/, fonts/, licenses/). From the
  live site: `https://computehub-sigma.vercel.app/files.txt`, then each path under the same
  origin; or build computehub (`bash scripts/build-web.sh`) and copy `dist/`.
- Serve Battlestation's page (and the mirrored files) with
  `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`
  (Vercel `headers` for `/battlestation/(.*)`). Everything the page loads must then be
  same-origin or send CORP, which mirrored files are.

## 2. Mount it

```js
import init, { Cartridge } from './os/os.js';
await init();                                    // os.js does nothing until mounted
const os = new Cartridge(1280, 720, 1, './os/', 'battlestation.');
```

`w, h` are the screen's CSS px (the OS lays out its desktop at that size), `dpr` its pixel
ratio (1 is right for a texture), `base` where the files are (ending in `/`), `ns` the storage
namespace (its profiles, files and links, apart from anything else in the origin). One mount a
page. It boots to its welcome: the player signs in on the virtual monitor.

## 3. Show its frames on the glass

The OS draws into an `OffscreenCanvas` (no page element) and schedules its own frames. Each of
your frames, when `os.frames()` changed since you last copied:

- WebGPU: `queue.copyExternalImageToTexture({ source: os.surface() }, { texture }, [os.width(), os.height()])`
  into the glass's texture (`RENDER_ATTACHMENT | COPY_DST`, rgba8unorm); or
- pixels: `os.pixels(u8)` with `u8 = new Uint8Array(os.width() * os.height() * 4)`, then
  `queue.write_texture` as the monitor does now (rows top first).

## 4. Give it input

Raycast the desk's mouse onto the screen quad; its UV times `(w, h)` is the point:

```js
os.pointer(0, x, y, 0);   // pressed (kind 0), button 0
os.pointer(1, x, y, 0);   // moved (with or without a button down)
os.pointer(2, x, y, 0);   // released
os.pointer(3, 0, 0, 0);   // left the screen
os.wheel(x, y, dy);       // dy in CSS px, down positive
os.key(down, e.code, e.key, mods, e.repeat); // mods: Shift 1, Ctrl 2, Alt 4, Meta 8, AltGr 16
if (down && os.typing() && e.key.length === 1 && !e.ctrlKey && !e.metaKey) os.text(e.key);
```

`pointer`, `wheel` and `key` return whether the OS took the event (then don't handle it too).
`os.cursor()` is the CSS cursor it wants over its screen (to draw the desk's arrow as it says).
`os.resize(w, h, dpr)` if the screen changes; `os.visible(false)` when the monitor is out of
view, `true` back (its clock catches up).

## What it does not have mounted

The OS's `/api/*` functions (the free AI, feedback, the mesh's signaling) are computehub's
origin's and refuse others, so a mounted OS has no AI and no pairing unless secretspace proxies
them. Everything else runs: the desktop, its apps and programs, the Terminal, Studio's editor,
the clocks (an app holding apps: Games, Clock shop), and the Monitor (Games, Monitor: the OS
inside itself, a desktop in a window, 3 deep), so the virtual computer can hold itself too.

Seen working (2026-10-10): a plain host page mounted the OS, signed in by its injected clicks,
opened the Clock shop, and showed its wall clock and grandfather clocks running.

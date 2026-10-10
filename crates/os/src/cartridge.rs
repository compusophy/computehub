//! The OS as a cartridge: the whole desktop mounted in another page, never in an iframe (see
//! `platform::mount`). A host page imports `os.js` (whose start does nothing without
//! `<canvas id="os">`), then `new Cartridge(w, h, dpr, base, ns)` boots the OS into an
//! `OffscreenCanvas` of its own, its files under `base` (beside the host's page, which must be
//! cross-origin isolated for its programs), its storage under `ns`. The host shows the frames as
//! it likes (a monitor's glass in a 3D room): it copies `surface()` (or reads `pixels()`) when
//! `frames()` moved, and gives the OS its input in the surface's CSS px: `pointer`, `wheel`,
//! `key` and, while `typing()`, the text typed. The OS cannot tell: it is the computer it always
//! is, signing in, opening windows, running its programs.

use platform::Event;
use wasm_bindgen::prelude::*;

/// The modifier bits of [`Cartridge::key`]: Shift, Ctrl, Alt, Meta (Command, Windows), AltGr.
const SHIFT: u8 = 1;
const CTRL: u8 = 2;
const ALT: u8 = 4;
const META: u8 = 8;
const ALTGR: u8 = 16;

// A plain comment: a doc comment would ship in os.js. The OS mounted in a host page (one a page).
#[wasm_bindgen]
pub struct Cartridge(());

#[wasm_bindgen]
impl Cartridge {
    // Boots the OS into a surface `w` x `h` CSS px at `dpr`, its files under `base` (ending in
    // `/`), its storage keys beginning `ns`.
    #[wasm_bindgen(constructor)]
    pub fn new(w: f32, h: f32, dpr: f32, base: String, ns: String) -> Result<Cartridge, JsValue> {
        report::install();
        platform::mount(crate::Desktop::new()?, platform::Mount { w, h, dpr, base, ns })?;
        Ok(Cartridge(()))
    }

    // The OffscreenCanvas the OS draws into; its last frame stays until the next.
    pub fn surface(&self) -> JsValue {
        platform::surface().map_or(JsValue::NULL, Into::into)
    }

    // How many frames the OS drew: copy the surface when it moved.
    pub fn frames(&self) -> u32 {
        platform::frames()
    }

    // The last frame's size in device px.
    pub fn width(&self) -> u32 {
        platform::backing().0
    }

    pub fn height(&self) -> u32 {
        platform::backing().1
    }

    // The last frame as RGBA rows, the top first, into `out` (width x height x 4 bytes).
    pub fn pixels(&self, out: &mut [u8]) -> bool {
        platform::pixels(out)
    }

    // The surface is now `w` x `h` CSS px at `dpr`.
    pub fn resize(&self, w: f32, h: f32, dpr: f32) {
        platform::resize_to(w, h, dpr);
    }

    // The pointer at `(x, y)` (the surface's CSS px): kind 0 pressed, 1 moved, 2 released, 3
    // left; `button` the DOM's (0 the main one). Whether the host should not handle it too.
    pub fn pointer(&self, kind: u8, x: f32, y: f32, button: u8) -> bool {
        platform::inject(match kind {
            0 => Event::PointerDown { x, y, button, touch: false },
            1 => Event::PointerMove { x, y },
            2 => Event::PointerUp { x, y, button },
            _ => Event::PointerLeave,
        })
    }

    // The wheel at `(x, y)`, `dy` CSS px down.
    pub fn wheel(&self, x: f32, y: f32, dy: f32) -> bool {
        platform::inject(Event::Wheel { x, y, dy })
    }

    // A key down or up: `code` and `key` as a KeyboardEvent has them, `mods` the bits Shift 1,
    // Ctrl 2, Alt 4, Meta 8, AltGr 16. Whether the host should not handle it too.
    pub fn key(&self, down: bool, code: String, key: String, mods: u8, repeat: bool) -> bool {
        let on = |bit| mods & bit != 0;
        let (shift, ctrl, alt, meta, altgr) = (on(SHIFT), on(CTRL), on(ALT), on(META), on(ALTGR));
        platform::inject(Event::Key { code, key, down, repeat, shift, ctrl, alt, meta, altgr })
    }

    // Text typed (or pasted) while `typing()`: what a keyboard's printable keys make.
    pub fn text(&self, text: String) {
        if !text.is_empty() {
            platform::inject(Event::Text(text));
        }
    }

    // Whether the OS takes typed text now (a field of it has the keys).
    pub fn typing(&self) -> bool {
        platform::typing()
    }

    // The CSS cursor the OS wants over its surface.
    pub fn cursor(&self) -> String {
        platform::cursor().into()
    }

    // Whether the host shows it (the monitor in view): shown again, its clock catches up.
    pub fn visible(&self, on: bool) {
        platform::visible(on);
    }
}

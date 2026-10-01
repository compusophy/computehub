//! compusophyOS: the wasm entry point.
//!
//! [`start`] hands [`platform::run`] a desktop made of the boot font, a
//! [`Vfs`] holding Studio's sample apps, and a [`Registry`]: [`apps::open`]
//! makes `welcome`, `terminal`, `launcher` and `about`; [`studio::open`]
//! makes `studio`, `studio:<path>` and any `*.app` path. The wm picks each
//! split's axis when a window opens, so the [`Shell`] and its startup
//! windows are created at the first Resize with room for the panel and some
//! desktop. Until then input is dropped (a Tick is replayed once the shell
//! exists) and frames only clear.
//!
//! # Fonts
//!
//! Three load groups, each with its own budget (`scripts/budget.sh`):
//!
//! - boot: Inter Regular, built into the wasm with `include_bytes!`. The
//!   first frame needs nothing else.
//! - deferred: Inter SemiBold and JetBrains Mono, fetched from
//!   `fonts/deferred/` right after the first frame and put into their
//!   [`TextSystem`] slots ([`TextSystem::set_font`]); then a frame is drawn.
//!   Until then, and for good if a fetch fails or a font does not parse
//!   (quietly: nothing is logged), bold text draws in Regular and monospace
//!   cells draw no glyphs. The terminal grid is the same either way.
//! - lazy: the symbol fallbacks, which the shell fetches from `fonts/` when
//!   a terminal first opens.
//!
//! The shell numbers its fetches and sockets up from 1, so the two deferred
//! fetches take the top two ids, `u32::MAX - 1` and `u32::MAX`. While a
//! deferred font is on its way, a result with its id is taken here; every
//! other fetch result goes to [`Shell::fetched`].
//!
//! # Pairing
//!
//! The location hash is taken first thing in [`start`], before anything
//! that can fail ([`platform::take_location_hash`]), and again at each
//! [`Event::HashChange`]. A hash that is not empty is removed from the
//! address bar at once, so a token does not linger there, not even when the
//! desktop cannot start (no WebGL2, say), and read as a computehub-node
//! pairing ([`ui::parse_pairing`]: `#node=<port>&token=<64 hex digits>`).
//! A pairing read before the shell exists goes to [`Shell::new`], so the
//! startup terminal connects with it; one read later goes to
//! [`Shell::set_pairing`], which opens a new terminal, focused, that
//! connects with it. A hash that pairs with nothing changes nothing else.
//!
//! # Leaving the page
//!
//! A node's shell dies with its connection, so while any socket is open
//! (reported open and not since closed) the page asks before it goes
//! ([`Ctl::guard_unload`]). That matters most for Ctrl+W: readline, Claude
//! Code and vim use it, but in a browser tab it closes the tab and never
//! reaches the terminal. In Chromium, installed as an app in its own
//! window or in fullscreen with Keyboard Lock, the terminal gets Ctrl+W.
//!
//! # Events
//!
//! - Key-downs, repeats too, become [`Input::Key`] by `KeyboardEvent.code`
//!   ([`Key::from_code`]), or by `KeyboardEvent.key` when a key reports no
//!   code (phone keyboards' Enter and Backspace, for one). A shortcut letter
//!   (Ctrl, Alt or Meta held, and `key` an ASCII letter) goes by its `key`,
//!   so Ctrl+Z is Ctrl+Z on AZERTY and Dvorak too; other layouts' letters
//!   (Cyrillic, macOS Option symbols) stay by position. A keypad key with
//!   NumLock off goes by the key it names (an arrow, Home, Delete). AltGr
//!   that also reports Ctrl and Alt (Chrome and Edge on Windows) types
//!   text, so such a key carries neither Ctrl nor Alt. Key-ups never reach
//!   the shell, but releasing Alt or Meta is `preventDefault`ed so Firefox
//!   does not focus its menu bar.
//! - Text, pointer and wheel events pass through. [`Event::Resize`] drops
//!   `dpr`: each frame gives the shell the renderer's. Every event and
//!   frame first gives the shell the page clock ([`Ctl::now_ms`], through
//!   [`Shell::set_now`]), so timeouts run between the minute ticks;
//!   [`Event::Tick`] carries it too. Socket events are handed on as they are.
//!
//! # Responses
//!
//! - [`Response::redraw`] requests a frame.
//! - [`Response::consumed`] becomes [`Handled::prevent_default`], except for
//!   a key-down that types text while text input is active: the text
//!   reaches apps only through the platform's textarea (as
//!   [`Event::Text`]), and a prevented key-down would never put it there.
//! - [`Response::text_input`] goes to [`Ctl::set_text_input`], and each
//!   [`Effect`] to the [`Ctl`] call of the same name.
//! - After every event and every frame, what the shell queued outside a
//!   response ([`Shell::take_effects`]) goes out too: a terminal's RESIZE
//!   after a frame changed its grid, for one.
//! - A primary-button release over the focused window while text input is
//!   active asks for text input again. That call runs inside the tap's user
//!   activation, which brings back a phone keyboard the user dismissed.

#![forbid(unsafe_code)]

use gfx::{DrawList, RectF, Rgba};
use platform::{App, Ctl, Event, Handled, Renderer};
use shell::{Effect, Input, Key, Mods, Registry, Response, Shell, WsEvent, theme};
use ui::{FontId, Pairing, TextSystem};
use vfs::Vfs;
use wasm_bindgen::prelude::*;

/// The boot font. The others are fetched: see the crate docs.
const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// The deferred fonts, as (fetch id, slot, URL): the top ids, which the
/// shell, counting up from 1, does not reach.
const DEFERRED: [(u32, FontId, &str); 2] = [
    (u32::MAX - 1, FontId::SansBold, "fonts/deferred/Inter-SemiBold.ttf"),
    (u32::MAX, FontId::Mono, "fonts/deferred/JetBrainsMono-Regular.ttf"),
];

/// The wasm entry point, run when the module is instantiated: starts the
/// desktop on `<canvas id="os">`.
///
/// # Errors
///
/// The boot font failing to load, or whatever [`platform::run`] reports:
/// no canvas, no WebGL2, bad shaders.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    // First: the hash may hold a token, which must leave the address bar
    // even if what follows fails.
    let hash = platform::take_location_hash();
    platform::run(Desktop::boot(&hash)?)
}

/// The shell once the canvas has a usable size, what it is made of until
/// then, and the draw list every frame reuses.
struct Desktop {
    /// The shell's text system and filesystem, until the shell takes them.
    parts: Option<(TextSystem, Vfs)>,
    shell: Option<Shell>,
    /// The node pairing for [`Shell::new`].
    pairing: Option<Pairing>,
    /// The sockets reported open and not since closed.
    open: Vec<u32>,
    /// Whether leaving the page asks first, as last asked of the platform.
    guarded: bool,
    /// Whether a Tick came before the shell did.
    missed_tick: bool,
    /// Whether text input is on, as last asked of the platform.
    typing: bool,
    /// Which [`DEFERRED`] fonts are on their way; `None` until the first
    /// frame asks for them.
    deferred: Option<[bool; 2]>,
    list: DrawList,
}

impl Desktop {
    /// A desktop waiting for its first usable size.
    fn new() -> Result<Desktop, String> {
        let text = TextSystem::new(SANS.to_vec())?;
        let mut vfs = Vfs::new();
        studio::install_samples(&mut vfs);
        Ok(Desktop {
            parts: Some((text, vfs)),
            shell: None,
            pairing: None,
            open: Vec::new(),
            guarded: false,
            missed_tick: false,
            typing: false,
            deferred: None,
            list: DrawList::new(),
        })
    }

    /// A desktop paired by the location `hash` that [`start`] took (and
    /// cleared) before anything else.
    fn boot(hash: &str) -> Result<Desktop, String> {
        let mut desk = Desktop::new()?;
        desk.pairing = ui::parse_pairing(hash);
        Ok(desk)
    }

    /// A hash change's `hash`: cleared from the address bar if not empty,
    /// and read as a pairing, which waits for [`Shell::new`] or goes to
    /// [`Shell::set_pairing`] (whose response this is).
    fn pair(&mut self, hash: &str, ctl: &mut Ctl) -> Option<Response> {
        if hash.is_empty() {
            return None;
        }
        ctl.clear_location_hash();
        let p = ui::parse_pairing(hash)?;
        match &mut self.shell {
            Some(shell) => Some(shell.set_pairing(p)),
            None => {
                self.pairing = Some(p);
                None
            }
        }
    }

    /// Hands `input` to the shell, first making it at the first usable
    /// size; `None` while there is no shell.
    fn input(&mut self, input: Input, ctl: &Ctl) -> Option<Response> {
        if let Some(shell) = &mut self.shell {
            return Some(shell.input(input));
        }
        match input {
            Input::Resize { w, h } if w >= 1.0 && h >= theme::PANEL_H + 1.0 => {
                let (text, vfs) = self.parts.take()?;
                let shell = Shell::new(w, h, text, vfs, registry(), self.pairing);
                let shell = self.shell.insert(shell);
                let mut r = shell.input(input);
                if self.missed_tick {
                    let (minutes, now_ms) = (ctl.local_minutes(), ctl.now_ms());
                    r = merge(r, shell.input(Input::Tick { minutes, now_ms }));
                }
                Some(r)
            }
            Input::Tick { .. } => {
                self.missed_tick = true;
                None
            }
            _ => None,
        }
    }

    /// One event, but for what the shell queued outside its response.
    fn handle(&mut self, ev: Event, ctl: &mut Ctl) -> Handled {
        if let Some(shell) = &mut self.shell {
            shell.set_now(ctl.now_ms());
        }
        if let Event::Ws { id, ev } = &ev {
            match ev {
                platform::WsEvent::Open => self.open.push(*id),
                platform::WsEvent::Closed { .. } => self.open.retain(|o| o != id),
                _ => {}
            }
        }
        let types = types_text(&ev);
        let release = match ev {
            Event::PointerUp { x, y, button: 0 } => Some((x, y)),
            _ => None,
        };
        let r = match ev {
            Event::Key { down: false, ref code, .. } => return key_up(code),
            Event::HashChange(hash) => self.pair(&hash, ctl),
            Event::Fetched { id, result } => match self.take_deferred(id) {
                Some(slot) => return self.set_font(slot, result),
                None => self.shell.as_mut().map(|s| s.fetched(id, result)),
            },
            ev => input_of(ev, ctl).and_then(|input| self.input(input, ctl)),
        };
        let Some(r) = r else {
            return Handled::default();
        };
        let asked = r.text_input.is_some();
        let mut h = self.apply(r, ctl);
        h.prevent_default &= !(types && self.typing);
        if release.is_some_and(|(x, y)| !asked && self.typing && self.over_focus(x, y)) {
            ctl.set_text_input(true);
        }
        h
    }

    /// Passes the shell's requests to the platform; the answer to the event.
    fn apply(&mut self, r: Response, ctl: &mut Ctl) -> Handled {
        r.effects.into_iter().for_each(|fx| self.effect(fx, ctl));
        if let Some(on) = r.text_input {
            self.typing = on;
            ctl.set_text_input(on);
        }
        Handled { redraw: r.redraw, prevent_default: r.consumed }
    }

    /// Passes on what the shell queued outside a response, then guards
    /// leaving the page while a socket is open (or stops).
    fn flush(&mut self, ctl: &mut Ctl) {
        let queued = self.shell.as_mut().map(Shell::take_effects);
        for fx in queued.into_iter().flatten() {
            self.effect(fx, ctl);
        }
        let live = !self.open.is_empty();
        if self.guarded != live {
            self.guarded = live;
            ctl.guard_unload(live);
        }
    }

    /// Hands one shell effect to the page. A socket opened (again) or closed
    /// here reports nothing more, so it is no longer open.
    fn effect(&mut self, fx: Effect, ctl: &mut Ctl) {
        match fx {
            Effect::WsOpen { id, url } => {
                self.open.retain(|&o| o != id);
                ctl.ws_open(id, &url);
            }
            Effect::WsSend { id, bytes } => ctl.ws_send(id, &bytes),
            Effect::WsClose { id } => {
                self.open.retain(|&o| o != id);
                ctl.ws_close(id);
            }
            Effect::Fetch { id, url } => ctl.fetch(id, &url),
        }
    }

    /// Draws a frame into the draw list (the shell at `dpr` and the page
    /// clock, or nothing before it exists); its clear color.
    fn paint(&mut self, dpr: f32, ctl: &Ctl) -> Rgba {
        let Some(shell) = &mut self.shell else {
            self.list.clear();
            return theme::BG;
        };
        shell.set_now(ctl.now_ms());
        shell.set_dpr(dpr);
        shell.draw(&mut self.list);
        shell.clear_color()
    }

    /// After a frame: asks for the deferred fonts after the first, and
    /// passes on what the shell queued while drawing.
    fn drawn(&mut self, ctl: &mut Ctl) {
        if self.deferred.is_none() {
            self.deferred = Some([true; 2]);
            for (id, _, url) in DEFERRED {
                ctl.fetch(id, url);
            }
        }
        self.flush(ctl);
    }

    /// The slot of the deferred font that fetch `id` brings, if it is still
    /// on its way (from now on it is not).
    fn take_deferred(&mut self, id: u32) -> Option<FontId> {
        let waiting = self.deferred.as_mut()?;
        let i = DEFERRED.iter().position(|d| d.0 == id)?;
        std::mem::take(&mut waiting[i]).then_some(DEFERRED[i].1)
    }

    /// A deferred font arrived (or failed): fills its slot and draws again,
    /// or, quietly, leaves the slot empty.
    fn set_font(&mut self, slot: FontId, got: Result<Vec<u8>, String>) -> Handled {
        let text = text_of(&mut self.shell, &mut self.parts);
        let redraw = match (got, text) {
            (Ok(bytes), Some(text)) => text.set_font(slot, bytes).is_ok(),
            _ => false,
        };
        Handled { redraw, prevent_default: false }
    }

    /// Whether the focused window is the topmost one at `(x, y)`.
    fn over_focus(&self, x: f32, y: f32) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        let layout = shell.wm().layout();
        let r = |p: &&wm::Placement| RectF::from_i32(p.rect.x, p.rect.y, p.rect.w, p.rect.h);
        let top = layout.iter().rev().find(|p| r(p).contains(x, y));
        top.is_some_and(|p| p.focused)
    }
}

impl App for Desktop {
    fn event(&mut self, ev: Event, ctl: &mut Ctl) -> Handled {
        let h = self.handle(ev, ctl);
        self.flush(ctl);
        h
    }

    fn frame(&mut self, r: &mut Renderer, ctl: &mut Ctl) {
        let bg = self.paint(r.dpr(), ctl);
        if let Some(text) = text_of(&mut self.shell, &mut self.parts) {
            r.draw(&self.list, bg, text.atlas_mut());
        }
        self.drawn(ctl);
    }
}

/// The text system: the shell's, or the one waiting for it.
fn text_of<'a>(
    shell: &'a mut Option<Shell>,
    parts: &'a mut Option<(TextSystem, Vfs)>,
) -> Option<&'a mut TextSystem> {
    match shell {
        Some(shell) => Some(shell.text_mut()),
        None => parts.as_mut().map(|p| &mut p.0),
    }
}

/// Makes apps by name: the built-ins, then Studio and `.app` files.
fn registry() -> Registry {
    Box::new(|name| apps::open(name).or_else(|| studio::open(name)))
}

/// `a` then `b`, as one response.
fn merge(mut a: Response, b: Response) -> Response {
    a.redraw |= b.redraw;
    a.consumed |= b.consumed;
    a.text_input = b.text_input.or(a.text_input);
    a.effects.extend(b.effects);
    a
}

/// The shell input for a platform event; `None` for a key-up, a hash change
/// or a fetch result, which do not go through [`Shell::input`].
fn input_of(ev: Event, ctl: &Ctl) -> Option<Input> {
    Some(match ev {
        Event::Key { down: false, .. } | Event::HashChange(_) | Event::Fetched { .. } => {
            return None;
        }
        Event::Key { code, key, shift, ctrl, alt, meta, altgr, .. } => {
            let (ctrl, alt) = without_altgr(ctrl, alt, altgr);
            let mods = Mods { shift, ctrl, alt, meta };
            Input::Key { key: key_of(&code, &key, ctrl || alt || meta), mods }
        }
        Event::Text(s) => Input::Text(s),
        Event::PointerMove { x, y } => Input::PointerMove { x, y },
        Event::PointerDown { x, y, button } => Input::PointerDown { x, y, button },
        Event::PointerUp { x, y, button } => Input::PointerUp { x, y, button },
        Event::PointerLeave => Input::PointerLeave,
        Event::Wheel { x, y, dy } => Input::Wheel { x, y, dy },
        Event::Resize { w, h, .. } => Input::Resize { w, h },
        Event::Tick { minutes } => Input::Tick { minutes, now_ms: ctl.now_ms() },
        Event::Ws { id, ev } => Input::Ws {
            id,
            ev: match ev {
                platform::WsEvent::Open => WsEvent::Open,
                platform::WsEvent::Data(bytes) => WsEvent::Data(bytes),
                platform::WsEvent::Closed { code, reason } => WsEvent::Closed { code, reason },
                platform::WsEvent::Error => WsEvent::Error,
            },
        },
    })
}

/// The shell key for a key-down. A `chord` (Ctrl, Alt or Meta held) whose
/// `key` is an ASCII letter is that letter, as the layout says: AZERTY's Z
/// sits at `KeyW`, and Ctrl+Z must not send Ctrl+W. A keypad digit or
/// decimal key whose `key` names a key (NumLock off: `ArrowUp`, `Home`,
/// `Delete` and the rest) is that key. Otherwise by its `code`
/// ([`Key::from_code`]; Cyrillic letters and macOS Option symbols too), or,
/// when it reports none (phone keyboards send Enter and Backspace so, as do
/// some remote desktops and synthetic events), by its `key`: a letter or
/// digit as [`Key::Char`], `" "` as [`Key::Space`], and named keys by name
/// (`Enter`, `ArrowLeft`, `F5` and the rest are spelled as their codes).
fn key_of(code: &str, key: &str, chord: bool) -> Key {
    let mut chars = key.chars();
    let one = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    if let Some(c) = one.filter(|c| chord && c.is_ascii_alphabetic()) {
        return Key::Char(c.to_ascii_lowercase());
    }
    let pad = code.strip_prefix("Numpad").is_some_and(|k| k.len() == 1 || k == "Decimal");
    if pad && Key::from_code(key) != Key::Other {
        return Key::from_code(key);
    }
    if !matches!(code, "" | "Unidentified") {
        return Key::from_code(code);
    }
    match one {
        Some(' ') => Key::Space,
        Some(c) if c.is_ascii_alphanumeric() => Key::Char(c.to_ascii_lowercase()),
        _ => Key::from_code(key),
    }
}

/// Ctrl and Alt as the shell sees them: AltGr that also reports both types
/// text, so it holds neither.
fn without_altgr(ctrl: bool, alt: bool, altgr: bool) -> (bool, bool) {
    let text = altgr && ctrl && alt;
    (ctrl && !text, alt && !text)
}

/// Whether a key-down types into a focused textarea unless prevented: its
/// `KeyboardEvent.key` is text (not a named key such as `Enter` or `F5`),
/// or a dead key, IME or phone keyboard owns it (`Dead`, `Process`,
/// `Unidentified`); and no Ctrl, Alt or Meta is held (AltGr is none).
fn types_text(ev: &Event) -> bool {
    let Event::Key { key, down: true, ctrl, alt, meta, altgr, .. } = ev else {
        return false;
    };
    let (ctrl, alt) = without_altgr(*ctrl, *alt, *altgr);
    // Named key values are ASCII words; text is one character or a
    // non-ASCII cluster.
    let named = key.len() > 1 && key.bytes().all(|b| b.is_ascii_alphanumeric());
    let text = !key.is_empty() && !named && !key.contains(char::is_control);
    let owned = matches!(key.as_str(), "Dead" | "Process" | "Unidentified");
    !(ctrl || alt || *meta) && (text || owned)
}

/// A key-up draws nothing; releasing Alt or Meta is still prevented.
fn key_up(code: &str) -> Handled {
    Handled {
        redraw: false,
        prevent_default: matches!(code, "AltLeft" | "AltRight" | "MetaLeft" | "MetaRight"),
    }
}

#[cfg(test)]
mod tests;

//! compusophyOS: the wasm entry point.
//!
//! [`start`] hands [`platform::run`] a desktop made of the boot font, a
//! [`Vfs`] holding Studio's sample apps, and a [`Registry`]: [`apps::open`]
//! makes `welcome`, `terminal` and `settings`; [`studio::open`] makes
//! `studio`, `studio:<path>` and any `*.app` path. The [`Shell`] opens its
//! startup window centered in the work area, so it is created at the first
//! Resize that leaves a work area (taller than the bar and the dock's
//! clearance). Until then input is dropped (a Tick is replayed once the
//! shell exists) and frames clear to the default theme's base.
//!
//! # Theme
//!
//! The shell starts in the theme `localStorage` names under [`THEME_KEY`]
//! (the default one when it names none). After every event and frame, a
//! theme other than the one last stored is stored, so a reload comes back
//! to it. Storage is a convenience: when it is blocked, the theme simply
//! starts at the default.
//!
//! # Time and frames
//!
//! Before every event and frame the shell gets the monotonic page clock
//! ([`Ctl::monotonic_ms`], through [`Shell::set_now`]): animations and app
//! timeouts run on it. [`Event::Tick`] carries the local date and time for
//! the top bar's clock. A frame whose [`Shell::draw`] reports a running
//! animation asks for the next one ([`Ctl::request_frame`]); the first frame
//! that does not is the last until the next input, so an idle desktop draws
//! nothing.
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
//! The shell numbers its fetches up from 1, so the two deferred fetches take
//! the top two ids, `u32::MAX - 1` and `u32::MAX`. While a deferred font is
//! on its way, a result with its id is taken here; every other fetch result
//! goes to [`Shell::fetched`].
//!
//! # Events
//!
//! - Key-downs, repeats too, become [`Input::Key`] by `KeyboardEvent.code`
//!   ([`Key::from_code`], and `Backquote` as a backquote [`Key::Char`] for
//!   the window switcher), or by `KeyboardEvent.key` when a key reports no
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
//!   `dpr`: each frame gives the shell the renderer's.
//!
//! # Responses
//!
//! - [`Response::redraw`] (or [`Response::animating`]) requests a frame.
//! - [`Response::consumed`] becomes [`Handled::prevent_default`], except for
//!   a key-down that types text while text input is active: the text
//!   reaches apps only through the platform's textarea (as
//!   [`Event::Text`]), and a prevented key-down would never put it there.
//! - [`Response::text_input`] goes to [`Ctl::set_text_input`],
//!   [`Response::cursor`] to [`Ctl::set_cursor`], and each [`Effect`] to the
//!   [`Ctl`] call of the same name.
//! - After every event and every frame, what the shell queued outside a
//!   response ([`Shell::take_effects`]) goes out too: what an app asked for
//!   while a frame was drawn, for one.
//! - A primary-button release over the focused window while text input is
//!   active asks for text input again. That call runs inside the tap's user
//!   activation, which brings back a phone keyboard the user dismissed.

#![forbid(unsafe_code)]

use gfx::{DrawList, RectF, Rgba};
use platform::{App, Ctl, Event, Handled, Renderer};
use shell::{Cursor, Effect, Input, Key, LocalTime, Mods, Registry, Response, Shell};
use ui::{FontId, TextSystem};
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

/// The `localStorage` key that keeps the theme's name.
pub const THEME_KEY: &str = "compusophy.theme";

/// The wasm entry point, run when the module is instantiated: starts the
/// desktop on `<canvas id="os">`.
///
/// # Errors
///
/// The boot font failing to load, or whatever [`platform::run`] reports:
/// no canvas, no WebGL2, bad shaders.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    platform::run(Desktop::new()?)
}

/// The shell once the canvas has a usable size, what it is made of until
/// then, and the draw list every frame reuses.
struct Desktop {
    /// The shell's text system and filesystem, until the shell takes them.
    parts: Option<(TextSystem, Vfs)>,
    shell: Option<Shell>,
    /// Whether a Tick came before the shell did.
    missed_tick: bool,
    /// Whether text input is on, as last asked of the platform.
    typing: bool,
    /// Which [`DEFERRED`] fonts are on their way; `None` until the first
    /// frame asks for them.
    deferred: Option<[bool; 2]>,
    /// The theme's name as storage last had it (or would have, at start).
    saved: &'static str,
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
            missed_tick: false,
            typing: false,
            deferred: None,
            saved: ui::THEMES[0].name,
            list: DrawList::new(),
        })
    }

    /// Hands `input` to the shell, first making it at the first usable
    /// size; `None` while there is no shell.
    fn input(&mut self, input: Input, ctl: &Ctl) -> Option<Response> {
        if let Some(shell) = &mut self.shell {
            return Some(shell.input(input));
        }
        match input {
            Input::Resize { w, h } if w >= 1.0 && h >= shell::BAR_H + shell::DOCK_CLEAR + 1.0 => {
                let (text, vfs) = self.parts.take()?;
                let theme = ctl.storage_get(THEME_KEY).unwrap_or_default();
                let shell = Shell::new(w, h, text, vfs, registry(), &theme);
                let shell = self.shell.insert(shell);
                self.saved = shell.theme_name();
                shell.set_now(ctl.monotonic_ms());
                let mut r = shell.input(input);
                if self.missed_tick {
                    r = merge(r, shell.input(Input::Tick { time: local(ctl.local_time()) }));
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
            shell.set_now(ctl.monotonic_ms());
        }
        let types = types_text(&ev);
        let release = match ev {
            Event::PointerUp { x, y, button: 0 } => Some((x, y)),
            _ => None,
        };
        let r = match ev {
            Event::Key { down: false, ref code, .. } => return key_up(code),
            Event::Fetched { id, result } => match self.take_deferred(id) {
                Some(slot) => return self.set_font(slot, result),
                None => self.shell.as_mut().map(|s| s.fetched(id, result)),
            },
            ev => input_of(ev).and_then(|input| self.input(input, ctl)),
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
        r.effects.into_iter().for_each(|fx| effect(fx, ctl));
        if let Some(on) = r.text_input {
            self.typing = on;
            ctl.set_text_input(on);
        }
        if let Some(c) = r.cursor {
            ctl.set_cursor(cursor(c));
        }
        Handled { redraw: r.redraw || r.animating, prevent_default: r.consumed }
    }

    /// Passes on what the shell queued outside a response, and stores a
    /// theme that changed.
    fn flush(&mut self, ctl: &mut Ctl) {
        let Some(shell) = &mut self.shell else {
            return;
        };
        for fx in shell.take_effects() {
            effect(fx, ctl);
        }
        let theme = shell.theme_name();
        if theme != self.saved {
            self.saved = theme;
            ctl.storage_set(THEME_KEY, theme);
        }
    }

    /// Draws a frame into the draw list (the shell at `dpr` and the page
    /// clock, or nothing before it exists); its clear color, and whether an
    /// animation wants the next frame.
    fn paint(&mut self, dpr: f32, ctl: &Ctl) -> (Rgba, bool) {
        let Some(shell) = &mut self.shell else {
            self.list.clear();
            return (ui::THEMES[0].base, false);
        };
        shell.set_now(ctl.monotonic_ms());
        shell.set_dpr(dpr);
        let animating = shell.draw(&mut self.list);
        (shell.clear_color(), animating)
    }

    /// After a frame: asks for the next while an animation runs, for the
    /// deferred fonts after the first, and passes on what the shell queued
    /// while drawing.
    fn drawn(&mut self, animating: bool, ctl: &mut Ctl) {
        if animating {
            ctl.request_frame();
        }
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
        let (bg, animating) = self.paint(r.dpr(), ctl);
        if let Some(text) = text_of(&mut self.shell, &mut self.parts) {
            r.draw(&self.list, bg, text.atlas_mut());
        }
        self.drawn(animating, ctl);
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

/// Hands one shell effect to the page.
fn effect(fx: Effect, ctl: &mut Ctl) {
    let Effect::Fetch { id, url } = fx;
    ctl.fetch(id, &url);
}

/// The page's cursor for the shell's.
fn cursor(c: Cursor) -> platform::Cursor {
    use platform::Cursor as P;
    match c {
        Cursor::Default => P::Default,
        Cursor::Text => P::Text,
        Cursor::Grab => P::Grab,
        Cursor::Grabbing => P::Grabbing,
        Cursor::EwResize => P::EwResize,
        Cursor::NsResize => P::NsResize,
        Cursor::NwseResize => P::NwseResize,
        Cursor::NeswResize => P::NeswResize,
    }
}

/// The shell's local time for the page's.
fn local(t: platform::LocalTime) -> LocalTime {
    let platform::LocalTime { year, month, day, weekday, hour, minute } = t;
    LocalTime { year, month, day, weekday, hour, minute }
}

/// Makes apps by name: the built-ins, then Studio and `.app` files.
fn registry() -> Registry {
    Box::new(|name| apps::open(name).or_else(|| studio::open(name)))
}

/// `a` then `b`, as one response.
fn merge(mut a: Response, b: Response) -> Response {
    a.redraw |= b.redraw;
    a.consumed |= b.consumed;
    a.animating |= b.animating;
    a.text_input = b.text_input.or(a.text_input);
    a.cursor = b.cursor.or(a.cursor);
    a.effects.extend(b.effects);
    a
}

/// The shell input for a platform event; `None` for a key-up or a fetch
/// result, which do not go through [`Shell::input`].
fn input_of(ev: Event) -> Option<Input> {
    Some(match ev {
        Event::Key { down: false, .. } | Event::Fetched { .. } => return None,
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
        Event::Tick { time } => Input::Tick { time: local(time) },
    })
}

/// The shell key for a key-down. A `chord` (Ctrl, Alt or Meta held) whose
/// `key` is an ASCII letter is that letter, as the layout says: AZERTY's Z
/// sits at `KeyW`, and Ctrl+Z must not send Ctrl+W. A keypad digit or
/// decimal key whose `key` names a key (NumLock off: `ArrowUp`, `Home`,
/// `Delete` and the rest) is that key. `Backquote`, the key above Tab, is a
/// backquote on every layout, for the window switcher. Otherwise by its
/// `code` ([`Key::from_code`]; Cyrillic letters and macOS Option symbols
/// too), or, when it reports none (phone keyboards send Enter and Backspace
/// so, as do some remote desktops and synthetic events), by its `key`: a
/// letter, digit or backquote as [`Key::Char`], `" "` as [`Key::Space`], and
/// named keys by name (`Enter`, `ArrowLeft`, `F5` and the rest are spelled
/// as their codes).
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
    match code {
        "Backquote" => Key::Char('`'),
        "" | "Unidentified" => match one {
            Some(' ') => Key::Space,
            Some(c) if c.is_ascii_alphanumeric() || c == '`' => Key::Char(c.to_ascii_lowercase()),
            _ => Key::from_code(key),
        },
        code => Key::from_code(code),
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

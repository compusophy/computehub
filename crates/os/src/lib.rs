//! compusophyOS: the wasm entry point.
//!
//! `start` runs a desktop on [`platform::run`]: the boot font, a [`Vfs`] with
//! Studio's samples, and a [`Registry`] of [`apps::open`] then
//! [`studio::open`]. The [`Shell`] is made at the first Resize that leaves a
//! work area; until then input is dropped (a missed Tick is replayed) and
//! frames clear to the default theme's base. The theme starts as
//! `localStorage` names it under [`THEME_KEY`] and is stored when it changes.
//!
//! Fonts: boot (Inter Regular, in the wasm); deferred (Inter SemiBold and
//! JetBrains Mono, fetched after the first frame under the top two fetch ids,
//! which the shell, counting from 1, never reaches; a failure quietly leaves
//! bold as Regular and mono cells empty); lazy (symbol fallbacks the shell
//! fetches). A key-down that types text is never prevented while text input
//! is on: text reaches apps only through the platform's textarea.

#![forbid(unsafe_code)]

use gfx::{DrawList, RectF, Rgba};
use platform::{App, Ctl, Event, Handled, Renderer};
use shell::{Cursor, Effect, Input, Key, LocalTime, Mods, Registry, Response, Shell};
use ui::{FontId, TextSystem};
use vfs::Vfs;
use wasm_bindgen::prelude::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// The deferred fonts as (fetch id, slot, URL).
const DEFERRED: [(u32, FontId, &str); 2] = [
    (u32::MAX - 1, FontId::SansBold, "fonts/deferred/Inter-SemiBold.ttf"),
    (u32::MAX, FontId::Mono, "fonts/deferred/JetBrainsMono-Regular.ttf"),
];

/// The `localStorage` key that keeps the theme's name.
pub const THEME_KEY: &str = "compusophy.theme";

// The wasm entry point. A plain comment: a doc comment would ship in os.js.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    platform::run(Desktop::new()?)
}

struct Desktop {
    /// The text system and filesystem, until the shell takes them.
    parts: Parts,
    shell: Option<Shell>,
    /// Whether a Tick came before the shell did.
    missed_tick: bool,
    /// Whether text input is on, as last asked of the platform.
    typing: bool,
    /// Which [`DEFERRED`] fonts are on their way; `None` before the first frame.
    deferred: Option<[bool; 2]>,
    /// The theme's name as storage has it.
    saved: &'static str,
    list: DrawList,
}

impl Desktop {
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

    /// Hands `input` to the shell, first making it at the first usable size.
    fn input(&mut self, input: Input, ctl: &Ctl) -> Option<Response> {
        if let Some(shell) = &mut self.shell {
            return Some(shell.input(input));
        }
        self.missed_tick |= matches!(input, Input::Tick { .. });
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
            _ => None,
        }
    }

    /// One event, but for what the shell queued outside its response.
    fn handle(&mut self, ev: Event, ctl: &mut Ctl) -> Handled {
        if let Some(shell) = &mut self.shell {
            shell.set_now(ctl.monotonic_ms());
        }
        let types = types_text(&ev);
        let release =
            if let Event::PointerUp { x, y, button: 0 } = ev { Some((x, y)) } else { None };
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
        r.effects.into_iter().for_each(|fx| effect(fx, ctl));
        if let Some(on) = r.text_input {
            self.typing = on;
            ctl.set_text_input(on);
        }
        if let Some(c) = r.cursor {
            ctl.set_cursor(cursor(c));
        }
        // A tap on the focused window while typing asks for text input again,
        // inside its user activation: that brings back a dismissed keyboard.
        if release.is_some_and(|(x, y)| !asked && self.typing && self.over_focus(x, y)) {
            ctl.set_text_input(true);
        }
        let prevent_default = r.consumed && !(types && self.typing);
        Handled { redraw: r.redraw || r.animating, prevent_default }
    }

    /// Passes on what the shell queued outside a response, and stores a
    /// theme that changed.
    fn flush(&mut self, ctl: &mut Ctl) {
        let Some(shell) = &mut self.shell else {
            return;
        };
        shell.take_effects().into_iter().for_each(|fx| effect(fx, ctl));
        let theme = shell.theme_name();
        if theme != self.saved {
            self.saved = theme;
            ctl.storage_set(THEME_KEY, theme);
        }
    }

    /// Draws into the draw list; the clear color, and whether an animation runs.
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

    /// After a frame: the next while animating, the deferred fonts after the
    /// first, and what the shell queued while drawing.
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

    /// The slot of the deferred font fetch `id` brings, if still awaited.
    fn take_deferred(&mut self, id: u32) -> Option<FontId> {
        let waiting = self.deferred.as_mut()?;
        let i = DEFERRED.iter().position(|d| d.0 == id)?;
        std::mem::take(&mut waiting[i]).then_some(DEFERRED[i].1)
    }

    /// Fills a deferred font's slot and redraws, or quietly leaves it empty.
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
        let layout = self.shell.as_ref().map(|s| s.wm().layout()).unwrap_or_default();
        let r = |p: &&wm::Placement| RectF::from_i32(p.rect.x, p.rect.y, p.rect.w, p.rect.h);
        layout.iter().rev().find(|p| r(p).contains(x, y)).is_some_and(|p| p.focused)
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

type Parts = Option<(TextSystem, Vfs)>;

/// The text system: the shell's, or the one waiting for it.
fn text_of<'a>(shell: &'a mut Option<Shell>, parts: &'a mut Parts) -> Option<&'a mut TextSystem> {
    match shell {
        Some(shell) => Some(shell.text_mut()),
        None => parts.as_mut().map(|p| &mut p.0),
    }
}

fn effect(fx: Effect, ctl: &mut Ctl) {
    let Effect::Fetch { id, url } = fx;
    ctl.fetch(id, &url);
}

/// The CSS `cursor` keyword for the shell's cursor.
fn cursor(c: Cursor) -> &'static str {
    match c {
        Cursor::Default => "default",
        Cursor::Text => "text",
        Cursor::Grab => "grab",
        Cursor::Grabbing => "grabbing",
        Cursor::EwResize => "ew-resize",
        Cursor::NsResize => "ns-resize",
        Cursor::NwseResize => "nwse-resize",
        Cursor::NeswResize => "nesw-resize",
    }
}

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

/// The shell input for an event; `None` for key-ups and fetch results.
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

/// The shell key for a key-down. With a `chord` (Ctrl, Alt or Meta) an ASCII
/// letter `key` is that letter (AZERTY's Ctrl+Z is not Ctrl+W); a keypad key
/// whose `key` names a key (NumLock off) is that key; `Backquote` is a
/// backquote everywhere. Otherwise by `code`, or with no code (phone
/// keyboards) by `key`: letters, digits and backquote as chars, `" "` as
/// Space, named keys by name.
fn key_of(code: &str, key: &str, chord: bool) -> Key {
    let one = key.chars().next().filter(|c| c.len_utf8() == key.len());
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

/// Ctrl and Alt as the shell sees them: AltGr reported as both types text.
fn without_altgr(ctrl: bool, alt: bool, altgr: bool) -> (bool, bool) {
    let text = altgr && ctrl && alt;
    (ctrl && !text, alt && !text)
}

/// Whether a key-down types into a focused textarea unless prevented: its
/// `key` is text or owned by a dead key, IME or phone keyboard, and no Ctrl,
/// Alt or Meta is held (AltGr is none).
fn types_text(ev: &Event) -> bool {
    let Event::Key { key, down: true, ctrl, alt, meta, altgr, .. } = ev else {
        return false;
    };
    let (ctrl, alt) = without_altgr(*ctrl, *alt, *altgr);
    // Named key values are ASCII words; text is one character or a cluster.
    let named = key.len() > 1 && key.bytes().all(|b| b.is_ascii_alphanumeric());
    let text = !key.is_empty() && !named && !key.contains(char::is_control);
    let owned = matches!(key.as_str(), "Dead" | "Process" | "Unidentified");
    !(ctrl || alt || *meta) && (text || owned)
}

/// A key-up draws nothing; releasing Alt or Meta is prevented, so Firefox
/// does not focus its menu bar.
fn key_up(code: &str) -> Handled {
    let prevent_default = matches!(code, "AltLeft" | "AltRight" | "MetaLeft" | "MetaRight");
    Handled { redraw: false, prevent_default }
}

#[cfg(test)]
mod tests;

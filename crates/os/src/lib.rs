//! compusophyOS: the wasm entry point.
//!
//! [`start`] hands [`platform::run`] an app that owns a [`shell::Shell`] and
//! a [`gfx::DrawList`] every frame reuses. The rest is mapping:
//!
//! - Key-downs, repeats too, become [`Input::Key`] by `KeyboardEvent.code`;
//!   `Digit1`-`9` and `Numpad1`-`9` are [`Key::Digit`], unbound codes
//!   [`Key::Other`]. Key-ups never reach the shell, but releasing Alt or
//!   Meta is `preventDefault`ed so Firefox does not focus its menu bar.
//! - AltGr that also reports Ctrl and Alt (Chrome and Edge on Windows) types
//!   text, so a key with all three carries neither Ctrl nor Alt.
//! - Pointer events pass through; [`Event::Resize`] drops `dpr`, which the
//!   renderer already has.
//! - [`Response::consumed`] becomes [`Handled::prevent_default`].
//!
//! The wm picks each split's axis when a window opens, so the shell and its
//! startup windows are created at the first Resize with room for the panel
//! and some desktop. Until then events are ignored and frames only clear.

#![forbid(unsafe_code)]

use gfx::DrawList;
use platform::{App, Event, Handled, Renderer};
use shell::{Input, Key, Mods, Response, Shell, theme};
use wasm_bindgen::prelude::*;

/// The wasm entry point, run when the module is instantiated: starts the
/// desktop on `<canvas id="os">`.
///
/// # Errors
///
/// Whatever [`platform::run`] reports: no canvas, no WebGL2, bad shaders.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    platform::run(Desktop::default())
}

/// The shell, once the canvas has a usable size, and the reused draw list.
#[derive(Default)]
struct Desktop {
    shell: Option<Shell>,
    list: DrawList,
}

impl App for Desktop {
    fn event(&mut self, ev: Event) -> Handled {
        let Some(input) = input_of(&ev) else {
            return key_up(&ev);
        };
        match (&mut self.shell, input) {
            (Some(shell), _) => handled(shell.input(input)),
            (None, Input::Resize { w, h }) if w >= 1.0 && h >= theme::PANEL_H + 1.0 => {
                handled(self.shell.insert(Shell::new(w, h)).input(input))
            }
            (None, _) => Handled::default(),
        }
    }

    fn frame(&mut self, r: &mut Renderer) {
        self.list.clear();
        if let Some(shell) = &self.shell {
            shell.draw(&mut self.list);
        }
        let bg = self.shell.as_ref().map_or(theme::BG, Shell::clear_color);
        r.draw(&self.list, bg);
    }
}

/// The shell input for a platform event; `None` for a key-up.
fn input_of(ev: &Event) -> Option<Input> {
    Some(match *ev {
        Event::Key { down: false, .. } => return None,
        Event::Key {
            ref code,
            shift,
            ctrl,
            alt,
            meta,
            altgr,
            ..
        } => {
            let text = altgr && ctrl && alt;
            let (ctrl, alt) = (ctrl && !text, alt && !text);
            let mut mods = Mods::default();
            [mods.shift, mods.ctrl, mods.alt, mods.meta] = [shift, ctrl, alt, meta];
            let key = key_of(code);
            Input::Key { key, mods }
        }
        Event::PointerMove { x, y } => Input::PointerMove { x, y },
        Event::PointerDown { x, y, button } => Input::PointerDown { x, y, button },
        Event::PointerUp { x, y, button } => Input::PointerUp { x, y, button },
        Event::PointerLeave => Input::PointerLeave,
        Event::Resize { w, h, .. } => Input::Resize { w, h },
    })
}

/// A key-up draws nothing; releasing Alt or Meta is still prevented.
fn key_up(ev: &Event) -> Handled {
    let mut h = Handled::default();
    if let Event::Key { code, .. } = ev {
        h.prevent_default = matches!(&**code, "AltLeft" | "AltRight" | "MetaLeft" | "MetaRight");
    }
    h
}

/// The shell key for a `KeyboardEvent.code`.
fn key_of(code: &str) -> Key {
    let digit = code.strip_prefix("Digit").or(code.strip_prefix("Numpad"));
    match code {
        "Enter" | "NumpadEnter" => Key::Enter,
        "Escape" => Key::Escape,
        "ArrowLeft" => Key::Left,
        "ArrowRight" => Key::Right,
        "ArrowUp" => Key::Up,
        "ArrowDown" => Key::Down,
        "KeyH" => Key::H,
        "KeyJ" => Key::J,
        "KeyK" => Key::K,
        "KeyL" => Key::L,
        "KeyQ" => Key::Q,
        "KeyF" => Key::F,
        "KeyO" => Key::O,
        _ => match digit.map(str::as_bytes) {
            Some(&[d @ b'1'..=b'9']) => Key::Digit(d - b'0'),
            _ => Key::Other,
        },
    }
}

/// The platform's answer for a shell [`Response`].
fn handled(r: Response) -> Handled {
    let mut h = Handled::default();
    [h.redraw, h.prevent_default] = [r.redraw, r.consumed];
    h
}

#[cfg(test)]
mod tests;

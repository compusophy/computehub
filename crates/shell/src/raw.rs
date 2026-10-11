//! Apps that take their input whole ([`ui::App::raw`]: a game in a window). While one has the
//! keys, the keys come to it as they came ([`Input::Raw`], made by the page's side, which leaves
//! the desktop its Alt and Meta bindings); while the pointer is over its content, or a press
//! began there, every move, button and wheel turn comes to it in its content's pixels, and a
//! press there gives it the keys. A locked pointer's motion, and the lock, go to what has the
//! keys. Menus, an open folder and the overlay come first.

use host::{Input, OVERLAY, Response, content_rect};
use ui::{AppEvent, Raw};
use wm::{Cmd, WinId};

use crate::{Shell, coord, desktop::Target, rectf};

/// The browser's own keys, left to it even in a game: reload and the developer tools.
fn browser_key(code: &str, mods: u8) -> bool {
    let ctrl = mods & Raw::CTRL != 0;
    matches!(code, "F5" | "F12") || ctrl && code == "KeyR" || ctrl && code == "KeyI"
}

impl Shell {
    /// Whether keys go whole to what has them now: an app that takes its input whole, with no
    /// menu or folder open (the page's side asks before it makes a key).
    pub fn takes_raw(&self) -> bool {
        self.menu.is_none() && self.grid.open.is_none() && self.raw_app(self.key_target())
    }

    fn raw_app(&self, win: Option<WinId>) -> bool {
        let win = win.filter(|&w| w != OVERLAY);
        win.and_then(|w| self.host.win(w)).is_some_and(|w| w.app.raw())
    }

    /// The window whose content is at `(x, y)`, if its app takes its input whole.
    fn raw_at(&self, x: f32, y: f32) -> Option<WinId> {
        match self.hit(x, y) {
            Some(Target::Body(win)) if self.raw_app(Some(win)) => Some(win),
            _ => None,
        }
    }

    /// `input`, if it is for an app that takes its input whole: delivered, and true; false
    /// leaves it to the desktop.
    pub(crate) fn raw_input(&mut self, input: &Input, out: &mut Response) -> bool {
        match *input {
            Input::Raw(ref raw) => {
                let win = self.key_target().filter(|&w| self.raw_app(Some(w)));
                if let Some(win) = win {
                    if let Raw::Key { ref code, mods, .. } = *raw {
                        out.consumed = !browser_key(code, mods);
                    }
                    self.host.deliver(win, AppEvent::Raw(raw.clone()), out);
                }
                true
            }
            Input::PointerDown { x, y, button, .. } => {
                let (x, y) = (coord(x), coord(y));
                let held = self.raw_press.filter(|p| self.host.win(p.0).is_some());
                let Some(win) = held.map(|p| p.0).or_else(|| self.raw_at(x, y)) else {
                    return false;
                };
                if held.is_none() && self.host.wm().focused() != Some(win) {
                    self.host.apply(Cmd::Focus(win));
                    out.redraw = true;
                }
                let mask = held.map_or(0, |p| p.1) | 1 << button.min(7);
                self.raw_press = Some((win, mask));
                self.raw_pointer(win, (0, x, y, button), out);
                true
            }
            Input::PointerMove { x, y } => {
                let (x, y) = (coord(x), coord(y));
                if let Some((win, _)) = self.raw_press {
                    self.raw_pointer(win, (1, x, y, 0), out);
                    return true;
                }
                let at = self.raw_at(x, y);
                if let Some(left) = self.raw_hover.filter(|&w| Some(w) != at) {
                    self.raw_pointer(left, (3, x, y, 0), out);
                }
                self.raw_hover = at;
                at.inspect(|&win| self.raw_pointer(win, (1, x, y, 0), out)).is_some()
            }
            Input::PointerUp { x, y, button } => {
                let Some((win, mask)) = self.raw_press else { return false };
                let mask = mask & !(1 << button.min(7));
                self.raw_press = (mask != 0).then_some((win, mask));
                self.raw_pointer(win, (2, coord(x), coord(y), button), out);
                true
            }
            Input::Wheel { x, y, dy } => {
                let (x, y) = (coord(x), coord(y));
                let Some(win) = self.raw_at(x, y) else { return false };
                let (lx, ly) = self.local(win, x, y);
                self.host.deliver(win, AppEvent::Raw(Raw::Wheel { x: lx, y: ly, dy }), out);
                out.consumed = true;
                true
            }
            Input::PointerLeave => {
                let gone = self.raw_hover.take().or(self.raw_press.take().map(|p| p.0));
                if let Some(win) = gone {
                    self.raw_pointer(win, (3, 0.0, 0.0, 0), out);
                }
                false
            }
            _ => false,
        }
    }

    /// `(x, y)` on the screen as in window `win`'s content.
    fn local(&self, win: WinId, x: f32, y: f32) -> (f32, f32) {
        let place = self.host.wm().layout().into_iter().find(|p| p.win == win);
        let c = place.map(|p| content_rect(rectf(p.rect))).unwrap_or_default();
        (x - c.x, y - c.y)
    }

    /// The pointer to window `win` as `(kind, x, y, button)` on the screen.
    fn raw_pointer(
        &mut self,
        win: WinId,
        (kind, x, y, button): (u8, f32, f32, u8),
        out: &mut Response,
    ) {
        let (x, y) = self.local(win, x, y);
        self.host.deliver(win, AppEvent::Raw(Raw::Pointer { kind, x, y, button }), out);
        out.consumed = true;
    }
}

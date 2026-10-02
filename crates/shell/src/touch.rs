//! Fingers: scrolling what a finger holds as the wheel does, flinging it on, and long presses
//! (see `home::touch`), which pick icons up.

use host::{content_rect, rectf};
use ui::AppEvent;
use wm::WinId;

use crate::desktop::Target;
use crate::{Response, Shell};

/// What a finger scrolls: nothing, or a window's content (pressed at a point).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Scroll {
    None,
    Win(WinId, (f32, f32)),
}

impl Shell {
    /// Scrolls `s` by `dy` (down if positive) as the wheel does; whether it is still there.
    pub(crate) fn scroll(&mut self, s: Scroll, dy: f32, out: &mut Response) -> bool {
        match s {
            Scroll::None => false,
            Scroll::Win(win, (x, y)) => {
                let Some(c) = self.placement(win).map(|p| content_rect(rectf(p.rect))) else {
                    return false;
                };
                self.host.deliver(win, AppEvent::Wheel { x: x - c.x, y: y - c.y, dy }, out);
                true
            }
        }
    }

    /// The pointer moved: a finger that travels scrolls what it holds, and its press is no
    /// longer a click.
    pub(crate) fn finger(&mut self, out: &mut Response) {
        let (at, now) = (self.pointer.unwrap_or_default(), self.host.now_ms);
        let Some((finger, s)) = &mut self.touch else { return };
        let (dy, s) = (finger.moved(at, now), *s);
        if finger.scrolling {
            (self.armed, self.app_press, self.down) = (None, None, None);
        }
        if let Some(dy) = dy {
            self.scroll(s, dy, out);
        }
    }

    /// A frame: a finger held still long enough ([`home::touch::Touch::held`]: after a stall,
    /// 100 ms more, so a lift queued behind it is heard first) picks up the icon under it
    /// where the finger is now (its drag starts from there; its menu waits for it to lift
    /// unmoved), else is a secondary press where it went down. Only a menu that opens ends what
    /// the finger holds (a button, a window, a press into content), and the menu is then what is
    /// under it; a long press that opens nothing, such as one on an app's widget, is still a tap
    /// when it lifts there. So a late lift (a busy page handles it after the frame that saw
    /// 500 ms pass) loses no tap, on what has a menu or not.
    pub(crate) fn hold(&mut self, out: &mut Response) {
        let now = self.host.now_ms;
        let Some((finger, _)) = &mut self.touch else { return };
        if finger.held(now) {
            let (at, menu, here) = (finger.at, self.menu.is_some(), self.pointer);
            if let (Some(Target::Icon(i)), false) = (self.hit(at.0, at.1), menu) {
                let carry = self.grid.pick(i, here.unwrap_or(at), true);
                (self.grid.carry, self.armed, out.redraw) = (carry, None, true);
                return;
            }
            self.secondary(at, true, out);
            if !menu && self.menu.is_some() {
                (self.armed, self.app_press, self.down, self.grab) = (None, None, None, None);
                self.hover = here.and_then(|(x, y)| self.hit(x, y));
            }
        }
    }

    /// A frame of a fling: its step's scroll, until it stops or what it scrolls goes.
    pub(crate) fn flinging(&mut self, out: &mut Response) {
        let Some((s, mut fling)) = self.fling else { return };
        let dy = fling.step(self.host.now_ms);
        let there = self.scroll(s, dy, out);
        self.fling = (there && fling.moving()).then_some((s, fling));
    }
}

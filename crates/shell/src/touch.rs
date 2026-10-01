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

    /// A finger held still long enough picks up the icon under it (its menu waits for it to
    /// lift unmoved), else is a secondary press where it went down: its press into content is
    /// over, and if that opens a menu, so is the button it holds.
    pub(crate) fn hold(&mut self, out: &mut Response) {
        let now = self.host.now_ms;
        let Some((finger, _)) = &mut self.touch else { return };
        if finger.held(now) {
            let (at, menu) = (finger.at, self.menu.is_some());
            (self.app_press, self.down, self.grab) = (None, None, None);
            if let (Some(Target::Icon(i)), false) = (self.hit(at.0, at.1), menu) {
                (self.carry, self.armed, out.redraw) = (self.pick(i, at, true), None, true);
                return;
            }
            self.secondary(at, true, out);
            if !menu && self.menu.is_some() {
                self.armed = None;
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

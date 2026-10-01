//! The pointer: what lies under it; pressing, dragging, resizing, snapping,
//! double clicks, buttons (which act on release) and clicks into apps.

use host::frame::{CTL_GAP, Zone, controls, edge, edge_cursor, resized, zone};
use host::{Cursor, TITLEBAR_H, content_rect, rectf};
use ui::{AppEvent, Sense};
use wm::{Cmd, Placement, Rect, State, WinId};

use crate::{BAR_H, Response, Shell};

/// Travel before a titlebar press drags; most time between double clicks.
const DRAG_PX: f32 = 4.0;
const DOUBLE_MS: f64 = 350.0;

/// What lies under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The top bar's buttons, and the bare bar.
    Mark,
    Settings,
    Theme,
    Bar,
    /// A dock tile, and the bare dock.
    Dock(usize),
    DockBar,
    /// A window's control (minimize, maximize, close), titlebar, content, and
    /// an edge or corner (which way it resizes).
    Ctl(WinId, usize),
    Title(WinId),
    Body(WinId),
    Edge(WinId, i8, i8),
    /// Outside the launcher's panel, inside it, and one of its results.
    Veil,
    Panel,
    Item(usize),
}

impl Target {
    /// Whether it acts on release (and highlights).
    pub(crate) fn is_button(self) -> bool {
        use Target::*;
        matches!(self, Mark | Settings | Theme | Dock(_) | Ctl(..) | Veil | Item(_))
    }

    fn win(self) -> Option<WinId> {
        use Target::*;
        match self {
            Ctl(w, _) | Title(w) | Body(w) | Edge(w, ..) => Some(w),
            _ => None,
        }
    }
}

/// A window held by the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Grab {
    /// Moving: the press, the pointer's offset in the window, whether it
    /// moved yet, where it would snap.
    Move { win: WinId, at: (f32, f32), off: (f32, f32), moving: bool, zone: Option<Zone> },
    /// Resizing by `edge`: the press and the rect then.
    Size { win: WinId, edge: (i8, i8), at: (f32, f32), from: Rect },
}

impl Shell {
    /// What is under `(x, y)`: the launcher, else the bar, the dock, the
    /// windows top to bottom.
    pub(crate) fn hit(&self, x: f32, y: f32) -> Option<Target> {
        if self.launcher.open {
            return Some(self.launcher_hit(x, y));
        }
        if (0.0..BAR_H).contains(&y) {
            return Some(self.bar_hit(x, y));
        }
        if let Some(t) = self.dock_hit(x, y) {
            return Some(t);
        }
        for p in self.host.wm().layout().iter().rev() {
            let r = rectf(p.rect);
            if let Some((dx, dy)) = edge(r, x, y).filter(|_| p.state != State::Maximized) {
                return Some(Target::Edge(p.win, dx, dy));
            }
            if !r.contains(x, y) {
                continue;
            }
            if y >= r.y + TITLEBAR_H {
                return Some(Target::Body(p.win));
            }
            let mut ctl = controls(r).into_iter().flatten();
            let i = ctl.position(|c| c.inset(-CTL_GAP / 2.0).contains(x, y));
            return Some(i.map_or(Target::Title(p.win), |i| Target::Ctl(p.win, i)));
        }
        None
    }

    /// The topmost hit of `win`'s last frame at `(x, y)`, inside its content.
    pub(crate) fn widget_at(&self, win: WinId, x: f32, y: f32) -> Option<ui::Hit> {
        let inside = content_rect(rectf(self.placement(win)?.rect)).contains(x, y);
        ui::hit_test(&self.host.win(win).filter(|_| inside)?.hits, x, y)
    }

    pub(crate) fn placement(&self, win: WinId) -> Option<Placement> {
        self.host.wm().layout().into_iter().find(|p| p.win == win)
    }

    /// The pointer's look; `text` when over text an app edits.
    pub(crate) fn cursor_for(&self, text: bool) -> Cursor {
        match (self.grab, self.hover) {
            (Some(Grab::Move { .. }), _) => Cursor::Grabbing,
            (Some(Grab::Size { edge, .. }), _) => edge_cursor(edge),
            (None, Some(Target::Title(_))) => Cursor::Grab,
            (None, Some(Target::Edge(_, dx, dy))) => edge_cursor((dx, dy)),
            _ if text => Cursor::Text,
            _ => Cursor::Default,
        }
    }

    /// Button 0 down: arms a button, focuses a window, grabs it, toggles
    /// maximize on a double click, or presses into content.
    pub(crate) fn press(&mut self, out: &mut Response) {
        let (x, y) = self.pointer.unwrap_or_default();
        let hit = self.hit(x, y);
        self.armed = hit.filter(|h| h.is_button());
        (self.grab, self.app_press) = (None, None);
        let (Some(hit), Some(win)) = (hit, hit.and_then(Target::win)) else {
            return;
        };
        self.host.apply(Cmd::Focus(win));
        // Focus events reach the apps before the press.
        self.settle(out);
        let (Some(p), now) = (self.placement(win), self.host.now_ms) else {
            return;
        };
        let r = rectf(p.rect);
        match hit {
            Target::Title(_) => {
                let last = self.last_title.take();
                if last.is_some_and(|(w, t)| w == win && now - t <= DOUBLE_MS) {
                    return self.host.apply(Cmd::ToggleMaximize(win));
                }
                self.last_title = Some((win, now));
                let off = (x - r.x, y - r.y);
                self.grab = Some(Grab::Move { win, at: (x, y), off, moving: false, zone: None });
            }
            Target::Edge(_, dx, dy) => {
                self.grab = Some(Grab::Size { win, edge: (dx, dy), at: (x, y), from: p.rect });
            }
            Target::Body(_) => {
                let (c, id) = (content_rect(r), self.widget_at(win, x, y).map(|h| h.id));
                self.app_press = id.map(|id| (win, id));
                self.host.deliver(win, AppEvent::PointerDown { x: x - c.x, y: y - c.y, id }, out);
            }
            _ => {}
        }
    }

    /// The held window follows the pointer; a maximized or snapped one comes
    /// back to its normal size under it, animated.
    pub(crate) fn drag_to(&mut self) {
        let ((x, y), Some(grab)) = (self.pointer.unwrap_or_default(), self.grab) else {
            return;
        };
        match grab {
            Grab::Move { win, at, mut off, moving, .. } => {
                if !moving && (x - at.0).abs().max((y - at.1).abs()) < DRAG_PX {
                    return;
                }
                let back =
                    |p: &Placement| !moving && (p.state == State::Maximized || p.snap.is_some());
                match (self.placement(win).filter(back), self.host.wm().normal_rect(win)) {
                    (Some(p), Some(n)) => {
                        off.0 = (off.0 / p.rect.w.max(1) as f32 * n.w as f32).round()
                    }
                    _ => self.instant = true,
                }
                let (nx, ny) = ((x - off.0).round() as i32, (y - off.1).round() as i32);
                self.host.apply(Cmd::Move { win, x: nx, y: ny });
                let zone = zone(self.size, x, y);
                self.grab = Some(Grab::Move { win, at, off, moving: true, zone });
            }
            Grab::Size { win, edge, at, from } => {
                self.instant = true;
                let rect = resized(from, edge, (x - at.0, y - at.1), self.host.wm().area().y);
                self.host.apply(Cmd::Resize { win, rect });
            }
        }
    }

    /// Drops a held window (snapping it); button 0 fires the armed button, or
    /// clicks the pressed widget, if still over it.
    pub(crate) fn release(&mut self, primary: bool, out: &mut Response) {
        let (x, y) = self.pointer.unwrap_or_default();
        if let Some(Grab::Move { win, zone: Some(zone), .. }) = self.grab.take() {
            self.host.apply(match zone {
                Zone::Max => Cmd::Maximize(win),
                Zone::Snap(snap) => Cmd::SnapTo { win, snap },
            });
        }
        let over = self.pointer.and_then(|_| self.hit(x, y));
        let press = self.app_press.take();
        if let Some(target) = self.armed.take().filter(|&a| primary && over == Some(a)) {
            self.activate(target, out);
        }
        if let (true, Some((win, id))) = (primary, press) {
            let hit = self.widget_at(win, x, y);
            if hit.is_some_and(|h| h.id == id && h.sense == Sense::Click)
                && over == Some(Target::Body(win))
            {
                self.host.deliver(win, AppEvent::Click(id), out);
            }
        }
    }

    fn activate(&mut self, target: Target, out: &mut Response) {
        let now = self.host.now_ms;
        match target {
            Target::Mark => self.toggle_launcher(),
            Target::Settings => self.host.show("settings", out),
            Target::Theme => _ = self.host.theme.set(self.host.theme.next(), now, host::THEME_MS),
            Target::Dock(i) => self.host.toggle(self.dock.get(i).map_or("", |d| d.0.as_str()), out),
            Target::Ctl(w, i) => {
                self.host.apply([Cmd::Minimize, Cmd::ToggleMaximize, Cmd::Close][i](w))
            }
            Target::Item(k) => self.launch(k, out),
            Target::Veil => self.hide_launcher(),
            _ => {}
        }
    }

    /// The wheel scrolls the launcher, else goes to the app under it.
    pub(crate) fn wheel(&mut self, x: f32, y: f32, dy: f32, out: &mut Response) {
        if self.launcher.open {
            self.launcher.search.scroll(dy, host::layout::ROW_H);
            out.consumed = true;
            return;
        }
        let Some(Target::Body(win)) = self.hit(x, y) else {
            return;
        };
        if let (Some(p), true) = (self.placement(win), dy.is_finite()) {
            let c = content_rect(rectf(p.rect));
            self.host.deliver(win, AppEvent::Wheel { x: x - c.x, y: y - c.y, dy }, out);
            out.consumed = true;
        }
    }
}

//! The pointer: what lies under it; pressing, dragging, resizing, snapping,
//! double clicks, buttons (which act on release), clicks into apps and secondary presses.

use gfx::RectF;
use host::frame::{
    CTL_STEP, TOUCH_STEP, Zone, controls, edge, edge_cursor, hit_box, resized, zone,
};
use host::{Cursor, TITLEBAR_H, content_rect, rectf};
use ui::{AppEvent, Sense};
use wm::{Cmd, Placement, Rect, State, WinId};

use crate::touch::Scroll;
use crate::{BAR_H, Response, Shell};

/// Travel before a titlebar press drags; most time between double clicks.
const DRAG_PX: f32 = 4.0;
const DOUBLE_MS: f64 = 350.0;

/// What lies under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The top bar's buttons, and the bare bar.
    Mark,
    Feedback,
    Settings,
    Top,
    /// A dock tile, the Apps button, and the bare dock.
    Dock(usize),
    Apps,
    Shelf,
    /// The everything bar.
    Field,
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
    /// A desktop icon, and the bare desktop.
    Icon(usize),
    Desktop,
    /// A menu's action, the rest of the menu, and anywhere else while it shows.
    Menu(usize),
    MenuPanel,
    Off,
}

impl Target {
    /// Whether it acts on release (and highlights).
    pub(crate) fn is_button(self) -> bool {
        use Target::*;
        matches!(
            self,
            Mark | Feedback
                | Settings
                | Dock(_)
                | Apps
                | Field
                | Ctl(..)
                | Veil
                | Item(_)
                | Icon(_)
                | Menu(_)
        )
    }

    pub(crate) fn win(self) -> Option<WinId> {
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
    /// What is under `(x, y)`: a menu (which hides the rest), the everything bar, the
    /// launcher, the top bar, the dock, the windows top to bottom, then the desktop.
    pub(crate) fn hit(&self, x: f32, y: f32) -> Option<Target> {
        if let Some((m, ..)) = &self.menu {
            return Some(
                m.at(x, y).map_or(Target::Off, |i| i.map_or(Target::MenuPanel, Target::Menu)),
            );
        }
        if home::field::rect(self.size).contains(x, y) {
            return Some(Target::Field);
        }
        if self.launcher.open {
            return Some(self.launcher_hit(x, y));
        }
        if (0.0..BAR_H).contains(&y) {
            return Some(self.bar_hit(x, y));
        }
        if let Some(t) = self.dock_hit(x, y) {
            return Some(t);
        }
        let step = self.ctl_step();
        for p in self.host.wm().layout().iter().rev() {
            let r = rectf(p.rect);
            let ctl = self.controls(r).into_iter().find(|c| hit_box(c.1, step).contains(x, y));
            if let Some((i, _)) = ctl {
                return Some(Target::Ctl(p.win, i));
            }
            if let Some((dx, dy)) = edge(r, x, y).filter(|_| p.state != State::Maximized) {
                return Some(Target::Edge(p.win, dx, dy));
            }
            if r.contains(x, y) {
                let body = y >= r.y + TITLEBAR_H;
                return Some(if body { Target::Body(p.win) } else { Target::Title(p.win) });
            }
        }
        let (a, narrow) = (rectf(self.host.wm().area()), self.host.narrow());
        let icon = home::icons::at(self.icons.len(), a, narrow, x, y);
        Some(icon.map_or(Target::Desktop, Target::Icon))
    }

    /// The window controls' reach: wide enough for a finger on a narrow screen.
    pub(crate) fn ctl_step(&self) -> f32 {
        if self.host.narrow() { TOUCH_STEP } else { CTL_STEP }
    }

    /// The controls of a window at `r`, by index (minimize, maximize, close); on a narrow
    /// screen, where every window stays maximized, no maximize: minimize takes its place.
    pub(crate) fn controls(&self, r: RectF) -> Vec<(usize, RectF)> {
        let Some([min, max, close]) = controls(r, self.ctl_step()) else { return Vec::new() };
        match self.host.narrow() {
            true => vec![(0, max), (2, close)],
            false => vec![(0, min), (1, max), (2, close)],
        }
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
            (None, Some(Target::Title(_))) if !self.host.narrow() => Cursor::Grab,
            (None, Some(Target::Edge(_, dx, dy))) => edge_cursor((dx, dy)),
            _ if text => Cursor::Text,
            _ => Cursor::Default,
        }
    }

    /// Button 0 down (a finger's if `touch`): closes a menu pressed outside (and does nothing
    /// else), arms a button, follows a finger, takes the keys from the everything bar, focuses a
    /// window, grabs it (not on a narrow screen), toggles maximize on a double click, or presses
    /// into content (a finger's press waits to be a tap: a scroll is no press).
    pub(crate) fn press(&mut self, touch: bool, out: &mut Response) {
        let ((x, y), now) = (self.pointer.unwrap_or_default(), self.host.now_ms);
        let hit = self.hit(x, y);
        (self.grab, self.app_press, self.armed, self.fling) = (None, None, None, None);
        self.down = None;
        if hit == Some(Target::Off) {
            self.menu = None;
            return;
        }
        self.armed = hit.filter(|h| h.is_button());
        let scroll = match hit {
            Some(Target::Body(win)) => Scroll::Win(win, (x, y)),
            Some(Target::Panel | Target::Item(_)) => Scroll::Launcher,
            _ => Scroll::None,
        };
        let finger = home::touch::Touch::new((x, y), now, scroll != Scroll::None);
        self.touch = touch.then_some((finger, scroll));
        let field = self.launcher.focus && !self.launcher.open && self.menu.is_none();
        if field && hit != Some(Target::Field) {
            self.hide_launcher();
        }
        let (Some(hit), Some(win)) = (hit, hit.and_then(Target::win)) else {
            return;
        };
        self.host.apply(Cmd::Focus(win));
        // Focus events reach the apps before the press.
        self.settle(out);
        let Some(p) = self.placement(win) else {
            return;
        };
        let r = rectf(p.rect);
        match hit {
            Target::Title(_) if self.host.narrow() => {}
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
                let (c, theme) = (content_rect(r), self.host.theme.at(now));
                let state = ui::UiState { focused: true, now_ms: now, ..ui::UiState::default() };
                self.host.fresh_hits(win, c, &theme, state);
                let id = self.widget_at(win, x, y).map(|h| h.id);
                self.app_press = id.map(|id| (win, id));
                let down = AppEvent::PointerDown { x: x - c.x, y: y - c.y, id };
                match touch {
                    true => self.down = Some((win, down)),
                    false => self.host.deliver(win, down, out),
                }
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

    /// Lifts a finger (it may fling; a gesture's lift says so) and drops a held window (snapping
    /// it); button 0 fires the armed button, or presses into content if a finger's tap, then
    /// clicks the pressed widget, if still over it.
    pub(crate) fn release(&mut self, primary: bool, out: &mut Response) {
        let ((x, y), now) = (self.pointer.unwrap_or_default(), self.host.now_ms);
        if let Some((finger, scroll)) = self.touch.take() {
            self.fling = finger.lift(now).map(|f| (scroll, f));
            out.gesture = finger.done;
        }
        if let Some(Grab::Move { win, zone: Some(zone), .. }) = self.grab.take() {
            self.host.apply(match zone {
                Zone::Max => Cmd::Maximize(win),
                Zone::Snap(snap) => Cmd::SnapTo { win, snap },
            });
        }
        let over = self.pointer.and_then(|_| self.hit(x, y));
        let (press, down) = (self.app_press.take(), self.down.take());
        if let Some(target) = self.armed.take().filter(|&a| primary && over == Some(a)) {
            self.activate(target, out);
        }
        if let (true, Some((win, down))) = (primary, down) {
            self.host.deliver(win, down, out);
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
        let name = |s: &Shell, i: usize| s.dock.get(i).map_or(String::new(), |d| d.0.clone());
        match target {
            Target::Mark => self.host.show("welcome", out),
            Target::Feedback => self.host.show("feedback", out),
            Target::Settings => self.host.show("settings", out),
            Target::Dock(i) => self.host.toggle(&name(self, i), out),
            Target::Apps => self.toggle_launcher(),
            Target::Field => self.focus_field(),
            Target::Ctl(w, i) => {
                self.host.apply([Cmd::Minimize, Cmd::ToggleMaximize, Cmd::Close][i](w))
            }
            Target::Item(k) => self.launch(k, out),
            Target::Veil => self.hide_launcher(),
            Target::Icon(i) => {
                let name = self.icons.get(i).map(|e| e.name.clone()).unwrap_or_default();
                self.host.show(&name, out);
            }
            Target::Menu(i) => self.choose(Some(i), out),
            _ => {}
        }
    }

    /// The wheel scrolls the launcher, else goes to the app under it.
    pub(crate) fn wheel(&mut self, x: f32, y: f32, dy: f32, out: &mut Response) {
        let scroll = match self.hit(x, y) {
            _ if self.launcher.open => Scroll::Launcher,
            Some(Target::Body(win)) => Scroll::Win(win, (x, y)),
            _ => return,
        };
        out.consumed |= dy.is_finite() && self.scroll(scroll, dy, out);
    }
}

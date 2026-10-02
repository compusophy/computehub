//! The pointer: what lies under it; pressing, windows held (moved, resized, snapped and
//! maximized by a double click: `host::grab`), buttons (which act on release), clicks into apps,
//! icons carried and secondary presses.

use gfx::RectF;
use host::frame::{CTL_STEP, TOUCH_STEP, controls, edge, edge_cursor, hit_box};
use host::grab::Grab;
use host::{Cursor, OVERLAY, TITLEBAR_H, content_rect, rectf};
use ui::{AppEvent, Sense};
use wm::{Cmd, Placement, State, WinId};

use crate::touch::Scroll;
use crate::{BAR_H, Response, Shell};

/// What lies under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A top bar's button, and the bare bar.
    Bar(home::bar::Button),
    Top,
    /// A dock tile, and the Assistant's in the corner.
    Dock(usize),
    Assistant,
    /// A window's control (minimize, maximize, close), titlebar, content, and
    /// an edge or corner (which way it resizes).
    Ctl(WinId, usize),
    Title(WinId),
    Body(WinId),
    Edge(WinId, i8, i8),
    /// A home screen icon, and the bare desktop.
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
        matches!(self, Bar(_) | Dock(_) | Assistant | Ctl(..) | Icon(_) | Menu(_))
    }

    pub(crate) fn win(self) -> Option<WinId> {
        use Target::*;
        match self {
            Ctl(w, _) | Title(w) | Body(w) | Edge(w, ..) => Some(w),
            _ => None,
        }
    }
}

impl Shell {
    /// What is under `(x, y)`: a menu (which hides the rest), the top bar, the bottom row's
    /// tiles, the windows top to bottom, then the home screen.
    pub(crate) fn hit(&self, x: f32, y: f32) -> Option<Target> {
        if let Some((m, ..)) = &self.menu {
            return Some(
                m.at(x, y).map_or(Target::Off, |i| i.map_or(Target::MenuPanel, Target::Menu)),
            );
        }
        if (0.0..BAR_H).contains(&y) {
            return Some(self.bar_hit(x, y));
        }
        if let Some(t) = self.dock_hit(x, y) {
            return Some(t);
        }
        if self.overlay.open && self.overlay_rects().0.contains(x, y) {
            return Some(Target::Body(OVERLAY));
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
        Some(self.grid.at(x, y).map_or(Target::Desktop, Target::Icon))
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

    /// Where window `win` is (the overlay's as if it were one).
    pub(crate) fn placement(&self, win: WinId) -> Option<Placement> {
        if win == OVERLAY {
            return self.overlay_placement();
        }
        self.host.wm().layout().into_iter().find(|p| p.win == win)
    }

    /// The pointer's look; `text` when over text an app edits.
    pub(crate) fn cursor_for(&self, text: bool) -> Cursor {
        let carried = self.carrying() && !self.finger;
        match (self.grab, self.hover) {
            (Some(Grab::Move { .. }), _) => Cursor::Grabbing,
            _ if carried => Cursor::Grabbing,
            (Some(Grab::Size { edge, .. }), _) => edge_cursor(edge),
            (None, Some(Target::Title(_))) if !self.host.narrow() => Cursor::Grab,
            (None, Some(Target::Edge(_, dx, dy))) => edge_cursor((dx, dy)),
            _ if text => Cursor::Text,
            _ => Cursor::Default,
        }
    }

    /// Button 0 down (a finger's if `touch`): closes a menu pressed outside (and does nothing
    /// else), arms a button, follows a finger, presses on the home screen, focuses a window,
    /// grabs it (not on a narrow screen), toggles maximize on a double click, or presses into
    /// content (a finger's press waits to be a tap: a scroll is no press; but on a
    /// [`Sense::Pad`] it presses at once, and the finger drags it, never scrolling).
    pub(crate) fn press(&mut self, touch: bool, out: &mut Response) {
        let ((x, y), now) = (self.pointer.unwrap_or_default(), self.host.now_ms);
        let hit = self.hit(x, y);
        (self.grab, self.app_press, self.armed, self.fling) = (None, None, None, None);
        self.down = None;
        if hit == Some(Target::Off) {
            self.menu = None;
            return;
        }
        let overlay = matches!(hit, Some(Target::Body(OVERLAY)));
        self.takeover(!overlay && hit != Some(Target::Assistant), out);
        match hit {
            Some(Target::Desktop) => self.overlay = Default::default(),
            Some(Target::Assistant) => {}
            _ => self.overlay.focus = overlay,
        }
        self.armed = hit.filter(|h| h.is_button());
        let scroll = match hit {
            Some(Target::Body(win)) => Scroll::Win(win, (x, y)),
            _ => Scroll::None,
        };
        let finger = home::touch::Touch::new((x, y), now, scroll != Scroll::None);
        self.touch = touch.then_some((finger, scroll));
        self.press_home(hit, touch);
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
            Target::Title(_) | Target::Edge(..) => {
                let edge = if let Target::Edge(_, dx, dy) = hit { Some((dx, dy)) } else { None };
                self.grab = self.host.grab(win, (x, y), edge, (now, &mut self.last_title));
            }
            Target::Body(_) => {
                let (c, theme) = (content_rect(r), self.host.theme.at(now));
                let state = ui::UiState { focused: true, now_ms: now, ..ui::UiState::default() };
                self.host.fresh_hits(win, c, &theme, state);
                let hit = self.widget_at(win, x, y);
                let (id, pad) = (hit.map(|h| h.id), hit.is_some_and(|h| h.sense == Sense::Pad));
                self.app_press = id.map(|id| (win, id));
                let down = AppEvent::PointerDown { x: x - c.x, y: y - c.y, id };
                if let Some(t) = self.touch.as_mut().filter(|_| pad) {
                    // A finger on a pad scrolls nothing and long-presses nothing.
                    let mut finger = home::touch::Touch::new((x, y), now, false);
                    finger.done = true;
                    *t = (finger, Scroll::None);
                }
                match touch && !pad {
                    true => self.down = Some((win, down)),
                    false => self.press_into(win, down, out),
                }
            }
            _ => {}
        }
    }

    /// The held window follows the pointer (exactly, unless a maximized or snapped one comes
    /// back to its normal size under it, animated).
    pub(crate) fn drag_to(&mut self) {
        let (Some(at), Some(mut grab)) = (self.pointer, self.grab) else { return };
        self.instant |= self.host.drag(&mut grab, at, self.size);
        self.grab = Some(grab);
    }

    /// A mouse held down on a window's content, or a finger on a pad, moved: its app hears where
    /// to.
    pub(crate) fn drag_app(&mut self, out: &mut Response) {
        let pad = self.touch.is_none_or(|t| t.1 == Scroll::None);
        let (Some((win, _)), Some((x, y)), true) = (self.app_press, self.pointer, pad) else {
            return;
        };
        if let Some(c) = self.placement(win).map(|p| content_rect(rectf(p.rect))) {
            self.host.deliver(win, AppEvent::Drag { x: x - c.x, y: y - c.y }, out);
        }
    }

    /// Lifts a finger (it may fling; a gesture's lift says so), drops a held window (snapping
    /// it), carried icons or a carried dock tile (a finger's pick-up lifted unmoved opens its
    /// menu instead), ends a selection box; button 0 fires the armed button, or presses into
    /// content if a finger's tap, then clicks the pressed widget, if still over it.
    pub(crate) fn release(&mut self, primary: bool, out: &mut Response) {
        let ((x, y), now) = (self.pointer.unwrap_or_default(), self.host.now_ms);
        if let Some((finger, scroll)) = self.touch.take() {
            self.fling = finger.lift(now).map(|f| (scroll, f));
            // A press into content that outlived a long press is a tap.
            out.gesture = finger.done && self.down.is_none();
        }
        let held = self.carried().and_then(|c| c.held());
        self.drop_icons(held.is_none());
        self.dock.drop(held.is_none(), &mut self.pending);
        self.grid.lasso = None;
        if let Some(at) = held {
            self.secondary(at, true, out);
        }
        if let Some(grab) = self.grab.take() {
            self.host.drop_grab(grab);
        }
        let over = self.pointer.and_then(|_| self.hit(x, y));
        let (press, down) = (self.app_press.take(), self.down.take());
        if let Some(target) = self.armed.take().filter(|&a| primary && over == Some(a)) {
            self.activate(target, out);
        }
        if let (true, Some((win, down))) = (primary, down) {
            self.press_into(win, down, out);
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

    /// Presses into `win`'s content; a press on the overlay's text field lets the keyboard come.
    fn press_into(&mut self, win: WinId, down: AppEvent, out: &mut Response) {
        let id = if let AppEvent::PointerDown { id, .. } = down { id } else { None };
        let text = |h: &ui::Hit| Some(h.id) == id && h.sense == Sense::Text;
        if win == OVERLAY && self.host.win(win).is_some_and(|w| w.hits.iter().any(text)) {
            self.overlay.quiet = false;
        }
        self.host.deliver(win, down, out);
    }

    fn activate(&mut self, target: Target, out: &mut Response) {
        let name = |s: &Shell, i: usize| s.tiles.get(i).map_or(String::new(), |d| d.0.clone());
        match target {
            Target::Bar(b) => self.host.show(b.app(), out),
            Target::Dock(i) => self.host.toggle(&name(self, i), out),
            Target::Assistant => self.toggle_overlay(false),
            Target::Ctl(w, i) => {
                self.host.apply([Cmd::Minimize, Cmd::ToggleMaximize, Cmd::Close][i](w))
            }
            Target::Icon(i) => {
                let name = self.grid.icons.get(i).map(|e| e.name.clone()).unwrap_or_default();
                self.host.show(&name, out);
            }
            Target::Menu(i) => self.choose(Some(i), out),
            _ => {}
        }
    }

    /// The wheel goes to the app under it.
    pub(crate) fn wheel(&mut self, x: f32, y: f32, dy: f32, out: &mut Response) {
        let hit = self.hit(x, y);
        self.takeover(hit != Some(Target::Body(OVERLAY)), out);
        let Some(Target::Body(win)) = hit else { return };
        let scroll = Scroll::Win(win, (x, y));
        out.consumed |= dy.is_finite() && self.scroll(scroll, dy, out);
    }
}

//! The overlay: the Assistant over the desktop (see `host::agent`), a layer above the windows,
//! never one of them. A glass card above the Assistant's tile in the bottom-right corner (a
//! sheet on a phone), a pill while it works; its content routes as a window's does. The tile
//! opens and hides it, as does Alt+Space; while no task runs, a press on the bare desktop hides
//! it too, as does Escape while it has the keys. Only Stop (the pill's, or the card's under a
//! question), or Escape while it has the keys, stops a task (Escape one that waits on the
//! person's answer too, as it hides): the person's own presses, keys and wheel go where they go,
//! the task running on (hidden by the tile, too). A press in it gives it the keys; one on a
//! window (or, while it works, on the bare desktop) takes them back, the overlay staying. Shown
//! by a finger, it holds the keyboard back until its text field is tapped. A failed program
//! starts again at the next summon (`Host::open_overlay`). What the agent touches flashes; while
//! it works, the dot under the Assistant's tile beats (`Shell::draw_dock`). A task done in a
//! window the person uses next steps aside for it (`Request::Yield`): the overlay hides, that
//! window has the keys, and the tile's dot, the accent's, says an answer waits until it shows.

use gfx::{DrawList, RectF};
use host::agent::FLASH_MS;
use host::paint::{faded, px, sheen};
use host::{OVERLAY, TITLEBAR_H};
use ui::{Theme, UiState};
use wm::{Placement, Rect, State, WinId};

use crate::{BAR_H, Response, Shell};

/// Whether the overlay shows, whether it has the keys, whether it holds the keyboard back
/// (shown by a finger, until its text field is tapped), and whether an answer waits unseen (it
/// stepped aside).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Overlay {
    pub open: bool,
    pub focus: bool,
    pub quiet: bool,
    pub unread: bool,
}

impl Shell {
    /// The overlay as drawn, and the rect its content lays out in: a pill's grows above and
    /// below, so the content's padding centers its one line of buttons.
    pub(crate) fn overlay_rects(&self) -> (RectF, RectF) {
        let pill = self.host.agent.working;
        let r = self.dock.strip.overlay(self.size, BAR_H, pill);
        let grow = if pill { ui::PAD - (r.h - ui::BUTTON_H) / 2.0 } else { 0.0 };
        (r, RectF::new(r.x, r.y - grow, r.w, r.h + 2.0 * grow))
    }

    /// The overlay as if it were a window: its frame around its content (whole px), so presses,
    /// the wheel and fingers reach it as they reach a window's content.
    pub(crate) fn overlay_placement(&self) -> Option<Placement> {
        let l = self.overlay_rects().1;
        let t = TITLEBAR_H as i32;
        let rect = Rect::new(l.x as i32 - 1, l.y as i32 - t, l.w as i32 + 2, l.h as i32 + t + 1);
        let focused = self.overlay.focus;
        let placed = Placement { win: OVERLAY, rect, state: State::Normal, snap: None, focused };
        self.overlay.open.then_some(placed)
    }

    /// What takes the keys: the overlay while it has them, else the focused window.
    pub(crate) fn key_target(&self) -> Option<WinId> {
        let overlay = self.overlay.open && self.overlay.focus;
        if overlay { Some(OVERLAY) } else { self.host.focused_app() }
    }

    /// Whether a tap at `(x, y)` lands on what takes the keys (it may bring the keyboard back).
    pub fn types_at(&self, x: f32, y: f32) -> bool {
        let target = self.hit(x, y).and_then(crate::desktop::Target::win);
        target.is_some() && target == self.key_target()
    }

    /// Shows the overlay with the keys (starting its app if it does not run, or again if it
    /// failed), `quiet`: the keyboard held back.
    pub(crate) fn show_overlay(&mut self, quiet: bool) {
        let open = self.host.open_overlay();
        self.overlay = Overlay { open, focus: open, quiet, unread: false };
    }

    /// Hides the overlay if it shows (and, by `keys`, has them), else shows it: by a key with
    /// the keyboard, else with it only if the last press was not a finger's.
    pub(crate) fn toggle_overlay(&mut self, keys: bool) {
        match self.overlay.open && (!keys || self.overlay.focus) {
            true => self.overlay = Overlay::default(),
            false => self.show_overlay(!keys && self.finger),
        }
    }

    /// Brings the overlay up to date: one that stepped aside hides, its answer unread; one asked
    /// for opens; its app gone, it goes; the host knows where it shows, the screen, whether the
    /// last press was a finger's, and the home screen's apps.
    pub(crate) fn place_overlay(&mut self, out: &mut Response) {
        if std::mem::take(&mut self.host.agent.yielded) {
            self.overlay = Overlay { unread: true, ..Overlay::default() };
        }
        if std::mem::take(&mut self.host.agent.summon) {
            self.show_overlay(self.finger);
        }
        if self.host.win(OVERLAY).is_none() {
            self.overlay = Overlay::default();
        }
        let layout = self.overlay.open.then(|| self.overlay_rects().1);
        (self.host.agent.screen, self.host.agent.touch) = (self.size, self.finger);
        self.host.place_overlay(layout, out);
    }

    /// The card or pill and its content.
    pub(crate) fn draw_overlay(&mut self, list: &mut DrawList, theme: &Theme) {
        if !self.overlay.open {
            return;
        }
        let ((r, layout), line) = (self.overlay_rects(), px(&self.host.text, 1.0));
        let radius = if self.host.agent.working { r.h / 2.0 } else { 18.0 };
        list.shadow_offset(r, radius, 40.0, 14.0, theme.shadow);
        list.fill(r, radius, theme.base);
        list.fill(r, radius, theme.surface);
        list.border(r, radius, line, theme.border);
        sheen(list, r, radius, line, theme.highlight);
        let pick = |w: Option<crate::Widget>| w.filter(|w| w.0 == OVERLAY).map(|w| w.1);
        let (hover, pressed, focused) =
            (pick(self.app_hover), pick(self.app_press), self.overlay.focus);
        let state = UiState { hover, pressed, focused, now_ms: self.host.now_ms };
        self.host.draw_content(list, OVERLAY, [layout, r], theme, state);
    }

    /// The ring of the last act, fading out.
    pub(crate) fn draw_agent(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        if let Some((r, k)) = self.host.agent.flash.map(|(r, t)| (r, 1.0 - (now - t) / FLASH_MS)) {
            if (0.0..=1.0).contains(&k) {
                list.border(r.inset(-3.0), 11.0, 2.0, faded(theme.accent, k as f32));
            }
        }
    }

    /// Whether the agent's marks move: a flash fading, or the Assistant's dot beating.
    pub(crate) fn agent_moves(&self, now: f64) -> bool {
        let flash = self.host.agent.flash.is_some_and(|(_, t)| (t..t + FLASH_MS).contains(&now));
        flash || self.host.agent.working
    }
}

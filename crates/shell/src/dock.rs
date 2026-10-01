//! The dock: the pinned apps, then the other running ones, as tiles on a
//! glass shelf centered at the bottom, with running dots and tooltips.

use gfx::{DrawList, RectF};
use host::paint::{ICON as TILE, cap_baseline, draw_icon, faded, px, sheen};
use host::{app_label, layout};
use ui::{AppIcon, FontId, TextStyle, Theme};
use wm::WinId;

use crate::Shell;
use crate::desktop::Target;

const RADIUS: f32 = 18.0;
const DOT: f32 = 4.0;
const LIFT: f32 = 2.0;
const TIP_GAP: f32 = 10.0;
const TIP_H: f32 = 24.0;
/// The apps the dock always shows, in order (those the registry knows).
pub(crate) const PINNED: [&str; 3] = ["terminal", "studio", "settings"];

/// One app on the dock: its registry name, icon and open windows.
pub(crate) type Item = (String, AppIcon, Vec<WinId>);

impl Shell {
    pub(crate) fn dock_rect(&self) -> RectF {
        layout::dock(self.dock.len(), self.size)
    }

    pub(crate) fn dock_tile(&self, i: usize) -> RectF {
        layout::dock_tile(self.dock.len(), self.size, i)
    }

    pub(crate) fn dock_hit(&self, x: f32, y: f32) -> Option<Target> {
        let at = layout::dock_at(self.dock.len(), self.size, x, y)?;
        Some(at.map_or(Target::DockBar, Target::Dock))
    }

    /// The shelf, the tiles (lifted while hovered) and a dot under running
    /// apps.
    pub(crate) fn draw_dock(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        if self.dock.is_empty() {
            return;
        }
        let (d, line) = (self.dock_rect(), px(&self.host.text, 1.0));
        list.shadow_offset(d, RADIUS, 30.0, 10.0, theme.shadow);
        list.fill(d, RADIUS, theme.glass);
        list.border(d, RADIUS, line, theme.border);
        sheen(list, d, RADIUS, line, theme.highlight);
        let focused = self.host.wm().focused();
        for (i, (name, icon, wins)) in self.dock.iter().enumerate() {
            let t = self.dock_tile(i);
            let lifted = RectF { y: t.y - LIFT * self.motion.lift(name, now), ..t };
            draw_icon(list, &mut self.host.text, lifted, *icon, &app_label(name), theme.shadow);
            if !wins.is_empty() {
                let mine = focused.is_some_and(|f| wins.contains(&f));
                let dot = RectF::new(t.x + (TILE - DOT) / 2.0, t.y + TILE + 3.0, DOT, DOT);
                list.fill(dot, DOT / 2.0, if mine { theme.accent } else { theme.text_dim });
            }
        }
    }

    /// The hovered app's name above its tile, fading in with the lift.
    pub(crate) fn draw_tooltip(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let Some(Target::Dock(i)) = self.hover else {
            return;
        };
        let Some(item) = self.dock.get(i) else {
            return;
        };
        let (a, label) = (self.motion.lift(&item.0, now), app_label(&item.0));
        let (t, top, sw) = (self.dock_tile(i), self.dock_rect().y, self.size.0);
        let text = &mut self.host.text;
        let line = px(text, 1.0);
        let style = TextStyle::new(FontId::Sans, 12.0, faded(theme.text, a));
        let tw = text.measure(&label, style);
        let w = (tw + 20.0).round();
        let x = (t.x + (TILE - w) / 2.0).round().min(sw - w - 4.0).max(4.0);
        let pill = RectF::new(x, top - TIP_GAP - TIP_H, w, TIP_H);
        list.shadow_offset(pill, 8.0, 12.0, 4.0, faded(theme.shadow, a / 2.0));
        list.fill(pill, 8.0, faded(theme.surface, a));
        list.border(pill, 8.0, line, faded(theme.border, a));
        let base = cap_baseline(text, pill.y, TIP_H, style.size);
        text.draw_text(list, text.snap(pill.x + (w - tw) / 2.0), base, &label, style);
    }
}

//! The dock: the favorites, then the other running apps, then the Apps button, on a glass shelf
//! above the everything bar (see `home::dock`), with tooltips.

use gfx::{DrawList, RectF};
use home::dock::Look;
use host::app_label;
use host::paint::{cap_baseline, faded, px};
use ui::{AppIcon, FontId, TextStyle, Theme};
use wm::WinId;

use crate::Shell;
use crate::desktop::Target;

const TIP_GAP: f32 = 10.0;
const TIP_H: f32 = 24.0;

/// One app on the dock: its registry name, icon and open windows.
pub(crate) type Item = (String, AppIcon, Vec<WinId>);

impl Shell {
    pub(crate) fn dock_rect(&self) -> RectF {
        self.shelf.rect
    }

    pub(crate) fn dock_tile(&self, i: usize) -> RectF {
        self.shelf.tile(i)
    }

    pub(crate) fn dock_hit(&self, x: f32, y: f32) -> Option<Target> {
        Some(match self.shelf.at(x, y)? {
            Some(i) if i < self.dock.len() => Target::Dock(i),
            Some(_) => Target::Apps,
            None => Target::Shelf,
        })
    }

    /// The shelf, the tiles (lifted while hovered) with a dot under running apps, the Apps
    /// button.
    pub(crate) fn draw_dock(&mut self, list: &mut DrawList, theme: &Theme) {
        let (focused, now) = (self.host.wm().focused(), self.host.now_ms);
        let look = |(name, icon, wins): &Item| Look {
            icon: *icon,
            lift: self.motion.lift(name, now),
            running: !wins.is_empty(),
            focused: focused.is_some_and(|f| wins.contains(&f)),
        };
        let looks: Vec<Look> = self.dock.iter().map(look).collect();
        let button = (self.hover == Some(Target::Apps)).then_some(self.armed == self.hover);
        self.shelf.draw(list, &mut self.host.text, theme, &looks, button);
    }

    /// The hovered tile's name above it, fading in with the lift.
    pub(crate) fn draw_tooltip(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let (i, a, label) = match self.hover {
            Some(Target::Dock(i)) if i < self.dock.len() => {
                let name = &self.dock[i].0;
                (i, self.motion.lift(name, now), app_label(name))
            }
            Some(Target::Apps) => (self.dock.len(), self.motion.apps.value(now), "Apps".into()),
            _ => return,
        };
        let (t, top, sw) = (self.dock_tile(i), self.dock_rect().y, self.size.0);
        let text = &mut self.host.text;
        let line = px(text, 1.0);
        let style = TextStyle::new(FontId::Sans, 12.0, faded(theme.text, a));
        let tw = text.measure(&label, style);
        let w = (tw + 20.0).round();
        let x = (t.x + (t.w - w) / 2.0).round().min(sw - w - 4.0).max(4.0);
        let pill = RectF::new(x, top - TIP_GAP - TIP_H, w, TIP_H);
        list.shadow_offset(pill, 8.0, 12.0, 4.0, faded(theme.shadow, a / 2.0));
        list.fill(pill, 8.0, faded(theme.surface, a));
        list.border(pill, 8.0, line, faded(theme.border, a));
        let base = cap_baseline(text, pill.y, TIP_H, style.size);
        text.draw_text(list, text.snap(pill.x + (w - tw) / 2.0), base, &label, style);
    }
}

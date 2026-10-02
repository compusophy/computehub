//! The bottom strip (see `home::dock`): the AI button, the dock's wings of favorites and other
//! running apps, and their tooltips.

use gfx::{DrawList, RectF};
use home::dock::{Look, Spot};
use host::paint::{cap_baseline, faded, px};
use host::{app_label, sigil};
use ui::{AppIcon, FontId, TextStyle, Theme};
use wm::WinId;

use crate::Shell;
use crate::desktop::Target;

const TIP_GAP: f32 = 10.0;
const TIP_H: f32 = 24.0;

/// One app on the dock: its registry name, icon and open windows.
pub(crate) type Item = (String, AppIcon, Vec<WinId>);

impl Shell {
    /// Where a window with no tile of its own minimizes to: the AI button.
    pub(crate) fn dock_rect(&self) -> RectF {
        self.strip.button
    }

    pub(crate) fn dock_tile(&self, i: usize) -> RectF {
        self.strip.tile(i)
    }

    pub(crate) fn dock_hit(&self, x: f32, y: f32) -> Option<Target> {
        Some(match self.strip.at(x, y)? {
            Spot::Button => Target::Ai,
            Spot::Tile(i) if i < self.dock.len() => Target::Dock(i),
            _ => Target::Shelf,
        })
    }

    /// The wings, their tiles lifted while hovered over a dot under each running app, and the AI
    /// button (washed while hovered).
    pub(crate) fn draw_dock(&mut self, list: &mut DrawList, theme: &Theme) {
        let (focused, now) = (self.host.wm().focused(), self.host.now_ms);
        let look = |(name, icon, wins): &Item| Look {
            icon: *icon,
            sigil: sigil(name),
            lift: self.motion.lift(name, now),
            running: !wins.is_empty(),
            focused: focused.is_some_and(|f| wins.contains(&f)),
        };
        let mut looks = Vec::new();
        for item in &self.dock {
            looks.push(look(item));
        }
        self.strip.draw(list, &mut self.host.text, theme, &looks);
        let button = (self.hover == Some(Target::Ai)).then_some(self.armed == self.hover);
        self.strip.draw_button(list, &mut self.host.text, theme, button);
    }

    /// The hovered tile's name (or the AI button's, the Assistant) above the strip, fading in
    /// with the lift.
    pub(crate) fn draw_tooltip(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let (t, a, label) = match self.hover {
            Some(Target::Dock(i)) if i < self.dock.len() => {
                let name = &self.dock[i].0;
                (self.dock_tile(i), self.motion.lift(name, now), app_label(name))
            }
            Some(Target::Ai) if !self.overlay.open => {
                (self.strip.button, self.motion.ai.value(now), "Assistant".into())
            }
            _ => return,
        };
        let (top, sw) = (self.size.1 - home::dock::BOTTOM - home::dock::H, self.size.0);
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

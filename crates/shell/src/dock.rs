//! The bottom row (see `home::dock`): the dock's tiles (favorites, then the other running apps)
//! and the Assistant's in the corner, kept tiles carried along the dock, and their tooltips (and
//! the top bar's buttons').

use gfx::{DrawList, RectF};
use home::dock::{Look, Spot};
use host::paint::{cap_baseline, faded, px};
use host::{ASSISTANT, app_label};
use ui::icon::sin;
use ui::{AppIcon, FontId, TextStyle, Theme};
use wm::WinId;

use crate::Shell;
use crate::desktop::Target;

/// A tooltip's height, its gap above a tile and below the top bar.
const TIP_H: f32 = 24.0;
const TIP_GAP: f32 = 10.0;
const TIP_BELOW: f32 = 6.0;
/// How long a working Assistant's dot takes to beat once.
const BEAT_MS: f64 = 1200.0;

/// One app on the dock: its registry name, icon and open windows.
pub(crate) type Item = (String, AppIcon, Vec<WinId>);

impl Shell {
    /// Where a window with no tile of its own minimizes to: the Assistant's.
    pub(crate) fn dock_rect(&self) -> RectF {
        self.dock.strip.assistant
    }

    pub(crate) fn dock_tile(&self, i: usize) -> RectF {
        self.dock.strip.tile(i)
    }

    pub(crate) fn dock_hit(&self, x: f32, y: f32) -> Option<Target> {
        Some(match self.dock.strip.at(x, y)? {
            Spot::Assistant => Target::Assistant,
            Spot::Tile(i) => Target::Dock(i),
        })
    }

    /// Icons or a dock tile pressed, or carried (never both: one press at a time).
    pub(crate) fn carried(&self) -> Option<&home::grid::Carry> {
        self.grid.carry.as_ref().or(self.dock.carry.as_ref())
    }

    /// Whether icons or a dock tile show carried.
    pub(crate) fn carrying(&self) -> bool {
        self.carried().is_some_and(|c| c.lifted)
    }

    /// The dock's tile `i` as it shows now.
    fn look(&self, i: usize, (name, icon, wins): &Item) -> Look {
        let (focused, now) = (self.key_target(), self.host.now_ms);
        let r = self.dock_tile(i);
        let (lift, x) = self.motion.tile(name, now).unwrap_or((0.0, r.x));
        let r = RectF { x, ..r };
        let (mark, dot) =
            (self.marks.get(i).copied().flatten(), if wins.is_empty() { 0.0 } else { 1.0 });
        Look {
            icon: *icon,
            mark,
            r,
            lift,
            dot,
            focused: focused.is_some_and(|f| wins.contains(&f)),
        }
    }

    /// The row: the dock's tiles (lifted while hovered, sliding to their places, none where one
    /// carried lands) over a dot under each running app, the hairline between the groups; the
    /// Assistant's tile, over a dot while the overlay shows or an answer it stepped aside from
    /// waits (the accent's then, and while it has the keys or works, beating then).
    pub(crate) fn draw_dock(&mut self, list: &mut DrawList, theme: &Theme) {
        let (now, carried) = (self.host.now_ms, self.dock.carried(self.pointer).map(|c| c.0));
        let mut looks = Vec::new();
        for (i, item) in self.tiles.iter().enumerate() {
            if Some(i) != carried {
                looks.push(self.look(i, item));
            }
        }
        let (o, working) = (self.overlay, self.host.agent.working);
        let beat = 0.65 + 0.35 * sin((now / BEAT_MS).fract() as f32 * std::f32::consts::TAU);
        let dot = if working { beat } else { f32::from(u8::from(o.open || o.unread)) };
        let (icon, r) = (self.host.icon(ASSISTANT).unwrap_or_default(), self.dock.strip.assistant);
        let (lift, focused) = (self.motion.ai.value(now), working || o.open && o.focus || o.unread);
        looks.push(Look { icon, mark: None, r, lift, dot, focused });
        self.dock.strip.draw(list, &mut self.host.text, theme, &looks);
    }

    /// The tile carried, over everything but menus.
    pub(crate) fn draw_carried_tile(&mut self, list: &mut DrawList, theme: &Theme) {
        let Some((i, r)) = self.dock.carried(self.pointer) else { return };
        let Some(item) = self.tiles.get(i) else { return };
        let look = Look { r, ..self.look(i, item) };
        look.draw_carried(list, &mut self.host.text, theme);
    }

    /// The hovered tile's name above it (the Assistant's while the overlay hides), or the top
    /// bar's button's below it (the mark's, what its press does next), fading in with the lift,
    /// kept on the screen; none once pressed, until the pointer leaves it and comes back.
    pub(crate) fn draw_tooltip(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        if self.hush.is_some() && self.hush == self.hover {
            return;
        }
        let (t, a, label) = match self.hover {
            Some(Target::Dock(i)) if i < self.tiles.len() => {
                let name = &self.tiles[i].0;
                let lift = self.motion.tile(name, now).map_or(0.0, |t| t.0);
                (self.dock_tile(i), lift, app_label(name))
            }
            Some(Target::Assistant) if !self.overlay.open => {
                (self.dock.strip.assistant, self.motion.ai.value(now), app_label(ASSISTANT))
            }
            Some(Target::Bar(b)) => {
                let r = home::bar::buttons(self.size.0).into_iter().find(|r| r.1 == b);
                let label = b.label(self.brings_back()).into();
                (r.map_or(RectF::default(), |r| r.0), self.motion.bar.value(now), label)
            }
            _ => return,
        };
        let below = matches!(self.hover, Some(Target::Bar(_)));
        let y = if below { t.y + t.h + TIP_BELOW } else { t.y - TIP_GAP - TIP_H };
        let (text, sw) = (&mut self.host.text, self.size.0);
        let line = px(text, 1.0);
        let style = TextStyle::new(FontId::Sans, 12.0, faded(theme.text, a));
        let tw = text.measure(&label, style);
        let w = (tw + 20.0).round();
        let x = (t.x + (t.w - w) / 2.0).round().min(sw - w - 4.0).max(4.0);
        let pill = RectF::new(x, y, w, TIP_H);
        list.shadow_offset(pill, 8.0, 12.0, 4.0, faded(theme.shadow, a / 2.0));
        list.fill(pill, 8.0, faded(theme.surface, a));
        list.border(pill, 8.0, line, faded(theme.border, a));
        let base = cap_baseline(text, pill.y, TIP_H, style.size);
        text.draw_text(list, text.snap(pill.x + (w - tw) / 2.0), base, &label, style);
    }
}

//! The top bar: compusophy's mark at the left (Welcome), the date and time in the middle (the
//! time alone on a phone), Feedback (a bug) and Settings at the right.

use gfx::{DrawList, RectF};
use host::paint::{cap_baseline, px};
use ui::icon::Glyph;
use ui::{FontId, TextStyle, Theme};

use crate::desktop::Target;
use crate::{BAR_H, Shell};

/// A button's side (its hit box), its distance from the screen's edge, the mark's and the other
/// glyphs' sides, the hover wash's diameter, the clock's size.
const BTN: f32 = 44.0;
const EDGE: f32 = 5.0;
const MARK: f32 = 26.0;
const GLYPH: f32 = 20.0;
const WASH: f32 = 34.0;
const CLOCK_SIZE: f32 = 13.0;

impl Shell {
    fn bar_buttons(&self) -> [(RectF, Target); 3] {
        let (at, w) = (|x: f32| RectF::new(x, 0.0, BTN, BTN), self.size.0);
        [
            (at(EDGE), Target::Mark),
            (at(w - EDGE - 2.0 * BTN), Target::Feedback),
            (at(w - EDGE - BTN), Target::Settings),
        ]
    }

    pub(crate) fn bar_hit(&self, x: f32, y: f32) -> Target {
        let mut buttons = self.bar_buttons().into_iter();
        buttons.find(|b| b.0.contains(x, y)).map_or(Target::Top, |b| b.1)
    }

    /// The glass, the buttons (a round wash while hovered), and the clock once ticked (the time
    /// alone if the date does not fit).
    pub(crate) fn draw_bar(&mut self, list: &mut DrawList, theme: &Theme) {
        let (w, buttons, text) = (self.size.0, self.bar_buttons(), &mut self.host.text);
        let line = px(text, 1.0);
        list.fill(RectF::new(0.0, 0.0, w, BAR_H), 0.0, theme.glass);
        list.fill(RectF::new(0.0, BAR_H - line, w, line), 0.0, theme.border);
        let square = |r: RectF, side: f32| r.inset((r.w - side) / 2.0);
        for (r, target) in buttons {
            if self.hover == Some(target) {
                list.fill(square(r, WASH), WASH / 2.0, theme.wash(self.armed == Some(target)));
            }
            let (side, glyph, ink) = match target {
                Target::Mark => (MARK, Glyph::Mark, theme.text),
                Target::Feedback => (GLYPH, Glyph::Bug, theme.text),
                _ => (GLYPH, Glyph::Cog, theme.text),
            };
            ui::icon::draw(list, text, square(r, side), glyph, ink);
        }
        let Some(time) = self.clock else {
            return;
        };
        let (date, clock) = (time.date(), time.clock());
        let style = TextStyle::new(FontId::Sans, CLOCK_SIZE, theme.text_dim);
        let (dw, sw, cw) =
            (text.measure(&date, style), text.measure("  ", style), text.measure(&clock, style));
        let base = cap_baseline(text, 0.0, BAR_H, CLOCK_SIZE);
        let x = text.snap((w - dw - sw - cw) / 2.0);
        let x = if !home::narrow(w) && dw + sw + cw <= w - 2.0 * (EDGE + 2.0 * BTN + 13.0) {
            text.draw_text(list, x, base, &date, style);
            x + dw + sw
        } else {
            text.snap((w - cw) / 2.0)
        };
        text.draw_text(list, x, base, &clock, style.with_color(theme.text));
    }
}

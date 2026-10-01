//! The top bar: the mark (the launcher) at the left, the date and time in
//! the middle, the settings and theme buttons at the right.

use gfx::{DrawList, RectF};
use host::paint::{cap_baseline, contrast, mark, px, sliders};
use ui::{FontId, TextStyle, Theme};

use crate::desktop::Target;
use crate::{BAR_H, Shell};

/// A button's size, its glyph's, and the glyphs' distance from the edges.
const BTN_W: f32 = 28.0;
const BTN_H: f32 = 24.0;
const GLYPH: f32 = 14.0;
const EDGE: f32 = 12.0;
const CLOCK_SIZE: f32 = 13.0;

impl Shell {
    fn bar_buttons(&self) -> [(RectF, Target); 3] {
        let at = |cx: f32| RectF::new(cx - BTN_W / 2.0, (BAR_H - BTN_H) / 2.0, BTN_W, BTN_H);
        let right = self.size.0 - EDGE - GLYPH / 2.0;
        [
            (at(EDGE + GLYPH / 2.0), Target::Mark),
            (at(right - BTN_W), Target::Settings),
            (at(right), Target::Theme),
        ]
    }

    pub(crate) fn bar_hit(&self, x: f32, y: f32) -> Target {
        let mut buttons = self.bar_buttons().into_iter();
        buttons.find(|b| b.0.contains(x, y)).map_or(Target::Bar, |b| b.1)
    }

    /// The glass, the buttons, and the clock once ticked (the time alone if
    /// the date does not fit).
    pub(crate) fn draw_bar(&mut self, list: &mut DrawList, theme: &Theme) {
        let (w, line) = (self.size.0, px(&self.host.text, 1.0));
        list.fill(RectF::new(0.0, 0.0, w, BAR_H), 0.0, theme.glass);
        list.fill(RectF::new(0.0, BAR_H - line, w, line), 0.0, theme.border);
        for (r, target) in self.bar_buttons() {
            if self.hover == Some(target) {
                list.fill(r, 8.0, theme.wash(self.armed == Some(target)));
            }
            let at = (r.x + r.w / 2.0, BAR_H / 2.0);
            match target {
                Target::Mark => mark(list, at, GLYPH, theme.accent),
                Target::Settings => sliders(list, at, GLYPH, theme.text),
                _ => contrast(list, at, GLYPH, theme.text),
            }
        }
        let Some(time) = self.clock else {
            return;
        };
        let (date, clock) = (time.date(), time.clock());
        let style = TextStyle::new(FontId::Sans, CLOCK_SIZE, theme.text_dim);
        let text = &mut self.host.text;
        let (dw, sw, cw) =
            (text.measure(&date, style), text.measure("  ", style), text.measure(&clock, style));
        let base = cap_baseline(text, 0.0, BAR_H, CLOCK_SIZE);
        let x = text.snap((w - dw - sw - cw) / 2.0);
        let x = if dw + sw + cw <= w - 2.0 * (EDGE + GLYPH + BTN_W + EDGE) {
            text.draw_text(list, x, base, &date, style);
            x + dw + sw
        } else {
            text.snap((w - cw) / 2.0)
        };
        text.draw_text(list, x, base, &clock, style.with_color(theme.text));
    }
}

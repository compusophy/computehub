//! The top bar: the mark at the left (the launcher), the date and time in
//! the middle, the settings and theme buttons at the right.

use gfx::{DrawList, RectF};
use ui::{FontId, TextStyle, Theme};

use crate::desktop::Target;
use crate::{BAR_H, Shell};
use host::paint::{cap_baseline, contrast, mark, px, sliders};

/// A bar button's hit (and hover) rect, the side of its glyph, and the
/// glyphs' distance from the screen's edges.
const BTN_W: f32 = 28.0;
const BTN_H: f32 = 24.0;
const GLYPH: f32 = 14.0;
const EDGE: f32 = 12.0;
const CLOCK_SIZE: f32 = 13.0;
impl Shell {
    /// The mark, settings and theme buttons.
    fn bar_buttons(&self) -> [(RectF, Target); 3] {
        let at = |cx: f32| RectF::new(cx - BTN_W / 2.0, (BAR_H - BTN_H) / 2.0, BTN_W, BTN_H);
        let right = self.size.0 - EDGE - GLYPH / 2.0;
        [
            (at(EDGE + GLYPH / 2.0), Target::Mark),
            (at(right - BTN_W), Target::Settings),
            (at(right), Target::Theme),
        ]
    }

    /// The bar's button at `(x, y)`, else the bare bar.
    pub(crate) fn bar_hit(&self, x: f32, y: f32) -> Target {
        let mut buttons = self.bar_buttons().into_iter();
        buttons.find(|b| b.0.contains(x, y)).map_or(Target::Bar, |b| b.1)
    }

    /// Glass across the top with a hairline under it, the buttons (a wash
    /// while hovered), and the clock once a tick has set it.
    pub(crate) fn draw_bar(&mut self, list: &mut DrawList, theme: &Theme) {
        let (w, line) = (self.size.0, px(self.host.text(), 1.0));
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
        let text = self.host.text_mut();
        let (dw, sw, cw) =
            (text.measure(&date, style), text.measure("  ", style), text.measure(&clock, style));
        let base = cap_baseline(text, 0.0, BAR_H, CLOCK_SIZE);
        // Without room for the date beside the buttons, the time alone.
        let room = w - 2.0 * (EDGE + GLYPH + BTN_W + EDGE);
        let x = text.snap((w - dw - sw - cw) / 2.0);
        if dw + sw + cw <= room {
            text.draw_text(list, x, base, &date, style);
            text.draw_text(list, x + dw + sw, base, &clock, style.with_color(theme.text));
        } else {
            text.draw_text(
                list,
                text.snap((w - cw) / 2.0),
                base,
                &clock,
                style.with_color(theme.text),
            );
        }
    }
}

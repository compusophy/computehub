//! The everything bar: a pill centered under the dock where the person types anything; the
//! launcher's [`crate::panel`] shows above it what that opens or asks.

use gfx::{DrawList, RectF};
use host::paint::{cap_baseline, faded, px, sheen};
use ui::icon::Glyph;
use ui::{FontId, TextStyle, TextSystem, Theme};

/// The bar's height and its space above the screen's bottom.
pub const H: f32 = 44.0;
pub const BOTTOM: f32 = 13.0;
/// Its largest width, its least margin from the screen's sides, its glyph's side and inset, and
/// the text's size.
const MAX_W: f32 = 560.0;
const SIDE: f32 = 16.0;
const GLYPH: f32 = 20.0;
const INSET: f32 = 13.0;
const SIZE: f32 = 15.0;
/// What it says while empty.
pub const HINT: &str = "Ask anything, or open an app";

/// The bar on a `w` x `h` screen.
pub fn rect((w, h): (f32, f32)) -> RectF {
    let fw = (w - 2.0 * SIDE).clamp(0.0, MAX_W).round();
    RectF::new(((w - fw) / 2.0).round(), h - BOTTOM - H, fw, H)
}

/// The bar in `r`: glass (washed while hovered, `Some(held)`), the Assistant's glyph in the
/// accent, then `query` (its end, if long) or the hint; while `focused`, an accent ring and a
/// caret.
pub fn draw(
    list: &mut DrawList,
    text: &mut TextSystem,
    theme: &Theme,
    r: RectF,
    query: &str,
    focused: bool,
    hover: Option<bool>,
) {
    let (line, radius) = (px(text, 1.0), H / 2.0);
    list.shadow_offset(r, radius, 21.0, 5.0, faded(theme.shadow, 0.6));
    list.fill(r, radius, theme.glass);
    if let Some(down) = hover {
        list.fill(r, radius, theme.wash(down));
    }
    let (ring, edge) = if focused { (px(text, 1.5), theme.accent) } else { (line, theme.border) };
    list.border(r, radius, ring, edge);
    sheen(list, r, radius, line, theme.highlight);
    let g = RectF::new(r.x + INSET, r.y + (H - GLYPH) / 2.0, GLYPH, GLYPH);
    ui::icon::draw(list, text, g, Glyph::Assistant, theme.accent);
    let x0 = g.x + GLYPH + INSET;
    let room = RectF::new(x0, r.y, (r.x + r.w - radius - x0).max(0.0), H);
    let style = TextStyle::new(FontId::Sans, SIZE, theme.text);
    let (base, tw, caret) =
        (cap_baseline(text, r.y, H, SIZE), text.measure(query, style), px(text, 1.5));
    let x = text.snap(room.x + (room.w - caret - tw).min(0.0));
    list.push_clip(room);
    if query.is_empty() {
        text.draw_text(list, x, base, HINT, style.with_color(theme.text_faint));
    } else {
        text.draw_text(list, x, base, query, style);
    }
    if focused {
        let (a, d) = (text.ascent(style), text.descent(style));
        list.fill(RectF::new(text.snap(x + tw), base - a, caret, a + d), 0.0, theme.accent);
    }
    list.pop_clip();
}

//! The desktop's icons, behind the windows: a grid in the work area from its top-left corner,
//! filling columns on a wide screen and rows of four on a narrow one; each cell a tile with its
//! label under it in up to two lines.

use gfx::{DrawList, RectF};
use host::paint::cap_baseline;
use ui::{AppIcon, FontId, TextStyle, TextSystem, Theme};

/// A cell's size, the grid's margin in the work area, a tile's side and its distance from the
/// cell's top, the label's size and line height.
pub const CELL: (f32, f32) = (89.0, 96.0);
const MARGIN: f32 = 13.0;
const TILE: f32 = 48.0;
const TOP: f32 = 5.0;
const SIZE: f32 = 12.0;
const LINE: f32 = 15.0;

/// Cell `i` in the work area `a`: down the columns from the top-left on a wide screen, across
/// rows of four evenly spaced cells on a narrow one.
pub fn cell(i: usize, a: RectF, narrow: bool) -> RectF {
    let (left, top) = (a.x + MARGIN, a.y + MARGIN);
    let (col, row, w) = if narrow {
        (i % 4, i / 4, ((a.w - 2.0 * MARGIN) / 4.0).max(0.0).floor())
    } else {
        let per = (((a.h - 2.0 * MARGIN) / CELL.1).max(1.0)) as usize;
        (i / per, i % per, CELL.0)
    };
    RectF::new((left + col as f32 * w).round(), top + row as f32 * CELL.1, w, CELL.1)
}

/// The icon of `n` under `(x, y)`.
pub fn at(n: usize, a: RectF, narrow: bool, x: f32, y: f32) -> Option<usize> {
    (0..n).find(|&i| cell(i, a, narrow).contains(x, y))
}

/// The icon of `label` in cell `r`: a wash while hovered (`Some(held)`), the tile, the label
/// centered under it in two lines at most, the second cut with an ellipsis.
pub fn draw(
    list: &mut DrawList,
    text: &mut TextSystem,
    theme: &Theme,
    r: RectF,
    (icon, label): (AppIcon, &str),
    hover: Option<bool>,
) {
    if let Some(down) = hover {
        list.fill(r.inset(2.0), 13.0, theme.wash(down));
    }
    let t = RectF::new(r.x + (r.w - TILE) / 2.0, r.y + TOP + 5.0, TILE, TILE);
    ui::icon::tile(list, text, t, icon.glyph, icon.hue, theme);
    let (style, room) = (TextStyle::new(FontId::Sans, SIZE, theme.text), r.w - 10.0);
    let lines = text.wrap(label, style, room);
    let first = lines.first().copied().unwrap_or("");
    // The rest of the label after the first line, from where the second began.
    let rest = lines.get(1).map_or("", |l| &label[l.as_ptr() as usize - label.as_ptr() as usize..]);
    let rest = text.ellipsize(rest, style, room);
    for (k, line) in [first, &rest].into_iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let lw = text.measure(line, style);
        let x = text.snap(r.x + (r.w - lw) / 2.0);
        let y = t.y + TILE + 8.0 + k as f32 * LINE;
        text.draw_text(list, x, cap_baseline(text, y, LINE, SIZE), line, style);
    }
}

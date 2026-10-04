//! The home screen's grid: every app, behind the windows, in the work area from its top-left
//! corner, its cells down the columns on a wide screen and across rows of four on a narrow one;
//! each cell a tile with its label under it in up to two lines. Where each app sits is the
//! person's ([`crate::place`]), kept as the [`PREF`] preference: icons move by drag, several at
//! once once selected by a box.

use gfx::{DrawList, RectF};
use host::Entry;
use host::paint::{cap_baseline, faded, px};
use ui::icon::Mark;
use ui::{AppIcon, FontId, TextStyle, TextSystem, Theme};

use crate::place::Dims;

/// The preference that keeps where the icons sit ([`crate::place::format`]; before, the apps'
/// names in order); not `home`, which the page's storage gives the saved `/home`
/// (`compusophy.home`).
pub const PREF: &str = "home.order";
/// A cell's size, the grid's margin in the work area, a tile's side and its distance from the
/// cell's top, the label's size and line height; how much larger a carried tile is; the
/// selection's wash (and the box's fill) alpha.
pub const CELL: (f32, f32) = (89.0, 96.0);
const MARGIN: f32 = 13.0;
const TILE: f32 = 48.0;
const TOP: f32 = 5.0;
const SIZE: f32 = 12.0;
const LINE: f32 = 15.0;
const LIFT: f32 = 0.08;
const WASH: u8 = 31;

/// The cell at position `i` in reading order in the work area `a`: down the columns from the
/// top-left on a wide screen, across rows of four evenly spaced cells on a narrow one.
pub fn cell(i: usize, a: RectF, narrow: bool) -> RectF {
    let (left, top) = (a.x + MARGIN, a.y + MARGIN);
    let (col, row, w) = if narrow {
        (i % 4, i / 4, ((a.w - 2.0 * MARGIN) / 4.0).max(0.0).floor())
    } else {
        (i / per(a), i % per(a), CELL.0)
    };
    RectF::new((left + col as f32 * w).round(), top + row as f32 * CELL.1, w, CELL.1)
}

/// How many cells a column of `a` holds (at least one).
fn per(a: RectF) -> usize {
    ((a.h - 2.0 * MARGIN) / CELL.1).max(1.0) as usize
}

/// The grid's shape in the work area `a`: four columns on a narrow screen, as many as fit on a
/// wide one; as many rows as fit; at least one of each.
pub fn dims(a: RectF, narrow: bool) -> Dims {
    let cols = if narrow { 4 } else { ((a.w - 2.0 * MARGIN) / CELL.0).max(1.0) as usize };
    Dims::new(cols, per(a), narrow)
}

/// Which of the icons at positions `spots` is under `(x, y)`.
pub fn at(spots: &[usize], a: RectF, narrow: bool, x: f32, y: f32) -> Option<usize> {
    spots.iter().position(|&s| cell(s, a, narrow).contains(x, y))
}

/// The position of the cell shown under `(x, y)` (or nearest it): where icons dropped with
/// their first cell's center there go.
pub fn slot(a: RectF, narrow: bool, (x, y): (f32, f32)) -> usize {
    let c = cell(0, a, narrow);
    // `as` saturates: NaN and negatives are 0, past the screen is far.
    dims(a, narrow).nearest(((x - c.x) / c.w) as usize, ((y - c.y) / c.h) as usize)
}

/// How an icon shows: hovered (`Some(held)`), selected, and lifted (0 to 1) while carried.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub hover: Option<bool>,
    pub selected: bool,
    pub lift: f32,
}

/// The icon (`icon`, or a `.app` file's mark; a folder's apps, if it holds any) of `label` in
/// cell `r`: a wash while hovered, the accent's ring and wash while selected, the tile (larger
/// and shadowed as it lifts), the label centered under it in two lines at most, the second cut
/// with an ellipsis.
pub fn draw(
    list: &mut DrawList,
    text: &mut TextSystem,
    theme: &Theme,
    r: RectF,
    (icon, mark, label, inside): (AppIcon, Option<&Mark>, &str, &[Entry]),
    s: State,
) {
    let well = r.inset(2.0);
    if s.selected {
        list.fill(well, 13.0, theme.accent.with_alpha(WASH));
        list.border(well, 13.0, px(text, 1.0), theme.accent);
    }
    if let Some(down) = s.hover {
        list.fill(well, 13.0, theme.wash(down));
    }
    let side = TILE * (1.0 + LIFT * s.lift);
    let t = RectF::new(r.x + (r.w - side) / 2.0, r.y + TOP + 5.0 + (TILE - side) / 2.0, side, side);
    if s.lift > 0.0 {
        list.shadow_offset(t, side / 6.0, 21.0, 8.0, faded(theme.shadow, s.lift));
    }
    match inside {
        [] => crate::tile(list, text, t, (icon, mark), theme),
        apps => crate::folders::tile(list, text, t, apps, theme),
    }
    let (style, room) = (TextStyle::new(FontId::Sans, SIZE, theme.text), r.w - 10.0);
    let lines = text.wrap(label, style, room);
    let first = lines.first().copied().unwrap_or("");
    // The rest of the label after the first line, from where the second began.
    let at = |l: &&str| l.as_ptr() as usize - label.as_ptr() as usize;
    let rest = lines.get(1).and_then(|l| label.get(at(l)..)).unwrap_or_default();
    let rest = text.ellipsize(rest, style, room);
    for (k, line) in [first, &rest].into_iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let lw = text.measure(line, style);
        let x = text.snap(r.x + (r.w - lw) / 2.0);
        let y = r.y + TOP + 5.0 + TILE + 8.0 + k as f32 * LINE;
        text.draw_text(list, x, cap_baseline(text, y, LINE, SIZE), line, style);
    }
}

/// The selection box from `a` to `b`: the accent's wash and hairline.
pub fn draw_box(
    list: &mut DrawList,
    text: &TextSystem,
    theme: &Theme,
    a: (f32, f32),
    b: (f32, f32),
) {
    let r = boxed(a, b);
    list.fill(r, 0.0, theme.accent.with_alpha(WASH));
    list.border(r, 0.0, px(text, 1.0), theme.accent);
}

/// The rect with corners `a` and `b`, either way round.
pub fn boxed(a: (f32, f32), b: (f32, f32)) -> RectF {
    RectF::new(a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

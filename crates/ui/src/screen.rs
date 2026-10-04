//! A terminal's screen as [`uiwire::Node::Screen`] carries it, drawn cell by cell: Mono 13 px in
//! [`uiwire::CELL`] cells from [`uiwire::INSET`] in (the same before that font arrives, its cells
//! empty), in the theme's xterm colors, with a steady accent block cursor (an outline
//! unfocused).

use gfx::{DrawList, RectF, Rgba};
use text::TextSystem;

use crate::{FontId, TextStyle, Theme};

/// The size the cells' characters are drawn at.
const SIZE: f32 = 13.0;
/// The attribute bits a cell's last byte holds that drawing reads.
const BOLD: u8 = 1;
const DIM: u8 = 1 << 1;
const UNDERLINE: u8 = 1 << 3;
const INVERSE: u8 = 1 << 5;
const HIDDEN: u8 = 1 << 6;
const STRIKE: u8 = 1 << 7;

/// One cell: its char and width, its colors as the wire carries them, its attributes.
#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    width: u8,
    fg: u32,
    bg: u32,
    attrs: u8,
}

/// Cell `i` of `cells` (blank past the end).
fn cell(cells: &[u8], i: usize) -> Cell {
    let at = i * uiwire::CELL_BYTES;
    let word = |k: usize| {
        cells
            .get(at + 4 * k..at + 4 * k + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let ch = char::from_u32(word(0) & 0x1f_ffff).unwrap_or(' ');
    let (width, attrs) = ((word(0) >> 24) as u8 & 3, cells.get(at + 12).copied().unwrap_or(0));
    Cell { ch, width, fg: word(1), bg: word(2), attrs }
}

/// The screen `cols` x `rows` of `cells` in `r`, and the cursor at (row, column) if shown:
/// background runs, then the characters and their lines.
pub fn draw(
    list: &mut DrawList,
    ts: &mut TextSystem,
    t: &Theme,
    r: RectF,
    (cols, rows, cursor, cells): (u16, u16, Option<(u16, u16)>, &[u8]),
    focused: bool,
) {
    let (cw, lh) = (f32::from(uiwire::CELL.0), f32::from(uiwire::CELL.1));
    let asc = ts.ascent(TextStyle::new(FontId::Mono, SIZE, t.text));
    let inset = f32::from(uiwire::INSET);
    let (ox, oy, cols) = (ts.snap(r.x + inset), ts.snap(r.y + inset), usize::from(cols));
    let shown = |c: &Cell| c.width != 0 && c.attrs & HIDDEN == 0;
    // Lines and the outline: whole device pixels, at least one.
    let one = ts.dpr().round().max(1.0) / ts.dpr();
    for row in 0..usize::from(rows) {
        let (y, at) = (oy + row as f32 * lh, |x: usize| cell(cells, row * cols + x));
        let mut x = 0;
        while x < cols {
            let bg = colors(&at(x), t).1;
            let n = (x..cols).take_while(|&k| colors(&at(k), t).1 == bg).count();
            if let Some(bg) = bg {
                list.fill(RectF::new(ox + x as f32 * cw, y, n as f32 * cw, lh), 0.0, bg);
            }
            x += n;
        }
        for x in 0..cols {
            let c = at(x);
            if !shown(&c) {
                continue;
            }
            let (fg, left) = (colors(&c, t).0, ox + x as f32 * cw);
            let (w, base) = (cw * f32::from(c.width), y + asc);
            ts.draw_cell_char(list, left, base, w, c.ch, SIZE, fg);
            let line = |dy| RectF::new(left, base + dy, w, one);
            if c.attrs & UNDERLINE != 0 {
                list.fill(line(one), 0.0, fg);
            }
            if c.attrs & STRIKE != 0 {
                list.fill(line(-(SIZE * 0.3).round()), 0.0, fg);
            }
        }
    }
    let Some((cr, cc)) = cursor else { return };
    let c = cell(cells, usize::from(cr) * cols + usize::from(cc));
    let w = cw * f32::from(c.width.max(1));
    let r = RectF::new(ox + f32::from(cc) * cw, oy + f32::from(cr) * lh, w, lh);
    if !focused {
        return list.border(r, 0.0, one, t.accent);
    }
    list.fill(r, 0.0, t.accent);
    if shown(&c) {
        ts.draw_cell_char(list, r.x, r.y + asc, w, c.ch, SIZE, t.accent_text);
    }
}

/// A cell's foreground and, unless default, background. Bold brightens the first eight colors;
/// inverse puts the opaque surface in front.
fn colors(c: &Cell, t: &Theme) -> (Rgba, Option<Rgba>) {
    let rgb = |col: u32| match col >> 24 {
        1 => Some(t.xterm(col as u8)),
        2 => Some(Rgba((col >> 16) as u8, (col >> 8) as u8, col as u8, 255)),
        _ => None,
    };
    let is = |a: u8| c.attrs & a != 0;
    let fg = match c.fg {
        f if f >> 24 == 1 && (f as u8) < 8 && is(BOLD) => t.xterm(f as u8 + 8),
        f => rgb(f).unwrap_or(t.text),
    };
    let (fg, bg) = match is(INVERSE) {
        true => (rgb(c.bg).unwrap_or(t.surface.with_alpha(255)), Some(fg)),
        false => (fg, rgb(c.bg)),
    };
    (if is(DIM) { fg.with_alpha(153) } else { fg }, bg)
}

//! Drawing helpers: device pixels, centered capitals, lit edges, fading, window control glyphs.

use gfx::{DrawList, Icon, RectF, Rgba};
use ui::TextSystem;

/// `v` logical pixels as whole device pixels, at least one.
pub fn px(text: &TextSystem, v: f32) -> f32 {
    let d = text.dpr();
    (v * d).round().max(1.0) / d
}

/// The baseline that centers capitals (0.727 em in Inter) in `h` from `top`.
pub fn cap_baseline(text: &TextSystem, top: f32, h: f32, size: f32) -> f32 {
    text.snap(top + (h + 0.727 * size) / 2.0)
}

/// The lit top edge of a surface: its outline clipped to the top `line`.
pub fn sheen(list: &mut DrawList, r: RectF, radius: f32, line: f32, color: Rgba) {
    list.push_clip(RectF { h: line, ..r });
    list.border(r, radius, line, color);
    list.pop_clip();
}

pub fn faded(c: Rgba, a: f32) -> Rgba {
    c.with_alpha((f32::from(c.3) * a.clamp(0.0, 1.0)).round() as u8)
}

/// Window control `i`'s glyph on its circle `c`: minimize, maximize (or restore), close.
pub fn control_glyph(
    list: &mut DrawList,
    text: &TextSystem,
    c: RectF,
    i: usize,
    maximized: bool,
    [fill, ink]: [Rgba; 2],
) {
    let line = px(text, 1.0);
    let (cx, cy) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
    let sq = |x: f32, y: f32, side: f32| RectF::new(text.snap(x), text.snap(y), side, side);
    match i {
        0 => list.fill(RectF::new(cx - 3.0, text.snap(cy - line / 2.0), 6.0, line), 0.0, ink),
        1 if maximized => {
            list.border(sq(cx - 1.0, cy - 4.0, 5.0), 1.0, line, ink);
            list.fill(sq(cx - 4.0, cy - 1.0, 5.0), 1.0, fill);
            list.border(sq(cx - 4.0, cy - 1.0, 5.0), 1.0, line, ink);
        }
        1 => list.border(sq(cx - 3.0, cy - 3.0, 6.0), 1.0, line, ink),
        _ => list.icon(RectF::new(cx - 4.5, cy - 4.5, 9.0, 9.0), Icon::Cross, 1.25, ink),
    }
}

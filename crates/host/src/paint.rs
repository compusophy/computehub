//! Drawing helpers: device pixels, centered capitals, lit edges, fading, icons, small glyphs.

use gfx::{DrawList, Icon, RectF, Rgba};
use ui::TextSystem;

/// The side an app icon is designed at.
pub const ICON: f32 = 44.0;

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

fn square((cx, cy): (f32, f32), side: f32) -> RectF {
    RectF::new(cx - side / 2.0, cy - side / 2.0, side, side)
}

/// compusophy's mark: a ring with a dot.
pub fn mark(list: &mut DrawList, at: (f32, f32), side: f32, color: Rgba) {
    list.border(square(at, side), side / 2.0, 1.5, color);
    list.fill(square(at, 5.0), 2.5, color);
}

/// Settings: two sliders.
pub fn sliders(list: &mut DrawList, (cx, cy): (f32, f32), side: f32, color: Rgba) {
    for d in [-3.0, 3.0] {
        list.fill(RectF::new(cx - side / 2.0, cy + d, side, 1.0), 0.5, color);
        list.fill(square((cx + d, cy + d + 0.5), 5.0), 2.5, color);
    }
}

/// Light and dark: a ring with its left half filled.
pub fn contrast(list: &mut DrawList, at: (f32, f32), side: f32, color: Rgba) {
    let g = square(at, side);
    list.border(g, side / 2.0, 1.5, color);
    list.push_clip(RectF { w: side / 2.0, ..g });
    list.fill(g, side / 2.0, color);
    list.pop_clip();
}

/// A magnifier: an 11 px ring and a handle of dots.
pub fn magnifier(list: &mut DrawList, (cx, cy): (f32, f32), color: Rgba) {
    list.border(square((cx, cy), 11.0), 5.5, 1.75, color);
    for i in 0..=6 {
        let d = 4.6 + i as f32 * 0.5;
        list.fill(square((cx + d, cy + d), 1.8), 0.9, color);
    }
}

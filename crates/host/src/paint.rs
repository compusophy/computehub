//! Drawing helpers the desktop's layers share: whole device pixels,
//! centered capitals, lit top edges, fading, app icons, and the desktop's
//! small glyphs.

use gfx::{DrawList, Icon, RectF, Rgba};
use ui::theme::mix;
use ui::{AppIcon, FontId, TextStyle, TextSystem};

/// Cap height of Inter, in ems: one-line labels center their capitals.
pub const CAP: f32 = 0.727;
/// The side an app icon is designed at (a dock tile, a [`ui::Ui::tile`]'s
/// icon); [`draw_icon`] scales from it.
pub const ICON: f32 = 44.0;
const WHITE: Rgba = Rgba(255, 255, 255, 255);
const BLACK: Rgba = Rgba(0, 0, 0, 255);

/// `v` logical pixels as whole device pixels, at least one.
pub fn px(text: &TextSystem, v: f32) -> f32 {
    let d = text.dpr();
    (v * d).round().max(1.0) / d
}

/// The baseline that centers capitals of `size` px in `h` px from `top`.
pub fn cap_baseline(text: &TextSystem, top: f32, h: f32, size: f32) -> f32 {
    text.snap(top + (h + CAP * size) / 2.0)
}

/// The lit top edge of glass or a raised surface: its `line`-wide outline
/// clipped to the top row.
pub fn sheen(list: &mut DrawList, r: RectF, radius: f32, line: f32, color: Rgba) {
    list.push_clip(RectF { h: line, ..r });
    list.border(r, radius, line, color);
    list.pop_clip();
}

/// `c` at `a` (0 to 1) of its alpha.
pub fn faded(c: Rgba, a: f32) -> Rgba {
    c.with_alpha((f32::from(c.3) * a.clamp(0.0, 1.0)).round() as u8)
}

/// An app icon in the square `r`, as [`ui::Ui::tile`] draws one at
/// [`ICON`] px and to scale: a rounded square in a vertical gradient of the
/// app's hue with a faint white rim and a drop shadow (half of `shadow`),
/// its glyph (else `label`'s first letter) centered in white.
pub fn draw_icon(
    list: &mut DrawList,
    text: &mut TextSystem,
    r: RectF,
    icon: AppIcon,
    label: &str,
    shadow: Rgba,
) {
    let k = r.w / ICON;
    let (radius, line) = (12.0 * k, px(text, 1.0));
    list.shadow_offset(r, radius, 8.0 * k, 2.0 * k, shadow.with_alpha(shadow.3 / 2));
    list.gradient(r, radius, mix(icon.hue, WHITE, 0.16), mix(icon.hue, BLACK, 0.16), 0.0);
    list.border(r, radius, line, WHITE.with_alpha(36));
    let first: String = label.chars().take(1).map(crate::search::upper).collect();
    let glyph = if icon.glyph.is_empty() { first.as_str() } else { icon.glyph };
    let font = match glyph.chars().all(|c| c.is_ascii_alphanumeric()) {
        true => FontId::SansBold,
        false => FontId::Mono,
    };
    let mut style = TextStyle::new(font, (80.0 * k).round() / 4.0, WHITE);
    let room = r.w - 16.0 * k;
    let w = text.measure(glyph, style);
    if w > room {
        style.size = (style.size * room / w * 4.0).floor() / 4.0;
    }
    let w = text.measure(glyph, style);
    let (x, base) = (r.x + (r.w - w) / 2.0, cap_baseline(text, r.y, r.h, style.size));
    text.draw_text(list, x, base + line, glyph, style.with_color(BLACK.with_alpha(56)));
    text.draw_text(list, x, base, glyph, style);
}

/// A window control's glyph on its circle `c`: minimize (`i` 0), maximize
/// or, when `maximized`, restore (1), close (2); in `ink` on the circle's
/// `fill`, its edges on whole pixels.
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

/// The square of side `side` centered at `(cx, cy)`.
fn square((cx, cy): (f32, f32), side: f32) -> RectF {
    RectF::new(cx - side / 2.0, cy - side / 2.0, side, side)
}

/// compusophy's mark, `side` across at `at`: a ring with a dot.
pub fn mark(list: &mut DrawList, at: (f32, f32), side: f32, color: Rgba) {
    list.border(square(at, side), side / 2.0, 1.5, color);
    list.fill(square(at, 5.0), 2.5, color);
}

/// Settings, `side` across at `at`: two sliders, their 1 px tracks on
/// whole pixels when `at` is.
pub fn sliders(list: &mut DrawList, (cx, cy): (f32, f32), side: f32, color: Rgba) {
    for d in [-3.0, 3.0] {
        list.fill(RectF::new(cx - side / 2.0, cy + d, side, 1.0), 0.5, color);
        list.fill(square((cx + d, cy + d + 0.5), 5.0), 2.5, color);
    }
}

/// Light and dark, `side` across at `at`: a ring with its left half filled.
pub fn contrast(list: &mut DrawList, at: (f32, f32), side: f32, color: Rgba) {
    let g = square(at, side);
    list.border(g, side / 2.0, 1.5, color);
    list.push_clip(RectF { w: side / 2.0, ..g });
    list.fill(g, side / 2.0, color);
    list.pop_clip();
}

/// A magnifier centered at `(cx, cy)`: an 11 px ring and a handle of
/// overlapping dots down to the right.
pub fn magnifier(list: &mut DrawList, (cx, cy): (f32, f32), color: Rgba) {
    list.border(square((cx, cy), 11.0), 5.5, 1.75, color);
    for i in 0..=6 {
        let d = 4.6 + i as f32 * 0.5;
        list.fill(square((cx + d, cy + d), 1.8), 0.9, color);
    }
}

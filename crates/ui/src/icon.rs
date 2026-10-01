//! Vector icons: the `icons` crate's [`Glyph`]s drawn crisp at any size
//! through [`TextSystem::draw_vector`], alone or on an app tile in a theme's
//! colors ([`Theme::icon_colors`]).

use gfx::{DrawList, RectF, Rgba};
pub use icons::{Glyph, MARK_HOLE, PHI, Point, outline, rings};
use text::TextSystem;

use crate::Theme;

/// `g` in `color`, fitted to the square of side `min(r.w, r.h)` centered in `r`.
pub fn draw(list: &mut DrawList, text: &mut TextSystem, r: RectF, g: Glyph, color: Rgba) {
    text.draw_vector(list, r, g as u16, g.shape(), color);
}

/// An app tile in the square `r` (centered in it, on device pixels): a rounded
/// square (radius 1/φ³ of its side) in the theme's tile colors for `hue`, its
/// shadow if the theme casts one, a hairline edge, and `g` 1/φ of its side.
pub fn tile(
    list: &mut DrawList,
    text: &mut TextSystem,
    r: RectF,
    g: Glyph,
    hue: Rgba,
    theme: &Theme,
) {
    let (d, side) = (text.dpr(), (r.w.min(r.h) * text.dpr()).round());
    let (at, s) = (|c: f32| (c * d - side / 2.0).round() / d, side / d);
    let t = RectF::new(at(r.x + r.w / 2.0), at(r.y + r.h / 2.0), s, s);
    // The mark is never on a tile: its dots in the theme's ink, on whatever is under them.
    if g == Glyph::Mark {
        return draw(list, text, t.inset(s * 0.09), g, theme.text);
    }
    let ([top, bottom, ink], radius, style) = (theme.icon_colors(hue), s / PHI.powi(3), theme.icon);
    if style.shadow > 0 {
        let a = u16::from(theme.shadow.3) * u16::from(style.shadow.min(100)) / 100;
        list.shadow_offset(t, radius, s * 0.18, s * 0.045, theme.shadow.with_alpha(a as u8));
    }
    if top == bottom {
        list.fill(t, radius, top);
    } else {
        list.gradient(t, radius, top, bottom, 0.0);
    }
    list.border(t, radius, 1.0 / d, theme.border);
    draw(list, text, t.inset(s * (1.0 - 1.0 / PHI) / 2.0), g, ink);
}

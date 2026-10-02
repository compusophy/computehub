//! Vector icons: the `icons` crate's [`Glyph`]s (and `.app` files' [`Mark`]s: the icon a file
//! [`made`] for itself, else its [`sigil`]) drawn crisp at any size through
//! [`TextSystem::draw_vector`], alone or on an app tile in a theme's colors
//! ([`Theme::icon_colors`]).

use gfx::{DrawList, RectF, Rgba};
pub use icons::{Glyph, MARK_HOLE, Made, PHI, Point, cos, made, outline, rings, sigil, sin};
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
    let t = square(text, r);
    // The mark is never on a tile: its dots in the theme's ink, on whatever is under them.
    if g == Glyph::Mark {
        return draw(list, text, t.inset(t.w * 0.09), g, theme.text);
    }
    let ink = plate(list, text, t, hue, theme);
    draw(list, text, t.inset(t.w * (1.0 - 1.0 / PHI) / 2.0), g, ink);
}

/// A `.app` file's tile: [`tile`]'s, with the [`sigil`] of `seed` (a hash of its name) as the
/// glyph.
pub fn sigil_tile(
    list: &mut DrawList,
    text: &mut TextSystem,
    r: RectF,
    seed: u32,
    hue: Rgba,
    theme: &Theme,
) {
    let t = square(text, r);
    let ink = plate(list, text, t, hue, theme);
    text.draw_seeded(list, t.inset(t.w * (1.0 - 1.0 / PHI) / 2.0), seed, sigil, ink);
}

/// What a `.app` file's tile shows: the sigil of a seed (a hash of its name), or the icon its
/// header draws ([`made`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Sigil(u32),
    Made(Made),
}

/// A `.app` file's tile: [`sigil_tile`]'s plate, with its [`Mark`] in the same ink and box.
pub fn mark_tile(
    list: &mut DrawList,
    text: &mut TextSystem,
    r: RectF,
    mark: &Mark,
    hue: Rgba,
    theme: &Theme,
) {
    let m = match mark {
        Mark::Sigil(seed) => return sigil_tile(list, text, r, *seed, hue, theme),
        Mark::Made(m) => m,
    };
    let t = square(text, r);
    let ink = plate(list, text, t, hue, theme);
    text.draw_hashed(
        list,
        t.inset(t.w * (1.0 - 1.0 / PHI) / 2.0),
        m.hash(),
        &|o| m.outline(o),
        ink,
    );
}

/// The square of side `min(r.w, r.h)` centered in `r`, on device pixels.
fn square(text: &TextSystem, r: RectF) -> RectF {
    let (d, side) = (text.dpr(), (r.w.min(r.h) * text.dpr()).round());
    let (at, s) = (|c: f32| (c * d - side / 2.0).round() / d, side / d);
    RectF::new(at(r.x + r.w / 2.0), at(r.y + r.h / 2.0), s, s)
}

/// A tile's plate at `t` for `hue`; its glyph's ink.
fn plate(list: &mut DrawList, text: &TextSystem, t: RectF, hue: Rgba, theme: &Theme) -> Rgba {
    let s = t.w;
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
    list.border(t, radius, 1.0 / text.dpr(), theme.border);
    ink
}

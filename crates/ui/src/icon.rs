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
    let ink = plate(list, text, (t, t.w / PHI.powi(3)), hue, theme);
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
    let ink = plate(list, text, (t, t.w / PHI.powi(3)), hue, theme);
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
    let ink = plate(list, text, (t, t.w / PHI.powi(3)), hue, theme);
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

/// A person's face in the square `r` (centered in it, on device pixels): a round plate in the
/// theme's tile colors for the hue of `seed` ([`crate::theme::app_tint`]), and on it the
/// [`sigil`] of `seed`, 1/φ of its side. People are round; apps are rounded squares.
pub fn avatar(list: &mut DrawList, text: &mut TextSystem, r: RectF, seed: u32, theme: &Theme) {
    let t = square(text, r);
    let ink = plate(list, text, (t, t.w / 2.0), crate::theme::app_tint(seed), theme);
    text.draw_seeded(list, t.inset(t.w * (1.0 - 1.0 / PHI) / 2.0), seed, sigil, ink);
}

/// The mark's reveal: band `k` (the center, then each of its seven rings of dots, the last
/// with the rim) fades in from `STEP * k` ms for `FADE` ms, Fibonacci numbers both: 618 ms.
const STEP: f64 = 55.0;
const FADE: f64 = 233.0;
pub const REVEAL_MS: f64 = 7.0 * STEP + FADE;

/// compusophy's mark as its 365 dots, fills that need no atlas, in `color` in the square
/// centered in `r` (its box on device pixels, at `dpr`): `ms` into its reveal, from the center
/// out (NaN, or past [`REVEAL_MS`]: whole).
pub fn mark_dots(list: &mut DrawList, r: RectF, dpr: f32, ms: f64, color: Rgba) {
    let side = (r.w.min(r.h) * dpr).round();
    let left = ((r.x + r.w / 2.0) * dpr - side / 2.0).round();
    let top = ((r.y + r.h / 2.0) * dpr + side / 2.0).round() - side;
    let (cx, cy, k) = ((left + side / 2.0) / dpr, (top + side / 2.0) / dpr, side / dpr / 1000.0);
    // Ring `i` (the center dot first) fades in, smoothstepped, `STEP` after the one inside it.
    let ink = |i: usize| {
        let x = ((ms - i as f64 * STEP) / FADE).clamp(0.0, 1.0) as f32;
        let x = if ms.is_nan() { 1.0 } else { x };
        color.with_alpha((f32::from(color.3) * x * x * (3.0 - 2.0 * x)).round() as u8)
    };
    let mut dot = |(x, y): (f32, f32), radius: f32, color: Rgba| {
        let s = radius * k;
        list.fill(RectF::new(x - s, y - s, 2.0 * s, 2.0 * s), s, color);
    };
    dot((cx, cy), MARK_HOLE, ink(0));
    for (i, (n, at, size)) in rings().enumerate() {
        for j in 0..n {
            let a = core::f32::consts::FRAC_PI_2 - core::f32::consts::TAU * j as f32 / n as f32;
            dot((cx + at * k * cos(a), cy - at * k * sin(a)), size, ink(i + 1));
        }
    }
}

/// A tile's plate at `t` with its corner radius, for `hue`; its glyph's ink.
fn plate(
    list: &mut DrawList,
    text: &TextSystem,
    (t, radius): (RectF, f32),
    hue: Rgba,
    theme: &Theme,
) -> Rgba {
    let s = t.w;
    let ([top, bottom, ink], style) = (theme.icon_colors(hue), theme.icon);
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

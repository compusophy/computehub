//! Themes: every color on screen, as plain data chosen at runtime (straight
//! sRGB, opaque unless noted). Nothing draws from a color constant.

use gfx::{DrawList, RectF, Rgba};
use text::{FontId, TextStyle};

/// A soft elliptical light in the backdrop: center and radii as fractions of
/// the screen (`cx`, `rx` of its width; `cy`, `ry` of its height), `color`'s
/// alpha its peak at the center (0 is an unused slot).
#[derive(Clone, Copy, Debug, PartialEq)]
#[rustfmt::skip]
pub struct Glow { pub cx: f32, pub cy: f32, pub rx: f32, pub ry: f32, pub color: Rgba }

/// How app tiles take an app's hue ([`Theme::icon_colors`], `icon::tile`), in
/// percent: the tile's top and bottom of the way from the hue to `ground`, the
/// glyph of the way from the hue to the theme's `text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconStyle {
    /// What tiles fade toward.
    pub ground: Rgba,
    /// Top and bottom; equal for a flat tile.
    pub tile: [u8; 2],
    /// 100 is monochrome: every glyph in `text`.
    pub ink: u8,
    /// The tile's drop shadow, in percent of the theme's `shadow` alpha; 0 is none.
    pub shadow: u8,
}

/// The colors of the whole desktop; `surface`, `glass`, `border`,
/// `highlight`, `shadow` and `selection` are translucent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// The name [`theme`] finds it by.
    pub name: &'static str,
    /// Light text on dark surfaces.
    pub dark: bool,
    /// The backdrop's flat color: the frame's clear color.
    pub base: Rgba,
    /// Lights over the base (see [`Theme::draw_backdrop`]).
    pub glows: [Glow; 4],
    /// Film-grain strength over the backdrop ([`DrawList::grain`]); 0 is none.
    pub grain: u8,
    /// Window bodies.
    pub surface: Rgba,
    /// Raised fills on a surface: buttons, cards, titlebars.
    pub surface_hi: Rgba,
    /// Sunken fills on a surface: text fields, wells.
    pub surface_lo: Rgba,
    /// Translucent chrome over the backdrop: panels, docks, menus.
    pub glass: Rgba,
    /// 1 px outlines and separators.
    pub border: Rgba,
    /// A light top edge on raised fills and glass.
    pub highlight: Rgba,
    /// Body text.
    pub text: Rgba,
    /// Secondary text: captions, keys, small print.
    pub text_dim: Rgba,
    /// Placeholders and disabled text.
    pub text_faint: Rgba,
    /// The one accent: focus rings, carets, primary buttons.
    pub accent: Rgba,
    /// Text on an `accent` fill.
    pub accent_text: Rgba,
    /// Errors and destructive actions.
    pub danger: Rgba,
    /// Drop shadows.
    pub shadow: Rgba,
    /// Selected text, drawn over the glyphs' cells.
    pub selection: Rgba,
    /// The terminal's 16 colors (black, red, green, yellow, blue, magenta,
    /// cyan, white, then bright), readable on `surface`.
    pub ansi: [Rgba; 16],
    /// App tiles and their glyphs.
    pub icon: IconStyle,
}

const fn rgba(rgb: u32, a: u8) -> Rgba {
    Rgba::hex(rgb).with_alpha(a)
}

const fn glow(rgb: u32, a: u8, at: (f32, f32), radii: (f32, f32)) -> Glow {
    Glow { cx: at.0, cy: at.1, rx: radii.0, ry: radii.1, color: rgba(rgb, a) }
}

const UNUSED: Glow = glow(0, 0, (0.0, 0.0), (0.0, 0.0));

const fn ansi(rgb: [u32; 16]) -> [Rgba; 16] {
    let mut out = [Rgba(0, 0, 0, 255); 16];
    let mut i = 0;
    while i < 16 {
        out[i] = Rgba::hex(rgb[i]);
        i += 1;
    }
    out
}

/// Near-black with violet, cyan and magenta light.
#[rustfmt::skip]
const MIDNIGHT: Theme = Theme {
    name: "Midnight", dark: true, base: Rgba::hex(0x07080c), grain: 9,
    glows: [
        glow(0x6d5cff, 90, (0.18, 0.22), (0.55, 0.50)),
        glow(0x22d3ee, 60, (0.82, 0.74), (0.50, 0.45)),
        glow(0xf472b6, 34, (0.62, 0.08), (0.35, 0.28)),
        UNUSED,
    ],
    surface: rgba(0x111219, 240), surface_hi: Rgba::hex(0x1b1d27), surface_lo: Rgba::hex(0x0c0d12),
    glass: rgba(0x14151d, 190), border: rgba(0xffffff, 20), highlight: rgba(0xffffff, 14),
    text: Rgba::hex(0xeceef5), text_dim: Rgba::hex(0x8b8fa3), text_faint: Rgba::hex(0x5a5e70),
    accent: Rgba::hex(0x8b7bff), accent_text: Rgba::hex(0x0b0b12), danger: Rgba::hex(0xff5f6d),
    shadow: rgba(0x000000, 150), selection: rgba(0x8b7bff, 70),
    ansi: ansi([
        0x323646, 0xf07a85, 0x7fd8a4, 0xebcb8b, 0x7aa2f7, 0xb69cff, 0x6fd3e0, 0xc8ccda, //
        0x686d82, 0xff959e, 0x9debbe, 0xf5dca6, 0x9ab8ff, 0xcdb8ff, 0x93e4ee, 0xeceef5,
    ]),
    // Deep tiles: the hue most of the way to the base; glyphs the hue lightened.
    icon: IconStyle { ground: Rgba::hex(0x07080c), tile: [70, 80], ink: 45, shadow: 50 },
};

/// Light: warm paper with peach, lilac and sky light.
#[rustfmt::skip]
const DAWN: Theme = Theme {
    name: "Dawn", dark: false, base: Rgba::hex(0xf4f1ec), grain: 6,
    glows: [
        glow(0xffb38a, 120, (0.14, 0.18), (0.50, 0.45)),
        glow(0xc4b5fd, 110, (0.86, 0.30), (0.45, 0.42)),
        glow(0x93c5fd, 100, (0.50, 0.98), (0.60, 0.40)),
        UNUSED,
    ],
    surface: rgba(0xffffff, 242), surface_hi: Rgba::hex(0xf1eff6), surface_lo: Rgba::hex(0xe9e6ef),
    glass: rgba(0xffffff, 178), border: rgba(0x000000, 22), highlight: rgba(0xffffff, 160),
    text: Rgba::hex(0x16161d), text_dim: Rgba::hex(0x62626f), text_faint: Rgba::hex(0x9a9aa6),
    accent: Rgba::hex(0x5b5bd6), accent_text: Rgba::hex(0xffffff), danger: Rgba::hex(0xd93a49),
    shadow: rgba(0x1e1a33, 70), selection: rgba(0x5b5bd6, 50),
    ansi: ansi([
        0x2b2a33, 0xc0313e, 0x18794e, 0x8f5b00, 0x2d5bd0, 0x7b3fc9, 0x0f7484, 0x6c6b78, //
        0x5f5e6a, 0xa51f2c, 0x116b3f, 0x6f4500, 0x1f4bb8, 0x6430b0, 0x0b6170, 0x71707d,
    ]),
    // Pale tiles: a tint of the hue on white; glyphs the hue darkened.
    icon: IconStyle { ground: Rgba::hex(0xffffff), tile: [86, 76], ink: 55, shadow: 50 },
};

/// Ultra minimal: black, white and grays, no light.
#[rustfmt::skip]
const MONO: Theme = Theme {
    name: "Mono", dark: true, base: Rgba::hex(0x000000), grain: 5,
    glows: [UNUSED; 4],
    surface: rgba(0x0a0a0a, 248), surface_hi: Rgba::hex(0x161616), surface_lo: Rgba::hex(0x050505),
    glass: rgba(0x0a0a0a, 210), border: rgba(0xffffff, 26), highlight: rgba(0xffffff, 10),
    text: Rgba::hex(0xf2f2f2), text_dim: Rgba::hex(0x8a8a8a), text_faint: Rgba::hex(0x4a4a4a),
    accent: Rgba::hex(0xffffff), accent_text: Rgba::hex(0x000000), danger: Rgba::hex(0xff4d4d),
    shadow: rgba(0x000000, 200), selection: rgba(0xffffff, 50),
    ansi: ansi([
        0x2a2a2a, 0xe5787a, 0x8fcb9b, 0xe3c887, 0x8aa9d6, 0xc3a3d9, 0x86c5c9, 0xbdbdbd, //
        0x6a6a6a, 0xf29c9d, 0xadddb6, 0xefd9a6, 0xa9c1e6, 0xd5bde6, 0xa6d9dc, 0xf2f2f2,
    ]),
    // Monochrome: flat raised tiles, every glyph in text, no shadow.
    icon: IconStyle { ground: Rgba::hex(0x161616), tile: [100, 100], ink: 100, shadow: 0 },
};

/// The built-in themes: Midnight, Dawn and Mono (the default).
pub static THEMES: [Theme; 3] = [MIDNIGHT, DAWN, MONO];

/// The theme named `name`, ignoring ASCII case; else the default, Mono.
pub fn theme(name: &str) -> &'static Theme {
    let all: &'static [Theme; 3] = &THEMES;
    all.iter().find(|t| t.name.eq_ignore_ascii_case(name)).unwrap_or(&all[2])
}

impl Theme {
    /// Titles: SansBold 24 in `text`.
    pub fn title(&self) -> TextStyle {
        TextStyle::new(FontId::SansBold, 24.0, self.text)
    }

    /// Section headings: SansBold 18 in `text`.
    pub fn heading(&self) -> TextStyle {
        TextStyle::new(FontId::SansBold, 18.0, self.text)
    }

    /// Group labels: SansBold 14 in `text`.
    pub fn subheading(&self) -> TextStyle {
        TextStyle::new(FontId::SansBold, 14.0, self.text)
    }

    /// Body text: Sans 14 in `text`.
    pub fn body(&self) -> TextStyle {
        TextStyle::new(FontId::Sans, 14.0, self.text)
    }

    /// Small print: Sans 12 in `text_dim`.
    pub fn small(&self) -> TextStyle {
        TextStyle::new(FontId::Sans, 12.0, self.text_dim)
    }

    /// Code: Mono 13 in `text`.
    pub fn mono(&self) -> TextStyle {
        TextStyle::new(FontId::Mono, 13.0, self.text)
    }

    /// A neutral fill under the pointer: 8% of the way to `text`.
    pub fn hover(&self, fill: Rgba) -> Rgba {
        mix(fill, self.text, 0.08)
    }

    /// A neutral fill held down: 60% of the way to `surface_lo`.
    pub fn pressed(&self, fill: Rgba) -> Rgba {
        mix(fill, self.surface_lo, 0.6)
    }

    /// An app tile's top and bottom and its glyph's ink for `hue` (see [`IconStyle`]).
    pub fn icon_colors(&self, hue: Rgba) -> [Rgba; 3] {
        let (s, pct) = (self.icon, |p: u8| f32::from(p) / 100.0);
        [
            mix(hue, s.ground, pct(s.tile[0])),
            mix(hue, s.ground, pct(s.tile[1])),
            mix(hue, self.text, pct(s.ink)),
        ]
    }

    /// A translucent wash over anything under the pointer (or held, `down`).
    pub fn wash(&self, down: bool) -> Rgba {
        self.text.with_alpha(if down { 26 } else { 16 })
    }

    /// The xterm 256-color index `i`: 0-15 are [`Theme::ansi`], 16-231 the
    /// 6 x 6 x 6 cube, 232-255 the gray ramp from 8 to 238.
    pub fn xterm(&self, i: u8) -> Rgba {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let n = usize::from(i.wrapping_sub(16));
        match i {
            0..=15 => self.ansi[usize::from(i)],
            16..=231 => Rgba(LEVELS[n / 36], LEVELS[n / 6 % 6], LEVELS[n % 6], 255),
            _ => Rgba::hex(0x01_0101 * u32::from(8 + 10 * (i - 232))),
        }
    }

    /// The backdrop over `screen`: the base, each used glow filling its
    /// ellipse's bounding box, then the grain (each `seed` a pattern).
    pub fn draw_backdrop(&self, list: &mut DrawList, screen: RectF, seed: f32) {
        list.fill(screen, 0.0, self.base);
        for g in self.glows.iter().filter(|g| g.color.3 > 0) {
            let (rx, ry) = (g.rx * screen.w, g.ry * screen.h);
            let (cx, cy) = (screen.x + g.cx * screen.w, screen.y + g.cy * screen.h);
            list.glow(RectF::new(cx - rx, cy - ry, 2.0 * rx, 2.0 * ry), g.color);
        }
        list.grain(screen, self.grain, seed);
    }
}

/// `a` moved `t` (0 to 1) of the way to `b`, in every channel, alpha too.
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgba(ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2), ch(a.3, b.3))
}

/// A stable, soft color for `id`: the hue steps by the golden angle
/// (137.508 degrees) per id at fixed saturation and value.
pub fn app_tint(id: u32) -> Rgba {
    // Millidegrees in integers, so the hue is exact for every id.
    let h = ((u64::from(id) * 137_508) % 360_000) as f32 / 60_000.0;
    let (v, s) = (0.9, 0.45);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let sixths = [(c, x, 0.0), (x, c, 0.0), (0.0, c, x), (0.0, x, c), (x, 0.0, c), (c, 0.0, x)];
    let (r, g, b) = sixths[(h as usize).min(5)];
    let byte = |f: f32| ((f + v - c) * 255.0).round() as u8;
    Rgba(byte(r), byte(g), byte(b), 255)
}

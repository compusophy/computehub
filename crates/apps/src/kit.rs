//! What the apps share: icons, a wheel-scrolled view, and drawing helpers
//! that keep shapes and text on device pixels.

use gfx::RectF;
use ui::theme::mix;
use ui::{AppIcon, FontId, Rgba, TextStyle, Theme, Ui, WidgetId};

/// The app icons; Studio's as the `studio` crate gives it, for Welcome.
pub(crate) const TERMINAL: AppIcon = AppIcon { glyph: ">_", hue: Rgba::hex(0x2dd4bf) };
pub(crate) const STUDIO: AppIcon = AppIcon { glyph: "{ }", hue: Rgba::hex(0x8b7bff) };
pub(crate) const SETTINGS: AppIcon = AppIcon { glyph: "::", hue: Rgba::hex(0x94a3b8) };
pub(crate) const WELCOME: AppIcon = AppIcon { glyph: "c", hue: Rgba::hex(0xf472b6) };

const WHITE: Rgba = Rgba(255, 255, 255, 255);
const BLACK: Rgba = Rgba(0, 0, 0, 255);
/// Inter's cap height and x-height, in ems (JetBrains Mono's caps match).
const CAP: f32 = 0.727;
const X_HEIGHT: f32 = 0.546;

/// `r` with every edge on the nearest device pixel.
pub(crate) fn snapped(ui: &mut Ui<'_>, r: RectF) -> RectF {
    let ts = ui.text_system();
    let (x, y) = (ts.snap(r.x), ts.snap(r.y));
    RectF::new(x, y, ts.snap(r.x + r.w) - x, ts.snap(r.y + r.h) - y)
}

/// `v` logical pixels as whole device pixels, at least one: a stroke.
pub(crate) fn px(ui: &mut Ui<'_>, v: f32) -> f32 {
    let d = ui.text_system().dpr();
    (v * d).round().max(1.0) / d
}

/// The baseline that centers ink `ink` ems tall at `size` in a band `h` tall.
pub(crate) fn centered_base(ui: &mut Ui<'_>, top: f32, h: f32, size: f32, ink: f32) -> f32 {
    ui.text_system().snap(top + (h + ink * size) / 2.0)
}

/// The baseline that centers the capitals of `style` in a band.
pub(crate) fn cap_base(ui: &mut Ui<'_>, top: f32, h: f32, style: TextStyle) -> f32 {
    centered_base(ui, top, h, style.size, CAP)
}

/// Draws `lines` down from `top`, from `x` or centered in `w`; returns their height.
pub(crate) fn lines(
    ui: &mut Ui<'_>,
    lines: &[&str],
    style: TextStyle,
    (x, w): (f32, f32),
    top: f32,
    center: bool,
) -> f32 {
    let ts = ui.text_system();
    let (lh, a, d) = (ts.line_height(style), ts.ascent(style), ts.descent(style));
    for (i, line) in lines.iter().enumerate() {
        let ts = ui.text_system();
        let lw = if center { ts.measure(line, style) } else { w };
        let lx = ts.snap(x + (w - lw) / 2.0);
        let base = ts.snap(top + i as f32 * lh + (lh - a - d) / 2.0 + a);
        ui.text(lx, base, line, style);
    }
    lines.len() as f32 * lh
}

/// Whether `id` is under the pointer, and whether it is also held.
pub(crate) fn pointer(ui: &Ui<'_>, id: WidgetId) -> (bool, bool) {
    let s = ui.state();
    let hover = s.hover == Some(id);
    (hover, hover && s.pressed == Some(id))
}

/// A clickable card's fill and edge: lighter under the pointer, sunk when held.
pub(crate) fn card_colors(t: &Theme, hover: bool, down: bool) -> (Rgba, Rgba) {
    let fill = match (hover, down) {
        (_, true) => t.pressed(t.surface_hi),
        (true, false) => t.hover(t.surface_hi),
        _ => t.surface_hi,
    };
    let edge = if hover { t.text.with_alpha(t.border.3.saturating_mul(2)) } else { t.border };
    (fill, edge)
}

/// A raised rounded rect: `fill`, a 1 px `edge`, and the theme's light top.
pub(crate) fn raised(ui: &mut Ui<'_>, r: RectF, radius: f32, fill: Rgba, edge: Rgba) {
    let (t, line) = (ui.theme(), px(ui, 1.0));
    ui.fill(r, radius, fill);
    ui.border(r, radius, line, edge);
    ui.push_clip(RectF::new(r.x, r.y, r.w, line));
    ui.border(r, radius, line, t.highlight);
    ui.pop_clip();
}

/// An app icon in the square `r`: a shadowed rounded square in a gradient of
/// its hue, and its glyph in white (SansBold if alphanumeric, else Mono),
/// centered on its ink and shrunk to fit.
pub(crate) fn icon(ui: &mut Ui<'_>, r: RectF, icon: AppIcon) {
    let t = ui.theme();
    let radius = (r.w * 0.28).round();
    let drop = RectF { y: r.y + px(ui, 1.0), ..r };
    ui.list().shadow(drop, radius, 6.0, t.shadow.with_alpha(t.shadow.3 / 2));
    ui.gradient(r, radius, mix(icon.hue, WHITE, 0.16), mix(icon.hue, BLACK, 0.16));
    let line = px(ui, 1.0);
    ui.border(r, radius, line, WHITE.with_alpha(36));
    let glyph = icon.glyph;
    let bold = glyph.chars().all(|c| c.is_ascii_alphanumeric());
    let font = if bold { FontId::SansBold } else { FontId::Mono };
    let mut style = TextStyle::new(font, (r.w * 1.8).floor() / 4.0, WHITE);
    let room = r.w * 0.64;
    let w = ui.text_system().measure(glyph, style);
    if w > room {
        style.size = (style.size * room / w * 4.0).floor() / 4.0;
    }
    let w = ui.text_system().measure(glyph, style);
    // Lowercase letters center their x-height, everything else its caps.
    let ink = if glyph.chars().all(|c| c.is_ascii_lowercase()) { X_HEIGHT } else { CAP };
    let base = centered_base(ui, r.y, r.h, style.size, ink);
    let x = r.x + (r.w - w) / 2.0;
    let shade = style.with_color(BLACK.with_alpha(56));
    ui.text(x, base + line, glyph, shade);
    ui.text(x, base, glyph, style);
}

/// A view taller than its window, scrolled by the wheel: offset and maximum.
#[derive(Debug, Default)]
pub(crate) struct Scroll {
    pub(crate) y: f32,
    max: f32,
}

impl Scroll {
    /// Records `content` px shown in `view` px; keeps the offset in range.
    pub(crate) fn measure(&mut self, content: f32, view: f32) {
        self.max = (content - view).max(0.0);
        self.y = self.y.min(self.max);
    }

    /// Scrolls by `dy` px; returns whether the offset moved.
    pub(crate) fn wheel(&mut self, dy: f32) -> bool {
        let old = self.y;
        if dy.is_finite() {
            self.y = (old + dy).max(0.0).min(self.max);
        }
        self.y != old
    }
}

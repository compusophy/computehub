//! What the apps share: icons, a wheel-scrolled view, helpers keeping text on device pixels.

use gfx::RectF;
use ui::icon::Glyph;
use ui::{AppIcon, Rgba, TextStyle, Theme, Ui, WidgetId};

/// The app icons; Studio's as the `studio` crate gives it, for Welcome.
pub(crate) const TERMINAL: AppIcon = AppIcon { glyph: Glyph::Terminal, hue: Rgba::hex(0x2dd4bf) };
pub(crate) const STUDIO: AppIcon = AppIcon { glyph: Glyph::Studio, hue: Rgba::hex(0x8b7bff) };
pub(crate) const SETTINGS: AppIcon = AppIcon { glyph: Glyph::Cog, hue: Rgba::hex(0x94a3b8) };
pub(crate) const WELCOME: AppIcon = AppIcon { glyph: Glyph::Mark, hue: Rgba::hex(0xf472b6) };

/// Inter's cap height in ems (JetBrains Mono's caps match).
const CAP: f32 = 0.727;

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
    let (t, line) = (ui.theme(), ui.px(1.0));
    ui.fill(r, radius, fill);
    ui.border(r, radius, line, edge);
    ui.push_clip(RectF::new(r.x, r.y, r.w, line));
    ui.border(r, radius, line, t.highlight);
    ui.pop_clip();
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

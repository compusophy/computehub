//! What the apps share: their icons, a wheel-scrolled view and its thumb, a switch, raised
//! cards and helpers keeping text on device pixels.

use gfx::RectF;
use ui::icon::Glyph;
use ui::{AppIcon, Rgba, TextStyle, Theme, Ui, WidgetId};

/// The app icons.
pub(crate) const TERMINAL: AppIcon = AppIcon { glyph: Glyph::Terminal, hue: Rgba::hex(0x2dd4bf) };
pub(crate) const SETTINGS: AppIcon = AppIcon { glyph: Glyph::Cog, hue: Rgba::hex(0x94a3b8) };

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

/// A switch, 34 x 21, centered on the right edge of `r`: an accent track with its knob right
/// when `on`, else a sunken one with its knob left.
pub(crate) fn switch(ui: &mut Ui<'_>, r: RectF, on: bool) {
    let (t, line) = (ui.theme(), ui.px(1.0));
    let track = ui.snapped(RectF::new(r.x + r.w - 34.0, r.y + (r.h - 21.0) / 2.0, 34.0, 21.0));
    let (fill, knob) = if on { (t.accent, t.accent_text) } else { (t.surface_lo, t.text_dim) };
    ui.fill(track, 10.5, fill);
    if !on {
        ui.border(track, 10.5, line, t.border);
    }
    let x = if on { track.x + track.w - 18.0 } else { track.x + 3.0 };
    let dot = ui.snapped(RectF::new(x, track.y + 3.0, 15.0, 15.0));
    ui.fill(dot, dot.w / 2.0, knob);
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

    /// While the content overflows, a thumb at the right edge of `view` ([`Ui::thumb`]).
    pub(crate) fn thumb(&self, ui: &mut Ui<'_>, view: RectF) {
        ui.thumb(view, self.y, view.h + self.max);
    }
}

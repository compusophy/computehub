//! What the apps share: icons, the mark and its reveal, a wheel-scrolled view and its thumb,
//! list rows, a switch, and helpers keeping text on device pixels.

use gfx::RectF;
use ui::icon::{Glyph, MARK_HOLE, rings};
use ui::theme::mix;
use ui::{AppIcon, RADIUS_SM, Rgba, TextStyle, Theme, Ui, WidgetId};

/// The app icons; Studio's and the Assistant's as their programs give them.
pub(crate) const TERMINAL: AppIcon = AppIcon { glyph: Glyph::Terminal, hue: Rgba::hex(0x2dd4bf) };
pub(crate) const STUDIO: AppIcon = AppIcon { glyph: Glyph::Studio, hue: Rgba::hex(0x8b7bff) };
pub(crate) const ASSISTANT: AppIcon = AppIcon { glyph: Glyph::Assistant, hue: Rgba::hex(0xa78bfa) };
pub(crate) const SETTINGS: AppIcon = AppIcon { glyph: Glyph::Cog, hue: Rgba::hex(0x94a3b8) };
pub(crate) const WELCOME: AppIcon = AppIcon { glyph: Glyph::Mark, hue: Rgba::hex(0xf472b6) };
pub(crate) const FILES: AppIcon = AppIcon { glyph: Glyph::Folder, hue: Rgba::hex(0x60a5fa) };
pub(crate) const ABOUT: AppIcon = AppIcon { glyph: Glyph::About, hue: Rgba::hex(0xfbbf24) };
pub(crate) const FEEDBACK: AppIcon = AppIcon { glyph: Glyph::Feedback, hue: Rgba::hex(0x34d399) };
/// A plain file's tile, in Files.
pub(crate) const FILE: AppIcon = AppIcon { glyph: Glyph::File, hue: Rgba::hex(0x94a3b8) };

/// A list row's height and its tile's side (Fibonacci numbers; the row is a touch target).
pub(crate) const ROW_H: f32 = 55.0;
pub(crate) const TILE: f32 = 34.0;
/// Inter's cap height in ems (JetBrains Mono's caps match).
const CAP: f32 = 0.727;
/// The mark's reveal: band `k` (the center, then each of its seven rings of dots, the last
/// with the rim) fades in from `STEP * k` ms for `FADE` ms, Fibonacci numbers both: 618 ms.
const STEP: f64 = 55.0;
const FADE: f64 = 233.0;
pub(crate) const REVEAL_MS: f64 = 7.0 * STEP + FADE;

/// The content column of a window `r`: `(x, w)`, at most `max` wide, centered, with a margin
/// of 34 on a wide window (21 on a narrow one).
pub(crate) fn column(ui: &mut Ui<'_>, r: RectF, max: f32) -> (f32, f32) {
    let side = if r.w >= 560.0 { 34.0 } else { 21.0 };
    let w = (r.w - 2.0 * side).max(0.0).min(max).floor();
    (ui.text_system().snap(r.x + (r.w - w) / 2.0), w)
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

/// `text` wrapped to `w` and drawn from `top`, from `x` or centered; returns its height.
pub(crate) fn para(
    ui: &mut Ui<'_>,
    text: &str,
    style: TextStyle,
    (x, w): (f32, f32),
    top: f32,
    center: bool,
) -> f32 {
    let wrapped = ui.text_system().wrap(text, style, w);
    lines(ui, &wrapped, style, (x, w), top, center)
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

/// A hairline across `x`..`x + w` at `y`, in the border color.
pub(crate) fn rule(ui: &mut Ui<'_>, x: f32, y: f32, w: f32) {
    let (line, color) = (ui.px(1.0), ui.theme().border);
    let r = ui.snapped(RectF::new(x, y, w, line));
    ui.fill(RectF { h: line, ..r }, 0.0, color);
}

/// What a list row shows: a tile, a name, a line under it, a note at the right, a chevron.
pub(crate) struct Row<'a> {
    pub(crate) icon: AppIcon,
    pub(crate) name: &'a str,
    pub(crate) line: &'a str,
    pub(crate) aside: &'a str,
    pub(crate) more: bool,
}

/// List row `id` in `r`: `row`'s tile, its name over its line (each cut to fit), its note and
/// chevron at the right, a wash under the pointer (deeper while held), a hairline under it from
/// the text on; a click hit.
pub(crate) fn row(ui: &mut Ui<'_>, id: WidgetId, r: RectF, row: &Row<'_>) {
    let t = ui.theme();
    let (hover, down) = pointer(ui, id);
    if hover {
        ui.fill(r, RADIUS_SM, t.wash(down));
    }
    let tile = ui.snapped(RectF::new(r.x + 8.0, r.y + (r.h - TILE) / 2.0, TILE, TILE));
    ui.app_icon(tile, row.icon);
    let x = tile.x + TILE + 13.0;
    let mut right = r.x + r.w - 8.0;
    if row.more {
        let at = ui.snapped(RectF::new(right - 13.0, r.y + (r.h - 13.0) / 2.0, 13.0, 13.0));
        ui.glyph(at, Glyph::Chevron, t.text_dim);
        right = at.x - 8.0;
    }
    let small = t.small();
    if !row.aside.is_empty() {
        let w = ui.text_system().measure(row.aside, small);
        let ax = ui.text_system().snap(right - w);
        let base = cap_base(ui, r.y, r.h, small);
        ui.text(ax, base, row.aside, small);
        right = ax - 13.0;
    }
    let (name, room) = (t.body(), (right - x).max(0.0));
    let ts = ui.text_system();
    let (shown, line) = (ts.ellipsize(row.name, name, room), ts.ellipsize(row.line, small, room));
    let (nh, sh) = (ts.line_height(name), ts.line_height(small));
    let h = if line.is_empty() { nh } else { nh + sh };
    let top = ts.snap(r.y + (r.h - h) / 2.0);
    lines(ui, &[&shown], name, (x, room), top, false);
    if !line.is_empty() {
        lines(ui, &[&line], small, (x, room), top + nh, false);
    }
    let y = r.y + r.h - ui.px(1.0);
    rule(ui, x, y, r.x + r.w - x);
    ui.hit(id, r, ui::Sense::Click);
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

/// compusophy's mark in `t.text`, in the square centered in `r`, `ms` into its reveal: from the
/// center out, each band (the center, a ring of dots, the rim) fades in, then the glyph stands.
pub(crate) fn mark(ui: &mut Ui<'_>, r: RectF, ms: f64) {
    let t = ui.theme();
    // Done, or no clock (NaN): whole.
    if ms.is_nan() || ms >= REVEAL_MS {
        return ui.glyph(r, Glyph::Mark, t.text);
    }
    // The box the glyph fills: a whole number of device pixels, its edges on them.
    let d = ui.text_system().dpr();
    let side = (r.w.min(r.h) * d).round();
    let left = ((r.x + r.w / 2.0) * d - side / 2.0).round();
    let top = ((r.y + r.h / 2.0) * d + side / 2.0).round() - side;
    let (cx, cy, k) = ((left + side / 2.0) / d, (top + side / 2.0) / d, side / d / 1000.0);
    let disc = |ui: &mut Ui<'_>, (x, y): (f32, f32), radius: f32, color: Rgba| {
        let s = radius * k;
        ui.fill(RectF::new(x - s, y - s, 2.0 * s, 2.0 * s), s, color);
    };
    // Band `i` reaches halfway from ring `i - 1` (the center hole first) to ring `i`; the last is
    // the whole disc. Its fade is smoothstepped.
    let band = |ui: &mut Ui<'_>, i: usize, edge: f32| {
        let x = ((ms - i as f64 * STEP) / FADE).clamp(0.0, 1.0) as f32;
        let alpha = f32::from(t.text.3) * x * x * (3.0 - 2.0 * x);
        disc(ui, (cx, cy), edge, t.text.with_alpha(alpha.round() as u8));
    };
    let (mut outer, mut i) = (MARK_HOLE, 0);
    for (_, at, dot) in rings() {
        band(ui, i, (outer + at - dot) / 2.0);
        (outer, i) = (at + dot, i + 1);
    }
    band(ui, i, 500.0);
    // The holes, in the color of the window under them.
    let under = mix(t.base, t.surface.with_alpha(255), f32::from(t.surface.3) / 255.0);
    disc(ui, (cx, cy), MARK_HOLE, under);
    for (n, at, dot) in rings() {
        for j in 0..n {
            let a = core::f32::consts::FRAC_PI_2 - core::f32::consts::TAU * j as f32 / n as f32;
            disc(ui, (cx + at * k * a.cos(), cy - at * k * a.sin()), dot, under);
        }
    }
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

    /// Scrolls the least that shows `top` to `bottom` (offsets into the content) in `view` px.
    pub(crate) fn show(&mut self, top: f32, bottom: f32, view: f32) {
        let y = if bottom > self.y + view { bottom - view } else { self.y };
        self.y = y.min(top).max(0.0).min(self.max);
    }

    /// While the content overflows, a thumb at the right edge of `view`: 3 px wide in the faint
    /// text color, as long as the share shown and as far down as the view.
    pub(crate) fn thumb(&self, ui: &mut Ui<'_>, view: RectF) {
        if self.max <= 0.0 || view.h <= 0.0 {
            return;
        }
        let h = (view.h * view.h / (view.h + self.max)).max(34.0).min(view.h);
        let y = view.y + (view.h - h) * self.y / self.max;
        let r = ui.snapped(RectF::new(view.x + view.w - 6.0, y, 3.0, h));
        ui.fill(r, 1.5, ui.theme().text_faint);
    }
}

/// FNV-1a of `name`: the seed of a `.app` file's tile color, as the launcher has it.
pub(crate) fn tint(name: &str) -> Rgba {
    let fnv = |h: u32, b: u8| (h ^ u32::from(b)).wrapping_mul(16_777_619);
    ui::theme::app_tint(name.bytes().fold(2_166_136_261, fnv))
}

/// `n` bytes for a person: `340 B`, `1.2 KB`, `3.4 MB`, in whole tenths (no float formatting).
pub(crate) fn size(n: u64) -> String {
    let mut out = String::new();
    let (unit, scale) = match n {
        0..1024 => ("B", 1),
        1024..1_048_576 => ("KB", 1024),
        _ => ("MB", 1_048_576),
    };
    let tenths = n * 10 / scale;
    ui::push_num(&mut out, (tenths / 10) as usize);
    if scale > 1 {
        out.push('.');
        ui::push_num(&mut out, (tenths % 10) as usize);
    }
    out + " " + unit
}

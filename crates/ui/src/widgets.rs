//! The immediate-mode builder apps draw with, and the hit regions the shell
//! routes the pointer by. Rects, strokes and baselines land on device pixels.

use gfx::{DrawList, RectF, Rgba};
use text::{TextStyle, TextSystem};

use crate::theme::{Theme, mix};

/// Padding between the [`Ui`] rect and its content.
pub const PAD: f32 = 20.0;
/// Space after each item, and between the items of a [`Ui::row`].
pub const SPACING: f32 = 8.0;
/// Space before a subheading that follows other items.
pub const SPACING_MD: f32 = 12.0;
/// Space before a heading that follows other items.
pub const SPACING_LG: f32 = 20.0;
pub const BUTTON_H: f32 = 32.0;
pub const FIELD_H: f32 = 34.0;
/// Corner radius of buttons and text fields.
pub const RADIUS_SM: f32 = 8.0;
/// Corner radius of cards, tiles and tile icons.
pub const RADIUS_LG: f32 = 12.0;
/// Padding inside a card.
pub const CARD_PAD: f32 = 16.0;
const BUTTON_PAD_X: f32 = 14.0;
const FIELD_PAD_X: f32 = 12.0;
/// Cap height of Inter and JetBrains Mono in ems: one-line controls center it.
const CAP: f32 = 0.727;

/// Names a widget across frames; the app picks the numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId(pub u32);

/// What a hit region wants from the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sense {
    /// A press and release over it is an [`crate::AppEvent::Click`].
    Click,
    /// A press focuses text input on it (a text cursor on hover).
    Text,
    /// It scrolls under the wheel.
    Scroll,
    /// A pad (a board a game is played on): a finger presses it at once and drags across it, so
    /// it never scrolls the window nor long-presses; it has no Click.
    Pad,
}

/// A region that answers the pointer: the visible part of a widget's rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub id: WidgetId,
    pub rect: RectF,
    pub sense: Sense,
}

/// The width of a button labeled `label` in theme `t`, on device pixels.
pub fn button_width(text: &mut TextSystem, t: &Theme, label: &str) -> f32 {
    let d = text.dpr();
    ((text.measure(label, t.body()) + 2.0 * BUTTON_PAD_X) * d).ceil() / d
}

/// The topmost hit (the last registered) containing the point.
pub fn hit_test(hits: &[Hit], x: f32, y: f32) -> Option<Hit> {
    hits.iter().rev().find(|h| h.rect.contains(x, y)).copied()
}

/// What the shell knows when a frame is built: the widget under the pointer
/// (by last frame's hits), the one held, window focus, the page clock (ms).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiState {
    pub hover: Option<WidgetId>,
    pub pressed: Option<WidgetId>,
    pub focused: bool,
    pub now_ms: f64,
}

/// One frame of one window's content. Items stack down from the corner
/// inset by [`PAD`], [`SPACING`] apart, with [`SPACING_LG`] before a heading
/// and [`SPACING_MD`] before a subheading (gaps collapse: the larger wins);
/// in a [`Ui::row`] they run left to right. The `Ui` clips to its rect, and
/// dropping it pops that clip and any left open. Widgets return the rect
/// they took; interactive ones register a [`Hit`].
pub struct Ui<'a> {
    list: &'a mut DrawList,
    text: &'a mut TextSystem,
    theme: &'a Theme,
    rect: RectF,
    pad: f32,
    hits: &'a mut Vec<Hit>,
    state: UiState,
    x: f32,
    y: f32,
    /// The lowest edge of anything placed.
    bottom: f32,
    /// In a row: where it starts, its tallest item and its width so far.
    row: Option<(f32, f32, f32)>,
    /// The space after the last item of the column; `None` before the first.
    gap: Option<f32>,
    /// Clips pushed through [`Ui::push_clip`] and not yet popped.
    clips: usize,
}

impl Drop for Ui<'_> {
    fn drop(&mut self) {
        for _ in 0..=self.clips {
            self.list.pop_clip();
        }
    }
}

impl<'a> Ui<'a> {
    /// A builder drawing into `list` inside `rect` in `theme`, appending hits
    /// to `hits`. Pushes `rect` as a clip.
    pub fn new(
        list: &'a mut DrawList,
        text: &'a mut TextSystem,
        rect: RectF,
        hits: &'a mut Vec<Hit>,
        state: UiState,
        theme: &'a Theme,
    ) -> Ui<'a> {
        list.push_clip(rect);
        let (x, y) = (text.snap(rect.x + PAD), text.snap(rect.y + PAD));
        let (row, gap, clips, pad) = (None, None, 0, PAD);
        Ui { list, text, theme, rect, pad, hits, state, x, y, bottom: y, row, gap, clips }
    }

    /// The same `Ui` with `pad` instead of [`PAD`].
    fn padded(mut self, pad: f32) -> Ui<'a> {
        let (x, y) = (self.snap(self.rect.x + pad), self.snap(self.rect.y + pad));
        (self.pad, self.x, self.y, self.bottom) = (pad, x, y, y);
        self
    }

    /// The whole rect, padding included.
    pub fn rect(&self) -> RectF {
        self.rect
    }

    /// The content width: the rect's width minus padding on both sides.
    pub fn width(&self) -> f32 {
        (self.rect.w - 2.0 * self.pad).max(0.0)
    }

    pub fn state(&self) -> UiState {
        self.state
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Where the next item goes.
    pub fn cursor(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    /// Moves the cursor; later items stack from there with no margin first.
    pub fn set_cursor(&mut self, x: f32, y: f32) {
        (self.x, self.y, self.gap) = (x, y, None);
    }

    /// Moves the cursor down to `y` if it is above it, after custom content.
    pub fn advance_to(&mut self, y: f32) {
        if y > self.y {
            (self.y, self.gap) = (y, Some(0.0));
        }
        self.bottom = self.bottom.max(y);
    }

    /// Empty space: down, or right inside a row.
    pub fn space(&mut self, px: f32) {
        match self.row {
            Some(_) => self.x += px,
            None => (self.y, self.gap) = (self.y + px, self.gap.map(|g| g + px)),
        }
    }

    pub fn text_system(&mut self) -> &mut TextSystem {
        self.text
    }

    /// The draw list, for custom drawing; keep its clips balanced.
    pub fn list(&mut self) -> &mut DrawList {
        self.list
    }

    /// Width from the cursor to the content's right edge.
    fn avail(&self) -> f32 {
        (self.rect.x + self.rect.w - self.pad - self.x).max(0.0)
    }

    fn snap(&self, v: f32) -> f32 {
        self.text.snap(v)
    }

    /// `r` with every edge on the nearest device pixel.
    pub fn snapped(&self, r: RectF) -> RectF {
        let (x, y) = (self.snap(r.x), self.snap(r.y));
        RectF::new(x, y, self.snap(r.x + r.w) - x, self.snap(r.y + r.h) - y)
    }

    /// A stroke of `v` logical pixels as whole device pixels, at least one.
    pub fn px(&self, v: f32) -> f32 {
        let d = self.text.dpr();
        (v * d).round().max(1.0) / d
    }

    /// Takes a `w` x `h` rect at the cursor and moves past it.
    fn place(&mut self, w: f32, h: f32) -> RectF {
        let r = RectF::new(self.x, self.y, w, h);
        match &mut self.row {
            Some((x0, tallest, widest)) => {
                (*tallest, *widest) = (tallest.max(h), widest.max(self.x + w - *x0));
                self.x += w + SPACING;
            }
            None => (self.y, self.gap) = (self.y + h + SPACING, Some(SPACING)),
        }
        self.bottom = self.bottom.max(r.y + h);
        self.snapped(r)
    }

    /// Grows the space after the last item of the column to `want`.
    fn margin(&mut self, want: f32) {
        if let Some(gap) = self.gap.filter(|&g| g < want && self.row.is_none()) {
            (self.y, self.gap) = (self.y + want - gap, Some(want));
        }
    }

    /// Lays out the items `f` adds left to right, [`SPACING`] apart, as one item.
    pub fn row(&mut self, f: impl FnOnce(&mut Self)) -> RectF {
        let (x0, y0) = (self.x, self.y);
        let outer = self.row.replace((x0, 0.0, 0.0));
        f(self);
        let row = std::mem::replace(&mut self.row, outer);
        let (w, h) = row.map_or((0.0, 0.0), |(_, tallest, widest)| (widest, self.y - y0 + tallest));
        (self.x, self.y) = (x0, y0);
        self.place(w, h)
    }

    /// A heading: SansBold 18, wrapped.
    pub fn heading(&mut self, text: &str) -> RectF {
        self.margin(SPACING_LG);
        self.wrapped(text, self.theme.heading())
    }

    /// A subheading: SansBold 14, wrapped.
    pub fn subheading(&mut self, text: &str) -> RectF {
        self.margin(SPACING_MD);
        self.wrapped(text, self.theme.subheading())
    }

    /// Body text: Sans 14, wrapped at spaces and at each `\n`.
    pub fn label(&mut self, text: &str) -> RectF {
        self.wrapped(text, self.theme.body())
    }

    /// Secondary text: Sans 12, dim, wrapped.
    pub fn small(&mut self, text: &str) -> RectF {
        self.wrapped(text, self.theme.small())
    }

    /// Text in any style, wrapped at the content's right edge, as one item.
    pub fn wrapped(&mut self, text: &str, style: TextStyle) -> RectF {
        let lines = self.text.wrap(text, style, self.avail());
        let lh = self.text.line_height(style);
        let (a, d) = (self.text.ascent(style), self.text.descent(style));
        let (base, clip) = (self.snap((lh - a - d) / 2.0 + a), self.list.clip());
        let mut widest: f32 = 0.0;
        for (i, line) in lines.iter().enumerate() {
            // Lines outside the clip are measured but not drawn.
            let top = self.y + i as f32 * lh;
            let w = if top + lh <= clip.y || top >= clip.y + clip.h {
                self.text.measure(line, style)
            } else {
                self.text.draw_text(self.list, self.x, top + base, line, style)
            };
            widest = widest.max(w);
        }
        self.place(widest, lines.len() as f32 * lh)
    }

    /// A button sized to its label that brightens under the pointer and sinks
    /// while held; a [`Sense::Click`] hit.
    pub fn button(&mut self, id: WidgetId, label: &str) -> RectF {
        self.button_as(id, label, None)
    }

    /// A button filled with the accent: the window's main action.
    pub fn button_primary(&mut self, id: WidgetId, label: &str) -> RectF {
        self.button_as(id, label, Some(self.theme.accent))
    }

    /// A button filled with the danger color: a destructive action.
    pub fn button_danger(&mut self, id: WidgetId, label: &str) -> RectF {
        self.button_as(id, label, Some(self.theme.danger))
    }

    fn button_as(&mut self, id: WidgetId, label: &str, solid: Option<Rgba>) -> RectF {
        let t = self.theme;
        let style = t.body();
        let tw = self.text.measure(label, style);
        let w = button_width(self.text, t, label);
        let r = self.place(w, BUTTON_H);
        let (hover, down) = self.pointer(id);
        let shift = [0.0, 0.12, 0.24][usize::from(hover) + usize::from(down)];
        let (fill, ink) = match solid {
            Some(c) => (mix(c, t.accent_text, shift), t.accent_text),
            None if down => (t.pressed(t.surface_hi), t.text),
            None if hover => (t.hover(t.surface_hi), t.text),
            None => (t.surface_hi, t.text),
        };
        self.list.fill(r, RADIUS_SM, fill);
        if solid.is_none() {
            self.list.border(r, RADIUS_SM, self.px(1.0), t.border);
        }
        let sheen = t.highlight.3.min(if solid.is_some() { 40 } else { 255 });
        self.sheen(r, RADIUS_SM, t.highlight.with_alpha(sheen));
        let base = self.cap_baseline(r, style);
        let x = r.x + (r.w - tw) / 2.0;
        self.text.draw_text(self.list, x, base, label, style.with_color(ink));
        self.hit(id, r, Sense::Click);
        r
    }

    /// A one-line field across the width showing `value` (or a faint `hint`);
    /// focused, an accent ring and a caret at the text's end, which stays
    /// visible. A [`Sense::Text`] hit.
    pub fn text_field(&mut self, id: WidgetId, value: &str, focus: bool, hint: &str) -> RectF {
        let (t, w) = (self.theme, self.avail());
        let r = self.place(w, FIELD_H);
        let focus = focus && self.state.focused;
        self.list.fill(r, RADIUS_SM, t.surface_lo);
        if focus {
            let (halo, wide) = (t.accent.with_alpha(t.selection.3 / 2), self.px(3.0));
            self.list.border(r.inset(-wide), RADIUS_SM + wide, wide, halo);
            self.list.border(r, RADIUS_SM, self.px(1.5), t.accent);
        } else {
            // The border doubles its alpha under the pointer.
            let hover = self.state.hover == Some(id);
            let edge = t.border.with_alpha(t.border.3.saturating_mul(1 + u8::from(hover)));
            self.list.border(r, RADIUS_SM, self.px(1.0), edge);
        }
        let style = t.body();
        let inner = RectF::new(r.x + FIELD_PAD_X, r.y, (r.w - 2.0 * FIELD_PAD_X).max(0.0), r.h);
        let base = self.cap_baseline(r, style);
        let caret_w = self.px(1.5);
        let tw = self.text.measure(value, style);
        let x = if tw + caret_w > inner.w { inner.x + inner.w - caret_w - tw } else { inner.x };
        self.list.push_clip(inner);
        let faint = (inner.x, hint, style.with_color(t.text_faint));
        let (at, shown, ink) = if value.is_empty() { faint } else { (x, value, style) };
        self.text.draw_text(self.list, at, base, shown, ink);
        if focus {
            let (a, d) = (self.text.ascent(style), self.text.descent(style));
            let caret = RectF::new(self.snap(x + tw), base - a, caret_w, a + d);
            self.list.fill(caret, 0.0, t.accent);
        }
        self.list.pop_clip();
        self.hit(id, r, Sense::Text);
        self.mark(id, crate::sem::TEXTBOX, if focus { crate::sem::FOCUSED } else { 0 }, value);
        r
    }

    /// Runs `f` on a `Ui` over `rect` with no padding, drawing into this
    /// one's list and hits, clipped as this one is (not to `rect`).
    pub fn within<R>(&mut self, rect: RectF, f: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        let clip = self.list.clip();
        let mut ui = Ui::new(self.list, self.text, clip, self.hits, self.state, self.theme);
        ui.rect = rect;
        f(&mut ui.padded(0.0))
    }

    /// A raised card across the width holding the items `f` adds, as in a
    /// fresh `Ui` with [`CARD_PAD`] padding. `f` runs twice (to measure, then
    /// to draw), so it must lay out the same both times and change nothing else.
    pub fn card(&mut self, mut f: impl FnMut(&mut Ui<'_>)) -> RectF {
        let (t, w) = (self.theme, self.avail());
        let (x, y) = (self.snap(self.x), self.snap(self.y));
        let h = {
            let (mut list, mut hits) = (DrawList::new(), Vec::new());
            let probe = RectF::new(x, y, w, 0.0);
            let ui = Ui::new(&mut list, self.text, probe, &mut hits, self.state, t);
            let mut ui = ui.padded(CARD_PAD);
            f(&mut ui);
            ui.bottom - y + CARD_PAD
        };
        let r = self.place(w, h);
        self.raised(r);
        f(&mut Ui::new(self.list, self.text, r, self.hits, self.state, t).padded(CARD_PAD));
        r
    }

    /// A card's surface in `r`: a raised fill, its edge and a lit top.
    pub fn raised(&mut self, r: RectF) {
        let t = self.theme;
        self.list.fill(r, RADIUS_LG, t.surface_hi);
        self.list.border(r, RADIUS_LG, self.px(1.0), t.border);
        self.sheen(r, RADIUS_LG, t.highlight);
    }

    /// An app's icon tile in the square `r` ([`crate::icon::tile`]).
    pub fn app_icon(&mut self, r: RectF, icon: crate::AppIcon) {
        crate::icon::tile(self.list, self.text, r, icon.glyph, icon.hue, self.theme);
    }

    /// A `.app` file's tile in the square `r`: the sigil of `seed` (a hash of its name) on that
    /// seed's tint ([`crate::icon::sigil_tile`], [`crate::theme::app_tint`]).
    pub fn sigil(&mut self, r: RectF, seed: u32) {
        let hue = crate::theme::app_tint(seed);
        crate::icon::sigil_tile(self.list, self.text, r, seed, hue, self.theme);
    }

    /// While `content` px overflow the view `r`, scrolled `down` px, a thumb 6 px in from its
    /// right edge: 3 px wide in the faint ink, as long as the share shown (34 px at least) and
    /// as far down as the view.
    pub fn thumb(&mut self, r: RectF, down: f32, content: f32) {
        let max = content - r.h;
        if max < 1.0 || r.h <= 0.0 {
            return;
        }
        let h = (r.h * r.h / content).max(34.0).min(r.h);
        let y = r.y + (r.h - h) * (down / max).min(1.0);
        let at = self.snapped(RectF::new(r.x + r.w - 6.0, y, 3.0, h));
        self.fill(at, 1.5, self.theme.text_faint);
    }

    /// The vector glyph `g` in `color`, fitted to the square centered in `r`.
    pub fn glyph(&mut self, r: RectF, g: crate::icon::Glyph, color: Rgba) {
        crate::icon::draw(self.list, self.text, r, g, color);
    }

    /// Face `n` ([`crate::icon::face`]) in the square centered in `r`: its ring, its dots.
    pub fn face(&mut self, r: RectF, n: u8, colors: [Rgba; 2]) {
        crate::icon::face(self.list, self.text, r, n, colors);
    }

    /// `r` filled with its rounded corners in a vertical gradient.
    pub fn gradient(&mut self, r: RectF, radius: f32, top: Rgba, bottom: Rgba) {
        self.list.gradient(r, radius, top, bottom, 0.0);
    }

    pub fn fill(&mut self, r: RectF, radius: f32, color: Rgba) {
        self.list.fill(r, radius, color);
    }

    /// A ring `width` px wide just inside the rounded rect's edge.
    pub fn border(&mut self, r: RectF, radius: f32, width: f32, color: Rgba) {
        self.list.border(r, radius, width, color);
    }

    /// One line of text on `baseline`; returns its advance.
    pub fn text(&mut self, x: f32, baseline: f32, text: &str, style: TextStyle) -> f32 {
        self.text.draw_text(self.list, x, baseline, text, style)
    }

    /// Clips what follows to `r` (within the current clip).
    pub fn push_clip(&mut self, r: RectF) {
        self.list.push_clip(r);
        self.clips += 1;
    }

    /// Undoes the last [`Ui::push_clip`]; never pops the `Ui`'s own clip.
    pub fn pop_clip(&mut self) {
        if self.clips > 0 {
            self.list.pop_clip();
            self.clips -= 1;
        }
    }

    /// Registers the visible part of `rect` as a hit region, if any.
    pub fn hit(&mut self, id: WidgetId, rect: RectF, sense: Sense) {
        let rect = rect.intersect(self.list.clip());
        if rect.w > 0.0 && rect.h > 0.0 {
            self.hits.push(Hit { id, rect, sense });
        }
    }

    /// Says what widget `id` is beyond its text (a [`crate::sem`] role, flags and value), for
    /// the AI reading a recording list; nothing otherwise.
    pub fn mark(&mut self, id: WidgetId, role: u8, flags: u8, value: &str) {
        self.list.mark(id.0, role, flags, value);
    }

    /// Whether `id` is under the pointer, and whether it is also held.
    fn pointer(&self, id: WidgetId) -> (bool, bool) {
        let hover = self.state.hover == Some(id);
        (hover, hover && self.state.pressed == Some(id))
    }

    /// The light top edge of a raised fill: its border's top device pixel row.
    fn sheen(&mut self, r: RectF, radius: f32, color: Rgba) {
        let line = self.px(1.0);
        self.list.push_clip(RectF::new(r.x, r.y, r.w, line));
        self.list.border(r, radius, line, color);
        self.list.pop_clip();
    }

    /// The baseline that centers capitals of `style` in `r`.
    fn cap_baseline(&self, r: RectF, style: TextStyle) -> f32 {
        self.snap(r.y + (r.h + CAP * style.size) / 2.0)
    }
}

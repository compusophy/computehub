//! The immediate-mode builder apps draw with: a layout cursor, the widgets,
//! and the hit regions the shell routes the pointer by. Every widget draws
//! from the frame's [`Theme`], and every rect a widget takes, stroke it
//! draws and baseline it sets lands on a device pixel.

use gfx::{DrawList, RectF, Rgba};
use text::{FontId, TextStyle, TextSystem};

use crate::theme::{Theme, mix};

/// Padding between the [`Ui`] rect and its content.
pub const PAD: f32 = 20.0;
/// Space after each item, and between the items of a [`Ui::row`].
pub const SPACING: f32 = 8.0;
/// Space around a separator, after a title and before a subheading.
pub const SPACING_MD: f32 = 12.0;
/// Space before a title or heading that follows other items.
pub const SPACING_LG: f32 = 20.0;
/// Height of a button.
pub const BUTTON_H: f32 = 32.0;
/// Height of a text field.
pub const FIELD_H: f32 = 34.0;
/// Corner radius of buttons and text fields.
pub const RADIUS_SM: f32 = 8.0;
/// Corner radius of cards, tiles and tile icons.
pub const RADIUS_LG: f32 = 12.0;
/// Padding inside a card.
pub const CARD_PAD: f32 = 16.0;
/// Width of a tile.
pub const TILE_W: f32 = 80.0;
/// Height of a tile: icon, label and their margins.
pub const TILE_H: f32 = 84.0;
/// Side of a tile's icon.
pub const TILE_ICON: f32 = 44.0;
const BUTTON_PAD_X: f32 = 14.0;
const FIELD_PAD_X: f32 = 12.0;
/// Cap height of Inter and of JetBrains Mono, in ems: one-line controls
/// center their capitals.
const CAP: f32 = 0.727;
const WHITE: Rgba = Rgba(255, 255, 255, 255);
const BLACK: Rgba = Rgba(0, 0, 0, 255);

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
}

/// A region of the screen that answers the pointer, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The widget.
    pub id: WidgetId,
    /// The part of its rect that was visible (inside every clip).
    pub rect: RectF,
    /// What it wants.
    pub sense: Sense,
}

/// The topmost hit (the last registered) containing the point.
pub fn hit_test(hits: &[Hit], x: f32, y: f32) -> Option<Hit> {
    hits.iter().rev().find(|h| h.rect.contains(x, y)).copied()
}

/// What the shell knows about the pointer and focus when a frame is built.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiState {
    /// The widget under the pointer, from last frame's hits.
    pub hover: Option<WidgetId>,
    /// The widget the pointer went down on, while it is held.
    pub pressed: Option<WidgetId>,
    /// Whether this window has keyboard focus.
    pub focused: bool,
    /// Milliseconds on the page clock.
    pub now_ms: f64,
}

/// An open [`Ui::row`] or [`Ui::flow`].
#[derive(Clone, Copy, Debug)]
struct Row {
    /// Where its lines start.
    x0: f32,
    /// The tallest item on the current line.
    tallest: f32,
    /// The widest line so far.
    widest: f32,
    /// Whether an item that does not fit starts a new line.
    wrap: bool,
}

/// One frame of one window's content.
///
/// Items stack down from the rect's corner inset by [`PAD`], [`SPACING`]
/// apart, with a vertical rhythm: [`SPACING_LG`] before a title or heading
/// and [`SPACING_MD`] before a subheading (when items precede them), after a
/// title, and on both sides of a separator. Gaps collapse: the larger wins.
/// Inside a [`Ui::row`] or [`Ui::flow`] items run left to right, [`SPACING`]
/// apart; text wraps at the content's right edge.
///
/// [`Ui::new`] clips to the rect and dropping the `Ui` pops that clip and
/// any left open. Widgets return the rect they took, snapped to device
/// pixels; interactive ones register a [`Hit`].
#[derive(Debug)]
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
    row: Option<Row>,
    /// The space after the last item of the column; `None` before the
    /// first.
    gap: Option<f32>,
    /// The lowest edge of anything placed.
    bottom: f32,
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
    /// A builder drawing into `list` inside `rect` in the colors of `theme`,
    /// registering hits into `hits` (which it only appends to). Pushes
    /// `rect` as a clip.
    pub fn new(
        list: &'a mut DrawList,
        text: &'a mut TextSystem,
        rect: RectF,
        hits: &'a mut Vec<Hit>,
        state: UiState,
        theme: &'a Theme,
    ) -> Ui<'a> {
        Ui::inset(list, text, rect, hits, state, theme, PAD)
    }

    fn inset(
        list: &'a mut DrawList,
        text: &'a mut TextSystem,
        rect: RectF,
        hits: &'a mut Vec<Hit>,
        state: UiState,
        theme: &'a Theme,
        pad: f32,
    ) -> Ui<'a> {
        list.push_clip(rect);
        let (x, y) = (text.snap(rect.x + pad), text.snap(rect.y + pad));
        let (row, gap, clips) = (None, None, 0);
        Ui { list, text, theme, rect, pad, hits, state, x, y, row, gap, bottom: y, clips }
    }

    /// The whole rect, padding included.
    pub fn rect(&self) -> RectF {
        self.rect
    }

    /// The content width: the rect's width minus padding on both sides.
    pub fn width(&self) -> f32 {
        (self.rect.w - 2.0 * self.pad).max(0.0)
    }

    /// Pointer and focus state for this frame.
    pub fn state(&self) -> UiState {
        self.state
    }

    /// The colors everything here draws with.
    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Where the next item goes.
    pub fn cursor(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    /// Moves the cursor; later items stack from there, with no margin
    /// before the first.
    pub fn set_cursor(&mut self, x: f32, y: f32) {
        (self.x, self.y, self.gap) = (x, y, None);
    }

    /// Moves the cursor down to `y` if it is above it: call after drawing
    /// custom content with the low-level calls.
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
            None => {
                self.y += px;
                self.gap = self.gap.map(|g| g + px);
            }
        }
    }

    /// The text system, for measuring and custom text.
    pub fn text_system(&mut self) -> &mut TextSystem {
        self.text
    }

    /// The draw list, for custom drawing. Keep pushes and pops of clips
    /// balanced here, or use [`Ui::push_clip`].
    pub fn list(&mut self) -> &mut DrawList {
        self.list
    }

    /// Width from the cursor to the content's right edge.
    fn avail(&self) -> f32 {
        (self.right() - self.x).max(0.0)
    }

    fn right(&self) -> f32 {
        self.rect.x + self.rect.w - self.pad
    }

    /// `v` rounded to the nearest device pixel.
    fn snap(&self, v: f32) -> f32 {
        self.text.snap(v)
    }

    /// `r` with every edge on the nearest device pixel.
    fn snapped(&self, r: RectF) -> RectF {
        let (x, y) = (self.snap(r.x), self.snap(r.y));
        RectF::new(x, y, self.snap(r.x + r.w) - x, self.snap(r.y + r.h) - y)
    }

    /// A stroke of `v` logical pixels as whole device pixels, at least one.
    fn px(&self, v: f32) -> f32 {
        let d = self.text.dpr();
        (v * d).round().max(1.0) / d
    }

    /// Takes a `w` x `h` rect at the cursor and moves past it.
    fn place(&mut self, w: f32, h: f32) -> RectF {
        let right = self.right();
        let r = match &mut self.row {
            Some(row) => {
                if row.wrap && self.x > row.x0 && self.x + w > right {
                    self.y += row.tallest + SPACING;
                    (self.x, row.tallest) = (row.x0, 0.0);
                }
                row.tallest = row.tallest.max(h);
                row.widest = row.widest.max(self.x + w - row.x0);
                let r = RectF::new(self.x, self.y, w, h);
                self.x += w + SPACING;
                r
            }
            None => {
                let r = RectF::new(self.x, self.y, w, h);
                (self.y, self.gap) = (self.y + h + SPACING, Some(SPACING));
                r
            }
        };
        self.bottom = self.bottom.max(r.y + h);
        self.snapped(r)
    }

    /// Grows the space after the last item of the column to `want`.
    fn margin(&mut self, want: f32) {
        if let Some(gap) = self.gap.filter(|&g| g < want && self.row.is_none()) {
            (self.y, self.gap) = (self.y + want - gap, Some(want));
        }
    }

    /// Lays out the items `f` adds left to right, [`SPACING`] apart, as one
    /// item as tall as the tallest of them.
    pub fn row(&mut self, f: impl FnOnce(&mut Self)) -> RectF {
        self.line(false, f)
    }

    /// Like [`Ui::row`], but an item that would cross the content's right
    /// edge starts a new line [`SPACING`] below: a grid of tiles.
    pub fn flow(&mut self, f: impl FnOnce(&mut Self)) -> RectF {
        self.line(true, f)
    }

    fn line(&mut self, wrap: bool, f: impl FnOnce(&mut Self)) -> RectF {
        let (x0, y0) = (self.x, self.y);
        let outer = self.row.replace(Row { x0, tallest: 0.0, widest: 0.0, wrap });
        f(self);
        let row = std::mem::replace(&mut self.row, outer);
        let (w, h) = row.map_or((0.0, 0.0), |r| (r.widest, self.y - y0 + r.tallest));
        (self.x, self.y) = (x0, y0);
        self.place(w, h)
    }

    /// A title: SansBold 24, wrapped.
    pub fn title(&mut self, text: &str) -> RectF {
        self.margin(SPACING_LG);
        let r = self.wrapped(text, self.theme.title());
        self.margin(SPACING_MD);
        r
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

    /// Monospaced text: Mono 13, wrapped.
    pub fn mono(&mut self, text: &str) -> RectF {
        self.wrapped(text, self.theme.mono())
    }

    /// Text in any style, wrapped at the content's right edge, as one item.
    pub fn wrapped(&mut self, text: &str, style: TextStyle) -> RectF {
        let lines = self.text.wrap(text, style, self.avail());
        let (x, y) = (self.x, self.y);
        let w = self.lines(x, y, &lines, style);
        let lh = self.text.line_height(style);
        self.place(w, lines.len() as f32 * lh)
    }

    /// A 1 px rule across the available width in the border color, with
    /// [`SPACING_MD`] above and below.
    pub fn separator(&mut self) -> RectF {
        self.margin(SPACING_MD);
        let (w, h) = (self.avail(), self.px(1.0));
        let r = self.place(w, h);
        self.list.fill(r, 0.0, self.theme.border);
        self.margin(SPACING_MD);
        r
    }

    /// A button sized to its label: a raised fill with a 1 px border that
    /// brightens under the pointer and sinks while held. Registers a
    /// [`Sense::Click`] hit.
    pub fn button(&mut self, id: WidgetId, label: &str) -> RectF {
        self.button_as(id, label, false)
    }

    /// A button filled with the accent: the window's main action.
    pub fn button_primary(&mut self, id: WidgetId, label: &str) -> RectF {
        self.button_as(id, label, true)
    }

    fn button_as(&mut self, id: WidgetId, label: &str, primary: bool) -> RectF {
        let t = self.theme;
        let style = t.body();
        let tw = self.text.measure(label, style);
        let d = self.text.dpr();
        let r = self.place(((tw + 2.0 * BUTTON_PAD_X) * d).ceil() / d, BUTTON_H);
        let (hover, down) = self.pointer(id);
        let shift = [0.0, 0.12, 0.24][usize::from(hover) + usize::from(down)];
        let (fill, ink) = match primary {
            true => (mix(t.accent, t.accent_text, shift), t.accent_text),
            false if down => (t.pressed(t.surface_hi), t.text),
            false if hover => (t.hover(t.surface_hi), t.text),
            false => (t.surface_hi, t.text),
        };
        self.list.fill(r, RADIUS_SM, fill);
        let sheen = match primary {
            true => t.highlight.with_alpha(t.highlight.3.min(40)),
            false => {
                self.list.border(r, RADIUS_SM, self.px(1.0), t.border);
                t.highlight
            }
        };
        self.sheen(r, RADIUS_SM, sheen);
        let base = self.cap_baseline(r, style);
        let x = r.x + (r.w - tw) / 2.0;
        self.text.draw_text(self.list, x, base, label, style.with_color(ink));
        self.hit(id, r, Sense::Click);
        r
    }

    /// A one-line text field across the available width: a sunken well with
    /// a 1 px border showing `value` (or `placeholder`, faint, when `value`
    /// is empty). When `has_focus` and the window is focused it gets an
    /// accent ring with a soft halo and a steady accent caret after the
    /// text, and a value too long to fit scrolls so its end stays visible.
    /// Registers a [`Sense::Text`] hit.
    pub fn text_field(
        &mut self,
        id: WidgetId,
        value: &str,
        has_focus: bool,
        placeholder: &str,
    ) -> RectF {
        let (t, w) = (self.theme, self.avail());
        let r = self.place(w, FIELD_H);
        let focus = has_focus && self.state.focused;
        self.list.fill(r, RADIUS_SM, t.surface_lo);
        if focus {
            let (halo, wide) = (t.accent.with_alpha(t.selection.3 / 2), self.px(3.0));
            self.list.border(r.inset(-wide), RADIUS_SM + wide, wide, halo);
            self.list.border(r, RADIUS_SM, self.px(1.5), t.accent);
        } else {
            let hover = self.state.hover == Some(id);
            let edge = t.border.with_alpha(t.border.3.saturating_mul(1 + u8::from(hover)));
            self.list.border(r, RADIUS_SM, self.px(1.0), edge);
        }
        let style = t.body();
        let inner = RectF::new(r.x + FIELD_PAD_X, r.y, (r.w - 2.0 * FIELD_PAD_X).max(0.0), r.h);
        let base = self.cap_baseline(r, style);
        let caret_w = self.px(1.5);
        let tw = self.text.measure(value, style);
        let x = match tw + caret_w > inner.w {
            true => inner.x + inner.w - caret_w - tw,
            false => inner.x,
        };
        self.list.push_clip(inner);
        if value.is_empty() {
            let style = style.with_color(t.text_faint);
            self.text.draw_text(self.list, inner.x, base, placeholder, style);
        } else {
            self.text.draw_text(self.list, x, base, value, style);
        }
        if focus {
            let (a, d) = (self.text.ascent(style), self.text.descent(style));
            let caret = RectF::new(self.snap(x + tw), base - a, caret_w, a + d);
            self.list.fill(caret, 0.0, t.accent);
        }
        self.list.pop_clip();
        self.hit(id, r, Sense::Text);
        r
    }

    /// A key, dim, in a left column (40% of the width, at most 180 px, but
    /// widened up to 60% so no word of the key splits) and its value in the
    /// rest; both wrap.
    pub fn key_value(&mut self, key: &str, value: &str) -> RectF {
        let avail = self.avail();
        let body = self.theme.body();
        let key_style = body.with_color(self.theme.text_dim);
        let word = (self.text.min_width(key, key_style) + SPACING).ceil();
        let kw = (avail * 0.4).min(180.0).floor().max(word);
        let kw = kw.min((avail * 0.6).floor());
        let keys = self.text.wrap(key, key_style, kw - SPACING);
        let values = self.text.wrap(value, body, avail - kw);
        let (x, y) = (self.x, self.y);
        self.lines(x, y, &keys, key_style);
        self.lines(x + kw, y, &values, body);
        let lh = self.text.line_height(body);
        self.place(avail, keys.len().max(values.len()) as f32 * lh)
    }

    /// A card across the available width holding the items `f` adds, laid
    /// out as in a fresh `Ui` with [`CARD_PAD`] padding: a raised fill with a
    /// [`RADIUS_LG`] corner and a 1 px border, as tall as its content.
    ///
    /// `f` runs twice, first to measure (drawing and hits discarded, the
    /// inner rect zero tall) and then to draw, so it must lay out the same
    /// both times and change nothing else.
    pub fn card(&mut self, mut f: impl FnMut(&mut Ui<'_>)) -> RectF {
        let (t, w) = (self.theme, self.avail());
        let (x, y) = (self.snap(self.x), self.snap(self.y));
        let h = {
            let (mut list, mut hits) = (DrawList::new(), Vec::new());
            let probe = RectF::new(x, y, w, 0.0);
            let mut ui = Ui::inset(&mut list, self.text, probe, &mut hits, self.state, t, CARD_PAD);
            f(&mut ui);
            ui.bottom - y + CARD_PAD
        };
        let r = self.place(w, h);
        self.list.fill(r, RADIUS_LG, t.surface_hi);
        self.list.border(r, RADIUS_LG, self.px(1.0), t.border);
        self.sheen(r, RADIUS_LG, t.highlight);
        f(&mut Ui::inset(self.list, self.text, r, self.hits, self.state, t, CARD_PAD));
        r
    }

    /// An app tile, [`TILE_W`] x [`TILE_H`]: a [`TILE_ICON`] rounded square
    /// in a vertical gradient of `hue` with `glyph` centered on it in white
    /// (SansBold when it is all letters and digits, else Mono; shrunk to
    /// fit; the label's first letter when empty), and `label` below in
    /// small type, cut with an ellipsis to fit. A wash marks it under the
    /// pointer. Registers a [`Sense::Click`] hit over the whole tile.
    pub fn tile(&mut self, id: WidgetId, label: &str, glyph: &str, hue: Rgba) -> RectF {
        let t = self.theme;
        let r = self.place(TILE_W, TILE_H);
        let (hover, down) = self.pointer(id);
        if hover {
            self.list.fill(r, RADIUS_LG, t.wash(down));
        }
        let at = RectF::new(r.x + (r.w - TILE_ICON) / 2.0, r.y + SPACING, TILE_ICON, TILE_ICON);
        let icon = self.snapped(at);
        let drop = RectF { y: icon.y + 2.0, ..icon };
        self.list.shadow(drop, RADIUS_LG, 8.0, t.shadow.with_alpha(t.shadow.3 / 2));
        self.gradient(icon, RADIUS_LG, mix(hue, WHITE, 0.16), mix(hue, BLACK, 0.16));
        self.list.border(icon, RADIUS_LG, self.px(1.0), WHITE.with_alpha(36));
        let first: String = label.chars().take(1).flat_map(char::to_uppercase).collect();
        self.icon_glyph(icon, if glyph.is_empty() { &first } else { glyph });
        let style = t.small().with_color(t.text);
        let name = self.text.ellipsize(label, style, r.w - SPACING);
        let nw = self.text.measure(&name, style);
        let band = RectF::new(r.x, icon.y + TILE_ICON + SPACING, r.w, 16.0);
        let base = self.cap_baseline(band, style);
        self.text.draw_text(self.list, r.x + (r.w - nw) / 2.0, base, &name, style);
        self.hit(id, r, Sense::Click);
        r
    }

    /// A tile's glyph, white with a faint shadow, centered in `icon`.
    fn icon_glyph(&mut self, icon: RectF, glyph: &str) {
        let font = match glyph.chars().all(char::is_alphanumeric) {
            true => FontId::SansBold,
            false => FontId::Mono,
        };
        let mut style = TextStyle::new(font, 20.0, WHITE);
        let room = icon.w - 16.0;
        let w = self.text.measure(glyph, style);
        if w > room {
            style.size = (style.size * room / w * 4.0).floor() / 4.0;
        }
        let w = self.text.measure(glyph, style);
        let x = icon.x + (icon.w - w) / 2.0;
        let base = self.cap_baseline(icon, style);
        let shade = style.with_color(BLACK.with_alpha(56));
        self.text.draw_text(self.list, x, base + self.px(1.0), glyph, shade);
        self.text.draw_text(self.list, x, base, glyph, style);
    }

    /// `r` filled with its rounded corners in a vertical gradient from `top`
    /// to `bottom`.
    pub fn gradient(&mut self, r: RectF, radius: f32, top: Rgba, bottom: Rgba) {
        self.list.gradient(r, radius, top, bottom, 0.0);
    }

    /// A filled rounded rect.
    pub fn fill(&mut self, r: RectF, radius: f32, color: Rgba) {
        self.list.fill(r, radius, color);
    }

    /// A ring `width` px wide just inside the rounded rect's edge.
    pub fn border(&mut self, r: RectF, radius: f32, width: f32, color: Rgba) {
        self.list.border(r, radius, width, color);
    }

    /// One line of text with its baseline at `baseline`; returns its
    /// advance.
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

    /// Registers the visible part of `rect` (inside the current clip) as a
    /// hit region; nothing when none of it is visible.
    pub fn hit(&mut self, id: WidgetId, rect: RectF, sense: Sense) {
        let rect = rect.intersect(self.list.clip());
        if rect.w > 0.0 && rect.h > 0.0 {
            self.hits.push(Hit { id, rect, sense });
        }
    }

    /// Whether `id` is under the pointer, and whether it is also held.
    fn pointer(&self, id: WidgetId) -> (bool, bool) {
        let hover = self.state.hover == Some(id);
        (hover, hover && self.state.pressed == Some(id))
    }

    /// The light top edge of a raised fill: its 1 px border clipped to the
    /// top device pixel row.
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

    /// Draws `lines` top-down from `(x, y)`, skipping those outside the
    /// clip; returns the widest line's advance.
    fn lines(&mut self, x: f32, y: f32, lines: &[&str], style: TextStyle) -> f32 {
        let lh = self.text.line_height(style);
        let (a, d) = (self.text.ascent(style), self.text.descent(style));
        let base = self.snap((lh - a - d) / 2.0 + a);
        let clip = self.list.clip();
        let mut widest: f32 = 0.0;
        for (i, line) in lines.iter().enumerate() {
            let top = y + i as f32 * lh;
            let w = if top + lh <= clip.y || top >= clip.y + clip.h {
                self.text.measure(line, style)
            } else {
                self.text.draw_text(self.list, x, top + base, line, style)
            };
            widest = widest.max(w);
        }
        widest
    }
}

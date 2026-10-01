//! The immediate-mode builder apps draw with: a layout cursor, a handful of
//! widgets, and the hit regions the shell routes the pointer by.

use gfx::{DrawList, RectF, Rgba};
use text::{FontId, TextStyle, TextSystem};

use crate::theme;

/// Padding between the [`Ui`] rect and its content.
pub const PAD: f32 = 12.0;
/// Space after each item, and between the items of a [`Ui::row`].
pub const SPACING: f32 = 8.0;
/// Height of a button.
pub const BUTTON_H: f32 = 30.0;
/// Height of a text field.
pub const FIELD_H: f32 = 32.0;
const BUTTON_PAD_X: f32 = 14.0;
const FIELD_PAD_X: f32 = 10.0;
const FIELD_RADIUS: f32 = 8.0;
const CARET_W: f32 = 1.5;

const HEADING: TextStyle = TextStyle::new(FontId::SansBold, 20.0, theme::TEXT_BRIGHT);
const SUBHEADING: TextStyle = TextStyle::new(FontId::SansBold, 15.0, theme::TEXT_BRIGHT);
const LABEL: TextStyle = TextStyle::new(FontId::Sans, 14.0, theme::TEXT);
const SMALL: TextStyle = TextStyle::new(FontId::Sans, 12.0, theme::TEXT_DIM);
const MONO: TextStyle = TextStyle::new(FontId::Mono, 13.0, theme::TEXT);

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

/// One frame of one window's content. Items stack down from the rect's
/// corner inset by [`PAD`], [`SPACING`] apart (left to right in a
/// [`Ui::row`]); text wraps at the content's right edge. [`Ui::new`] clips to
/// the rect and dropping the `Ui` pops that clip and any left open. Widgets
/// return the rect they took; interactive ones register a [`Hit`].
#[derive(Debug)]
pub struct Ui<'a> {
    list: &'a mut DrawList,
    text: &'a mut TextSystem,
    rect: RectF,
    hits: &'a mut Vec<Hit>,
    state: UiState,
    x: f32,
    y: f32,
    /// The tallest item so far, while laying out a row.
    row: Option<f32>,
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
    /// A builder drawing into `list` inside `rect`, registering hits into
    /// `hits` (which it only appends to). Pushes `rect` as a clip.
    pub fn new(
        list: &'a mut DrawList,
        text: &'a mut TextSystem,
        rect: RectF,
        hits: &'a mut Vec<Hit>,
        state: UiState,
    ) -> Ui<'a> {
        list.push_clip(rect);
        Ui { list, text, rect, hits, state, x: rect.x + PAD, y: rect.y + PAD, row: None, clips: 0 }
    }

    /// The whole rect, padding included.
    pub fn rect(&self) -> RectF {
        self.rect
    }

    /// The content width: the rect's width minus padding on both sides.
    pub fn width(&self) -> f32 {
        (self.rect.w - 2.0 * PAD).max(0.0)
    }

    /// Pointer and focus state for this frame.
    pub fn state(&self) -> UiState {
        self.state
    }

    /// Where the next item goes.
    pub fn cursor(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    /// Moves the cursor; later items stack from there.
    pub fn set_cursor(&mut self, x: f32, y: f32) {
        (self.x, self.y) = (x, y);
    }

    /// Moves the cursor down to `y` if it is above it: call after drawing
    /// custom content with the low-level calls.
    pub fn advance_to(&mut self, y: f32) {
        if y > self.y {
            self.y = y;
        }
    }

    /// Empty space: down, or right inside a row.
    pub fn space(&mut self, px: f32) {
        match self.row {
            Some(_) => self.x += px,
            None => self.y += px,
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
        (self.rect.x + self.rect.w - PAD - self.x).max(0.0)
    }

    /// Takes a `w` x `h` rect at the cursor and moves past it.
    fn place(&mut self, w: f32, h: f32) -> RectF {
        let r = RectF::new(self.x, self.y, w, h);
        match &mut self.row {
            Some(tallest) => {
                *tallest = tallest.max(h);
                self.x += w + SPACING;
            }
            None => self.y += h + SPACING,
        }
        r
    }

    /// Lays out the items `f` adds left to right, [`SPACING`] apart, as one
    /// item as tall as the tallest of them.
    pub fn row(&mut self, f: impl FnOnce(&mut Self)) -> RectF {
        let (x0, y0, outer) = (self.x, self.y, self.row.replace(0.0));
        f(self);
        let h = self.row.unwrap_or(0.0);
        let w = (self.x - x0 - SPACING).max(0.0);
        (self.x, self.y, self.row) = (x0, y0, outer);
        self.place(w, h)
    }

    /// A heading: SansBold 20, bright, wrapped.
    pub fn heading(&mut self, text: &str) -> RectF {
        self.block(text, HEADING)
    }

    /// A subheading: SansBold 15, bright, wrapped.
    pub fn subheading(&mut self, text: &str) -> RectF {
        self.block(text, SUBHEADING)
    }

    /// Body text: Sans 14, wrapped at spaces and at each `\n`.
    pub fn label(&mut self, text: &str) -> RectF {
        self.block(text, LABEL)
    }

    /// Secondary text: Sans 12, dim, wrapped.
    pub fn small(&mut self, text: &str) -> RectF {
        self.block(text, SMALL)
    }

    /// Monospaced text: Mono 13, wrapped.
    pub fn mono(&mut self, text: &str) -> RectF {
        self.block(text, MONO)
    }

    /// A 1 px rule across the available width.
    pub fn separator(&mut self) -> RectF {
        let w = self.avail();
        let r = self.place(w, 1.0);
        self.list.fill(r, 0.0, theme::BORDER);
        r
    }

    /// A pill button sized to its label. Registers a [`Sense::Click`] hit.
    pub fn button(&mut self, id: WidgetId, label: &str) -> RectF {
        self.pill(id, label, false)
    }

    /// A button filled with the accent: the window's main action.
    pub fn button_primary(&mut self, id: WidgetId, label: &str) -> RectF {
        self.pill(id, label, true)
    }

    fn pill(&mut self, id: WidgetId, label: &str, primary: bool) -> RectF {
        let tw = self.text.measure(label, LABEL);
        let r = self.place((tw + 2.0 * BUTTON_PAD_X).ceil(), BUTTON_H);
        let hover = self.state.hover == Some(id);
        let down = hover && self.state.pressed == Some(id);
        let (fill, fg) = match (primary, down, hover) {
            (true, true, _) => (theme::ACCENT_PRESSED, theme::ON_ACCENT),
            (true, false, true) => (theme::ACCENT_HOVER, theme::ON_ACCENT),
            (true, false, false) => (theme::ACCENT, theme::ON_ACCENT),
            (false, true, _) => (theme::BUTTON_PRESSED, theme::TEXT_BRIGHT),
            (false, false, true) => (theme::BUTTON_HOVER, theme::TEXT_BRIGHT),
            (false, false, false) => (theme::BUTTON, theme::TEXT),
        };
        self.list.fill(r, r.h / 2.0, fill);
        if !primary {
            self.list.border(r, r.h / 2.0, 1.0, theme::BORDER);
        }
        let style = LABEL.with_color(fg);
        let base = self.baseline_in(r.y, r.h, style);
        let x = self.text.snap(r.x + (r.w - tw) / 2.0);
        self.text.draw_text(self.list, x, base, label, style);
        self.hit(id, r, Sense::Click);
        r
    }

    /// A one-line text field across the available width showing `value` (or
    /// `placeholder`, dim, when `value` is empty). When `has_focus` and the
    /// window is focused it gets an accent ring and a steady caret after the
    /// text, and a value too long to fit scrolls so its end stays visible.
    /// Registers a [`Sense::Text`] hit.
    pub fn text_field(
        &mut self,
        id: WidgetId,
        value: &str,
        has_focus: bool,
        placeholder: &str,
    ) -> RectF {
        let w = self.avail();
        let r = self.place(w, FIELD_H);
        let focus = has_focus && self.state.focused;
        let (well, ring) = match focus {
            true => (theme::FIELD_FOCUSED, theme::ACCENT),
            false => (theme::FIELD, theme::BORDER),
        };
        self.list.fill(r, FIELD_RADIUS, well);
        self.list.border(r, FIELD_RADIUS, 1.0, ring);
        let iw = (r.w - 2.0 * FIELD_PAD_X).max(0.0);
        let inner = RectF::new(r.x + FIELD_PAD_X, r.y, iw, r.h);
        let base = self.baseline_in(r.y, r.h, LABEL);
        let tw = self.text.measure(value, LABEL);
        let x = match tw + CARET_W > inner.w {
            true => inner.x + inner.w - CARET_W - tw,
            false => inner.x,
        };
        self.list.push_clip(inner);
        if value.is_empty() {
            let style = LABEL.with_color(theme::TEXT_DIM);
            self.text.draw_text(self.list, inner.x, base, placeholder, style);
        } else {
            let style = LABEL.with_color(theme::TEXT_BRIGHT);
            self.text.draw_text(self.list, x, base, value, style);
        }
        if focus {
            let (a, d) = (self.text.ascent(LABEL), self.text.descent(LABEL));
            let caret = RectF::new(self.text.snap(x + tw), base - a, CARET_W, a + d);
            self.list.fill(caret, 0.0, theme::ACCENT);
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
        let key_style = LABEL.with_color(theme::TEXT_DIM);
        let word = (self.text.min_width(key, key_style) + SPACING).ceil();
        let kw = (avail * 0.4).min(180.0).floor().max(word);
        let kw = kw.min((avail * 0.6).floor());
        let keys = self.text.wrap(key, key_style, kw - SPACING);
        let values = self.text.wrap(value, LABEL, avail - kw);
        let (x, y) = (self.x, self.y);
        self.lines(x, y, &keys, key_style);
        self.lines(x + kw, y, &values, LABEL);
        let lh = self.text.line_height(LABEL);
        self.place(avail, keys.len().max(values.len()) as f32 * lh)
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

    /// The baseline that centers a line of `style` in `h` px from `top`.
    fn baseline_in(&self, top: f32, h: f32, style: TextStyle) -> f32 {
        let (a, d) = (self.text.ascent(style), self.text.descent(style));
        self.text.snap(top + (h - a - d) / 2.0 + a)
    }

    /// Wrapped text at the cursor, as one item.
    fn block(&mut self, text: &str, style: TextStyle) -> RectF {
        let lines = self.text.wrap(text, style, self.avail());
        let (x, y) = (self.x, self.y);
        let w = self.lines(x, y, &lines, style);
        let lh = self.text.line_height(style);
        self.place(w, lines.len() as f32 * lh)
    }

    /// Draws `lines` top-down from `(x, y)`, skipping those outside the
    /// clip; returns the widest line's advance.
    fn lines(&mut self, x: f32, y: f32, lines: &[&str], style: TextStyle) -> f32 {
        let lh = self.text.line_height(style);
        let base = self.baseline_in(0.0, lh, style);
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

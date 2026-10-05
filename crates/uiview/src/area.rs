//! [`Area`]: a multi-line input. Its text wraps to the box, which grows with it; the caret goes
//! where a press lands (under the last row: to the end), and Left, Right, Home and End move it,
//! Up and Down by the rows drawn.

use gfx::RectF;
use ui::{CODE_MAX, Key, RADIUS_SM, Sense, TextStyle, TextSystem, Ui, WidgetId, sem};

/// The box's least height, and its inset.
pub(crate) const MIN_H: f32 = 144.0;
const INSET: f32 = 13.0;

/// An Area's text, its caret and its layout (see the module docs).
#[derive(Debug, Default)]
pub struct Area {
    pub text: String,
    /// The caret's byte offset in `text`, and the edits so far: the version of its Changes.
    pub at: usize,
    pub version: u32,
    /// Whether the next draw scrolls the caret into view, and the box's top in the content as
    /// last laid out.
    pub(crate) follow: bool,
    top: f32,
    /// For the next layout, which can measure: a press to put the caret under (from the box's
    /// corner), and rows to move it by.
    press: Option<(f32, f32)>,
    rows: i32,
    /// As last laid out: each row's byte range, the caret's row and x, a row's height; and the
    /// box as last drawn.
    spans: Vec<(usize, usize)>,
    row: usize,
    x: f32,
    lh: f32,
    well: RectF,
}

impl Area {
    /// An Area holding `text`, the caret at its end.
    pub fn new(text: &str) -> Area {
        Area { text: text.into(), at: text.len(), ..Area::default() }
    }

    /// Takes `text` from a frame (no one is editing it); a new text puts the caret at its end.
    pub fn set(&mut self, text: &str) {
        if self.text != text {
            (self.text, self.at) = (text.into(), text.len());
        }
    }

    /// Inserts `s` at the caret (new lines kept, other controls dropped), up to [`CODE_MAX`]
    /// bytes in all; whether the text changed.
    pub fn insert(&mut self, s: &str) -> bool {
        let mut clean = String::new();
        for c in s.chars().filter(|&c| c == '\n' || !c.is_control()) {
            if self.text.len() + clean.len() + c.len_utf8() > CODE_MAX {
                break;
            }
            clean.push(c);
        }
        self.text.insert_str(self.at, &clean);
        self.at += clean.len();
        self.edited(!clean.is_empty())
    }

    /// The caret moved, and the text changed if `changed` (one more version); returns that.
    fn edited(&mut self, changed: bool) -> bool {
        self.version = self.version.wrapping_add(u32::from(changed));
        self.follow = true;
        changed
    }

    /// The char boundary next to the caret, before it (`back`) or after it (or the caret, at an
    /// end).
    fn step(&self, back: bool) -> usize {
        let (t, mut i) = (&self.text, self.at);
        while (back && i > 0) || (!back && i < t.len()) {
            i = if back { i - 1 } else { i + 1 };
            if t.is_char_boundary(i) {
                break;
            }
        }
        i
    }

    /// A key: whether it changed the text, for one an Area takes (Enter is a new line); `None`
    /// for the others.
    pub fn key(&mut self, key: Key) -> Option<bool> {
        // The caret's line, by bytes: `\n` is never inside a char.
        let (t, at) = (self.text.as_bytes(), self.at);
        let start = t[..at].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let end = t[at..].iter().position(|&b| b == b'\n').map_or(t.len(), |i| at + i);
        match key {
            Key::Enter => return Some(self.insert("\n")),
            Key::Backspace | Key::Delete => {
                self.at = self.step(key == Key::Backspace).min(at);
                let gone = self.at < self.text.len() && (key == Key::Delete || self.at < at);
                if gone {
                    self.text.drain(self.at..self.step(false));
                }
                return Some(self.edited(gone));
            }
            Key::Left | Key::Right => self.at = self.step(key == Key::Left),
            Key::Home => self.at = start,
            Key::End => self.at = end,
            Key::Up | Key::Down => self.rows += if key == Key::Up { -1 } else { 1 },
            _ => return None,
        }
        Some(self.edited(false))
    }

    /// A press at `(x, y)`, where the box was last drawn: the caret goes there at the next layout.
    pub fn click(&mut self, x: f32, y: f32) {
        self.press = Some((x - self.well.x, y - self.well.y));
    }

    /// Wraps the text in `style` to a box `w` wide at `top` in the content, placing the caret
    /// where a press or a move by rows asks; the box's height.
    pub(crate) fn layout(
        &mut self,
        ts: &mut TextSystem,
        style: TextStyle,
        w: f32,
        top: f32,
    ) -> f32 {
        let lh = ts.line_height(style);
        let text = self.text.as_str();
        // Each row's byte range: where its slice lies in the text.
        let mut spans = Vec::new();
        for r in ts.wrap(text, style, w - 2.0 * INSET) {
            let at = r.as_ptr() as usize - text.as_ptr() as usize;
            spans.push((at, at + r.len()));
        }
        let x_of = |ts: &mut TextSystem, row: usize, at: usize| {
            ts.measure(text.get(spans[row].0..at.max(spans[row].0)).unwrap_or_default(), style)
        };
        let mut row = spans.iter().rposition(|s| s.0 <= self.at).unwrap_or(0);
        let want = match (self.press.take(), std::mem::take(&mut self.rows)) {
            (Some((px, py)), _) => Some(((py - INSET) / lh, px - INSET)),
            (None, 0) => None,
            (None, n) => Some((row as f32 + n as f32 + 0.5, x_of(ts, row, self.at))),
        };
        // The char boundary nearest that point; under the last row, the end.
        if let Some((r, x)) = want {
            row = (r.max(0.0) as usize).min(spans.len() - 1);
            let ((s, e), below) = (spans[row], r >= spans.len() as f32);
            let mut best = (f32::MAX, if below { e } else { s });
            for i in (s..=e).filter(|&i| !below && text.is_char_boundary(i)) {
                let gap = (ts.measure(text.get(s..i).unwrap_or_default(), style) - x).abs();
                if gap < best.0 {
                    best = (gap, i);
                }
            }
            (self.at, self.follow) = (best.1, true);
        }
        self.x = x_of(ts, row, self.at);
        (self.spans, self.row, self.lh, self.top) = (spans, row, lh, top);
        (self.spans.len() as f32 * lh + 2.0 * INSET).max(MIN_H).round()
    }

    /// The caret's row in the content, with the inset around it, as last laid out.
    pub(crate) fn caret_band(&self) -> (f32, f32) {
        let y = self.top + INSET + self.row as f32 * self.lh;
        (y - INSET, y + self.lh + INSET)
    }

    /// The box in `r` as last laid out: its rows (or `hint`), the focus ring and caret when
    /// `focus`; a [`Sense::Text`] hit, marked a text field holding its text.
    pub(crate) fn draw(
        &mut self,
        ui: &mut Ui<'_>,
        id: WidgetId,
        r: RectF,
        focus: bool,
        hint: &str,
    ) {
        let (t, style) = (ui.theme(), ui.theme().body());
        self.well = r;
        ui.fill(r, RADIUS_SM, t.surface_lo);
        let focus = focus && ui.state().focused;
        if focus {
            let (halo, wide) = (t.accent.with_alpha(t.selection.3 / 2), ui.px(3.0));
            ui.border(r.inset(-wide), RADIUS_SM + wide, wide, halo);
            let ring = ui.px(1.5);
            ui.border(r, RADIUS_SM, ring, t.accent);
        } else {
            let line = ui.px(1.0);
            ui.border(r, RADIUS_SM, line, t.border);
        }
        let ts = ui.text_system();
        let (a, d) = (ts.ascent(style), ts.descent(style));
        let base = ts.snap((self.lh - a - d) / 2.0 + a);
        let (tx, ty) = (r.x + INSET, r.y + INSET);
        if self.text.is_empty() {
            ui.text(tx, ty + base, hint, style.with_color(t.text_dim));
        }
        for (i, &(s, e)) in self.spans.iter().enumerate() {
            let y = ui.text_system().snap(ty + i as f32 * self.lh);
            ui.text(tx, y + base, self.text.get(s..e).unwrap_or_default(), style);
        }
        if focus {
            let (y, w) = (ty + self.row as f32 * self.lh + base - a, ui.px(1.5));
            let caret = ui.snapped(RectF::new(tx + self.x, y, w, a + d));
            ui.fill(RectF { w, ..caret }, 0.0, t.accent);
        }
        ui.hit(id, r, Sense::Text);
        // Its first KiB: the screen the AI reads keeps less.
        let mut end = self.text.len().min(1024);
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        let flags = if focus { sem::FOCUSED } else { 0 };
        ui.mark(id, sem::TEXTBOX, flags, self.text.get(..end).unwrap_or_default());
    }
}

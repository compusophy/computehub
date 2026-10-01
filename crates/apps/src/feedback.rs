//! Feedback: what to fix, what to build, what to keep, straight to compusophy.

use gfx::RectF;
use ui::{App, AppEvent, AppIcon, BUTTON_H, Cx, Key, Mods, RADIUS_SM, Sense, TextStyle};
use ui::{TextSystem, Ui, WidgetId};

use crate::kit::{self, Scroll};

/// The kinds as (chip, what is sent).
const KINDS: [(&str, &str); 3] = [("Bug", "bug"), ("Idea", "idea"), ("Love", "love")];
const INTRO: &str = "Tell compusophy what broke, what to build next, or what you love.";
const HINT: &str = "What happened, or what would make it better?";
const CONTEXT: &str = "Include what\u{2019}s open and recent events";
const CONTEXT_NOTE: &str = "The build, your browser and screen size, the theme, the apps open \
and the last 50 events. Never your files.";
const SEND: &str = "Send";
/// What Send says: true whether the report goes at once or waits in the page's outbox, as the app
/// cannot know which.
pub(crate) const THANKS: &str = "Thank you \u{2014} it goes when it can";
/// The most text kept, in bytes.
const MAX: usize = 8000;
/// Widget ids: kind `i` is `KIND + i`; the text, the context box, Send.
const KIND: u32 = 1;
const AREA: u32 = 10;
const BOX: u32 = 11;
const GO: u32 = 12;
/// The column's widest, the text's least height and its inset.
const MAX_W: f32 = 610.0;
const AREA_MIN: f32 = 144.0;
const INSET: f32 = 13.0;
/// The context row's inset; its switch and a gap take 47 at the right.
const CHECK_X: f32 = 8.0;

/// Feedback: chips for the kind (Bug, Idea, Love), a text that wraps and grows, a box to include
/// the desktop's context, and Send, which hands it all to the page ([`Cx::feedback`]) and thanks;
/// the page scrolls, following the caret.
#[derive(Debug)]
pub struct Feedback {
    /// An index into [`KINDS`]: Idea at first.
    pub(crate) kind: usize,
    pub(crate) text: String,
    /// The caret's byte offset in `text`.
    pub(crate) at: usize,
    pub(crate) context: bool,
    pub(crate) focus: bool,
    /// What the last Send said, until the next edit.
    pub(crate) status: Option<&'static str>,
    scroll: Scroll,
    /// For the next draw, which can measure: a press in the text to put the caret under, rows to
    /// move it by, and whether to scroll it into view.
    press: Option<(f32, f32)>,
    rows: i32,
    follow: bool,
}

impl Default for Feedback {
    fn default() -> Feedback {
        let (text, scroll, press) = (String::new(), Scroll::default(), None);
        #[rustfmt::skip]
        let f = Feedback { kind: 1, text, at: 0, context: true, focus: true, status: None, scroll,
            press, rows: 0, follow: false };
        f
    }
}

/// The text's layout: rows as byte ranges, the caret's row and x, the box's height, a row's.
struct Lay {
    spans: Vec<(usize, usize)>,
    row: usize,
    x: f32,
    h: f32,
    lh: f32,
}

impl Feedback {
    /// Inserts `s` (newlines kept, other controls dropped) at the caret, up to [`MAX`] bytes.
    fn insert(&mut self, s: &str) {
        let mut clean = String::new();
        for c in s.chars().filter(|&c| c == '\n' || !c.is_control()) {
            if self.text.len() + clean.len() + c.len_utf8() > MAX {
                break;
            }
            clean.push(c);
        }
        self.text.insert_str(self.at, &clean);
        self.at += clean.len();
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

    /// A key while the text has focus; whether anything changed.
    fn key(&mut self, key: Key, mods: Mods, cx: &mut Cx<'_>) -> bool {
        // The caret's line, by bytes: `\n` is never inside a char.
        let (t, at) = (self.text.as_bytes(), self.at);
        let start = t[..at].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let end = t[at..].iter().position(|&b| b == b'\n').map_or(t.len(), |i| at + i);
        match key {
            Key::Enter if mods.ctrl || mods.meta => return self.send(cx),
            Key::Enter => self.insert("\n"),
            Key::Backspace | Key::Delete => {
                self.at = self.step(key == Key::Backspace).min(at);
                if self.at < self.text.len() && (key == Key::Delete || self.at < at) {
                    self.text.remove(self.at);
                }
            }
            Key::Left | Key::Right => self.at = self.step(key == Key::Left),
            Key::Home => self.at = start,
            Key::End => self.at = end,
            Key::Up | Key::Down => self.rows += if key == Key::Up { -1 } else { 1 },
            Key::Escape => self.focus = false,
            _ => return false,
        }
        (self.status, self.follow) = (None, true);
        true
    }

    /// Sends the text, if there is any, and clears it; whether it went.
    fn send(&mut self, cx: &mut Cx<'_>) -> bool {
        let text = self.text.trim();
        if text.is_empty() {
            return false;
        }
        cx.feedback(KINDS[self.kind].1, text, self.context);
        self.status = Some(THANKS);
        (self.text, self.at) = (String::new(), 0);
        true
    }

    /// The text laid out in the box `w` wide, `left` and `top` into the content: each row's byte
    /// range, the caret's row and x, the box's height. A press, or rows to move by, resolve here,
    /// where text can be measured.
    fn layout(&mut self, ts: &mut TextSystem, style: TextStyle, [left, top, w]: [f32; 3]) -> Lay {
        let lh = ts.line_height(style);
        let text = self.text.as_str();
        // Each row's byte range: where its slice lies in the text.
        let mut spans = Vec::new();
        for r in ts.wrap(text, style, w - 2.0 * INSET) {
            let at = r.as_ptr() as usize - text.as_ptr() as usize;
            spans.push((at, at + r.len()));
        }
        let row_of = |at: usize| spans.iter().rposition(|s| s.0 <= at).unwrap_or(0);
        let x_of = |ts: &mut TextSystem, row: usize, at: usize| {
            ts.measure(&text[spans[row].0..at.max(spans[row].0)], style)
        };
        let mut row = row_of(self.at);
        let want = match (self.press.take(), std::mem::take(&mut self.rows)) {
            (Some((px, py)), _) => {
                Some(((py + self.scroll.y - top - INSET) / lh, px - left - INSET))
            }
            (None, 0) => None,
            (None, n) => Some((row as f32 + n as f32 + 0.5, x_of(ts, row, self.at))),
        };
        // The char boundary nearest that point.
        if let Some((r, x)) = want {
            row = (r.max(0.0) as usize).min(spans.len() - 1);
            let (s, e) = spans[row];
            let mut best = (f32::MAX, s);
            for i in (s..=e).filter(|&i| text.is_char_boundary(i)) {
                let gap = (ts.measure(&text[s..i], style) - x).abs();
                if gap < best.0 {
                    best = (gap, i);
                }
            }
            (self.at, self.follow) = (best.1, true);
        }
        let h = (spans.len() as f32 * lh + 2.0 * INSET).max(AREA_MIN).round();
        let x = x_of(ts, row, self.at);
        Lay { spans, row, x, h, lh }
    }

    /// The box laid out as `lay` in `well`: its rows (or the hint), the focus ring, the caret.
    fn paint(&self, ui: &mut Ui<'_>, lay: &Lay, well: RectF) {
        let (t, style) = (ui.theme(), ui.theme().body());
        ui.fill(well, RADIUS_SM, t.surface_lo);
        let focus = self.focus && ui.state().focused;
        if focus {
            let (halo, wide) = (t.accent.with_alpha(t.selection.3 / 2), ui.px(3.0));
            ui.border(well.inset(-wide), RADIUS_SM + wide, wide, halo);
            let ring = ui.px(1.5);
            ui.border(well, RADIUS_SM, ring, t.accent);
        } else {
            let line = ui.px(1.0);
            ui.border(well, RADIUS_SM, line, t.border);
        }
        let ts = ui.text_system();
        let (a, d) = (ts.ascent(style), ts.descent(style));
        let base = ts.snap((lay.lh - a - d) / 2.0 + a);
        let (tx, ty) = (well.x + INSET, well.y + INSET);
        if self.text.is_empty() {
            ui.text(tx, ty + base, HINT, style.with_color(t.text_dim));
        }
        for (i, &(s, e)) in lay.spans.iter().enumerate() {
            let y = ui.text_system().snap(ty + i as f32 * lay.lh);
            ui.text(tx, y + base, &self.text[s..e], style);
        }
        if focus {
            let (y, w) = (ty + lay.row as f32 * lay.lh + base - a, ui.px(1.5));
            let caret = ui.snapped(RectF::new(tx + lay.x, y, w, a + d));
            ui.fill(RectF { w, ..caret }, 0.0, t.accent);
        }
        ui.hit(WidgetId(AREA), well, Sense::Text);
    }
}

impl App for Feedback {
    fn title(&self) -> String {
        "Feedback".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let (r, t) = (ui.rect(), ui.theme());
        let (x, w) = kit::column(ui, r, MAX_W);
        let (small, body) = (t.small(), t.body());
        let chip_h = if r.w < 560.0 { 44.0 } else { 34.0 };
        // Laid out first, as offsets into the content, so the caret is followed before anything
        // is drawn: the intro, the chips, the text, the box and its note, Send.
        let ts = ui.text_system();
        let (lh, note_w) = (ts.line_height(small), w - 2.0 * CHECK_X);
        let intro = ts.wrap(INTRO, small, w).len() as f32 * lh;
        let note = ts.wrap(CONTEXT_NOTE, small, note_w).len() as f32 * lh;
        let well_y = 21.0 + intro + 13.0 + chip_h + 13.0;
        let lay = self.layout(ts, body, [x - r.x, well_y, w]);
        let box_y = well_y + lay.h + 8.0;
        let send_y = box_y + 44.0 + note + 21.0;
        self.scroll.measure(send_y + BUTTON_H + 21.0, r.h);
        if std::mem::take(&mut self.follow) {
            let caret = well_y + INSET + lay.row as f32 * lay.lh;
            self.scroll.show(caret - INSET, caret + lay.lh + INSET, r.h);
        }
        let y0 = ui.text_system().snap(r.y - self.scroll.y);
        kit::para(ui, INTRO, small, (x, w), y0 + 21.0, false);
        let mut cx = x;
        for (i, (label, _)) in KINDS.iter().enumerate() {
            let cw = (ui.text_system().measure(label, body) + 42.0).ceil();
            let at = ui.snapped(RectF::new(cx, y0 + 21.0 + intro + 13.0, cw, chip_h));
            chip(ui, WidgetId(KIND + i as u32), at, label, i == self.kind);
            cx = at.x + at.w + 8.0;
        }
        let well = ui.snapped(RectF::new(x, y0 + well_y, w, lay.h));
        self.paint(ui, &lay, well);
        // Whether to include the context.
        let row = ui.snapped(RectF::new(x, y0 + box_y, w, 44.0));
        let (hover, down) = kit::pointer(ui, WidgetId(BOX));
        if hover {
            ui.fill(row, RADIUS_SM, t.wash(down));
        }
        kit::switch(ui, row.inset(8.0), self.context);
        let base = kit::cap_base(ui, row.y, row.h, body);
        let shown = ui.text_system().ellipsize(CONTEXT, body, note_w - 47.0);
        ui.text(x + CHECK_X, base, &shown, body);
        ui.hit(WidgetId(BOX), row, Sense::Click);
        kit::para(ui, CONTEXT_NOTE, small, (x + CHECK_X, note_w), row.y + row.h, false);
        // Send (in the accent once there is something to send), and what the last one did.
        let y = y0 + send_y;
        let bw = ui::button_width(ui.text_system(), t, SEND);
        ui.set_cursor(x + w - bw, y);
        if self.text.trim().is_empty() {
            ui.button(WidgetId(GO), SEND);
        } else {
            ui.button_primary(WidgetId(GO), SEND);
        }
        if let Some(said) = self.status {
            let base = kit::cap_base(ui, y, BUTTON_H, body);
            let shown = ui.text_system().ellipsize(said, body, w - bw - 13.0);
            ui.text(x, base, &shown, body);
        }
        self.scroll.thumb(ui, r);
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        match ev {
            AppEvent::Click(WidgetId(id @ 1..=3)) => {
                let kind = (id - KIND) as usize;
                std::mem::replace(&mut self.kind, kind) != kind
            }
            AppEvent::Click(WidgetId(BOX)) => {
                self.context = !self.context;
                true
            }
            AppEvent::Click(WidgetId(GO)) => self.send(cx),
            AppEvent::PointerDown { x, y, id } => {
                let on = id == Some(WidgetId(AREA));
                if on {
                    self.press = Some((x, y));
                }
                // A press on nothing leaves the text; one on a control keeps it.
                let focus = on || (self.focus && id.is_some());
                let changed = on || focus != self.focus;
                self.focus = focus;
                changed
            }
            AppEvent::Key { key, mods } if self.focus => self.key(key, mods, cx),
            AppEvent::Text(s) if self.focus => {
                self.insert(&s);
                (self.status, self.follow) = (None, true);
                true
            }
            AppEvent::Wheel { dy, .. } => self.scroll.wheel(dy),
            AppEvent::Resized { .. } => true,
            _ => false,
        }
    }

    fn wants_text_input(&self) -> bool {
        self.focus
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((520.0, 420.0))
    }

    fn icon(&self) -> AppIcon {
        kit::FEEDBACK
    }

    fn compact(&self) -> bool {
        true
    }
}

/// Kind chip `id` in `r`: a pill, filled in the accent when `on`, else raised and dim.
fn chip(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str, on: bool) {
    let t = ui.theme();
    let (hover, down) = kit::pointer(ui, id);
    let ink = if on {
        ui.fill(r, r.h / 2.0, t.accent);
        t.accent_text
    } else {
        let (fill, edge) = kit::card_colors(t, hover, down);
        kit::raised(ui, r, r.h / 2.0, fill, edge);
        if hover { t.text } else { t.text_dim }
    };
    let style = t.body().with_color(ink);
    let w = ui.text_system().measure(label, style);
    let base = kit::cap_base(ui, r.y, r.h, style);
    let x = ui.text_system().snap(r.x + (r.w - w) / 2.0);
    ui.text(x, base, label, style);
    ui.hit(id, r, Sense::Click);
}

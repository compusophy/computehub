//! The code editor widget: text its owner edits in an [`Editor`], drawn in
//! Mono with a line-number gutter, a caret and highlight spans in the theme's
//! colors.

use gfx::RectF;
use text::Editor;

use crate::theme::mix;
use crate::{Key, RADIUS_SM, Sense, Ui, WidgetId};

/// The most bytes of text a [`Code`] takes: an edit that grows it past this
/// is refused, so the whole text fits in one 64 KiB message.
pub const CODE_MAX: usize = 65_000;
/// Space between the well and its text.
const INSET: f32 = 6.0;

/// A highlighted byte range of a [`Code`]'s text: start, length and class
/// (0 plain, 1 keyword, 2 string, 3 number, 4 comment, 5 name, 6
/// punctuation, 7 error, underlined).
pub type Span = (u32, u32, u8);
/// A colored byte range of one line: start, end and class.
type Run = (usize, usize, u8);

/// Where the last frame put the text: the well, the first cell's corner, a
/// cell and a row, and how many whole rows fit (at least one).
#[derive(Clone, Copy, Debug)]
struct Geo {
    well: RectF,
    text: RectF,
    cell: f32,
    row: f32,
    rows: usize,
}

/// A multi-line code editor. Arrows, Home, End, Page Up and Down move the
/// caret; Enter keeps the indent, Tab types two spaces; a press places the
/// caret and the wheel scrolls whole rows, taking the caret along. Each
/// edit adds one to `version`. Spans color the text they were set for:
/// after an edit, each line keeps its colors while its text is unchanged.
#[derive(Clone, Debug, Default)]
pub struct Code {
    pub ed: Editor,
    /// Edits so far, or the version the text was last set at.
    pub version: u32,
    /// The first line and column in view.
    pub top: usize,
    left: usize,
    /// Wheel pixels not yet a whole row.
    wheel: f32,
    geo: Option<Geo>,
    /// Each line of the text the spans were set for, with its runs.
    marks: Vec<(String, Vec<Run>)>,
}

impl Code {
    /// An editor holding `text` at `version`, the caret at its start.
    pub fn new(text: &str, version: u32) -> Code {
        Code { ed: Editor::new(text), version, ..Code::default() }
    }

    /// Replaces the text at `version`; the caret stays where it still fits.
    pub fn set_text(&mut self, text: &str, version: u32) {
        let (line, col) = self.ed.caret();
        (self.ed, self.version) = (Editor::new(text), version);
        self.ed.set_caret(line, col);
    }

    /// Colors the current text with `spans`, in order and apart.
    pub fn set_spans(&mut self, spans: &[Span]) {
        let (mut at, mut k) = (0, 0);
        self.marks.clear();
        for line in self.ed.lines() {
            let (end, mut runs) = (at + line.len(), Vec::new());
            while let Some(&(s, n, class)) = spans.get(k).filter(|s| s.0 as usize <= end) {
                let (s, e) = (s as usize, (s as usize).saturating_add(n as usize));
                if e.min(end) > s.max(at) {
                    runs.push((s.max(at) - at, e.min(end) - at, class));
                }
                // A span that goes on past the line's `\n` colors the next too.
                if e > end + 1 {
                    break;
                }
                k += 1;
            }
            self.marks.push((line.to_string(), runs));
            at = end + 1;
        }
    }

    /// Types `s` at the caret; whether the text changed.
    pub fn insert(&mut self, s: &str) -> bool {
        self.edit(|ed| ed.insert(s))
    }

    /// An editing or caret key: `Some(true)` if it changed the text,
    /// `Some(false)` if it only moved the caret, `None` if it is not one.
    pub fn key(&mut self, key: Key) -> Option<bool> {
        let (line, page) = (self.ed.caret().0, self.geo.map_or(20, |g| g.rows));
        let last = self.ed.line_count() - 1;
        let mut known = true;
        let edited = self.edit(|ed| match key {
            Key::Enter => ed.newline(),
            Key::Tab => ed.insert("  "),
            Key::Backspace | Key::Delete => ed.delete(key == Key::Backspace),
            Key::Left | Key::Right => ed.step(key == Key::Left),
            Key::Up if line > 0 => ed.move_to_line(line - 1),
            Key::Down if line < last => ed.move_to_line(line + 1),
            // On the first line Up goes to its start, on the last Down to its end.
            Key::Home | Key::Up => ed.set_caret(line, 0),
            Key::End | Key::Down => ed.set_caret(line, usize::MAX),
            Key::PageUp => ed.move_to_line(line.saturating_sub(page)),
            Key::PageDown => ed.move_to_line(line + page),
            _ => known = false,
        });
        known.then_some(edited)
    }

    /// Does `f` to the text, undone if it grew past [`CODE_MAX`]; whether
    /// the text changed (then `version` moves on).
    fn edit(&mut self, f: impl FnOnce(&mut Editor)) -> bool {
        let before = self.ed.clone();
        f(&mut self.ed);
        if self.ed.len() > CODE_MAX.max(before.len()) {
            self.ed = before;
            return false;
        }
        // Every edit inserts or deletes, so the length tells.
        let edited = self.ed.len() != before.len();
        self.version = self.version.wrapping_add(u32::from(edited));
        edited
    }

    /// Whether the last frame drew the editor over `(x, y)`.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.geo.is_some_and(|g| g.well.contains(x, y))
    }

    /// A press at `(x, y)` (in the frame's coordinates) puts the caret at the
    /// nearest char boundary, clamped into the text.
    pub fn click(&mut self, x: f32, y: f32) {
        if let Some(g) = self.geo {
            // `as` saturates, and NaN (a NaN point) becomes 0.
            let col = ((x - g.text.x + self.left as f32 * g.cell) / g.cell).round().max(0.0);
            let line = ((y - g.text.y + self.top as f32 * g.row) / g.row).max(0.0);
            self.ed.set_caret(line as usize, col as usize);
        }
    }

    /// The wheel: whole rows, the caret kept on a row in view; whether it moved.
    pub fn wheel(&mut self, dy: f32) -> bool {
        let Some(g) = self.geo.filter(|_| dy.is_finite()) else { return false };
        self.wheel += dy;
        let steps = (self.wheel / g.row).trunc();
        self.wheel -= steps * g.row;
        let max = self.ed.line_count().saturating_sub(g.rows) as f32;
        let top = (self.top as f32 + steps).max(0.0).min(max) as usize;
        if top == self.top {
            return false;
        }
        self.top = top;
        self.ed.move_to_line(self.ed.caret().0.clamp(top, top + g.rows - 1));
        true
    }

    /// The runs of line `i` (`line`) of `count`: from the marks of the same
    /// line, or of the line as far from the end, while its text is the same.
    pub(crate) fn runs(&self, i: usize, count: usize, line: &str) -> &[Run] {
        let shift = self.marks.len() as isize - count as isize;
        let same = |j: Option<usize>| j.and_then(|j| self.marks.get(j)).filter(|m| m.0 == line);
        same(Some(i)).or_else(|| same(i.checked_add_signed(shift))).map_or(&[], |m| &m.1)
    }

    /// Draws the editor filling `rect`: a well, the gutter (with
    /// `numbers`), the colored text, and the caret if `focus`; a
    /// [`Sense::Text`] hit for `id`. Scrolls to keep the caret in view.
    pub fn draw(&mut self, ui: &mut Ui<'_>, id: WidgetId, rect: RectF, numbers: bool, focus: bool) {
        let (t, line_px, mono) = (ui.theme(), ui.px(1.0), ui.theme().mono());
        let ts = ui.text_system();
        let (row, asc, cell) = (ts.line_height(mono), ts.ascent(mono), ts.cell_width(mono.size));
        let count = self.ed.line_count();
        let gutter = if numbers { (count.max(10).ilog10() + 3) as f32 * cell } else { INSET };
        let (w, h) = (rect.w - gutter - INSET, rect.h - 2.0 * INSET);
        let text = RectF::new(rect.x + gutter, rect.y + INSET, w.max(cell), h.max(row));
        let (rows, cols) = (((text.h / row) as usize).max(1), ((text.w / cell) as usize).max(1));
        self.geo = Some(Geo { well: rect, text, cell, row, rows });
        let (cl, cc) = self.ed.caret();
        self.top = self.top.clamp((cl + 1).saturating_sub(rows), cl);
        self.left = self.left.clamp((cc + 1).saturating_sub(cols), cc);
        ui.fill(rect, RADIUS_SM, t.surface_lo);
        ui.push_clip(rect);
        if numbers {
            let sep = ui.text_system().snap(text.x - 0.625 * cell);
            ui.fill(RectF::new(sep, rect.y, line_px, rect.h), 0.0, t.border);
        }
        // Each class's ink. Plain code is a shade softer than body text, so
        // keywords stand out even where the accent is white.
        let plain = mix(t.text, t.text_dim, 0.2);
        let inks = [plain, t.accent, t.ansi[2], t.ansi[3], t.text_faint, t.text, t.text_dim];
        for (i, line) in self.ed.lines().enumerate().skip(self.top).take(rows + 1) {
            let (y, lit) = (text.y + (i - self.top) as f32 * row, i == cl && focus);
            let base = ui.text_system().snap(y + asc);
            if lit {
                let band = RectF::new(rect.x + line_px, y, rect.w - 2.0 * line_px, row);
                ui.fill(band, 0.0, t.surface_hi);
            }
            if numbers {
                let mut num = String::new();
                push_num(&mut num, i + 1);
                let nx = text.x - 1.25 * cell - ui.text_system().measure(&num, mono);
                let dim = mono.with_color(if lit { t.text_dim } else { t.text_faint });
                ui.text(nx, base, &num, dim);
            }
            let runs = self.runs(i, count, line);
            for (k, (at, c)) in line.char_indices().skip(self.left).take(cols + 1).enumerate() {
                let x = text.x + k as f32 * cell;
                let class = runs.iter().rfind(|r| (r.0..r.1).contains(&at)).map_or(0, |r| r.2);
                if class == 7 {
                    ui.fill(RectF::new(x, base + 2.0 * line_px, cell, line_px), 0.0, t.danger);
                }
                if c != ' ' {
                    let style = mono.with_color(*inks.get(usize::from(class)).unwrap_or(&plain));
                    ui.text(x, base, c.encode_utf8(&mut [0; 4]), style);
                }
            }
        }
        // The clamps above keep the caret in view.
        if focus {
            let w = ui.px(2.0);
            let x = ui.text_system().snap(text.x + (cc - self.left) as f32 * cell - w / 2.0);
            let y = text.y + (cl - self.top) as f32 * row;
            let color = if ui.state().focused { t.accent } else { t.text_faint };
            ui.fill(RectF::new(x, y, w, row), 0.0, color);
        }
        ui.pop_clip();
        ui.border(rect, RADIUS_SM, line_px, t.border);
        ui.hit(id, rect, Sense::Text);
    }
}

/// Appends `n` in decimal (no `format!` in shipped code).
pub fn push_num(out: &mut String, n: usize) {
    if n >= 10 {
        push_num(out, n / 10);
    }
    out.push(char::from(b'0' + (n % 10) as u8));
}

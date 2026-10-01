//! The Terminal: a [`term::Term`] drawn cell by cell, running the guest shell.

use gfx::{DrawList, RectF, Rgba};
use guest::Guest;
use term::{Attrs, Cell, Color, Term};
use ui::{App, AppEvent, AppIcon, Cx, FontId, Key, Sense, TextStyle, TextSystem, Theme};
use ui::{Ui, WidgetId};

/// The grid's font size and its inset from the content edge, in pixels.
const SIZE: f32 = 13.0;
const INSET: f32 = 14.0;
const BANNER: &str = "\x1b[1mcompusophyOS terminal\x1b[m — type 'help'.\n";

/// A terminal window running [`guest::Guest`]: a Mono 13 px grid (the same before that font
/// arrives), [`Theme::ansi`] colors, a steady accent block cursor (an outline unfocused). The
/// greeting waits for the first grid; the wheel scrolls back, any key snaps back. Programs the
/// shell starts run in the foreground, their output on [`AppEvent::Io`].
#[derive(Debug)]
pub struct Terminal {
    pub(crate) term: Term,
    guest: Guest,
    /// Whether it has had an event, and whether the greeting waits.
    started: bool,
    greet: bool,
    /// Cell width and row height, from the last draw.
    cell: (f32, f32),
    /// Rows scrolled back, and wheel movement short of a whole row.
    pub(crate) scroll: usize,
    wheel: f32,
}

/// The grid (columns, rows) that fits `w` x `h`: 1 x 1 to 1000 x 500.
pub(crate) fn grid_size(w: f32, h: f32, cell_w: f32, line_h: f32) -> (u16, u16) {
    // NaN becomes 1. Not clamp: its panic path links in float formatting.
    let fit = |l: f32, unit: f32, max| ((l - 2.0 * INSET) / unit).floor().max(1.0).min(max) as u16;
    (fit(w, cell_w, 1e3).max(1), fit(h, line_h, 500.0).max(1))
}

impl Default for Terminal {
    fn default() -> Terminal {
        let (term, guest, cell) = (Term::new(80, 24), Guest::new(), (8.0, 17.0));
        Terminal { term, guest, started: false, greet: false, cell, scroll: 0, wheel: 0.0 }
    }
}

impl Terminal {
    /// Shows `s`, `\n` as CR LF (ONLCR).
    fn print(&mut self, s: &[u8]) {
        for (i, line) in s.split(|&b| b == b'\n').enumerate() {
            self.term.feed(&b"\r\n"[..2 * usize::from(i > 0)]);
            self.term.feed(line);
        }
    }

    /// Shows what waited for the grid: the greeting, wrapped, and a prompt.
    fn begin(&mut self) {
        if std::mem::take(&mut self.greet) {
            let mut text = String::new();
            guest::wrap(BANNER, usize::from(self.term.cols()), &mut text);
            self.print(text.as_bytes());
            self.guest.render();
            self.shell_output();
        }
    }

    /// Shows what the guest shell printed.
    fn shell_output(&mut self) {
        let out = std::mem::take(&mut self.guest.out);
        self.print(out.as_bytes());
    }

    /// While a program runs: shows its output and, once it ended, the prompt.
    /// No live resizes or terminal replies: the kernel ignores both until step 3.
    fn io(&mut self, cx: &mut Cx<'_>) {
        let Some(pid) = self.guest.running() else { return };
        let (raw, out) = (cx.kernel.mode(pid).raw, cx.kernel.take_output(pid));
        self.output(&out, raw, cx.kernel.reap(pid));
    }

    /// Shows a program's output, ONLCR unless `raw`, moving the typed-ahead
    /// line below it; then the prompt if it ended with `status`.
    pub(crate) fn output(&mut self, out: &[u8], raw: bool, status: Option<i32>) {
        if !out.is_empty() || status.is_some() {
            self.guest.hide();
            self.shell_output();
            if raw { self.term.feed(out) } else { self.print(out) }
        }
        let col = self.term.cursor().1;
        match status {
            Some(status) => self.guest.finished(status, col),
            None if !out.is_empty() => self.guest.show(col),
            None => {}
        }
        self.shell_output();
    }

    /// Resizes the grid, and the guest shell's idea of it.
    fn fit(&mut self, (cols, rows): (u16, u16)) {
        if (cols, rows) != (self.term.cols(), self.term.rows()) {
            self.term.resize(cols, rows);
            (self.guest.cols, self.guest.rows) = (self.term.cols(), self.term.rows());
        }
    }

    /// After a key or text: shows the shell's output, snaps back; whether it was scrolled.
    fn typed(&mut self) -> bool {
        self.shell_output();
        std::mem::take(&mut self.scroll) > 0
    }

    /// The wheel, in whole rows, scrolls back (not on the alternate screen).
    fn wheel(&mut self, dy: f32) -> bool {
        let dy = if dy.is_finite() { dy } else { 0.0 };
        self.wheel += dy / self.cell.1.max(1.0);
        let rows = self.wheel.trunc();
        self.wheel -= rows;
        let (old, n) = (self.scroll, rows.abs().min(1e4) as usize);
        let max = if self.term.alt_screen() { 0 } else { self.term.scrollback_len() };
        let (up, down) = ((old + n).min(max), old.saturating_sub(n));
        self.scroll = if rows < 0.0 { up } else { down };
        self.scroll != old
    }

    /// Draws the visible rows (`scroll` back) inset in `r`, and the cursor:
    /// background runs, then glyphs and their lines.
    fn paint(&self, ts: &mut TextSystem, list: &mut DrawList, r: RectF, focused: bool, t: &Theme) {
        let ((cw, lh), asc) = (self.cell, ts.ascent(TextStyle::new(FontId::Mono, SIZE, t.text)));
        let (ox, oy) = (ts.snap(r.x + INSET), ts.snap(r.y + INSET));
        let shown = |c: &Cell| c.width != 0 && !c.attrs.contains(Attrs::HIDDEN);
        // Lines and the outline: whole device pixels, at least one.
        let one = ts.dpr().round().max(1.0) / ts.dpr();
        let term = &self.term;
        let (sb, cols) = (term.scrollback_len(), usize::from(term.cols()));
        let top = sb - self.scroll.min(sb);
        for vr in 0..usize::from(term.rows()) {
            let cells = match (top + vr).checked_sub(sb) {
                Some(r) => term.row(r as u16),
                None => term.scrollback_row(top + vr),
            };
            let cells = &cells[..cells.len().min(cols)];
            let (y, mut x) = (oy + vr as f32 * lh, 0);
            while x < cells.len() {
                let bg = colors(&cells[x], t).1;
                let n = cells[x..].iter().take_while(|c| colors(c, t).1 == bg).count();
                let r = RectF::new(ox + x as f32 * cw, y, n as f32 * cw, lh);
                if let Some(bg) = bg {
                    list.fill(r, 0.0, bg);
                }
                x += n;
            }
            for (x, c) in cells.iter().enumerate().filter(|c| shown(c.1)) {
                let (fg, left) = (colors(c, t).0, ox + x as f32 * cw);
                let (w, base) = (cw * f32::from(c.width), y + asc);
                ts.draw_cell_char(list, left, base, w, c.ch, SIZE, fg);
                let line = |dy| RectF::new(left, base + dy, w, one);
                if c.attrs.contains(Attrs::UNDERLINE) {
                    list.fill(line(one), 0.0, fg);
                }
                if c.attrs.contains(Attrs::STRIKE) {
                    list.fill(line(-(SIZE * 0.3).round()), 0.0, fg);
                }
            }
        }
        let (cr, cc) = term.cursor();
        if !term.cursor_visible() || self.scroll > 0 || cr >= term.rows() || cc >= term.cols() {
            return;
        }
        let cell = term.row(cr).get(usize::from(cc)).copied().unwrap_or(Cell::BLANK);
        let w = cw * f32::from(cell.width.max(1));
        let r = RectF::new(ox + f32::from(cc) * cw, oy + f32::from(cr) * lh, w, lh);
        if !focused {
            return list.border(r, 0.0, one, t.accent);
        }
        list.fill(r, 0.0, t.accent);
        if shown(&cell) {
            ts.draw_cell_char(list, r.x, r.y + asc, w, cell.ch, SIZE, t.accent_text);
        }
    }
}

impl App for Terminal {
    fn title(&self) -> String {
        match self.term.title() {
            "" => "Terminal".to_string(),
            t => ["Terminal — ", t].concat(),
        }
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let (r, focused, theme) = (ui.rect(), ui.state().focused, ui.theme());
        ui.hit(WidgetId(1), r, Sense::Text);
        // A Ui lends its draw list and its text system one at a time.
        let mut list = std::mem::take(ui.list());
        let ts = ui.text_system();
        let (cw, lh) = (ts.cell_width(SIZE), ts.snap((SIZE * 1.3).round()));
        self.cell = (cw, lh);
        self.fit(grid_size(r.w, r.h, cw, lh));
        self.begin();
        if self.term.alt_screen() {
            self.scroll = 0;
        }
        self.scroll = self.scroll.min(self.term.scrollback_len());
        self.paint(ts, &mut list, r, focused, theme);
        *ui.list() = list;
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        let before = self.term.generation();
        if !std::mem::replace(&mut self.started, true) {
            cx.load_fallback_fonts();
            self.greet = true;
        }
        if let AppEvent::Key { .. } | AppEvent::Text(_) = ev {
            self.begin();
        }
        let redraw = match ev {
            AppEvent::Key { key: Key::Other, .. } => false,
            // Enter came as a key already, should the page also report it.
            AppEvent::Text(s) if matches!(s.as_str(), "\n" | "\r" | "\r\n") => false,
            AppEvent::Key { key, mods } => {
                self.guest.key(key, mods, cx);
                self.typed()
            }
            AppEvent::Text(s) => {
                self.guest.text(&s, cx);
                self.typed()
            }
            AppEvent::Wheel { dy, .. } => self.wheel(dy),
            AppEvent::Focus(_) => true,
            AppEvent::Resized { w, h } => {
                self.fit(grid_size(w, h, self.cell.0, self.cell.1));
                true
            }
            AppEvent::Click(_) | AppEvent::PointerDown { .. } | AppEvent::Tick { .. } => false,
            AppEvent::Io => false,
        };
        self.io(cx);
        redraw || self.term.generation() != before
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((80.0 * 8.0 + 2.0 * INSET, 24.0 * 17.0 + 2.0 * INSET))
    }

    fn icon(&self) -> AppIcon {
        crate::kit::TERMINAL
    }
}

/// A cell's foreground and, unless default, background. Bold brightens the
/// first eight colors; inverse puts the opaque surface in front.
fn colors(c: &Cell, t: &Theme) -> (Rgba, Option<Rgba>) {
    let rgb = |col| match col {
        Color::Default => None,
        Color::Indexed(i) => Some(t.xterm(i)),
        Color::Rgb(r, g, b) => Some(Rgba(r, g, b, 255)),
    };
    let is = |a| c.attrs.contains(a);
    let fg = match c.fg {
        Color::Indexed(i @ 0..=7) if is(Attrs::BOLD) => t.xterm(i + 8),
        fg => rgb(fg).unwrap_or(t.text),
    };
    let (fg, bg) = match is(Attrs::INVERSE) {
        true => (rgb(c.bg).unwrap_or(t.surface.with_alpha(255)), Some(fg)),
        false => (fg, rgb(c.bg)),
    };
    (if is(Attrs::DIM) { fg.with_alpha(153) } else { fg }, bg)
}

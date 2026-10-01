//! The Terminal: an xterm-compatible screen ([`term::Term`]) drawn cell by
//! cell on the glyph atlas, running the built-in guest shell.

use gfx::{DrawList, Kind, RectF, Rgba};
use guest::Guest;
use term::{Attrs, Cell, Color, Term};
use ui::{App, AppEvent, AppIcon, Cx, FontId, Key, Mods, Sense, TextStyle, TextSystem, Theme};
use ui::{Ui, WidgetId};

/// The grid's font size and its inset from the content edge, in pixels.
const SIZE: f32 = 13.0;
const INSET: f32 = 14.0;
const BANNER: &str = "\x1b[1mcompusophyOS terminal\x1b[m — type 'help'.\n";

/// A terminal window running the guest shell ([`guest::Guest`]): keys and
/// text go to its line editor, and what it prints goes to the screen. The
/// grid is JetBrains Mono 13 px, [`TextSystem::cell_width`] by
/// `round(13 * 1.3)`, 14 px in from the content's edges, the same before the
/// deferred font arrives (the cells then draw colors and the cursor, no
/// glyphs). It draws in the frame's theme: the default text is
/// [`Theme::text`] over the window's own surface, the 16 colors are
/// [`Theme::ansi`], and the cursor is a steady [`Theme::accent`] block (an
/// outline while the window is not focused). From its first event the
/// greeting and the first prompt wait for the grid: the next draw, or a key
/// or text before it. The wheel scrolls back (any key snaps back).
#[derive(Debug)]
pub struct Terminal {
    pub(crate) term: Term,
    guest: Guest,
    /// Whether it has had an event yet, and whether the greeting waits.
    started: bool,
    greet: bool,
    /// Cell width and row height, from the last draw.
    cell: (f32, f32),
    /// Rows scrolled back, and wheel movement short of a whole row.
    pub(crate) scroll: usize,
    wheel: f32,
    /// Cells drawn with the text system, then copied into the window's list.
    scratch: DrawList,
}

/// The grid (columns, rows) that fits a `w` x `h` content rect with cells
/// `cell_w` x `line_h`: at least 1 x 1, at most the terminal's 1000 x 500.
pub(crate) fn grid_size(w: f32, h: f32, cell_w: f32, line_h: f32) -> (u16, u16) {
    // NaN becomes 1. Not clamp: its panic path links in float formatting.
    let fit = |l: f32, unit: f32, max| ((l - 2.0 * INSET) / unit).floor().max(1.0).min(max) as u16;
    (fit(w, cell_w, 1e3).max(1), fit(h, line_h, 500.0).max(1))
}

/// A terminal; it greets once it has had an event.
impl Default for Terminal {
    fn default() -> Terminal {
        Terminal {
            term: Term::new(80, 24),
            guest: Guest::new(),
            started: false,
            greet: false,
            cell: (8.0, 17.0),
            scroll: 0,
            wheel: 0.0,
            scratch: DrawList::new(),
        }
    }
}

impl Terminal {
    /// Shows `s`, `\n` as CR LF.
    fn print(&mut self, s: &str) {
        let mut rest = s.as_bytes();
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            self.term.feed(&rest[..i]);
            self.term.feed(b"\r\n");
            rest = &rest[i + 1..];
        }
        self.term.feed(rest);
    }

    /// Shows what waited for the grid: the greeting, broken between words
    /// to the screen's width, and the first prompt.
    fn begin(&mut self) {
        if std::mem::take(&mut self.greet) {
            let mut text = String::new();
            guest::wrap(BANNER, usize::from(self.term.cols()), &mut text);
            self.print(&text);
            self.guest.render();
            self.shell_output();
        }
    }

    /// Shows what the guest shell printed.
    fn shell_output(&mut self) {
        let out = std::mem::take(&mut self.guest.out);
        self.print(&out);
    }

    /// Resizes the grid, and the guest shell's idea of it.
    fn fit(&mut self, (cols, rows): (u16, u16)) {
        if (cols, rows) != (self.term.cols(), self.term.rows()) {
            self.term.resize(cols, rows);
            self.guest.cols = self.term.cols();
        }
    }

    /// Handles a key; returns whether the view snapped back (what changes
    /// on screen redraws by its generation).
    fn key(&mut self, key: Key, mods: Mods, cx: &mut Cx<'_>) -> bool {
        if key == Key::Other {
            return false;
        }
        let snapped = std::mem::take(&mut self.scroll) > 0;
        self.guest.key(key, mods, cx);
        self.shell_output();
        snapped
    }

    /// Handles text, as [`Terminal::key`] does a key.
    fn text(&mut self, s: &str, cx: &mut Cx<'_>) -> bool {
        // Enter came as a key already, should the page also report it.
        if matches!(s, "\n" | "\r" | "\r\n") {
            return false;
        }
        let snapped = std::mem::take(&mut self.scroll) > 0;
        self.guest.text(s, cx);
        self.shell_output();
        snapped
    }

    /// The wheel, in whole rows: it scrolls back, except on the alternate
    /// screen, which keeps none.
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
        let ts = ui.text_system();
        let (cw, lh) = (ts.cell_width(SIZE), ts.snap((SIZE * 1.3).round()));
        let (x, y) = (ts.snap(r.x + INSET), ts.snap(r.y + INSET));
        self.cell = (cw, lh);
        self.fit(grid_size(r.w, r.h, cw, lh));
        self.begin();
        if self.term.alt_screen() {
            self.scroll = 0;
        }
        self.scroll = self.scroll.min(self.term.scrollback_len());
        let mut list = std::mem::take(&mut self.scratch);
        list.clear();
        self.paint(ts, &mut list, (x, y), focused, theme);
        replay(&list, ui.list());
        self.scratch = list;
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
            AppEvent::Key { key, mods } => self.key(key, mods, cx),
            AppEvent::Text(s) => self.text(&s, cx),
            AppEvent::Wheel { dy, .. } => self.wheel(dy),
            AppEvent::Focus(_) => true,
            AppEvent::Resized { w, h } => {
                self.fit(grid_size(w, h, self.cell.0, self.cell.1));
                true
            }
            AppEvent::Click(_) | AppEvent::PointerDown { .. } | AppEvent::Tick { .. } => false,
        };
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

/// A cell's foreground in `t`, and its background unless it is the
/// default (the window's surface). Bold makes the first eight colors
/// bright; inverse puts the default background, made opaque, in front.
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
    let dim = is(Attrs::DIM);
    (if dim { fg.with_alpha(153) } else { fg }, bg)
}

impl Terminal {
    /// Draws the visible rows (`scroll` rows back into the scrollback) and
    /// the cursor in `t`: background runs, then glyphs and their lines.
    fn paint(
        &self,
        ts: &mut TextSystem,
        list: &mut DrawList,
        (ox, oy): (f32, f32),
        focused: bool,
        t: &Theme,
    ) {
        let ((cw, lh), asc) = (self.cell, ts.ascent(TextStyle::new(FontId::Mono, SIZE, t.text)));
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
            for (x, c) in cells.iter().enumerate() {
                if c.width == 0 || c.attrs.contains(Attrs::HIDDEN) {
                    continue;
                }
                let fg = colors(c, t).0;
                let left = ox + x as f32 * cw;
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
        let x = ox + f32::from(cc) * cw;
        let r = RectF::new(x, oy + f32::from(cr) * lh, w, lh);
        if !focused {
            return list.border(r, 0.0, one, t.accent);
        }
        list.fill(r, 0.0, t.accent);
        if cell.width != 0 && !cell.attrs.contains(Attrs::HIDDEN) {
            ts.draw_cell_char(list, r.x, r.y + asc, w, cell.ch, SIZE, t.accent_text);
        }
    }
}

/// Copies `src`'s fills, borders and glyphs into `dst`, under its clip: a
/// [`Ui`] lends its draw list and its text system only one at a time.
fn replay(src: &DrawList, dst: &mut DrawList) {
    const FILL: u8 = Kind::Fill as u8;
    const BORDER: u8 = Kind::Border as u8;
    const GLYPH: u8 = Kind::Glyph as u8;
    for i in src.instances() {
        let ([x, y, w, h], [u, v, uw, vh]) = (i.rect, i.uv);
        let r = RectF::new(x, y, w, h);
        match i.kind as u8 {
            FILL => dst.fill(r, i.radius, i.color),
            BORDER => dst.border(r, i.radius, i.p0, i.color),
            GLYPH => dst.glyph(r, RectF::new(u, v, uw, vh), i.color),
            _ => {}
        }
    }
}

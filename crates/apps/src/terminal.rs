//! The Terminal: a [`term::Term`] drawn cell by cell, the console of the shell it runs.

use gfx::{DrawList, RectF, Rgba};
use term::{Attrs, Cell, Color, KeyMods, Term};
use ui::kernel::{Spawn, wire};
use ui::{App, AppEvent, AppIcon, Cx, FontId, Key, Sense, THEMES, TextStyle, TextSystem, Theme};
use ui::{Ui, WidgetId};
use vfs::Vfs;

/// The grid's font size and its inset from the content edge, in pixels.
const SIZE: f32 = 13.0;
const INSET: f32 = 14.0;
/// The shell each terminal runs.
pub const SHELL: &str = "/bin/sh";

/// A terminal window: the console of [`SHELL`], started at the window's first size (in the
/// guest's home, the whole tree its roots), drawn as a Mono 13 px grid (the same before that
/// font arrives) in [`Theme::ansi`] colors with a steady accent block cursor (an outline
/// unfocused). Keys and pastes go to the console as xterm sends them; what its programs write
/// comes on [`AppEvent::Io`], the terminal's replies go back, and its asks (`OSC 1729`) are
/// done: `open` an app, switch to a `theme`. The wheel scrolls back, any key snaps back. The
/// shell's clean end closes the window; a failure, or a shell that cannot start, says why.
#[derive(Debug)]
pub struct Terminal {
    pub(crate) term: Term,
    /// The shell while it runs, and the console size it was last told.
    pub(crate) pid: Option<u32>,
    told: (u16, u16),
    /// Whether it has had an event, and a size (the shell starts at the first).
    started: bool,
    sized: bool,
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
        let (term, pid, told, cell) = (Term::new(80, 24), None, (80, 24), (8.0, 17.0));
        let (started, sized, scroll, wheel) = (false, false, 0, 0.0);
        Terminal { term, pid, told, started, sized, cell, scroll, wheel }
    }
}

impl Terminal {
    /// Starts the shell on a console the grid's size, or says why it cannot run.
    fn start(&mut self, cx: &mut Cx<'_>) {
        let size = (self.term.cols(), self.term.rows());
        let (argv, cwd, roots) = (vec!["sh".into()], Vfs::HOME.into(), vec!["/".into()]);
        let stdout = wire::Stdout::Console;
        let started = ui::kernel::program(cx.vfs, SHELL)
            .map_err(|missing| if missing { "not found" } else { "cannot execute" })
            .and_then(|program| {
                cx.kernel.spawn(Spawn { argv, program, cwd, tty: Some(size), stdout, roots })
            });
        match started {
            Ok(pid) => (self.pid, self.told) = (Some(pid), size),
            Err(why) => self.say(&["sh: ", why]),
        }
    }

    /// Shows a line of its own, on a row of its own.
    fn say(&mut self, parts: &[&str]) {
        let start = if self.term.cursor().1 > 0 { "\r\n" } else { "" };
        self.term.feed([&[start][..], parts, &["\r\n"]].concat().concat().as_bytes());
    }

    /// The bytes of a key or a paste go to the console; whether that snapped back a scroll.
    fn send(&mut self, bytes: &[u8], cx: &mut Cx<'_>) -> bool {
        if let Some(pid) = self.pid.filter(|_| !bytes.is_empty()) {
            cx.kernel.input(pid, bytes);
        }
        std::mem::take(&mut self.scroll) > 0
    }

    /// What the programs wrote: shown, the terminal's replies sent back, its asks done; once
    /// the shell ended, the window closes, or says the status it failed with.
    fn io(&mut self, cx: &mut Cx<'_>) {
        let Some(pid) = self.pid else { return };
        let out = cx.kernel.take_output(pid);
        self.term.feed(&out);
        let replies = self.term.take_replies();
        if !replies.is_empty() {
            cx.kernel.input(pid, &replies);
        }
        for (verb, arg) in self.term.take_asks() {
            match verb.as_str() {
                "open" => cx.open(&arg),
                "theme" if THEMES.iter().any(|t| t.name == arg) => cx.set_theme(&arg),
                _ => {}
            }
        }
        if (self.term.cols(), self.term.rows()) != self.told {
            self.told = (self.term.cols(), self.term.rows());
            cx.kernel.resize(pid, self.told.0, self.told.1);
        }
        match cx.kernel.reap(pid) {
            Some(0) => cx.close_self(),
            Some(status) => {
                let mut n = String::new();
                ui::push_num(&mut n, status as u32 as usize);
                self.say(&["[sh stopped with status ", &n, "]"]);
            }
            None => return,
        }
        self.pid = None;
    }

    /// Resizes the grid.
    fn fit(&mut self, (cols, rows): (u16, u16)) {
        self.term.resize(cols, rows);
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

/// The xterm key for a key the desktop reports: printable text comes as text, so a letter, a
/// digit or Space only with Ctrl.
fn xterm(key: Key, ctrl: bool) -> Option<term::Key> {
    use term::Key as K;
    Some(match key {
        Key::Enter => K::Enter,
        Key::Escape => K::Escape,
        Key::Backspace => K::Backspace,
        Key::Delete => K::Delete,
        Key::Tab => K::Tab,
        Key::Left => K::Left,
        Key::Right => K::Right,
        Key::Up => K::Up,
        Key::Down => K::Down,
        Key::Home => K::Home,
        Key::End => K::End,
        Key::PageUp => K::PageUp,
        Key::PageDown => K::PageDown,
        Key::Insert => K::Insert,
        Key::F(n) => K::F(n),
        Key::Char(c) if ctrl => K::Char(c),
        Key::Space if ctrl => K::Char(' '),
        Key::Char(_) | Key::Space | Key::Other => return None,
    })
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
        }
        let redraw = match ev {
            AppEvent::Key { key, mods } => match xterm(key, mods.ctrl) {
                Some(k) => {
                    let m = KeyMods { shift: mods.shift, ctrl: mods.ctrl, alt: mods.alt };
                    let bytes = term::encode_key(k, m, self.term.app_cursor_keys());
                    self.send(&bytes, cx)
                }
                None => false,
            },
            // Enter came as a key already, should the page also report it.
            AppEvent::Text(s) if matches!(s.as_str(), "\n" | "\r" | "\r\n") => false,
            AppEvent::Text(s) if s.chars().nth(1).is_none() => self.send(s.as_bytes(), cx),
            AppEvent::Text(s) => self.send(&term::paste(&s, self.term.bracketed_paste()), cx),
            AppEvent::Wheel { dy, .. } => self.wheel(dy),
            AppEvent::Focus(_) => true,
            AppEvent::Resized { w, h } => {
                self.fit(grid_size(w, h, self.cell.0, self.cell.1));
                if !std::mem::replace(&mut self.sized, true) {
                    self.start(cx);
                }
                true
            }
            AppEvent::Click(_) | AppEvent::PointerDown { .. } | AppEvent::Drag { .. } => false,
            AppEvent::Tick { .. } => false,
            AppEvent::Io | AppEvent::Ask(_) | AppEvent::Agent(_) => false,
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

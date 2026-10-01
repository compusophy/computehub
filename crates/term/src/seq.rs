//! What parsed sequences do: C0 controls, escape sequences, CSI and OSC.

use crate::{Attrs, Cell, Color, MAX_REPLIES, MAX_TITLE, Term};
use vt::{Params, Perform};

/// Routes the parser's events to the terminal.
pub(crate) struct Sink<'a>(pub(crate) &'a mut Term);

impl Perform for Sink<'_> {
    fn print(&mut self, c: char) {
        self.0.print(c);
    }

    fn execute(&mut self, byte: u8) {
        let t = &mut *self.0;
        t.last = None;
        match byte {
            0x08 => t.goto(t.cur.y, t.cur.x.saturating_sub(1)),
            0x09 => t.tab(1, false),
            0x0A..=0x0C => t.index(),
            0x0D => t.goto(t.cur.y, 0),
            0x0E => t.cur.gl = 1,
            0x0F => t.cur.gl = 0,
            _ => return,
        }
        t.dirty = true;
    }

    fn esc(&mut self, inter: &[u8], byte: u8) {
        let t = &mut *self.0;
        t.last = None;
        match (inter, byte) {
            ([], b'7') => t.save_cursor(),
            ([], b'8') => t.restore_cursor(),
            ([], b'D') => t.index(),
            ([], b'E') => {
                t.cur.x = 0;
                t.index();
            }
            ([], b'M') => t.reverse_index(),
            ([], b'c') => t.reset(),
            ([], b'H') => t.tabs[t.cur.x] = true,
            ([b'#'], b'8') => {
                for line in &mut t.screen {
                    line.fill(Cell::BLANK);
                    line.iter_mut().for_each(|c| c.ch = 'E');
                }
                (t.top, t.bottom) = (0, t.rows - 1);
                t.goto(0, 0);
            }
            ([g @ (b'(' | b')')], _) => t.cur.charset[usize::from(*g == b')')] = byte == b'0',
            _ => return,
        }
        t.dirty = true;
    }

    fn csi(&mut self, p: &Params, inter: &[u8], private: Option<u8>, action: u8) {
        let t = &mut *self.0;
        let last = t.last.take();
        let arg = |i: usize| usize::from(p.get(i).unwrap_or(0));
        let n = arg(0).max(1);
        let (y, x, cols) = (t.cur.y, t.cur.x, t.cols);
        let in_region = (t.top..=t.bottom).contains(&y);
        match (private, inter, action) {
            (None, [], b'A') => t.up(n),
            (None, [], b'B' | b'e') => t.down(n),
            (None, [], b'C' | b'a') => t.goto(y, x.saturating_add(n)),
            (None, [], b'D') => t.goto(y, x.saturating_sub(n)),
            (None, [], b'E') => {
                t.down(n);
                t.cur.x = 0;
            }
            (None, [], b'F') => {
                t.up(n);
                t.cur.x = 0;
            }
            (None, [], b'G' | b'`') => t.goto(y, n - 1),
            (None, [], b'H' | b'f') => t.goto_origin(n - 1, arg(1).max(1) - 1),
            (None, [], b'd') => t.goto_origin(n - 1, x),
            (None, [], b'I' | b'Z') => t.tab(n, action == b'Z'),
            (None, [], b'J') => {
                match arg(0) {
                    0 => (y..t.rows).for_each(|r| t.erase(r, if r == y { x } else { 0 }, cols)),
                    1 => (0..=y).for_each(|r| t.erase(r, 0, if r == y { x + 1 } else { cols })),
                    2 => (0..t.rows).for_each(|r| t.erase(r, 0, cols)),
                    3 => t.scrollback.clear(),
                    _ => {}
                }
                t.cur.wrap = false;
            }
            (None, [], b'K') => {
                match arg(0) {
                    0 => t.erase(y, x, cols),
                    1 => t.erase(y, 0, x + 1),
                    2 => t.erase(y, 0, cols),
                    _ => {}
                }
                t.cur.wrap = false;
            }
            (None, [], b'X') => {
                t.erase(y, x, x.saturating_add(n).min(cols));
                t.cur.wrap = false;
            }
            (None, [], b'P') => t.delete_cells(n),
            (None, [], b'@') => t.insert_cells(n),
            (None, [], b'L') if in_region => {
                t.scroll_down(y, t.bottom, n);
                t.goto(y, 0);
            }
            (None, [], b'M') if in_region => {
                t.scroll_up(y, t.bottom, n, false);
                t.goto(y, 0);
            }
            (None, [], b'S') => {
                let save = t.top == 0 && !t.alt;
                t.scroll_up(t.top, t.bottom, n, save);
            }
            (None, [], b'T') if p.len() <= 1 => t.scroll_down(t.top, t.bottom, n),
            // REP, at most to the end of the line (as VTE): ncurses asks no
            // more, and chained REPs would otherwise cost a screen each.
            (None, [], b'b') => {
                if let Some(c) = last {
                    for _ in 0..n.min(cols - x) {
                        t.print(c);
                    }
                }
            }
            (None, [], b'r') => {
                let top = arg(0).max(1) - 1;
                let bottom = if arg(1) == 0 { t.rows } else { arg(1).min(t.rows) } - 1;
                if top < bottom {
                    (t.top, t.bottom) = (top, bottom);
                    t.goto_origin(0, 0);
                }
            }
            (None, [], b's') => t.save_cursor(),
            (None, [], b'u') => t.restore_cursor(),
            (None, [], b'm') => t.sgr(p),
            (None, [], b'g') => match arg(0) {
                0 => t.tabs[x] = false,
                3 => t.tabs.fill(false),
                _ => {}
            },
            (None, [], b'h' | b'l') => {
                if p.iter().any(|g| g[0] == Some(4)) {
                    t.insert = action == b'h';
                }
            }
            (Some(b'?'), [], b'h' | b'l') => {
                for i in 0..p.len() {
                    t.dec_mode(p.get(i).unwrap_or(0), action == b'h');
                }
            }
            (None, [b'!'], b'p') => t.soft_reset(),
            (None, [], b'n') if arg(0) == 5 => return t.reply(b"\x1b[0n"),
            (None, [], b'n') if arg(0) == 6 => {
                let row = y.saturating_sub(if t.cur.origin { t.top } else { 0 });
                return t.reply(format!("\x1b[{};{}R", row + 1, x + 1).as_bytes());
            }
            (None, [], b'c') if arg(0) == 0 => return t.reply(b"\x1b[?62;22c"),
            (Some(b'>'), [], b'c') if arg(0) == 0 => return t.reply(b"\x1b[>0;10;1c"),
            (None | Some(b'?'), [b'$'], b'p') => {
                return t.report_mode(private.is_some(), p.get(0).unwrap_or(0));
            }
            _ => return,
        }
        t.dirty = true;
    }

    /// Only the title (OSC 0 and 2) is used; the icon name, links, color
    /// queries and the clipboard (52, for safety) are ignored.
    fn osc(&mut self, p: &[&[u8]]) {
        if p.len() < 2 || !(p[0] == b"0" || p[0] == b"2") {
            return;
        }
        let raw = p[1..].join(&b';');
        let text = String::from_utf8_lossy(&raw);
        let title: String = text.chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
        if title != self.0.title {
            self.0.title = title;
            self.0.dirty = true;
        }
    }
}

/// The [`Attrs`] bit SGR `n` (1 to 9: bold, dim, italic, underline, blink
/// twice, inverse, hidden, strike) sets and SGR `20 + n` clears.
const SGR_BITS: [u16; 10] = [0, 1, 2, 4, 8, 16, 16, 32, 64, 128];

/// The extended color (SGR 38, 48, 58) at parameter `i`, and how many more
/// parameters it took: none in the colon forms (`38:5:n`, `38:2::r:g:b`).
fn ext_color(p: &Params, i: usize) -> (Option<Color>, usize) {
    let sub = p.sub(i);
    let colon = sub.len() > 1;
    let at = |k: usize| {
        if colon { sub.get(k).copied().flatten() } else { p.get(i + k) }
    };
    let byte = |k: usize| u8::try_from(at(k).unwrap_or(0)).ok();
    let (color, used) = match at(1) {
        Some(5) => (at(2).and_then(|v| u8::try_from(v).ok()).map(Color::Indexed), 2),
        Some(2) => {
            let o = if colon && sub.len() >= 6 { 3 } else { 2 };
            match (byte(o), byte(o + 1), byte(o + 2)) {
                (Some(r), Some(g), Some(b)) => (Some(Color::Rgb(r, g, b)), 4),
                _ => (None, 4),
            }
        }
        _ => (None, 1),
    };
    (color, if colon { 0 } else { used })
}

impl Term {
    fn reply(&mut self, bytes: &[u8]) {
        if self.replies.len() + bytes.len() <= MAX_REPLIES {
            self.replies.extend_from_slice(bytes);
        }
    }

    /// DECRQM: reports a mode as set (1), reset (2) or unknown (0).
    fn report_mode(&mut self, private: bool, mode: u16) {
        let on = match (private, mode) {
            (false, 4) => Some(self.insert),
            (true, 1) => Some(self.app_cursor),
            (true, 6) => Some(self.cur.origin),
            (true, 7) => Some(self.autowrap),
            (true, 25) => Some(self.cursor_visible),
            (true, 47 | 1047 | 1049) => Some(self.alt),
            (true, 1000 | 1002 | 1003) => Some(self.mouse == mode),
            (true, 1004) => Some(self.focus),
            (true, 1006) => Some(self.mouse_sgr),
            (true, 2004) => Some(self.bracketed),
            (true, 2026) => Some(self.sync),
            _ => None,
        };
        let state = on.map_or(0, |on| if on { 1 } else { 2 });
        let q = if private { "?" } else { "" };
        self.reply(format!("\x1b[{q}{mode};{state}$y").as_bytes());
    }

    /// DECSET (`on`) or DECRST of one private mode.
    fn dec_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.app_cursor = on,
            6 => {
                self.cur.origin = on;
                self.goto_origin(0, 0);
            }
            7 => (self.autowrap, self.cur.wrap) = (on, false),
            25 => self.cursor_visible = on,
            47 => self.set_alt(on),
            1047 => {
                if !on && self.alt {
                    self.clear_screen();
                }
                self.set_alt(on);
            }
            1048 if on => self.save_cursor(),
            1048 => self.restore_cursor(),
            1049 if on && !self.alt => {
                self.save_cursor();
                self.set_alt(true);
                self.clear_screen();
            }
            1049 if on => self.clear_screen(),
            1049 if self.alt => {
                self.set_alt(false);
                self.restore_cursor();
            }
            1000 | 1002 | 1003 => self.mouse = if on { mode } else { 0 },
            1004 => self.focus = on,
            1006 => self.mouse_sgr = on,
            2004 => self.bracketed = on,
            2026 => self.sync = on,
            _ => {}
        }
    }

    fn clear_screen(&mut self) {
        let blank = self.blank();
        for line in &mut self.screen {
            line.fill(blank);
        }
    }

    /// SGR: sets colors and attributes. Unknown codes are skipped.
    fn sgr(&mut self, p: &Params) {
        if p.is_empty() {
            self.cur.pen = Cell::BLANK;
        }
        let mut i = 0;
        while i < p.len() {
            let pen = &mut self.cur.pen;
            match p.get(i).unwrap_or(0) {
                0 => *pen = Cell::BLANK,
                4 => match p.sub(i).get(1) {
                    Some(Some(0) | None) => pen.attrs.remove(Attrs::UNDERLINE),
                    _ => pen.attrs.insert(Attrs::UNDERLINE),
                },
                v @ 1..=9 => pen.attrs.insert(Attrs(SGR_BITS[usize::from(v)])),
                21 => pen.attrs.insert(Attrs::UNDERLINE),
                22 => pen.attrs.remove(Attrs::BOLD | Attrs::DIM),
                v @ (23..=25 | 27..=29) => pen.attrs.remove(Attrs(SGR_BITS[usize::from(v - 20)])),
                v @ 30..=37 => pen.fg = Color::Indexed((v - 30) as u8),
                39 => pen.fg = Color::Default,
                v @ 40..=47 => pen.bg = Color::Indexed((v - 40) as u8),
                49 => pen.bg = Color::Default,
                v @ 90..=97 => pen.fg = Color::Indexed((v - 82) as u8),
                v @ 100..=107 => pen.bg = Color::Indexed((v - 92) as u8),
                v @ (38 | 48 | 58) => {
                    let (color, used) = ext_color(p, i);
                    match (v, color) {
                        (38, Some(c)) => pen.fg = c,
                        (48, Some(c)) => pen.bg = c,
                        _ => {}
                    }
                    i += used;
                }
                _ => {}
            }
            i += 1;
        }
    }
}

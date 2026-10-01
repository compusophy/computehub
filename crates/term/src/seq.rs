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
        self.0.execute(byte);
    }
    fn csi(&mut self, params: &Params, inter: &[u8], private: Option<u8>, action: u8) {
        self.0.csi(params, inter, private, action);
    }
    fn esc(&mut self, inter: &[u8], byte: u8) {
        self.0.esc(inter, byte);
    }
    fn osc(&mut self, params: &[&[u8]]) {
        self.0.osc(params);
    }
}

/// An extended color (SGR 38, 48 or 58) at parameter `i`, and how many of
/// the following parameters it used: none in the colon forms (`38:5:n`,
/// `38:2::r:g:b`, `38:2:r:g:b`), where the color is a single parameter.
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
    fn execute(&mut self, byte: u8) {
        self.last = None;
        match byte {
            0x08 => self.goto(self.cur.y, self.cur.x.saturating_sub(1)),
            0x09 => self.tab(1),
            0x0A..=0x0C => self.index(),
            0x0D => self.goto(self.cur.y, 0),
            0x0E => self.cur.gl = 1,
            0x0F => self.cur.gl = 0,
            _ => return,
        }
        self.dirty = true;
    }

    fn esc(&mut self, inter: &[u8], byte: u8) {
        self.last = None;
        match (inter, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.index(),
            ([], b'E') => {
                self.cur.x = 0;
                self.index();
            }
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.reset(),
            ([], b'H') => self.tabs[self.cur.x] = true,
            ([b'#'], b'8') => {
                for line in &mut self.screen {
                    line.fill(Cell::BLANK);
                    line.iter_mut().for_each(|c| c.ch = 'E');
                }
                (self.top, self.bottom) = (0, self.rows - 1);
                self.goto(0, 0);
            }
            ([g @ (b'(' | b')')], _) => self.cur.charset[usize::from(*g == b')')] = byte == b'0',
            _ => return,
        }
        self.dirty = true;
    }

    fn csi(&mut self, p: &Params, inter: &[u8], private: Option<u8>, action: u8) {
        let last = self.last.take();
        let arg = |i: usize| usize::from(p.get(i).unwrap_or(0));
        let n = arg(0).max(1);
        let (y, x, cols) = (self.cur.y, self.cur.x, self.cols);
        let in_region = (self.top..=self.bottom).contains(&y);
        match (private, inter, action) {
            (None, [], b'A') => self.up(n),
            (None, [], b'B' | b'e') => self.down(n),
            (None, [], b'C' | b'a') => self.goto(y, x.saturating_add(n)),
            (None, [], b'D') => self.goto(y, x.saturating_sub(n)),
            (None, [], b'E') => {
                self.down(n);
                self.cur.x = 0;
            }
            (None, [], b'F') => {
                self.up(n);
                self.cur.x = 0;
            }
            (None, [], b'G' | b'`') => self.goto(y, n - 1),
            (None, [], b'H' | b'f') => self.goto_origin(n - 1, arg(1).max(1) - 1),
            (None, [], b'd') => self.goto_origin(n - 1, x),
            (None, [], b'I') => self.tab(n),
            (None, [], b'Z') => self.back_tab(n),
            (None, [], b'J') => {
                match arg(0) {
                    0 => {
                        (y..self.rows).for_each(|r| self.erase(r, if r == y { x } else { 0 }, cols))
                    }
                    1 => (0..=y).for_each(|r| self.erase(r, 0, if r == y { x + 1 } else { cols })),
                    2 => (0..self.rows).for_each(|r| self.erase(r, 0, cols)),
                    3 => self.scrollback.clear(),
                    _ => {}
                }
                self.cur.wrap = false;
            }
            (None, [], b'K') => {
                match arg(0) {
                    0 => self.erase(y, x, cols),
                    1 => self.erase(y, 0, x + 1),
                    2 => self.erase(y, 0, cols),
                    _ => {}
                }
                self.cur.wrap = false;
            }
            (None, [], b'X') => {
                self.erase(y, x, x.saturating_add(n).min(cols));
                self.cur.wrap = false;
            }
            (None, [], b'P') => self.delete_cells(n),
            (None, [], b'@') => self.insert_cells(n),
            (None, [], b'L') if in_region => {
                self.scroll_down(y, self.bottom, n);
                self.goto(y, 0);
            }
            (None, [], b'M') if in_region => {
                self.scroll_up(y, self.bottom, n, false);
                self.goto(y, 0);
            }
            (None, [], b'S') => {
                let save = self.top == 0 && !self.alt;
                self.scroll_up(self.top, self.bottom, n, save);
            }
            (None, [], b'T') if p.len() <= 1 => self.scroll_down(self.top, self.bottom, n),
            // REP, at most to the end of the line (as VTE): ncurses asks no
            // more, and chained REPs would otherwise cost a screen each.
            (None, [], b'b') => {
                if let Some(c) = last {
                    for _ in 0..n.min(cols - x) {
                        self.print(c);
                    }
                }
            }
            (None, [], b'r') => {
                let top = arg(0).max(1) - 1;
                let bottom = if arg(1) == 0 { self.rows } else { arg(1).min(self.rows) } - 1;
                if top < bottom {
                    (self.top, self.bottom) = (top, bottom);
                    self.goto_origin(0, 0);
                }
            }
            (None, [], b's') => self.save_cursor(),
            (None, [], b'u') => self.restore_cursor(),
            (None, [], b'm') => self.sgr(p),
            (None, [], b'g') => match arg(0) {
                0 => self.tabs[x] = false,
                3 => self.tabs.fill(false),
                _ => {}
            },
            (None, [], b'h' | b'l') => {
                if p.iter().any(|g| g[0] == Some(4)) {
                    self.insert = action == b'h';
                }
            }
            (Some(b'?'), [], b'h' | b'l') => {
                for i in 0..p.len() {
                    self.dec_mode(p.get(i).unwrap_or(0), action == b'h');
                }
            }
            (None, [b'!'], b'p') => self.soft_reset(),
            (None, [], b'n') if arg(0) == 5 => return self.reply(b"\x1b[0n"),
            (None, [], b'n') if arg(0) == 6 => {
                let row = y.saturating_sub(if self.cur.origin { self.top } else { 0 });
                return self.reply(format!("\x1b[{};{}R", row + 1, x + 1).as_bytes());
            }
            (None, [], b'c') if arg(0) == 0 => return self.reply(b"\x1b[?62;22c"),
            (Some(b'>'), [], b'c') if arg(0) == 0 => return self.reply(b"\x1b[>0;10;1c"),
            (None | Some(b'?'), [b'$'], b'p') => {
                return self.report_mode(private.is_some(), p.get(0).unwrap_or(0));
            }
            _ => return,
        }
        self.dirty = true;
    }

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
            1049 if on => {
                if !self.alt {
                    self.save_cursor();
                    self.set_alt(true);
                }
                self.clear_screen();
            }
            1049 => {
                if self.alt {
                    self.set_alt(false);
                    self.restore_cursor();
                }
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
                1 => pen.attrs.insert(Attrs::BOLD),
                2 => pen.attrs.insert(Attrs::DIM),
                3 => pen.attrs.insert(Attrs::ITALIC),
                4 => match p.sub(i).get(1) {
                    Some(Some(0) | None) => pen.attrs.remove(Attrs::UNDERLINE),
                    _ => pen.attrs.insert(Attrs::UNDERLINE),
                },
                5 | 6 => pen.attrs.insert(Attrs::BLINK),
                7 => pen.attrs.insert(Attrs::INVERSE),
                8 => pen.attrs.insert(Attrs::HIDDEN),
                9 => pen.attrs.insert(Attrs::STRIKE),
                21 => pen.attrs.insert(Attrs::UNDERLINE),
                22 => pen.attrs.remove(Attrs::BOLD | Attrs::DIM),
                23 => pen.attrs.remove(Attrs::ITALIC),
                24 => pen.attrs.remove(Attrs::UNDERLINE),
                25 => pen.attrs.remove(Attrs::BLINK),
                27 => pen.attrs.remove(Attrs::INVERSE),
                28 => pen.attrs.remove(Attrs::HIDDEN),
                29 => pen.attrs.remove(Attrs::STRIKE),
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

    /// OSC: only the title (0 and 2) is used. 1 (icon name), 8 (links), 10
    /// to 12 (color queries) and 52 (clipboard, for safety) are ignored.
    fn osc(&mut self, p: &[&[u8]]) {
        if p.len() < 2 || !(p[0] == b"0" || p[0] == b"2") {
            return;
        }
        let raw = p[1..].join(&b';');
        let text = String::from_utf8_lossy(&raw);
        let title: String = text.chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
        if title != self.title {
            self.title = title;
            self.dirty = true;
        }
    }
}

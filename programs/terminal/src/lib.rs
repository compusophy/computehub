//! The Terminal: an xterm screen ([`term::Term`]) of the shell the desktop runs on a console for
//! its window ([`Request::Tty`]), a wasm32-wasip1 GUI program on the [`uiwire`] protocol, drawn
//! as a [`Node::Screen`] and fetched when a terminal first opens, never with the boot download.
//! Keys and pastes go to the console as xterm sends them ([`Request::Input`]); what the shell's
//! programs write comes as [`Event::Output`], the terminal's replies go back, and its asks
//! (`OSC 1729`) are done: `open` an app, switch to a `theme`. The wheel scrolls back, any key
//! snaps back. The shell's clean end closes the window; a failure, or a shell that could not
//! start, says why. [`serve`] runs it until its window closes.

#![forbid(unsafe_code)]

use std::io::{self, ErrorKind, Read, Write};
use std::mem;

use term::{Cell, Color, KeyMods, Term};
use uiwire::client::Client;
use uiwire::{CELL, Event, Frame, Key, Node, Request, mods};

/// The themes an ask may switch to.
const THEMES: [&str; 3] = ["Midnight", "Dawn", "Mono"];

/// A terminal window: the screen, the console size last asked for, rows scrolled back and wheel
/// movement short of a whole row, whether the shell ended, and the requests since the last
/// frame.
#[derive(Debug)]
pub struct Terminal {
    pub term: Term,
    told: Option<(u16, u16)>,
    pub scroll: usize,
    wheel: i32,
    ended: bool,
    requests: Vec<Request>,
}

impl Default for Terminal {
    fn default() -> Terminal {
        let (term, told, scroll, wheel) = (Term::new(80, 24), None, 0, 0);
        Terminal { term, told, scroll, wheel, ended: false, requests: Vec::new() }
    }
}

impl Terminal {
    /// Handles one event; whether the window changed. The first size starts the shell on a
    /// console that size, and a new one resizes it.
    pub fn event(&mut self, ev: &Event) -> bool {
        let (before, asked) = (self.term.generation(), self.requests.len());
        let changed = match ev {
            Event::Resize { w, h } => {
                let (cols, rows) = uiwire::screen(*w, *h);
                self.term.resize(cols, rows);
                if self.told.replace((cols, rows)) != Some((cols, rows)) && !self.ended {
                    self.requests.push(Request::Tty { cols, rows });
                }
                true
            }
            Event::Output { data } => {
                self.feed(data);
                false
            }
            Event::Key { key, mods: m, ch, .. } => match xterm(*key, *ch, m & mods::CTRL != 0) {
                Some(k) => {
                    let (shift, ctrl) = (m & mods::SHIFT != 0, m & mods::CTRL != 0);
                    let km = KeyMods { shift, ctrl, alt: m & mods::ALT != 0 };
                    self.send(term::encode_key(k, km, self.term.app_cursor_keys()))
                }
                None => false,
            },
            // Enter came as a key already, should the page also report it.
            Event::Text { text } if matches!(text.as_str(), "\n" | "\r" | "\r\n") => false,
            Event::Text { text } if text.chars().nth(1).is_none() => {
                self.send(text.as_bytes().to_vec())
            }
            Event::Text { text } => self.send(term::paste(text, self.term.bracketed_paste())),
            Event::Wheel { dy } => self.wheel(*dy),
            Event::Ended { status } => {
                self.ended = true;
                match *status {
                    0 => self.requests.push(Request::Close),
                    s => self.say(&["[sh stopped with status ", &s.to_string(), "]"]),
                }
                true
            }
            Event::Focus { .. } => true,
            _ => false,
        };
        changed || self.term.generation() != before || self.requests.len() != asked
    }

    /// What the programs wrote: shown, the terminal's replies sent back, its asks done.
    fn feed(&mut self, data: &[u8]) {
        self.term.feed(data);
        let replies = self.term.take_replies();
        if !replies.is_empty() {
            self.requests.push(Request::Input { data: replies });
        }
        for (verb, arg) in self.term.take_asks() {
            match verb.as_str() {
                "open" => self.requests.push(Request::Open { name: arg }),
                "theme" if THEMES.contains(&arg.as_str()) => {
                    self.requests.push(Request::Pref { key: "theme".into(), value: arg })
                }
                _ => {}
            }
        }
    }

    /// Shows a line of its own, on a row of its own.
    fn say(&mut self, parts: &[&str]) {
        let start = if self.term.cursor().1 > 0 { "\r\n" } else { "" };
        self.term.feed([&[start][..], parts, &["\r\n"]].concat().concat().as_bytes());
    }

    /// The bytes of a key or a paste go to the console; whether that snapped back a scroll.
    fn send(&mut self, data: Vec<u8>) -> bool {
        if !data.is_empty() && !self.ended {
            self.requests.push(Request::Input { data });
        }
        mem::take(&mut self.scroll) > 0
    }

    /// The wheel, in whole rows, scrolls back (not on the alternate screen).
    fn wheel(&mut self, dy: i32) -> bool {
        let row = i32::from(CELL.1);
        self.wheel = self.wheel.saturating_add(dy);
        let rows = self.wheel / row;
        self.wheel -= rows * row;
        let (old, n) = (self.scroll, rows.unsigned_abs().min(10_000) as usize);
        let max = if self.term.alt_screen() { 0 } else { self.term.scrollback_len() };
        self.scroll = if rows < 0 { (old + n).min(max) } else { old.saturating_sub(n) };
        self.scroll != old
    }

    /// The window now: its title (the screen's, if it set one), the requests since, and the
    /// screen's visible rows (`scroll` back), the cursor if it shows there.
    pub fn frame(&mut self) -> Frame {
        let term = &self.term;
        let (cols, rows, sb) = (term.cols(), term.rows(), term.scrollback_len());
        self.scroll = if term.alt_screen() { 0 } else { self.scroll.min(sb) };
        let top = sb - self.scroll;
        let mut cells = Vec::with_capacity(usize::from(cols) * usize::from(rows) * 13);
        for vr in 0..usize::from(rows) {
            let row = match (top + vr).checked_sub(sb) {
                Some(r) => term.row(r as u16),
                None => term.scrollback_row(top + vr),
            };
            for c in 0..usize::from(cols) {
                put(&mut cells, row.get(c).unwrap_or(&Cell::BLANK));
            }
        }
        let (cr, cc) = term.cursor();
        let shown = term.cursor_visible() && self.scroll == 0 && cr < rows && cc < cols;
        let cursor = shown.then_some((cr, cc));
        let title = match term.title() {
            "" => "Terminal".to_string(),
            t => ["Terminal \u{2014} ", t].concat(),
        };
        let nodes = vec![Node::Screen { id: 1, cols, rows, cursor, cells }];
        Frame { seq: 0, title, requests: mem::take(&mut self.requests), nodes }
    }
}

/// A cell as a [`Node::Screen`] holds it.
fn put(out: &mut Vec<u8>, c: &Cell) {
    let color = |col| match col {
        Color::Default => 0,
        Color::Indexed(i) => 1 << 24 | u32::from(i),
        Color::Rgb(r, g, b) => 2 << 24 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
    };
    out.extend(((c.ch as u32) | u32::from(c.width) << 24).to_le_bytes());
    out.extend(color(c.fg).to_le_bytes());
    out.extend(color(c.bg).to_le_bytes());
    out.push(c.attrs.bits() as u8);
}

/// The xterm key for a key the desktop reports: printable text comes as text, so a character
/// key only with Ctrl.
fn xterm(key: Key, ch: char, ctrl: bool) -> Option<term::Key> {
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
        Key::F => K::F(ch as u8),
        Key::Char if ctrl => K::Char(ch),
        Key::Char => return None,
    })
}

/// Runs `terminal` on `ui`, a frame per event that changes it, until Close or the end of the
/// events.
pub fn serve<R: Read, W: Write>(ui: &mut Client<R, W>, terminal: &mut Terminal) -> io::Result<()> {
    let mut seq = 0u32;
    loop {
        let changed = match ui.next_event() {
            Ok(Event::Close) => return Ok(()),
            Ok(ev) => terminal.event(&ev),
            Err(e) if e.kind() == ErrorKind::InvalidData => false,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if changed {
            ui.show(&Frame { seq, ..terminal.frame() })?;
            seq = seq.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests;

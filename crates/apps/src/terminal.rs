//! The Terminal: an xterm-compatible screen ([`term::Term`]) drawn cell by
//! cell on the glyph atlas, running a real shell from computehub-node or the
//! built-in guest shell.

use gfx::{DrawList, Kind, RectF, Rgba};
use guest::{Guest, NO_PAIRING, push_int};
use term::{Attrs, Cell, Color, Term};
use ui::{App, AppEvent, Cx, FontId, Key, Mods, Pairing, Sense, SocketId, TextStyle, TextSystem};
use ui::{Ui, WidgetId, WsEvent, theme};

/// The grid's font size and its inset from the content edge, in pixels.
const SIZE: f32 = 13.0;
const INSET: f32 = 6.0;
/// The default foreground and background, and the cursor.
const FG: Rgba = theme::TEXT;
const BG: Rgba = theme::WINDOW;
const CURSOR: Rgba = theme::ACCENT;
/// A synchronized update (DEC 2026, begun by BSU) holds the last frame for
/// up to 250 ms, or until a second key or text (should the program die
/// inside it).
const SYNC_MS: f64 = 250.0;
const BSU: &[u8] = b"\x1b[?2026h";
/// Most input bytes in one DATA message (the node takes up to 1 MiB).
const CHUNK: usize = 60_000;
/// Undoes what a program may have left on: the alternate screen, modes,
/// reports, bracketed paste, synchronized output and its title.
const TIDY: &str = "\x1b[?1049l\x1b[!p\x1b[?1000;1002;1003;1004;1006;2004;2026l\x1b]2;\x07";
const BANNER: &str = "\x1b[1mcompusophyOS terminal\x1b[m — built-in guest shell. Type 'help'. \
For a real shell on your machine, type 'node'.\n";
const AGAIN: &str = "Press Enter to reconnect, or Ctrl+D for the guest shell.";
/// Around the port: the node could not be reached.
const UNREACHABLE: [&str; 2] = [
    "[could not reach computehub-node on 127.0.0.1:",
    ". Is it running? Ctrl+D, then 'node', shows how to start it. If it is, \
     the browser may be blocking this site from reaching your device: allow \
     local network access for this site (the icon left of the address bar), \
     then press Enter.]",
];
const REFUSED: &str = "[the node refused this page's token: open the pairing link it printed \
when it last started]";

// The computehub-node protocol (`node/README.md`): one binary WebSocket
// message each, the first byte the opcode, integers little-endian.
const VERSION: u8 = 1;
const HELLO: u8 = 1;
const READY: u8 = 2;
const DATA: u8 = 3;
const RESIZE: u8 = 4;
const EXIT: u8 = 5;
const ERROR: u8 = 6;

/// DATA: bytes for the shell's input (or, from the node, its output).
pub(crate) fn data(bytes: &[u8]) -> Vec<u8> {
    [&[DATA][..], bytes].concat()
}

/// RESIZE: the terminal's grid.
pub(crate) fn resize(cols: u16, rows: u16) -> Vec<u8> {
    [&[RESIZE][..], &cols.to_le_bytes(), &rows.to_le_bytes()].concat()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// The built-in shell has the screen.
    Guest,
    /// Waiting for the socket to open, then (HELLO sent) for READY.
    Connecting(SocketId),
    Hello(SocketId),
    /// A node shell has the screen.
    Live(SocketId),
    /// A session ended: Enter reconnects, Ctrl+D returns to the guest shell.
    Ended,
}

/// A terminal window. With [`Cx::pairing`] set (the page was opened with
/// the node's pairing link) it connects at its first event, as the guest
/// shell's `connect` does later: HELLO with the token on open, RESIZE after
/// READY and whenever the grid changes, DATA both ways; EXIT or a close
/// ends the session. It keeps its first node (the title names its port):
/// reconnecting goes there, whatever pairs later. Keys go to the program as
/// xterm sends them; text goes as UTF-8, several chars as a paste. The grid
/// is JetBrains Mono 13 px, [`TextSystem::cell_width`] by `round(13 * 1.3)`,
/// the same before the deferred font arrives (the cells then draw colors
/// and the cursor, no glyphs); its first text waits for the first draw.
/// The wheel scrolls back (any key snaps back), or reaches a program on the
/// alternate screen or with mouse reporting on.
#[derive(Debug)]
pub struct Terminal {
    pub(crate) term: Term,
    guest: Guest,
    mode: Mode,
    started: bool,
    /// The greeting (or connecting line) waits; a draw measured the grid.
    greet: bool,
    drawn: bool,
    /// The pairing of the current or last connection.
    link: Option<Pairing>,
    /// Cell width and row height, from the last draw.
    cell: (f32, f32),
    /// The grid changed; tell the node at the end of the event.
    resized: bool,
    /// Rows scrolled back, and wheel movement short of a whole row.
    pub(crate) scroll: usize,
    wheel: f32,
    /// The frame a synchronized update holds.
    hold: Hold,
    /// Cells drawn with the text system, then copied into the window's list.
    scratch: DrawList,
}

/// A synchronized update: when it began while it holds (`spent` once it
/// gave way), the keys and text sent since, and the screen then (rows,
/// cursor, its visibility, generation).
#[derive(Debug, Default)]
struct Hold {
    since: Option<f64>,
    spent: bool,
    inputs: u8,
    rows: Vec<Vec<Cell>>,
    cursor: (u16, u16),
    visible: bool,
    generation: u64,
}

/// The grid (columns, rows) that fits a `w` x `h` content rect with cells
/// `cell_w` x `line_h`: at least 1 x 1, at most the terminal's 1000 x 500.
pub(crate) fn grid_size(w: f32, h: f32, cell_w: f32, line_h: f32) -> (u16, u16) {
    // NaN becomes 1. Not clamp: its panic path links in float formatting.
    let fit = |l: f32, unit: f32, max| ((l - 2.0 * INSET) / unit).floor().max(1.0).min(max) as u16;
    (fit(w, cell_w, 1e3).max(1), fit(h, line_h, 500.0).max(1))
}

/// The bytes a key sends to the program, as xterm sends them, or `None`
/// when it sends nothing: printable keys (their text follows as
/// [`AppEvent::Text`]), Ctrl+V (the browser pastes), Meta combos, and keys
/// it cannot name. Ctrl+2 to Ctrl+8 send the controls of `@ [ \ ] ^ _ ?`;
/// Alt puts ESC before a letter or an unshifted digit (US QWERTY, by code).
pub(crate) fn encode(key: Key, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    use term::Key as T;
    // The keys both enums name alike.
    macro_rules! same {
        ($($k:ident)*) => { match key { $(Key::$k => T::$k,)* _ => return None } };
    }
    let meta = mods.alt && !mods.ctrl;
    let k = match key {
        _ if mods.meta => return None,
        Key::F(n) => T::F(n),
        Key::Space if mods.ctrl => T::Char(' '),
        Key::Char(c) if mods.ctrl && c != 'v' && c.is_ascii_lowercase() => T::Char(c),
        Key::Char(c @ '2'..='8') if mods.ctrl => T::Char(b"@[\\]^_?"[c as usize - 50].into()),
        Key::Char(c) if meta && !mods.shift => T::Char(c),
        Key::Char(c) if meta && c.is_ascii_lowercase() => T::Char(c.to_ascii_uppercase()),
        _ => {
            same!(Enter Escape Backspace Delete Tab Left Right Up Down Home End PageUp PageDown Insert)
        }
    };
    let (shift, ctrl, alt) = (mods.shift, mods.ctrl, mods.alt);
    let mods = term::KeyMods { shift, ctrl, alt };
    Some(term::encode_key(k, mods, app_cursor))
}

/// A terminal; it greets or connects at its first event.
impl Default for Terminal {
    fn default() -> Terminal {
        Terminal {
            term: Term::new(80, 24),
            guest: Guest::new(),
            mode: Mode::Guest,
            started: false,
            greet: false,
            drawn: false,
            link: None,
            cell: (8.0, 17.0),
            resized: false,
            scroll: 0,
            wheel: 0.0,
            hold: Hold::default(),
            scratch: DrawList::new(),
        }
    }
}

/// `n` in decimal.
fn num(n: i64) -> String {
    let mut s = String::from(if n < 0 { "-" } else { "" });
    push_int(&mut s, n.unsigned_abs(), 0, '0');
    s
}

/// A wheel report (button 64 up, 65 down) at the zero-based cell (`row`,
/// `col`): SGR, else X10 (nothing past row or column 223).
fn wheel_report(b: u8, (row, col): (u16, u16), sgr: bool) -> Vec<u8> {
    let (x, y) = (i64::from(col) + 1, i64::from(row) + 1);
    let text = ["\x1b[<", &num(b.into()), ";", &num(x), ";", &num(y), "M"];
    match (sgr, u8::try_from(x + 32), u8::try_from(y + 32)) {
        (true, ..) => text.concat().into(),
        (_, Ok(x), Ok(y)) => vec![0x1B, b'[', b'M', 32 + b, x, y],
        _ => Vec::new(),
    }
}

/// `b` as UTF-8 (lossy), without control chars.
fn clean(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    s.chars().filter(|c| !c.is_control()).collect()
}

impl Terminal {
    /// Shows `parts`, `\n` as CR LF, on a fresh line when `fresh`.
    fn print(&mut self, fresh: bool, parts: &[&str]) {
        if fresh && self.term.cursor().1 > 0 {
            self.term.feed(b"\r\n");
        }
        for part in parts {
            let mut rest = part.as_bytes();
            while let Some(i) = rest.iter().position(|&b| b == b'\n') {
                self.term.feed(&rest[..i]);
                self.term.feed(b"\r\n");
                rest = &rest[i + 1..];
            }
            self.term.feed(rest);
        }
    }

    /// Prints prose: like [`Terminal::print`], but broken between words to
    /// the screen's width, never inside one.
    fn say(&mut self, fresh: bool, parts: &[&str]) {
        let mut text = String::new();
        guest::wrap(&parts.concat(), usize::from(self.term.cols()), &mut text);
        self.print(fresh, &[&text]);
    }

    /// Connects to this terminal's node, or the page's the first time.
    fn connect(&mut self, cx: &mut Cx<'_>) {
        let Some(p) = self.link.or(cx.pairing) else {
            self.say(true, &[NO_PAIRING, "\n"]);
            return self.guest_shell();
        };
        self.link = Some(p);
        self.mode = Mode::Connecting(cx.connect(p.port));
        self.greet = true;
        if self.drawn {
            self.begin();
        }
    }

    /// Shows what waited for the grid: the greeting and the first prompt,
    /// or where it connects.
    fn begin(&mut self) {
        match (std::mem::take(&mut self.greet), self.mode, self.link) {
            (true, Mode::Guest, _) => {
                self.say(false, &[BANNER]);
                self.guest_shell();
            }
            (true, _, Some(p)) => {
                let at = "\x1b[2mconnecting to computehub-node on 127.0.0.1:";
                self.say(true, &[at, &num(p.port.into()), "…\x1b[m\n"]);
            }
            _ => {}
        }
    }

    /// Returns to the guest shell with a fresh prompt.
    fn guest_shell(&mut self) {
        self.mode = Mode::Guest;
        self.print(true, &[]);
        self.guest.render();
        let out = std::mem::take(&mut self.guest.out);
        self.print(false, &[&out]);
    }

    /// Shows what the guest shell printed, and connects if it asked.
    fn guest_done(&mut self, cx: &mut Cx<'_>) {
        let out = std::mem::take(&mut self.guest.out);
        self.print(false, &[&out]);
        if std::mem::take(&mut self.guest.connect) {
            self.connect(cx);
        }
    }

    /// Ends the session with `what`, in parts; Enter or Ctrl+D comes next.
    fn end(&mut self, what: &[&str]) {
        self.mode = Mode::Ended;
        self.print(false, &[TIDY]);
        self.print(true, &["\x1b[2m"]);
        self.say(false, what);
        self.say(false, &["\x1b[m\n", AGAIN, "\n"]);
    }

    /// Sends `bytes` to the shell as DATA, in chunks.
    fn send(&self, s: SocketId, bytes: &[u8], cx: &mut Cx<'_>) {
        bytes.chunks(CHUNK).for_each(|b| cx.send(s, data(b)));
    }

    /// Feeds the program's output. An update that begins here holds the
    /// screen as it was just before it, so output is fed up to each BSU.
    fn output(&mut self, mut p: &[u8], now: f64) {
        while let Some(i) = p.windows(BSU.len()).position(|w| w == BSU) {
            self.term.feed(&p[..i]);
            let g = self.term.generation();
            self.track(now, g);
            self.term.feed(BSU);
            self.track(now, g);
            p = &p[i + BSU.len()..];
        }
        self.term.feed(p);
        self.track(now, self.term.generation());
    }

    /// Follows the program's synchronized updates: a hold ends with its
    /// update, and one begins (the screen being at `generation`) with each
    /// update that has not given way.
    fn track(&mut self, now: f64, generation: u64) {
        let (t, h) = (&self.term, &mut self.hold);
        if !t.synchronized() {
            (h.since, h.spent) = (None, false);
        } else if h.since.is_none() && !h.spent {
            h.rows = (0..t.rows()).map(|r| t.row(r).to_vec()).collect();
            (h.cursor, h.visible, h.generation) = (t.cursor(), t.cursor_visible(), generation);
            (h.since, h.inputs) = (Some(now), 0);
        }
    }

    /// Gives up the hold (until its update ends) once it is [`SYNC_MS`] old
    /// at `now`, or at the second key or text sent (`typed`) during it.
    fn wane(&mut self, now: f64, typed: bool) {
        let h = &mut self.hold;
        let young = h.since.is_some_and(|t| now - t < SYNC_MS);
        h.inputs += u8::from(typed && h.since.is_some());
        if h.since.is_some() && (!young || h.inputs >= 2) {
            (h.since, h.spent) = (None, true);
        }
    }

    /// The generation on screen: the held frame's, or the terminal's.
    fn shown(&self) -> u64 {
        let h = &self.hold;
        h.since.map_or(self.term.generation(), |_| h.generation)
    }

    /// Resizes the grid, to be told to the node.
    fn fit(&mut self, (cols, rows): (u16, u16)) {
        if (cols, rows) != (self.term.cols(), self.term.rows()) {
            self.term.resize(cols, rows);
            (self.guest.cols, self.resized) = (self.term.cols(), true);
        }
    }

    /// Handles a key; returns whether the view snapped back (what changes
    /// on screen redraws by its generation).
    fn key(&mut self, key: Key, mods: Mods, cx: &mut Cx<'_>) -> bool {
        if key == Key::Other {
            return false;
        }
        let snapped = std::mem::take(&mut self.scroll) > 0;
        let ctrl = |c| key == Key::Char(c) && mods.ctrl;
        match self.mode {
            Mode::Guest => {
                self.guest.key(key, mods, cx);
                self.guest_done(cx);
            }
            Mode::Connecting(s) | Mode::Hello(s) if ctrl('c') => {
                cx.close_socket(s);
                self.print(false, &["^C\n"]);
                self.guest_shell();
            }
            Mode::Live(s) => {
                if let Some(bytes) = encode(key, mods, self.term.app_cursor_keys()) {
                    self.send(s, &bytes, cx);
                    self.wane(cx.now_ms, true);
                }
            }
            Mode::Ended if key == Key::Enter => self.connect(cx),
            Mode::Ended if ctrl('d') => self.guest_shell(),
            _ => {}
        }
        snapped
    }

    /// Handles text, as [`Terminal::key`] does a key.
    fn text(&mut self, s: &str, cx: &mut Cx<'_>) -> bool {
        // Enter came as a key already, should the page also report it.
        if matches!(s, "\n" | "\r" | "\r\n") {
            return false;
        }
        let snapped = std::mem::take(&mut self.scroll) > 0;
        let paste = || term::paste(s, self.term.bracketed_paste());
        match self.mode {
            Mode::Guest => {
                self.guest.text(s, cx);
                self.guest_done(cx);
            }
            Mode::Live(sock) if s.chars().count() == 1 => self.send(sock, s.as_bytes(), cx),
            Mode::Live(sock) => self.send(sock, &paste(), cx),
            _ => {}
        }
        if let Mode::Live(_) = self.mode {
            self.wane(cx.now_ms, true);
        }
        snapped
    }

    /// The cell (row, column) under `(x, y)` in the content rect, or the
    /// grid's nearest.
    fn cell_at(&self, x: f32, y: f32) -> (u16, u16) {
        let at = |v: f32, w: f32, n: u16| ((v - INSET) / w).max(0.0).min(f32::from(n - 1)) as u16;
        let ((cw, lh), t) = (self.cell, &self.term);
        (at(y, lh, t.rows()), at(x, cw, t.cols()))
    }

    /// The wheel at `(x, y)`, in whole rows: wheel reports at that cell
    /// with mouse reporting on, cursor keys on the alternate screen, else
    /// the scrollback.
    fn wheel(&mut self, (x, y): (f32, f32), dy: f32, cx: &mut Cx<'_>) -> bool {
        let dy = if dy.is_finite() { dy } else { 0.0 };
        self.wheel += dy / self.cell.1.max(1.0);
        let rows = self.wheel.trunc();
        self.wheel -= rows;
        let (old, n) = (self.scroll, rows.abs().min(1e4) as usize);
        let (alt, sb) = (self.term.alt_screen(), self.term.scrollback_len());
        let t = &self.term;
        if let (Mode::Live(s), true) = (self.mode, n > 0 && (alt || t.mouse_mode() != 0)) {
            let bytes = match t.mouse_mode() {
                0 => {
                    let k = if rows < 0.0 { Key::Up } else { Key::Down };
                    let key = encode(k, Mods::default(), t.app_cursor_keys()).unwrap_or_default();
                    key.repeat(n.min(t.rows().into()))
                }
                _ => wheel_report(64 + u8::from(rows > 0.0), self.cell_at(x, y), t.mouse_sgr()),
            };
            self.send(s, &bytes, cx);
            return false;
        }
        let max = if alt { 0 } else { sb };
        let (up, down) = ((old + n).min(max), old.saturating_sub(n));
        self.scroll = if rows < 0.0 { up } else { down };
        self.scroll != old
    }

    fn ws(&mut self, socket: SocketId, ev: WsEvent, cx: &mut Cx<'_>) {
        let (Mode::Connecting(s) | Mode::Hello(s) | Mode::Live(s)) = self.mode else {
            return;
        };
        let port = self.link.map_or(0, |p| p.port);
        match (ev, self.mode) {
            _ if s != socket => {}
            (WsEvent::Open, Mode::Connecting(_)) => {
                let token = self.link.map(|p| p.token).unwrap_or_default();
                cx.send(s, [&[HELLO, VERSION][..], &token].concat());
                self.mode = Mode::Hello(s);
            }
            (WsEvent::Data(bytes), _) => self.message(s, &bytes, cx),
            (WsEvent::Closed { .. }, Mode::Connecting(_)) => {
                let [before, after] = UNREACHABLE;
                self.end(&[before, &num(port.into()), after]);
            }
            (WsEvent::Closed { code: 4401, .. }, Mode::Hello(_)) => self.end(&[REFUSED]),
            (WsEvent::Closed { code, .. }, _) => {
                self.end(&["[session ended: code ", &num(code.into()), "]"]);
            }
            _ => {}
        }
    }

    /// One message from the node: an opcode, then its payload.
    fn message(&mut self, s: SocketId, msg: &[u8], cx: &mut Cx<'_>) {
        let Some((&op, p)) = msg.split_first() else {
            return;
        };
        match (op, self.mode) {
            // [version][u16 cols][u16 rows][info]; the size is the node's
            // default, which the terminal replaces with its own at once.
            (READY, Mode::Hello(_)) if p.len() >= 5 && p[0] != VERSION => {
                cx.close_socket(s);
                self.end(&["[the node speaks protocol version ", &num(p[0].into()), "]"]);
            }
            (READY, Mode::Hello(_)) if p.len() >= 5 => {
                self.mode = Mode::Live(s);
                let info = clean(&p[5..]);
                self.say(true, &["\x1b[2m[connected: ", &info, "]\x1b[m\n"]);
                // A new shell (ConPTY above all) expects its cursor at the top
                // left: move what is on screen into the scrollback.
                let (rows, used) = (self.term.rows(), self.term.cursor().0);
                self.print(false, &["\x1b[", &num(rows.into()), "H"]);
                (0..used).for_each(|_| self.term.feed(b"\r\n"));
                self.print(false, &["\x1b[H\x1b[2J"]);
                cx.send(s, resize(self.term.cols(), rows));
            }
            (DATA, Mode::Live(_)) => {
                let before = self.term.scrollback_len();
                self.output(p, cx.now_ms);
                // Scrolled back, the view stays on the same rows.
                let after = self.term.scrollback_len();
                if self.scroll > 0 {
                    self.scroll = (self.scroll + after.saturating_sub(before)).min(after);
                }
                let replies = self.term.take_replies();
                self.send(s, &replies, cx);
            }
            (EXIT, _) if p.len() == 4 => {
                let code = i32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                cx.close_socket(s);
                self.end(&["[session ended: code ", &num(code.into()), "]"]);
            }
            (ERROR, _) => {
                let m = clean(p);
                self.say(true, &["\x1b[31mcomputehub-node: ", &m, "\x1b[m\n"]);
            }
            _ => {}
        }
    }
}

impl App for Terminal {
    fn title(&self) -> String {
        let mut s = "Terminal".to_string();
        if let (Some(p), false) = (self.link, self.mode == Mode::Guest) {
            s = s + " (node :" + &num(p.port.into()) + ")";
        }
        match self.term.title() {
            "" => s,
            t => s + " — " + t,
        }
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let (r, focused, now) = (ui.rect(), ui.state().focused, ui.state().now_ms);
        ui.hit(WidgetId(1), r, Sense::Text);
        let ts = ui.text_system();
        let (cw, lh) = (ts.cell_width(SIZE), ts.snap((SIZE * 1.3).round()));
        let (x, y) = (ts.snap(r.x + INSET), ts.snap(r.y + INSET));
        self.cell = (cw, lh);
        self.fit(grid_size(r.w, r.h, cw, lh));
        self.drawn = true;
        self.begin();
        self.wane(now, false);
        if self.term.alt_screen() {
            self.scroll = 0;
        }
        self.scroll = self.scroll.min(self.term.scrollback_len());
        let mut list = std::mem::take(&mut self.scratch);
        list.clear();
        self.paint(ts, &mut list, (x, y), focused);
        replay(&list, ui.list());
        self.scratch = list;
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        let before = self.shown();
        if !std::mem::replace(&mut self.started, true) {
            cx.load_fallback_fonts();
            match cx.pairing {
                Some(_) => self.connect(cx),
                None => self.greet = true,
            }
        }
        if let AppEvent::Key { .. } | AppEvent::Text(_) | AppEvent::Ws { .. } = ev {
            self.begin();
        }
        let redraw = match ev {
            AppEvent::Key { key, mods } => self.key(key, mods, cx),
            AppEvent::Text(s) => self.text(&s, cx),
            AppEvent::Wheel { x, y, dy } => self.wheel((x, y), dy, cx),
            AppEvent::Focus(on) => {
                if let (Mode::Live(s), true) = (self.mode, self.term.focus_reporting()) {
                    self.send(s, if on { b"\x1b[I" } else { b"\x1b[O" }, cx);
                }
                true
            }
            AppEvent::Resized { w, h } => {
                self.fit(grid_size(w, h, self.cell.0, self.cell.1));
                true
            }
            AppEvent::Ws { socket, ev } => {
                self.ws(socket, ev, cx);
                false
            }
            AppEvent::Click(_) | AppEvent::PointerDown { .. } | AppEvent::Tick { .. } => false,
        };
        if let (true, Mode::Live(s)) = (std::mem::take(&mut self.resized), self.mode) {
            cx.send(s, resize(self.term.cols(), self.term.rows()));
        }
        self.wane(cx.now_ms, false);
        redraw || self.shown() != before
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((80.0 * 8.0 + 2.0 * INSET, 24.0 * 17.0 + 2.0 * INSET))
    }
}

/// A cell's foreground, and its background unless it is the default.
fn colors(c: &Cell) -> (Rgba, Option<Rgba>) {
    let rgb = |col| match col {
        Color::Default => None,
        Color::Indexed(i) => Some(theme::xterm_color(i)),
        Color::Rgb(r, g, b) => Some(Rgba(r, g, b, 255)),
    };
    let is = |a| c.attrs.contains(a);
    let fg = match c.fg {
        Color::Default if is(Attrs::BOLD) => theme::TEXT_BRIGHT,
        Color::Indexed(i @ 0..=7) if is(Attrs::BOLD) => theme::xterm_color(i + 8),
        fg => rgb(fg).unwrap_or(FG),
    };
    let (fg, bg) = match is(Attrs::INVERSE) {
        true => (rgb(c.bg).unwrap_or(BG), Some(fg)),
        false => (fg, rgb(c.bg)),
    };
    let dim = is(Attrs::DIM);
    (if dim { fg.with_alpha(153) } else { fg }, bg)
}

impl Terminal {
    /// Draws the visible rows (`scroll` rows back into the scrollback; the
    /// held frame's screen during a hold) and the cursor: background runs,
    /// then glyphs and their lines.
    fn paint(&self, ts: &mut TextSystem, list: &mut DrawList, (ox, oy): (f32, f32), focused: bool) {
        let ((cw, lh), asc) = (self.cell, ts.ascent(TextStyle::new(FontId::Mono, SIZE, FG)));
        let (t, held) = (&self.term, self.hold.since.map(|_| &self.hold));
        let screen =
            |r: u16| held.map_or(t.row(r), |h| h.rows.get(usize::from(r)).map_or(&[], |v| v));
        let (sb, cols) = (t.scrollback_len(), usize::from(t.cols()));
        let top = sb - self.scroll.min(sb);
        for vr in 0..usize::from(t.rows()) {
            let cells = match (top + vr).checked_sub(sb) {
                Some(r) => screen(r as u16),
                None => t.scrollback_row(top + vr),
            };
            let cells = &cells[..cells.len().min(cols)];
            let (y, mut x) = (oy + vr as f32 * lh, 0);
            while x < cells.len() {
                let bg = colors(&cells[x]).1;
                let n = cells[x..].iter().take_while(|c| colors(c).1 == bg).count();
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
                let fg = colors(c).0;
                let left = ox + x as f32 * cw;
                let (w, base) = (cw * f32::from(c.width), y + asc);
                ts.draw_cell_char(list, left, base, w, c.ch, SIZE, fg);
                let line = |dy| RectF::new(left, base + dy, w, 1.0);
                if c.attrs.contains(Attrs::UNDERLINE) {
                    list.fill(line(1.0), 0.0, fg);
                }
                if c.attrs.contains(Attrs::STRIKE) {
                    list.fill(line(-(SIZE * 0.3).round()), 0.0, fg);
                }
            }
        }
        let (visible, (cr, cc)) =
            held.map_or((t.cursor_visible(), t.cursor()), |h| (h.visible, h.cursor));
        if !visible || self.scroll > 0 || cr >= t.rows() || cc >= t.cols() {
            return;
        }
        let row = screen(cr);
        let cell = row.get(usize::from(cc)).copied().unwrap_or(Cell::BLANK);
        let w = cw * f32::from(cell.width.max(1));
        let x = ox + f32::from(cc) * cw;
        let r = RectF::new(x, oy + f32::from(cr) * lh, w, lh);
        if !focused {
            return list.border(r, 0.0, 1.0, CURSOR);
        }
        list.fill(r, 0.0, CURSOR);
        if cell.width != 0 && !cell.attrs.contains(Attrs::HIDDEN) {
            ts.draw_cell_char(list, r.x, r.y + asc, w, cell.ch, SIZE, BG);
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

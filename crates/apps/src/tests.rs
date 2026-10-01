use super::launcher::rank;
use super::terminal::{self as tm, encode, grid_size};
use super::*;
use gfx::{DrawList, Instance, RectF, Rgba};
use ui::{App, AppEvent, Cx, FontId, Hit, Key, Mods, Pairing, Request, SocketId, TextSystem, Ui};
use ui::{UiState, WidgetId, WsEvent, theme};
use vfs::Vfs;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");
const SYM_A: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-a.ttf");
const NO: Mods = Mods { shift: false, ctrl: false, alt: false, meta: false };
const CTRL: Mods = Mods { ctrl: true, ..NO };
const PAIR: Pairing = Pairing { port: 7878, token: [7; 32] };
/// The content size of an 80 x 24 terminal at dpr 1, and its prompt.
const W80: f32 = 652.0;
const H24: f32 = 420.0;
const PROMPT: &str = "guest@compusophy:~$";

/// An app, what its [`Cx`]s are made from, and helpers that show what it
/// asked for as text.
struct Sim<A> {
    app: A,
    fs: Vfs,
    next: u32,
    pairing: Option<Pairing>,
    now: f64,
}

impl<A: App> Sim<A> {
    fn new(app: A, pairing: Option<Pairing>) -> Sim<A> {
        let (fs, next, now) = (Vfs::new(), 1, 0.0);
        Sim { app, fs, next, pairing, now }
    }
    /// Sends `ev`; returns whether the app redraws and its requests.
    fn both(&mut self, ev: AppEvent) -> (bool, String) {
        let mut cx = Cx::new(&mut self.fs, self.now, self.pairing, &mut self.next);
        let redraw = self.app.event(ev, &mut cx);
        let reqs: Vec<String> = cx.take_requests().iter().map(show).collect();
        (redraw, reqs.join("; "))
    }
    fn ev(&mut self, ev: AppEvent) -> String {
        self.both(ev).1
    }
    fn key(&mut self, key: Key, mods: Mods) -> String {
        self.ev(AppEvent::Key { key, mods })
    }
    fn keys(&mut self, keys: &[Key]) {
        keys.iter().for_each(|&k| _ = self.key(k, NO));
    }
    fn text(&mut self, s: &str) -> String {
        self.ev(AppEvent::Text(s.into()))
    }
    fn wheel(&mut self, dy: f32) -> bool {
        self.both(AppEvent::Wheel { x: 0.0, y: 0.0, dy }).0
    }
    fn touch(&mut self, path: &str) {
        self.fs.write(&path.replace('~', Vfs::HOME), b"").unwrap();
    }
}

impl Sim<Terminal> {
    fn term(pairing: Option<Pairing>) -> Sim<Terminal> {
        Sim::new(Terminal::default(), pairing)
    }
    /// Types `s` and Enter.
    fn line(&mut self, s: &str) -> String {
        self.text(s) + &self.key(Key::Enter, NO)
    }
    fn ws(&mut self, n: u32, ev: WsEvent) -> String {
        let socket = SocketId(n);
        self.ev(AppEvent::Ws { socket, ev })
    }
    /// A message from the node on socket `n`.
    fn node(&mut self, n: u32, msg: &[u8]) -> String {
        self.ws(n, WsEvent::Data(msg.to_vec()))
    }
    fn closed(&mut self, n: u32, code: u16) -> String {
        let reason = String::new();
        self.ws(n, WsEvent::Closed { code, reason })
    }
    /// Opens socket `n` and sends READY for protocol `version`.
    fn ready(&mut self, n: u32, version: u8) -> String {
        self.ws(n, WsEvent::Open);
        self.node(n, &[2, version, 80, 0, 24, 0, b'u', b'n', b'i', b'x'])
    }
    /// The scrollback and the screen as text.
    fn all(&self) -> String {
        let t = &self.app.term;
        let sb = (0..t.scrollback_len()).map(|i| text(t.scrollback_row(i)));
        let lines: Vec<String> = sb.chain((0..t.rows()).map(|r| text(t.row(r)))).collect();
        lines.join("\n").trim_end().to_string()
    }
    fn has(&self, s: &str) {
        let all = self.all();
        assert!(all.contains(s), "{s:?} is not in\n{all}");
    }
    fn row(&self, r: u16) -> String {
        text(self.app.term.row(r))
    }
    /// The cursor, and its row as text.
    fn at(&self) -> (u16, u16, String) {
        let (r, c) = self.app.term.cursor();
        (r, c, self.row(r))
    }
}

/// A request as text; bytes sent show as `<socket> <what>`.
fn show(r: &Request) -> String {
    match r {
        Request::Open { name, floating } => {
            format!("{} {name}", ["open", "float"][*floating as usize])
        }
        Request::CloseSelf => "close".into(),
        Request::Connect { socket, port } => format!("connect {} {port}", socket.0),
        Request::CloseSocket(s) => format!("close {}", s.0),
        Request::LoadFallbackFonts => "fonts".into(),
        Request::Send { socket, bytes } => {
            let what = match bytes.split_first() {
                Some((1, rest)) if rest[1..] == PAIR.token => "hello".into(),
                Some((3, rest)) => esc(rest),
                Some((4, &[c0, c1, r0, r1])) => {
                    let (c, r) = (u16::from_le_bytes([c0, c1]), u16::from_le_bytes([r0, r1]));
                    format!("resize {c}x{r}")
                }
                _ => format!("?{}", esc(bytes)),
            };
            format!("{} {what}", socket.0)
        }
    }
}
/// Bytes as ASCII, others as `\xNN`.
fn esc(b: &[u8]) -> String {
    let one = |&b: &u8| match b {
        0x20..0x7f => char::from(b).to_string(),
        _ => format!("\\x{b:02x}"),
    };
    b.iter().map(one).collect()
}
/// A row of cells as text, trailing blanks trimmed.
fn text(cells: &[term::Cell]) -> String {
    let s: String = cells.iter().filter(|c| c.width != 0).map(|c| c.ch).collect();
    s.trim_end().to_string()
}
fn draw(app: &mut dyn App, ts: &mut TextSystem, r: RectF, focused: bool) -> (DrawList, Vec<Hit>) {
    let (mut list, mut hits, mut state) = (DrawList::new(), Vec::new(), UiState::default());
    state.focused = focused;
    app.draw(&mut Ui::new(&mut list, ts, r, &mut hits, state));
    (list, hits)
}
/// A text system once the deferred fonts arrived.
fn text_system() -> TextSystem {
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    ts.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    ts.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    ts
}
fn resized(w: f32, h: f32) -> AppEvent {
    AppEvent::Resized { w, h }
}

#[test]
fn guest_line_editing_and_history() {
    let mut s = Sim::term(None);
    assert_eq!(s.ev(resized(W80, H24)), "fonts");
    s.draw_at(W80, H24);
    s.has("compusophyOS terminal — built-in guest shell.");
    s.line("echo one");
    s.line("cd /tmp");
    s.text("draft");
    let mut seen = Vec::new();
    for key in [Key::Up, Key::Up, Key::Up, Key::Down, Key::Down] {
        s.key(key, NO);
        seen.push(s.at().2.replace("guest@compusophy:/tmp$ ", ""));
    }
    assert_eq!(seen.join(","), "cd /tmp,echo one,echo one,cd /tmp,draft");
    s.keys(&[Key::Home, Key::Right, Key::Delete]);
    s.text("X");
    assert_eq!(s.at().1, 25);
    s.key(Key::Char('c'), CTRL);
    s.has("guest@compusophy:/tmp$ dXaft^C\nguest@compusophy:/tmp$");
    s.text("pwd");
    s.key(Key::Char('l'), CTRL);
    assert_eq!(s.all(), "guest@compusophy:/tmp$ pwd");
    s.key(Key::Enter, NO);
    s.line("history");
    s.has("/tmp\nguest@compusophy:/tmp$ history\n   1  echo one\n   2  cd /tmp");
    // Enter reported as text too is no second Enter; a pasted line runs.
    assert!(!s.both(AppEvent::Text("\n".into())).0);
    s.text("echo pasted\necho tw");
    assert!(s.all().ends_with("pasted\nguest@compusophy:/tmp$ echo tw"));
}

#[test]
fn guest_line_wraps_like_the_terminal() {
    let mut s = Sim::term(None);
    s.ev(resized(12.0 + 8.0 * 30.0, H24));
    s.draw_at(12.0 + 8.0 * 30.0, H24);
    let top = s.at().0;
    s.text("abcdefghijklmnop"); // the prompt is 20 wide
    assert_eq!(s.at(), (top + 1, 6, "klmnop".into()));
    s.keys(&[Key::Left; 8]);
    s.text("X");
    assert_eq!(s.at(), (top, 29, "guest@compusophy:~$ abcdefghXi".into()));
    s.keys(&[Key::Right]);
    assert_eq!(s.at(), (top + 1, 0, "jklmnop".into()), "the next row");
    s.keys(&[Key::Delete; 7]);
    assert_eq!(s.at(), (top + 1, 0, String::new()), "a full row: the next");
    s.key(Key::Enter, NO);
    assert_eq!(s.row(top + 1), "abcdefghXi: command not found", "no gap");
    assert_eq!(s.at(), (top + 2, 20, PROMPT.into()));
}

#[test]
fn terminal_talks_to_the_node() {
    let mut s = Sim::term(Some(PAIR));
    assert_eq!(s.ev(resized(W80, H24)), "fonts; connect 1 7878");
    s.draw_at(W80, H24);
    s.has("connecting to computehub-node on 127.0.0.1:7878…");
    assert_eq!(s.key(Key::Enter, NO), "", "nothing before READY");
    assert_eq!(s.ws(1, WsEvent::Open), "1 hello");
    assert_eq!(s.node(1, b"\x02\x01\x50\0\x18\0unix/sh"), "1 resize 80x24");
    s.has("[connected: unix/sh]");
    assert_eq!(s.at(), (0, 0, String::new()));
    assert_eq!(s.app.title(), "Terminal (node :7878)");
    // Output in, replies out.
    assert_eq!(s.node(1, b"\x03$ hi\x1b[6n\x1b]0;vim\x07"), "1 \\x1b[1;5R");
    assert_eq!(s.row(0), "$ hi");
    assert_eq!(s.app.title(), "Terminal (node :7878) — vim");
    // Keys and text.
    let shift = Mods { shift: true, ..NO };
    for (key, mods, want) in [
        (Key::Char('c'), CTRL, "1 \\x03"),
        (Key::Up, NO, "1 \\x1b[A"),
        (Key::Tab, shift, "1 \\x1b[Z"),
        (Key::Char('a'), NO, ""),
        (Key::Char('v'), CTRL, ""),
    ] {
        assert_eq!(s.key(key, mods), want, "{key:?}");
    }
    assert_eq!(s.text("é"), "1 \\xc3\\xa9");
    assert_eq!(s.text("a\r\nb\x1b"), "1 a\\x0db");
    s.node(1, b"\x03\x1b[?1h\x1b[?2004h\x1b[?1004h");
    assert_eq!(s.key(Key::Up, NO), "1 \\x1bOA");
    assert_eq!(s.text("ab"), "1 \\x1b[200~ab\\x1b[201~");
    assert_eq!(s.ev(AppEvent::Focus(false)), "1 \\x1b[O");
    assert_eq!(s.text(&"x".repeat(150_000)).matches("; ").count(), 2);
    // Resizes, from the event and from a draw.
    assert_eq!(s.ev(resized(W80 + 160.0, H24)), "1 resize 100x24");
    let r = RectF::new(0.0, 0.0, W80, H24 + 34.0);
    draw(&mut s.app, &mut text_system(), r, true);
    assert_eq!(s.ev(AppEvent::Tick { now_ms: 0.0 }), "1 resize 80x26");
    // EXIT ends the session and tidies up; Enter reconnects.
    assert_eq!(s.node(1, &[5, 3, 0, 0, 0]), "close 1");
    s.has("[session ended: code 3]\nPress Enter to reconnect");
    let t = &s.app.term;
    assert!(!t.bracketed_paste() && !t.app_cursor_keys());
    assert_eq!(s.closed(1, 1000) + &s.text("x"), "");
    assert_eq!(s.key(Key::Enter, NO), "connect 2 7878");
    s.closed(2, 1006);
    s.has("[could not reach computehub-node on 127.0.0.1:7878.");
    s.key(Key::Char('d'), CTRL);
    s.line("echo back");
    assert!(s.all().ends_with("back\nguest@compusophy:~$"));
}

#[test]
fn terminal_session_failures() {
    let mut s = Sim::term(Some(PAIR));
    s.ws(1, WsEvent::Open);
    s.closed(1, 4401);
    s.has("[the node refused this page's token");
    s.key(Key::Enter, NO);
    s.ws(1, WsEvent::Open);
    s.ws(2, WsEvent::Open);
    s.node(2, b"\x06busy\x07");
    s.closed(2, 1013);
    s.has("computehub-node: busy\n[session ended: code 1013]");
    s.key(Key::Enter, NO);
    assert_eq!(s.ready(3, 9), "close 3");
    s.has("[the node speaks protocol version 9]");
    s.key(Key::Enter, NO);
    assert_eq!(s.key(Key::Char('c'), CTRL), "close 4", "Ctrl+C gives up");
    assert_eq!(s.at().2, PROMPT);
}

#[test]
fn terminal_scrollback_alt_screen_and_sync() {
    let mut s = Sim::term(Some(PAIR));
    s.ready(1, 1);
    let lines: String = (0..60).map(|i| format!("line {i}\r\n")).collect();
    s.node(1, &tm::data(lines.as_bytes()));
    let sb = s.app.term.scrollback_len();
    assert!(s.wheel(-17.0 * 3.0) && s.app.scroll == 3);
    assert!(!s.wheel(8.5), "half a row");
    assert!(s.wheel(8.5) && s.app.scroll == 2);
    s.node(1, b"\x03more\r\n");
    assert_eq!((s.app.scroll, s.app.term.scrollback_len()), (3, sb + 1));
    assert!(s.wheel(-1e9) && s.app.scroll == sb + 1);
    s.key(Key::Char('x'), NO);
    assert_eq!(s.app.scroll, 0, "a key snaps back");
    s.node(1, b"\x03\x1b[?1049h");
    assert!(!s.wheel(-34.0), "no scrollback on the alternate screen");
    // A synchronized update holds redraws for up to 250 ms.
    let mut feed = |now, bytes: &[u8]| {
        let (ev, socket) = (WsEvent::Data(tm::data(bytes)), SocketId(1));
        s.now = now;
        s.both(AppEvent::Ws { socket, ev }).0
    };
    assert!(!feed(1e3, b"\x1b[?2026h."));
    assert!(!feed(1.1e3, b"."));
    assert!(feed(1.3e3, b"."));
    assert!(feed(1.3e3, b"\x1b[?2026l"));
}

#[test]
fn keys_and_grid() {
    let enc = |key, mods, app| encode(key, mods, app).map(|b| esc(&b));
    assert_eq!(enc(Key::Char('d'), CTRL, true).unwrap(), "\\x04");
    assert_eq!(enc(Key::Left, NO, true).unwrap(), "\\x1bOD");
    assert_eq!(enc(Key::Right, CTRL, true).unwrap(), "\\x1b[1;5C");
    assert_eq!(enc(Key::Char('7'), CTRL, false).unwrap(), "\\x1f");
    assert_eq!(enc(Key::Char('9'), CTRL, false), None);
    assert_eq!(enc(Key::Char('c'), Mods { meta: true, ..NO }, false), None);
    assert_eq!(grid_size(W80, H24, 8.0, 17.0), (80, 24));
    assert_eq!(grid_size(0.0, -5.0, 8.0, 17.0), (1, 1));
    assert_eq!(grid_size(f32::NAN, f32::INFINITY, 8.0, 17.0), (1, 500));
    assert_eq!(Terminal::default().preferred_size(), Some((W80, H24)));
    assert_eq!(tm::resize(300, 2), [4, 44, 1, 2, 0]);
}

#[test]
fn terminal_draws_cells() {
    let (mut ts, mut t) = (text_system(), Terminal::default());
    ts.add_fallback(SYM_A.to_vec()).unwrap();
    let styled = "\x1b[1;31mred\x1b[m \x1b[7minv\x1b[m \x1b[4;9mls\x1b[m \x1b[2mdim\x1b[m \
                  \x1b[8mhid\x1b[m ✻⏺╭─╮日\r\n\x1b[48;2;10;20;30m  \x1b[m";
    t.term.feed(styled.as_bytes());
    let rect = RectF::new(10.0, 40.0, W80 + 160.0, H24);
    // Before Mono arrives: the same grid and fills, but no glyphs.
    let mut boot = TextSystem::new(SANS.to_vec()).unwrap();
    let early = draw(&mut t, &mut boot, rect, true).0;
    assert_eq!((t.term.cols(), t.term.rows()), (100, 24));
    let (list, hits) = draw(&mut t, &mut ts, rect, true);
    assert_eq!((t.term.cols(), t.term.rows(), hits.len()), (100, 24, 1));
    let of = |kind| list.instances().iter().filter(move |i| i.kind == kind);
    // Every visible char but the hidden ones and the one in no font (a box).
    assert_eq!((of(4.0).count(), of(1.0).count()), (16, 1));
    let bright = of(4.0).filter(|i| i.color == Rgba::hex(0xf07f88));
    assert_eq!(bright.count(), 3, "bold makes red bright");
    assert_eq!(of(4.0).filter(|i| i.color.3 == 153).count(), 3, "dim");
    let fills: Vec<_> = of(0.0).map(|i| (i.rect, i.color)).collect();
    let same = early.instances().iter().eq(of(0.0));
    assert!(same, "before Mono: the same fills, and no glyphs");
    let has = |r: [f32; 4], color| fills.contains(&(r, color));
    assert!(has([48.0, 46.0, 24.0, 17.0], theme::TEXT), "inverse");
    assert!(has([16.0, 63.0, 16.0, 17.0], Rgba(10, 20, 30, 255)));
    assert!(has([32.0, 63.0, 8.0, 17.0], theme::ACCENT), "cursor");
    assert_eq!(fills.iter().filter(|f| f.0[3] == 1.0).count(), 4, "lines");
    let (list, _) = draw(&mut t, &mut ts, rect, false);
    let ring = |i: &&Instance| i.kind == 1.0 && i.color == theme::ACCENT;
    assert_eq!(list.instances().iter().filter(ring).count(), 1, "unfocused");
}

#[test]
fn launcher_filters_and_opens() {
    let r = |q| rank(q, "Terminal");
    let got = [r(""), r("TML"), r("al"), r("mt"), r("x")];
    assert_eq!(got, [Some(0), Some(0), Some(6), None, None]);
    let mut s = Sim::new(Launcher::default(), None);
    s.touch("/apps/counter.app");
    s.touch("~/notes.app");
    s.touch("~/readme.txt");
    // "o": counter and notes (at 1), About (2), Welcome (4), Studio (5).
    s.text("O");
    s.keys(&[Key::Down, Key::Down, Key::Down, Key::Up]);
    assert_eq!(s.key(Key::Enter, NO), "open about; close");
    s.app = Launcher::default();
    s.keys(&[Key::Down; 9]);
    let notes = format!("open {}/notes.app; close", Vfs::HOME);
    assert_eq!(s.key(Key::Enter, NO), notes);
    s.app = Launcher::default();
    s.text("zz");
    s.keys(&[Key::Enter, Key::Backspace, Key::Backspace]);
    assert_eq!(s.ev(AppEvent::Click(WidgetId(101))), "open studio; close");
    assert_eq!(s.key(Key::Escape, NO), "close");
    // Rows are hits; a short window shows fewer and follows the choice.
    let mut ts = text_system();
    let mut ids = |l: &mut Launcher, height| {
        let hits = draw(l, &mut ts, RectF::new(0.0, 0.0, 440.0, height), true).1;
        hits.iter().map(|h| h.id.0).collect::<Vec<_>>()
    };
    assert_eq!(ids(&mut s.app, 380.0), [1, 100, 101, 102, 103, 104, 105]);
    assert_eq!(ids(&mut s.app, 140.0), [1, 100, 101]);
    s.keys(&[Key::Down; 4]);
    assert_eq!(ids(&mut s.app, 140.0), [1, 103, 104]);
}

#[test]
fn welcome_and_about() {
    let mut ts = text_system();
    for name in NAMES {
        let mut app = open(name).unwrap();
        let r = RectF::new(0.0, 36.0, 700.0, 500.0);
        assert!(!draw(app.as_mut(), &mut ts, r, true).0.is_empty(), "{name}");
    }
    let mut s = Sim::new(Welcome::default(), None);
    let r = RectF::new(0.0, 0.0, 700.0, 900.0);
    assert_eq!(draw(&mut s.app, &mut ts, r, true).1.len(), 4, "the buttons");
    let mut click = |i| s.ev(AppEvent::Click(WidgetId(i)));
    assert_eq!(click(1) + "," + &click(2), "open terminal,open studio");
    assert_eq!(click(3) + "," + &click(4), "float launcher,open about");
    assert!(!s.wheel(50.0), "it all fits");
    let mut s = Sim::new(About::default(), None);
    let r = RectF::new(0.0, 0.0, 400.0, 200.0);
    draw(&mut s.app, &mut ts, r, true);
    assert!(s.wheel(50.0) && s.wheel(-80.0) && !s.wheel(-1.0));
}

impl Sim<Terminal> {
    /// Draws at `w` x `h`, dpr 1, with every font; the glyphs drawn.
    fn draw_at(&mut self, w: f32, h: f32) -> usize {
        let r = RectF::new(0.0, 0.0, w, h);
        let list = draw(&mut self.app, &mut text_system(), r, true).0;
        list.instances().iter().filter(|i| i.kind == 4.0).count()
    }
    /// Output from the node on socket 1; whether the terminal redraws.
    fn out(&mut self, bytes: &[u8]) -> bool {
        let (socket, ev) = (SocketId(1), WsEvent::Data(tm::data(bytes)));
        self.both(AppEvent::Ws { socket, ev }).0
    }
    /// Whether the first rows hold `s`, spaces aside (a wrap may eat one).
    fn wraps(&self, s: &str) -> bool {
        let rows: String = (0..3).map(|r| self.row(r).replace(' ', "")).collect();
        rows.contains(&s.replace(' ', ""))
    }
}

#[test]
fn meta_and_control_symbols_reach_the_program() {
    let alt = Mods { alt: true, ..NO };
    let (sa, ca) = (Mods { shift: true, ..alt }, Mods { ctrl: true, ..alt });
    let enc = |c, mods| encode(Key::Char(c), mods, false).map_or("-".into(), |b| esc(&b));
    let got = [enc('b', alt), enc('d', sa), enc('5', alt), enc('c', ca), enc('5', sa)];
    // A shifted digit's char depends on the layout.
    assert_eq!(got.join(" "), "\\x1bb \\x1bD \\x1b5 \\x1b\\x03 -");
    // Ctrl+2 to Ctrl+8 as xterm sends them: NUL, ESC, FS, GS, RS, US, DEL.
    let c0: Vec<String> = "1234567890".chars().map(|c| enc(c, CTRL)).collect();
    assert_eq!(c0.concat(), "-\\x00\\x1b\\x1c\\x1d\\x1e\\x1f\\x7f--");
    let mut s = Sim::term(Some(PAIR));
    s.ready(1, 1);
    let got = s.key(Key::Char('b'), alt) + &s.key(Key::Char('3'), CTRL);
    assert_eq!(got, "1 \\x1bb1 \\x1b");
}

#[test]
fn a_synchronized_update_holds_the_last_frame() {
    let mut s = Sim::term(Some(PAIR));
    s.ready(1, 1);
    let drawn = |s: &mut Sim<Terminal>| s.draw_at(W80, H24);
    assert!(s.out(b"\x1b[?2026hFRAME-ONE\x1b[?2026l") && drawn(&mut s) == 9);
    // The next frame clears first and spans two messages: in between, a
    // draw shows the last frame, and a key asks for none.
    assert!(!s.out(b"\x1b[?2026h\x1b[H\x1b[2J"));
    assert_eq!(drawn(&mut s), 9, "not the half-built frame");
    let up = s.both(AppEvent::Key { key: Key::Up, mods: NO });
    assert_eq!(up, (false, "1 \\x1b[A".into()));
    assert!(s.out(b"TWO\x1b[?2026l") && drawn(&mut s) == 3);
    // Back to back: the frame that ended in the message is the one held.
    assert!(s.out(b"\x1b[?2026hAB\x1b[?2026l\x1b[?2026h\x1b[2J") && drawn(&mut s) == 5);
    assert!(s.out(b"\x1b[?2026l\x1b[H\x1b[2J") && drawn(&mut s) == 0);
    // A program that dies inside an update leaves it set: the second key
    // or text then gives way.
    let typed = |s: &mut Sim<Terminal>, t: &str| s.both(AppEvent::Text(t.into()));
    assert!(!s.out(b"\x1b[?2026h"));
    assert_eq!(typed(&mut s, "l"), (false, "1 l".into()));
    assert!(!s.out(b"l"), "one keystroke may still be part of a frame");
    assert_eq!(typed(&mut s, "s"), (true, "1 s".into()));
    assert!(s.out(b"s") && s.row(0) == "ls");
    // So does the clock, after 250 ms.
    assert!(s.out(b"\x1b[?2026l\x1b[?2026h") && !s.out(b"x"));
    s.now = 300.0;
    assert!(s.both(AppEvent::Tick { now_ms: 300.0 }).0 && s.out(b"y"));
}

#[test]
fn a_session_keeps_its_own_node() {
    let mut s = Sim::term(Some(PAIR));
    s.ready(1, 1);
    s.node(1, b"\x03\x1b]0;sudo\x07");
    assert_eq!(s.app.title(), "Terminal (node :7878) — sudo");
    s.node(1, &[5, 0, 0, 0, 0]);
    assert_eq!(s.app.title(), "Terminal (node :7878)", "no stale title");
    // Another pairing arrives (a new link, or a page that navigated this
    // tab's fragment): this terminal still reconnects to its own node.
    let token = [0xee; 32];
    s.pairing = Some(Pairing { port: 9999, token });
    let got = s.key(Key::Enter, NO) + "," + &s.ws(2, WsEvent::Open);
    assert_eq!(got, "connect 2 7878,2 hello");
    s.closed(2, 1006);
    s.key(Key::Char('d'), CTRL);
    assert_eq!(s.app.title(), "Terminal");
    assert_eq!(s.line("connect"), "connect 3 7878");
    // A new terminal takes the new pairing.
    let got = Sim::term(s.pairing).ev(resized(W80, H24));
    assert_eq!(got, "fonts; connect 1 9999");
}

#[test]
fn the_greeting_waits_for_the_grid() {
    let banner = "guest shell. Type 'help'. For a real shell on your machine, type 'node'.";
    let mut s = Sim::term(None);
    s.ev(resized(500.0, 400.0));
    s.draw_at(500.0, 400.0);
    assert!(s.app.term.cols() == 61 && s.wraps(banner), "{}", s.all());
    assert_eq!(s.at(), (2, 20, PROMPT.into()));
    // A key before any draw greets first.
    let mut s = Sim::term(None);
    s.ev(resized(W80, H24));
    s.text("ls");
    assert!(s.wraps(banner) && s.at().2 == PROMPT.to_string() + " ls");
    // Where a paired terminal connects, at a phone's width.
    let mut s = Sim::term(Some(PAIR));
    s.ev(resized(300.0, H24));
    s.draw_at(300.0, H24);
    assert!(s.wraps("connecting to computehub-node on 127.0.0.1:7878…"));
}

#[test]
fn the_wheel_reaches_full_screen_programs() {
    // At the pointer's cell (row 2, column 9 of 8 x 17 px cells, 6 px in).
    let (x, y) = (82.0, 44.0);
    let wheel = |s: &mut Sim<Terminal>, dy| s.ev(AppEvent::Wheel { x, y, dy });
    let mut s = Sim::term(Some(PAIR));
    s.ready(1, 1);
    s.node(1, b"\x03\x1b[?1049h\x1b[5;7H");
    // No mouse mode: a cursor key a row (alternate scroll); short of a
    // row, nothing yet.
    assert_eq!(wheel(&mut s, -34.0), "1 \\x1b[A\\x1b[A");
    s.node(1, b"\x03\x1b[?1h");
    let got = [17.0, 8.0, 9.0].map(|dy| wheel(&mut s, dy));
    assert_eq!(got, ["1 \\x1bOB", "", "1 \\x1bOB"]);
    // Mouse mode: one report an event, at the pointer's cell (not the
    // cursor's), SGR or X10; off the grid, at the nearest cell.
    s.node(1, b"\x03\x1b[?1000h\x1b[?1006h");
    assert_eq!(wheel(&mut s, -51.0), "1 \\x1b[<64;10;3M");
    let (x, y) = (-50.0, 1e9);
    assert_eq!(s.ev(AppEvent::Wheel { x, y, dy: -17.0 }), "1 \\x1b[<64;1;24M");
    s.node(1, b"\x03\x1b[?1006l");
    assert_eq!(wheel(&mut s, 17.0), "1 \\x1b[Ma*#");
    // On the main screen with no mouse mode, the wheel scrolls back.
    s.node(1, b"\x03\x1b[?1000l\x1b[?1049l");
    assert_eq!(wheel(&mut s, -17.0), "");
}

#[test]
fn welcome_buttons_wrap_on_a_phone() {
    let (mut ts, mut app) = (text_system(), Welcome::default());
    for (w, rows) in [(700.0, 1), (370.0, 2), (250.0, 3)] {
        let r = RectF::new(10.0, 0.0, w, 2000.0);
        let hits = draw(&mut app, &mut ts, r, true).1;
        let fits = |h: &Hit| h.rect.w > 60.0 && h.rect.x + h.rect.w <= r.x + r.w - 12.0;
        assert!(hits.len() == 4 && hits.iter().all(fits), "{w}: {hits:?}");
        let breaks = hits.windows(2).filter(|p| p[1].rect.y > p[0].rect.y);
        assert_eq!(1 + breaks.count(), rows);
    }
}

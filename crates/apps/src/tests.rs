use super::terminal::grid_size;
use super::*;
use gfx::{DrawList, Instance, Kind, RectF, Rgba};
use ui::icon::Glyph;
use ui::{App, AppEvent, Cx, FontId, Hit, Key, Mods, Request, TextSystem, Ui, UiState};
use ui::{THEMES, Theme};
use vfs::Vfs;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");
const SYM_A: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-a.ttf");
const NO: Mods = Mods { shift: false, ctrl: false, alt: false, meta: false };
const CTRL: Mods = Mods { ctrl: true, ..NO };
/// The content size of an 80 x 24 terminal at dpr 1.
const W80: f32 = 668.0;
const H24: f32 = 436.0;
const MIDNIGHT: &Theme = &THEMES[0];

/// An app with the VFS, kernel and AI status its [`Cx`]s are made from (kept as the host
/// keeps them); helpers show requests as text.
struct Sim<A> {
    app: A,
    fs: Vfs,
    kernel: ui::kernel::Kernel,
    ai: ui::AiStatus,
}

impl<A: App> Sim<A> {
    fn new(app: A) -> Sim<A> {
        Sim { app, fs: Vfs::new(), kernel: ui::kernel::Kernel::new(), ai: Default::default() }
    }
    /// Sends `ev`; returns whether the app redraws and its requests.
    fn both(&mut self, ev: AppEvent) -> (bool, String) {
        let mut cx = Cx::new(&mut self.fs, &mut self.kernel, 0.0);
        cx.ai = self.ai.clone();
        let redraw = self.app.event(ev, &mut cx);
        self.ai = cx.ai.clone();
        let reqs: Vec<String> = cx.take_requests().iter().map(show).collect();
        (redraw, reqs.join("; "))
    }
    fn ev(&mut self, ev: AppEvent) -> String {
        self.both(ev).1
    }
    fn key(&mut self, key: Key, mods: Mods) -> String {
        self.ev(AppEvent::Key { key, mods })
    }
    fn text(&mut self, s: &str) -> String {
        self.ev(AppEvent::Text(s.into()))
    }
    fn wheel(&mut self, dy: f32) -> bool {
        self.both(AppEvent::Wheel { x: 0.0, y: 0.0, dy }).0
    }
}

impl Sim<Terminal> {
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
}

/// A request as text.
fn show(r: &Request) -> String {
    match r {
        Request::Open { name, floating: false } => format!("open {name}"),
        Request::LoadFallbackFonts => "fonts".into(),
        Request::SetTheme(name) => format!("theme {name}"),
        Request::Pref { key, value } => format!("pref {key}={value}"),
        other => format!("{other:?}"),
    }
}
/// A row of cells as text, trailing blanks trimmed.
fn text(cells: &[term::Cell]) -> String {
    cells.iter().filter(|c| c.width != 0).map(|c| c.ch).collect::<String>().trim_end().into()
}
/// One frame of `app` in `r` with this pointer state, in `theme`.
fn frame(app: &mut dyn App, ts: &mut TextSystem, r: RectF, state: UiState, theme: &Theme) -> Frame {
    let (mut list, mut hits) = (DrawList::new(), Vec::new());
    app.draw(&mut Ui::new(&mut list, ts, r, &mut hits, state, theme));
    (list, hits)
}
type Frame = (DrawList, Vec<Hit>);
/// A frame with the window focused.
fn draw(app: &mut dyn App, ts: &mut TextSystem, r: RectF, theme: &Theme) -> Frame {
    frame(app, ts, r, UiState { focused: true, ..UiState::default() }, theme)
}
/// A text system once the deferred fonts arrived.
fn text_system() -> TextSystem {
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    ts.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    ts.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    ts
}
fn of(list: &DrawList, kind: Kind) -> impl Iterator<Item = &Instance> {
    list.instances().iter().filter(move |i| i.kind == kind as u8 as f32)
}

/// A terminal on an isolated kernel whose /bin holds the shell, resized to 80 x 24 (its first
/// event asks for fonts): the shell's pid, past its READY, and its console set `raw`.
fn console(raw: bool) -> (Sim<Terminal>, u32) {
    use ui::kernel::wire::{self, Msg};
    let mut s = Sim::new(Terminal::default());
    s.kernel.set_isolated(true);
    s.fs.mkdir("/bin").and(s.fs.write(SHELL, b"#!wasm bin/sh.wasm\n")).unwrap();
    assert_eq!(s.ev(AppEvent::Resized { w: W80, h: H24 }), "fonts");
    let pid = s.app.pid.expect("the shell runs");
    s.kernel.message(&mut s.fs, pid, &[wire::READY, wire::VERSION]);
    s.kernel.message(&mut s.fs, pid, &Msg::ConsMode { bits: u8::from(raw) }.encode());
    s.kernel.take_effects();
    (s, pid)
}

impl Sim<Terminal> {
    /// What the shell writes, as its worker sends it; the requests the Terminal then makes.
    fn wrote(&mut self, pid: u32, data: &[u8]) -> String {
        use ui::kernel::wire::Msg;
        self.kernel.message(&mut self.fs, pid, &Msg::ConsWrite { data }.encode());
        self.ev(AppEvent::Io)
    }
    /// What the shell's next read of its console gets.
    fn read(&mut self, pid: u32) -> Vec<u8> {
        use ui::kernel::{Effect, wire::Msg};
        self.kernel.message(&mut self.fs, pid, &Msg::ConsRead { max: 4096 }.encode());
        let reply = self.kernel.take_effects().into_iter().find_map(|e| match e {
            Effect::Reply { data, .. } => Some(data),
            _ => None,
        });
        reply.expect("input waiting")
    }
}

#[test]
fn the_terminal_starts_the_shell_on_its_console_at_its_first_size() {
    use ui::kernel::{Effect, Load, wire};
    let mut s = Sim::new(Terminal::default());
    s.kernel.set_isolated(true);
    s.fs.mkdir("/bin").and(s.fs.write(SHELL, b"#!wasm bin/sh.wasm\n")).unwrap();
    assert_eq!((s.ev(AppEvent::Focus(true)), s.app.pid), ("fonts".into(), None), "no size yet");
    s.ev(AppEvent::Resized { w: W80, h: H24 });
    assert_eq!(s.kernel.procs(), [(2, "sh".into(), true)]);
    s.kernel.message(&mut s.fs, 2, &[wire::READY, wire::VERSION]);
    let start = s.kernel.take_effects().into_iter().find_map(|e| match e {
        Effect::Start { msg, program: Load::Url(url), .. } => {
            Some((wire::Start::decode(&msg), url))
        }
        _ => None,
    });
    let (start, url) = start.expect("a Start");
    let start = start.unwrap();
    assert_eq!(
        (url.as_str(), start.argv, start.tty),
        ("bin/sh.wasm", vec!["sh".into()], Some((80, 24)))
    );
    assert_eq!((start.cwd.as_str(), start.roots), (Vfs::HOME, vec!["/".into()]));
    // One shell, whatever sizes follow; a new size reaches its console.
    s.ev(AppEvent::Resized { w: W80 - 80.0, h: H24 });
    assert_eq!((s.kernel.procs().len(), s.app.term.cols()), (1, 70));
    let words = s.kernel.take_effects();
    assert!(words.contains(&Effect::Word { pid: 2, index: wire::COLS, value: 70 }), "{words:?}");
    // A shell that cannot start says why, on screen.
    let mut s = Sim::new(Terminal::default());
    s.fs.mkdir("/bin").and(s.fs.write(SHELL, b"#!wasm bin/sh.wasm\n")).unwrap();
    s.ev(AppEvent::Resized { w: W80, h: H24 });
    s.has("sh: programs need a cross-origin isolated page (COOP/COEP headers)");
    let mut s = Sim::new(Terminal::default());
    s.kernel.set_isolated(true);
    s.ev(AppEvent::Resized { w: W80, h: H24 });
    assert_eq!((s.row(0), s.app.pid), ("sh: not found".into(), None));
}

#[test]
fn keys_and_pastes_go_to_the_console_as_xterm_sends_them() {
    let (mut s, pid) = console(true);
    s.key(Key::Up, NO);
    s.key(Key::Char('c'), CTRL);
    s.key(Key::Char('x'), NO); // Comes as text.
    s.key(Key::Other, NO);
    s.text("é");
    assert!(!s.both(AppEvent::Text("\n".into())).0, "Enter came as a key already");
    s.key(Key::Enter, NO);
    s.text("a\nb");
    s.key(Key::Home, Mods { shift: true, ..NO });
    assert_eq!(s.read(pid), "\x1b[A\x03é\ra\rb\x1b[1;2H".as_bytes());
    // Application cursor keys, and a bracketed paste, as the program asked.
    s.app.term.feed(b"\x1b[?1h\x1b[?2004h");
    s.key(Key::Left, NO);
    s.text("p\x1bq");
    assert_eq!(s.read(pid), b"\x1bOD\x1b[200~pq\x1b[201~");
}

#[test]
fn the_terminal_shows_its_programs_answers_their_queries_and_does_their_asks() {
    use ui::kernel::wire::Msg;
    // Cooked, the kernel puts a CR before each LF; raw, output is as written.
    let (mut s, pid) = console(false);
    assert_eq!(s.wrote(pid, b"one\ntwo"), "");
    assert_eq!((s.row(0), s.row(1), s.app.term.cursor()), ("one".into(), "two".into(), (1, 3)));
    let (mut s, pid) = console(true);
    s.wrote(pid, b"x\ny");
    assert_eq!((s.row(1), s.app.term.cursor()), (" y".into(), (1, 2)));
    // A query's answer goes back to the console.
    s.wrote(pid, b"\x1b[6n");
    assert_eq!(s.read(pid), b"\x1b[2;3R");
    // Asks: an app opens, a known theme applies.
    let asks = b"\x1b]1729;open;editor:/tmp/a\x07\x1b]1729;theme;Dawn\x07\x1b]1729;theme;Sepia\x07";
    assert_eq!(s.wrote(pid, asks), "open editor:/tmp/a; theme Dawn");
    // The wheel scrolls back by whole rows; text snaps back.
    (0..40).for_each(|i| _ = s.wrote(pid, format!("line {i}\r\n").as_bytes()));
    let sb = s.app.term.scrollback_len();
    assert!(sb > 10 && s.wheel(-17.0 * 3.0) && s.app.scroll == 3, "{sb}");
    assert!(!s.wheel(8.5), "half a row");
    assert!(s.wheel(8.5) && s.app.scroll == 2);
    assert!(s.wheel(-1e9) && s.app.scroll == sb);
    assert!(!s.wheel(f32::NAN) && s.app.scroll == sb);
    assert!(s.both(AppEvent::Text("x".into())).0 && s.app.scroll == 0, "text snaps back");
    s.app.term.feed(b"\x1b[?1049h");
    assert!(!s.wheel(-34.0), "no scrollback on the alternate screen");
    // The shell's clean end closes the window; a failure stays, saying so.
    s.kernel.message(&mut s.fs, pid, &Msg::Exit { status: 0 }.encode());
    assert_eq!((s.ev(AppEvent::Io), s.app.pid), ("CloseSelf".into(), None));
    let (mut s, pid) = console(false);
    s.wrote(pid, b"half");
    s.kernel.message(&mut s.fs, pid, &Msg::Exit { status: 3 }.encode());
    assert_eq!(s.ev(AppEvent::Io), "");
    assert_eq!([s.row(0), s.row(1)], ["half", "[sh stopped with status 3]"]);
}

#[test]
fn terminal_draws_cells_in_every_theme() {
    let (mut ts, mut t) = (text_system(), Terminal::default());
    ts.add_fallback(SYM_A.to_vec()).unwrap();
    let styled = "\x1b[1;31mred\x1b[m \x1b[7minv\x1b[m \x1b[4;9mls\x1b[m \x1b[2mdim\x1b[m \
                  \x1b[8mhid\x1b[m ✻⏺╭─╮日\r\n\x1b[48;2;10;20;30m  \x1b[m";
    t.term.feed(styled.as_bytes());
    let rect = RectF::new(10.0, 40.0, W80 + 160.0, H24);
    // Before Mono arrives: the same grid and fills, but no glyphs.
    let mut boot = TextSystem::new(SANS.to_vec()).unwrap();
    let early = draw(&mut t, &mut boot, rect, MIDNIGHT).0;
    assert_eq!((t.term.cols(), t.term.rows()), (100, 24));
    for theme in &THEMES {
        let (list, hits) = draw(&mut t, &mut ts, rect, theme);
        assert_eq!((t.term.cols(), t.term.rows(), hits.len()), (100, 24, 1));
        // Every visible char but the hidden ones and the one in no font (a box).
        let ink = inks(&list);
        assert_eq!((ink.len(), of(&list, Kind::Border).count()), (16, 1));
        // Bold makes red bright; dim is dim; inverse puts the opaque surface in front.
        let (red, dim, opaque) = (theme.ansi[9], theme.text.with_alpha(153), theme.surface);
        let n = |c: Rgba| ink.iter().filter(|&&i| i == c).count();
        assert_eq!([red, dim, opaque.with_alpha(255)].map(n), [3; 3]);
        let fills: Vec<_> = of(&list, Kind::Fill).map(|i| (i.rect, i.color)).collect();
        let has = |r: [f32; 4], color| fills.contains(&(r, color));
        assert!(has([56.0, 54.0, 24.0, 17.0], theme.text), "inverse: text behind");
        assert!(has([24.0, 71.0, 16.0, 17.0], Rgba(10, 20, 30, 255)));
        assert!(has([40.0, 71.0, 8.0, 17.0], theme.accent), "a steady block cursor");
        assert_eq!(fills.iter().filter(|f| f.0[3] == 1.0).count(), 4, "lines");
        if theme == MIDNIGHT {
            let same = early.instances().iter().eq(of(&list, Kind::Fill));
            assert!(same, "before Mono: the same fills, and no glyphs");
        }
        let (list, _) = frame(&mut t, &mut ts, rect, UiState::default(), theme);
        let ring = |i: &&Instance| i.kind == 1.0 && i.color == theme.accent;
        assert_eq!(list.instances().iter().filter(ring).count(), 1, "unfocused: an outline");
    }
    // On a char the block cursor shows it in the accent's own ink.
    let mut t = Terminal::default();
    t.term.feed(b"ab\x1b[D");
    let list = draw(&mut t, &mut ts, rect, MIDNIGHT).0;
    assert_eq!(inks(&list).iter().filter(|&&i| i == MIDNIGHT.accent_text).count(), 1);
}

/// The glyph colors of a frame, in draw order.
fn inks(list: &DrawList) -> Vec<Rgba> {
    of(list, Kind::Glyph).map(|i| i.color).collect()
}

#[test]
fn the_terminal_has_a_grid_a_title_an_icon_and_a_size_and_draws_in_every_theme() {
    let sizes = [(W80, H24), (0.0, -5.0), (f32::NAN, f32::INFINITY)];
    assert_eq!(sizes.map(|(w, h)| grid_size(w, h, 8.0, 17.0)), [(80, 24), (1, 1), (1, 500)]);
    let mut t = Terminal::default();
    assert_eq!((t.preferred_size(), t.title()), (Some((W80, H24)), "Terminal".into()));
    t.term.feed(b"\x1b]2;notes\x07");
    assert_eq!(t.title(), "Terminal — notes");
    assert!(t.wants_text_input() && open("launcher").is_none(), "the shell owns the launcher");
    // About, Feedback, Files, Settings and Welcome are programs (the system crate), not built in.
    assert_eq!(NAMES, ["terminal"]);
    let programs = ["about", "feedback", "files", "files:~/apps", "settings", "welcome"];
    assert!(programs.iter().all(|n| open(n).is_none()));
    let (app, icon) = (
        open("terminal").unwrap(),
        ui::AppIcon { glyph: Glyph::Terminal, hue: Rgba::hex(0x2dd4bf) },
    );
    assert_eq!((app.icon(), app.preferred_size(), app.compact()), (icon, Some((W80, H24)), false));
    for dpr in [1.0, 1.5, 2.0] {
        let mut ts = text_system();
        ts.set_dpr(dpr);
        for theme in &THEMES {
            let mut app = open("terminal").unwrap();
            for (w, h) in [(720.0, 520.0), (360.0, 640.0)] {
                let r = RectF::new(0.0, 36.0, w, h);
                // Under the pointer and held: the first widget.
                let hits = frame(app.as_mut(), &mut ts, r, UiState::default(), theme).1;
                let id = hits.first().map(|h| h.id);
                let state = UiState { hover: id, pressed: id, focused: true, now_ms: 1e4 };
                assert!(!frame(app.as_mut(), &mut ts, r, state, theme).0.is_empty());
            }
        }
    }
}

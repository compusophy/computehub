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
/// The content size of an 80 x 24 terminal at dpr 1, and its prompt.
const W80: f32 = 668.0;
const H24: f32 = 436.0;
const PROMPT: &str = "guest@compusophy:~$";
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
    fn keys(&mut self, keys: &[Key]) {
        keys.iter().for_each(|&k| _ = self.key(k, NO));
    }
    fn text(&mut self, s: &str) -> String {
        self.ev(AppEvent::Text(s.into()))
    }
    fn wheel(&mut self, dy: f32) -> bool {
        self.both(AppEvent::Wheel { x: 0.0, y: 0.0, dy }).0
    }
}

impl Sim<Terminal> {
    /// A terminal resized to `w` x `h` (its first event asks for fonts), drawn there if `drawn`.
    fn term(w: f32, h: f32, drawn: bool) -> Sim<Terminal> {
        let mut s = Sim::new(Terminal::default());
        assert_eq!(s.ev(AppEvent::Resized { w, h }), "fonts");
        if drawn {
            draw(&mut s.app, &mut text_system(), RectF::new(0.0, 0.0, w, h), MIDNIGHT);
        }
        s
    }
    /// Types `s` and Enter.
    fn line(&mut self, s: &str) -> String {
        self.text(s) + &self.key(Key::Enter, NO)
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

#[test]
fn terminal_line_editing_history_wheel_wrapping_and_greeting_at_first_grid() {
    let mut s = Sim::term(W80, H24, true);
    s.has("compusophyOS terminal — type 'help'.\nguest@compusophy:~$");
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
    // The shell reaches the desktop: a theme, an app.
    s.key(Key::Char('c'), CTRL);
    assert_eq!(s.line("theme dawn") + "," + &s.line("open settings"), "theme Dawn,open settings");
    // The wheel scrolls back by whole rows; a key snaps back.
    (0..30).for_each(|i| _ = s.line(&format!("echo line {i}")));
    let sb = s.app.term.scrollback_len();
    assert!(sb > 30, "{sb}");
    assert!(s.wheel(-17.0 * 3.0) && s.app.scroll == 3);
    assert!(!s.wheel(8.5), "half a row");
    assert!(s.wheel(8.5) && s.app.scroll == 2);
    assert!(s.wheel(-1e9) && s.app.scroll == sb);
    assert!(!s.wheel(f32::NAN) && s.app.scroll == sb);
    assert!(s.both(AppEvent::Key { key: Key::Char('x'), mods: NO }).0);
    assert_eq!(s.app.scroll, 0, "a key snaps back");
    s.app.term.feed(b"\x1b[?1049h");
    assert!(!s.wheel(-34.0), "no scrollback on the alternate screen");
    // It wraps like the screen.
    let mut s = Sim::term(28.0 + 8.0 * 30.0, H24, true);
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
    // At a phone's width (25 columns) the greeting breaks between words.
    let s = Sim::term(230.0, 400.0, true);
    assert_eq!(s.app.term.cols(), 25);
    assert_eq!([s.row(0), s.row(1)], ["compusophyOS terminal —", "type 'help'."]);
    assert_eq!(s.at(), (2, 20, PROMPT.into()));
    // A key before any draw greets first.
    let mut s = Sim::term(W80, H24, false);
    s.text("ls");
    let rows: String = (0..3).map(|r| s.row(r).replace(' ', "")).collect();
    assert!(rows.contains("compusophyOSterminal—type'help'."), "{rows}");
    assert_eq!(s.at().2, PROMPT.to_string() + " ls");
}

#[test]
fn terminal_runs_programs_in_the_foreground() {
    use ui::kernel::{Effect, wire::Msg};
    let mut s = Sim::term(W80, H24, true);
    s.kernel.set_isolated(true);
    s.fs.mkdir("/bin").and(s.fs.write("/bin/hi", b"#!wasm bin/toolbox.wasm")).unwrap();
    let worker = |s: &mut Sim<Terminal>, pid, msg: Msg<'_>| {
        s.kernel.message(&mut s.fs, pid, &msg.encode());
        s.both(AppEvent::Io).0
    };
    // Output comes on Io, `\n` as CR LF, above the typed-ahead line; the exit status shows.
    s.line("hi");
    s.text("ab");
    assert!(worker(&mut s, 2, Msg::ConsWrite { data: b"one\ntwo" }));
    s.has("$ hi\none\ntwoab");
    assert!(worker(&mut s, 2, Msg::Exit { status: 3 }));
    assert_eq!(s.at().2, "3 guest@compusophy:~$ ab");
    // Ctrl+C ends a program at once.
    s.key(Key::Char('c'), CTRL);
    s.line("hi");
    s.key(Key::Char('c'), CTRL);
    s.has("$ hi\n^C\n130 guest@compusophy:~$");
    assert!(s.kernel.take_effects().contains(&Effect::Kill { pid: 3 }));
    s.line("hi");
    // Raw output keeps `\n` as it is.
    s.app.output(b"x\ny", true, None);
    assert_eq!(s.at(), (s.at().0, 2, " y".into()));
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

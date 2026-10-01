use super::terminal::grid_size;
use super::*;
use gfx::{DrawList, Instance, Kind, RectF, Rgba};
use ui::{App, AppEvent, Cx, FontId, Hit, Key, Mods, Request, TextSystem, Ui, UiState, WidgetId};
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

/// An app, the VFS and kernel its [`Cx`]s are made from, and helpers that
/// show what it asked for as text.
struct Sim<A> {
    app: A,
    fs: Vfs,
    kernel: ui::kernel::Kernel,
}

impl<A: App> Sim<A> {
    fn new(app: A) -> Sim<A> {
        Sim { app, fs: Vfs::new(), kernel: ui::kernel::Kernel::new() }
    }
    /// Sends `ev`; returns whether the app redraws and its requests.
    fn both(&mut self, ev: AppEvent) -> (bool, String) {
        let mut cx = Cx::new(&mut self.fs, &mut self.kernel, 0.0);
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
    fn click(&mut self, id: u32) -> String {
        self.ev(AppEvent::Click(WidgetId(id)))
    }
}

impl Sim<Terminal> {
    /// A terminal resized to `w` x `h` (its first event, which asks for the
    /// fonts), then drawn there with every font if `drawn`.
    fn term(w: f32, h: f32, drawn: bool) -> Sim<Terminal> {
        let mut s = Sim::new(Terminal::default());
        assert_eq!(s.ev(AppEvent::Resized { w, h }), "fonts");
        if drawn {
            draw(&mut s.app, &mut text_system(), RectF::new(0.0, 0.0, w, h), true);
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
        Request::Open { name, floating } => {
            format!("{} {name}", ["open", "float"][*floating as usize])
        }
        Request::CloseSelf => "close".into(),
        Request::LoadFallbackFonts => "fonts".into(),
        Request::SetTheme(name) => format!("theme {name}"),
    }
}
/// A row of cells as text, trailing blanks trimmed.
fn text(cells: &[term::Cell]) -> String {
    let s: String = cells.iter().filter(|c| c.width != 0).map(|c| c.ch).collect();
    s.trim_end().to_string()
}
/// One frame of `app` in `r` with this pointer state, in `theme`.
fn frame(app: &mut dyn App, ts: &mut TextSystem, r: RectF, state: UiState, theme: &Theme) -> Frame {
    let (mut list, mut hits) = (DrawList::new(), Vec::new());
    app.draw(&mut Ui::new(&mut list, ts, r, &mut hits, state, theme));
    (list, hits)
}
type Frame = (DrawList, Vec<Hit>);
fn draw(app: &mut dyn App, ts: &mut TextSystem, r: RectF, focused: bool) -> Frame {
    let state = UiState { focused, ..UiState::default() };
    frame(app, ts, r, state, MIDNIGHT)
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
fn hit(hits: &[Hit], id: u32) -> Hit {
    *hits.iter().find(|h| h.id == WidgetId(id)).expect("no such hit")
}

#[test]
fn terminal_line_editing_history_and_wheel() {
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
    for i in 0..30 {
        s.line(&format!("echo line {i}"));
    }
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
}

#[test]
fn terminal_wraps_like_the_screen_and_greets_at_first_grid() {
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
    // Output comes on Io with `\n` as CR LF, above the typed-ahead line, and
    // the exit status is shown. (Terminal replies and live resizes reach the
    // program from step 3 on.)
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
fn grid_title_and_icons() {
    assert_eq!(grid_size(W80, H24, 8.0, 17.0), (80, 24));
    assert_eq!(grid_size(0.0, -5.0, 8.0, 17.0), (1, 1));
    assert_eq!(grid_size(f32::NAN, f32::INFINITY, 8.0, 17.0), (1, 500));
    let mut t = Terminal::default();
    assert_eq!((t.preferred_size(), t.title()), (Some((W80, H24)), "Terminal".into()));
    t.term.feed(b"\x1b]2;notes\x07");
    assert_eq!(t.title(), "Terminal — notes");
    assert!(t.wants_text_input() && open("launcher").is_none(), "the shell owns the launcher");
    // Every icon's glyph is ASCII: the boot font has it.
    let icons: Vec<_> = NAMES
        .iter()
        .map(|&name| {
            let app = open(name).unwrap();
            let (icon, size) = (app.icon(), app.preferred_size().unwrap());
            assert!(icon.glyph.bytes().all(|b| b.is_ascii_graphic() || b == b' '), "{name}");
            (icon.glyph, icon.hue, size)
        })
        .collect();
    let welcome = ("c", Rgba::hex(0xf472b6), (720.0, 420.0));
    let terminal = (">_", Rgba::hex(0x2dd4bf), (W80, H24));
    let settings = ("::", Rgba::hex(0x94a3b8), (720.0, 520.0));
    assert_eq!(icons, [welcome, terminal, settings]);
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
    let early = draw(&mut t, &mut boot, rect, true).0;
    assert_eq!((t.term.cols(), t.term.rows()), (100, 24));
    for theme in &THEMES {
        let state = UiState { focused: true, ..UiState::default() };
        let (list, hits) = frame(&mut t, &mut ts, rect, state, theme);
        assert_eq!((t.term.cols(), t.term.rows(), hits.len()), (100, 24, 1));
        // Every visible char but the hidden ones, the one in no font (a box)
        // and the one under the cursor (none: it is past the text).
        assert_eq!((of(&list, Kind::Glyph).count(), of(&list, Kind::Border).count()), (16, 1));
        let red = of(&list, Kind::Glyph).filter(|i| i.color == theme.ansi[9]);
        assert_eq!(red.count(), 3, "bold makes red bright");
        let dim = of(&list, Kind::Glyph).filter(|i| i.color == theme.text.with_alpha(153));
        assert_eq!(dim.count(), 3, "dim");
        let fills: Vec<_> = of(&list, Kind::Fill).map(|i| (i.rect, i.color)).collect();
        let has = |r: [f32; 4], color| fills.contains(&(r, color));
        assert!(has([56.0, 54.0, 24.0, 17.0], theme.text), "inverse: text behind");
        let opaque = theme.surface.with_alpha(255);
        let inverse = of(&list, Kind::Glyph).filter(|i| i.color == opaque).count();
        assert_eq!(inverse, 3, "inverse: the surface in front");
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
    let list = draw(&mut t, &mut ts, rect, true).0;
    let ink = of(&list, Kind::Glyph).filter(|i| i.color == MIDNIGHT.accent_text);
    assert_eq!(ink.count(), 1);
}

/// The glyph colors of a frame, in draw order.
fn inks(list: &DrawList) -> Vec<Rgba> {
    of(list, Kind::Glyph).map(|i| i.color).collect()
}

/// Checks that every glyph of `list` is in a readable ink of `t` (never the
/// faint one) or white on an icon, and every shape lies on device pixels.
fn refined(list: &DrawList, t: &Theme, dpr: f32, what: &str) {
    let white = Rgba(255, 255, 255, 255);
    let ok = [t.text, t.text_dim, t.accent, white, Rgba(0, 0, 0, 56)];
    for ink in inks(list) {
        assert!(ok.contains(&ink), "{what} in {}: ink {ink:?}", t.name);
    }
    let on = |v: f32| ((v * dpr) - (v * dpr).round()).abs() < 1e-3;
    let kinds = [Kind::Fill, Kind::Border, Kind::Gradient].map(|k| k as u8 as f32);
    for i in list.instances().iter().filter(|i| kinds.contains(&i.kind)) {
        let [x, y, w, h] = i.rect;
        let edges = [x, y, x + w, y + h];
        assert!(edges.iter().all(|&v| on(v)), "{what} in {}: {i:?} at {dpr}", t.name);
    }
}

#[test]
fn welcome_cards_open_apps_and_wrap() {
    let mut ts = text_system();
    let mut s = Sim::new(Welcome::default());
    let r = RectF::new(0.0, 36.0, 720.0, 420.0);
    let hits = draw(&mut s.app, &mut ts, r, true).1;
    assert_eq!(hits.iter().map(|h| h.id.0).collect::<Vec<_>>(), [1, 2, 3]);
    let ys: Vec<f32> = hits.iter().map(|h| h.rect.y).collect();
    assert!(ys.iter().all(|&y| y == ys[0]), "three across: {hits:?}");
    let opened = [s.click(1), s.click(2), s.click(3)].join(",");
    assert_eq!(opened, "open terminal,open studio,open settings");
    assert!(!s.wheel(50.0), "it all fits");
    assert!(!s.both(AppEvent::Key { key: Key::Enter, mods: NO }).0);
    // Narrow: the cards stack, each across the content, all in the window.
    let r = RectF::new(0.0, 0.0, 360.0, 720.0);
    let hits = draw(&mut s.app, &mut ts, r, true).1;
    let fits = |h: &Hit| h.rect.x >= 20.0 && h.rect.x + h.rect.w <= 340.0 && h.rect.w >= 300.0;
    assert!(hits.len() == 3 && hits.iter().all(fits), "{hits:?}");
    assert!(hits[0].rect.y < hits[1].rect.y && hits[1].rect.y < hits[2].rect.y);
    // Short: it scrolls, and the cards move up with it.
    let r = RectF::new(0.0, 0.0, 360.0, 300.0);
    let y0 = draw(&mut s.app, &mut ts, r, true).1[0].rect.y;
    assert!(s.wheel(40.0) && s.wheel(1e9) && !s.wheel(1.0));
    let y1 = draw(&mut s.app, &mut ts, r, true).1[0].rect.y;
    assert!(y1 < y0 - 40.0, "{y0} {y1}");
}

#[test]
fn every_app_is_refined_in_every_theme() {
    for dpr in [1.0, 1.5, 2.0] {
        let mut ts = text_system();
        ts.set_dpr(dpr);
        for (theme, &name) in THEMES.iter().flat_map(|t| NAMES.iter().map(move |n| (t, n))) {
            let mut app = open(name).unwrap();
            for (w, h) in [(720.0, 520.0), (360.0, 640.0)] {
                let r = RectF::new(0.0, 36.0, w, h);
                // Under the pointer and held: the first widget.
                let hits = frame(app.as_mut(), &mut ts, r, UiState::default(), theme).1;
                let id = hits.first().map(|h| h.id);
                let state = UiState { hover: id, pressed: id, focused: true, now_ms: 0.0 };
                let (list, _) = frame(app.as_mut(), &mut ts, r, state, theme);
                assert!(!list.is_empty(), "{name}");
                if name != "terminal" {
                    refined(&list, theme, dpr, name);
                }
            }
        }
    }
}

#[test]
fn settings_switches_pages_and_themes() {
    let mut ts = text_system();
    let mut s = Sim::new(Settings::default());
    assert_eq!(s.app.title(), "Settings");
    let r = RectF::new(0.0, 36.0, 720.0, 520.0);
    for (i, theme) in THEMES.iter().enumerate() {
        let state = UiState { focused: true, ..UiState::default() };
        let (list, hits) = frame(&mut s.app, &mut ts, r, state, theme);
        let ids: Vec<u32> = hits.iter().map(|h| h.id.0).collect();
        assert_eq!(ids, [1, 2, 10, 11, 12], "nav, then the theme cards");
        // The current theme's card wears a 2 px accent ring outside it.
        let card = hit(&hits, 10 + i as u32).rect;
        let ring = |b: &&Instance| b.color == theme.accent && b.p0 == 2.0;
        let rings: Vec<_> = of(&list, Kind::Border).filter(ring).collect();
        assert_eq!(rings.len(), 1, "{}", theme.name);
        assert_eq!(rings[0].rect, [card.x - 3.0, card.y - 3.0, card.w + 6.0, card.h + 6.0]);
        // Each miniature shows its own theme's light, whatever the current.
        let lit = |t: &Theme| t.glows.iter().filter(|g| g.color.3 > 0).count();
        assert_eq!(of(&list, Kind::Glow).count(), THEMES.iter().map(lit).sum::<usize>());
    }
    // Three across, then fewer as the window narrows.
    let mut rows = |s: &mut Sim<Settings>, w| {
        let hits = draw(&mut s.app, &mut ts, RectF::new(0.0, 0.0, w, 900.0), true).1;
        let cards: Vec<f32> = hits.iter().filter(|h| h.id.0 >= 10).map(|h| h.rect.y).collect();
        1 + cards.windows(2).filter(|p| p[1] > p[0]).count()
    };
    assert_eq!([rows(&mut s, 720.0), rows(&mut s, 560.0), rows(&mut s, 360.0)], [1, 2, 2]);
    assert_eq!([s.click(10), s.click(12)].join(","), "theme Midnight,theme Mono");
    // About: the version and the stack, taller than the window: it scrolls.
    assert!(!s.both(AppEvent::Click(WidgetId(1))).0, "already there");
    assert!(s.both(AppEvent::Click(WidgetId(2))).0 && s.app.page == 1);
    let (list, hits) = draw(&mut s.app, &mut ts, r, true);
    assert_eq!(hits.len(), 2, "only the nav");
    assert!(inks(&list).len() > 400, "the stack and the credits");
    assert!(s.wheel(100.0) && s.wheel(1e9) && !s.wheel(5.0));
    assert!(draw(&mut s.app, &mut ts, r, true).0.instances() != list.instances());
    // A narrow window has tabs instead of the nav, and both still switch.
    let tabs = draw(&mut s.app, &mut ts, RectF::new(0.0, 0.0, 360.0, 640.0), true).1;
    let (a, b) = (hit(&tabs, 1).rect, hit(&tabs, 2).rect);
    assert!(a.y == b.y && a.w == b.w && b.x > a.x, "{a:?} {b:?}");
    assert!(s.both(AppEvent::Click(WidgetId(1))).0 && s.app.page == 0);
}

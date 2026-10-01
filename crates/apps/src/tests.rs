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
    fn click(&mut self, id: u32) -> String {
        self.ev(AppEvent::Click(WidgetId(id)))
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
fn hit(hits: &[Hit], id: u32) -> Hit {
    *hits.iter().find(|h| h.id == WidgetId(id)).expect("no such hit")
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

/// Checks that every glyph of `list` is in a readable ink of `t` (never the
/// faint one) or white on an icon, and every shape lies on device pixels.
fn refined(list: &DrawList, t: &Theme, dpr: f32, what: &str) {
    let ok = [t.text, t.text_dim, t.accent, Rgba(255, 255, 255, 255), Rgba(0, 0, 0, 56)];
    inks(list).iter().for_each(|i| assert!(ok.contains(i), "{what} in {}: ink {i:?}", t.name));
    let on = |v: f32| ((v * dpr) - (v * dpr).round()).abs() < 1e-3;
    let kinds = [Kind::Fill, Kind::Border, Kind::Gradient].map(|k| k as u8 as f32);
    for i in list.instances().iter().filter(|i| kinds.contains(&i.kind)) {
        let [x, y, w, h] = i.rect;
        assert!([x, y, x + w, y + h].into_iter().all(on), "{what} in {}: {i:?} at {dpr}", t.name);
    }
}

#[test]
fn apps_have_grids_titles_icons_and_sizes_and_are_refined_in_every_theme() {
    let sizes = [(W80, H24), (0.0, -5.0), (f32::NAN, f32::INFINITY)];
    assert_eq!(sizes.map(|(w, h)| grid_size(w, h, 8.0, 17.0)), [(80, 24), (1, 1), (1, 500)]);
    let mut t = Terminal::default();
    assert_eq!((t.preferred_size(), t.title()), (Some((W80, H24)), "Terminal".into()));
    t.term.feed(b"\x1b]2;notes\x07");
    assert_eq!(t.title(), "Terminal — notes");
    assert!(t.wants_text_input() && open("launcher").is_none(), "the shell owns the launcher");
    // Every icon's glyph is ASCII: the boot font has it.
    let sizes = [(720.0, 420.0), (W80, H24), (720.0, 520.0)];
    let icons = [("c", 0xf472b6), (">_", 0x2dd4bf), ("::", 0x94a3b8)];
    assert_eq!(NAMES, ["welcome", "terminal", "settings"]);
    for ((name, (glyph, hue)), size) in NAMES.iter().zip(icons).zip(sizes) {
        let (app, icon) = (open(name).unwrap(), ui::AppIcon { glyph, hue: Rgba::hex(hue) });
        assert!(glyph.bytes().all(|b| b.is_ascii_graphic() || b == b' '), "{name}");
        assert_eq!((app.icon(), app.preferred_size()), (icon, Some(size)));
    }
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
fn welcome_cards_open_apps_and_wrap_and_settings_switches_pages_and_themes() {
    let mut ts = text_system();
    let mut s = Sim::new(Welcome::default());
    let r = RectF::new(0.0, 36.0, 720.0, 420.0);
    let hits = draw(&mut s.app, &mut ts, r, MIDNIGHT).1;
    assert_eq!(hits.iter().map(|h| h.id.0).collect::<Vec<_>>(), [1, 2, 3]);
    let ys: Vec<f32> = hits.iter().map(|h| h.rect.y).collect();
    assert!(ys.iter().all(|&y| y == ys[0]), "three across: {hits:?}");
    let opened = [s.click(1), s.click(2), s.click(3)].join(",");
    assert_eq!(opened, "open terminal,open studio,open settings");
    assert!(!s.wheel(50.0), "it all fits");
    assert!(!s.both(AppEvent::Key { key: Key::Enter, mods: NO }).0);
    // Narrow: the cards stack, each across the content, all in the window.
    let r = RectF::new(0.0, 0.0, 360.0, 720.0);
    let hits = draw(&mut s.app, &mut ts, r, MIDNIGHT).1;
    let fits = |h: &Hit| h.rect.x >= 20.0 && h.rect.x + h.rect.w <= 340.0 && h.rect.w >= 300.0;
    assert!(hits.len() == 3 && hits.iter().all(fits), "{hits:?}");
    assert!(hits[0].rect.y < hits[1].rect.y && hits[1].rect.y < hits[2].rect.y);
    // Short: it scrolls, and the cards move up with it.
    let r = RectF::new(0.0, 0.0, 360.0, 300.0);
    let y0 = draw(&mut s.app, &mut ts, r, MIDNIGHT).1[0].rect.y;
    assert!(s.wheel(40.0) && s.wheel(1e9) && !s.wheel(1.0));
    let y1 = draw(&mut s.app, &mut ts, r, MIDNIGHT).1[0].rect.y;
    assert!(y1 < y0 - 40.0, "{y0} {y1}");
    // Settings switches pages and themes.
    let mut s = Sim::new(Settings::default());
    assert_eq!(s.app.title(), "Settings");
    let r = RectF::new(0.0, 36.0, 720.0, 520.0);
    for (i, theme) in THEMES.iter().enumerate() {
        let (list, hits) = draw(&mut s.app, &mut ts, r, theme);
        let ids: Vec<u32> = hits.iter().map(|h| h.id.0).collect();
        assert_eq!(ids, [1, 2, 3, 10, 11, 12], "nav, then the theme cards");
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
        let hits = draw(&mut s.app, &mut ts, RectF::new(0.0, 0.0, w, 900.0), MIDNIGHT).1;
        let cards: Vec<f32> = hits.iter().filter(|h| h.id.0 >= 10).map(|h| h.rect.y).collect();
        1 + cards.windows(2).filter(|p| p[1] > p[0]).count()
    };
    assert_eq!([rows(&mut s, 720.0), rows(&mut s, 560.0), rows(&mut s, 360.0)], [1, 2, 2]);
    assert_eq!([s.click(10), s.click(12)].join(","), "theme Midnight,theme Mono");
    // About: the version and the stack, taller than the window: it scrolls.
    assert!(!s.both(AppEvent::Click(WidgetId(1))).0, "already there");
    assert!(s.both(AppEvent::Click(WidgetId(3))).0 && s.app.page == 2);
    let (list, hits) = draw(&mut s.app, &mut ts, r, MIDNIGHT);
    assert_eq!(hits.len(), 3, "only the nav");
    assert!(inks(&list).len() > 400, "the stack and the credits");
    assert!(s.wheel(100.0) && s.wheel(1e9) && !s.wheel(5.0));
    assert!(draw(&mut s.app, &mut ts, r, MIDNIGHT).0.instances() != list.instances());
    // A narrow window has tabs instead of the nav, and both still switch.
    let tabs = draw(&mut s.app, &mut ts, RectF::new(0.0, 0.0, 360.0, 640.0), MIDNIGHT).1;
    let (a, b) = (hit(&tabs, 1).rect, hit(&tabs, 2).rect);
    assert!(a.y == b.y && a.w == b.w && b.x > a.x, "{a:?} {b:?}");
    assert!(s.both(AppEvent::Click(WidgetId(1))).0 && s.app.page == 0);
}

#[test]
fn settings_offers_the_free_ai_models_and_marks_the_one_in_use() {
    let (mut ts, mut s) = (text_system(), Sim::new(Settings::default()));
    assert!(s.both(AppEvent::Click(WidgetId(2))).0 && s.app.page == 1);
    // The nav, then a row per model: no fields, so no text input. The ring marks the model in
    // use (none named yet: the default).
    let mut look = |s: &mut Sim<Settings>, w| {
        let (list, hits) = draw(&mut s.app, &mut ts, RectF::new(0.0, 36.0, w, 520.0), MIDNIGHT);
        let ids: Vec<u32> = hits.iter().map(|h| h.id.0).collect();
        let ring = |b: &&Instance| b.color == MIDNIGHT.accent && b.p0 == 2.0;
        let rings: Vec<[f32; 4]> = of(&list, Kind::Border).filter(ring).map(|b| b.rect).collect();
        let around = |id| {
            let r = hit(&hits, id).rect;
            [r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0]
        };
        let on = (20..22).filter(|&id| rings == [around(id)]).collect::<Vec<u32>>();
        (ids, on, hits)
    };
    let (ids, on, hits) = look(&mut s, 720.0);
    assert_eq!((ids, on), (vec![1, 2, 3, 20, 21], vec![20]));
    assert!(!s.app.wants_text_input());
    let (a, b) = (hit(&hits, 20).rect, hit(&hits, 21).rect);
    assert!(a.x == b.x && a.w == b.w && b.y >= a.y + a.h && a.w <= 440.0, "{a:?} {b:?}");
    // A click sets the preference; the status follows at once, and the ring.
    assert_eq!(s.click(21), "pref ai.model=zai/glm-5.3-flash");
    assert!(s.ai.model == "zai/glm-5.3-flash" && s.app.model == s.ai.model);
    assert_eq!(look(&mut s, 720.0).1, [21]);
    // A model from the host shows at the app's next event; narrow, the rows fill the width.
    s.ai.model = "zai/glm-5.3".into();
    assert!(s.both(AppEvent::Focus(true)).0, "a new status redraws");
    assert!(!s.both(AppEvent::Focus(true)).0);
    let (_, on, hits) = look(&mut s, 360.0);
    assert!(on == [20] && hit(&hits, 20).rect.w == 320.0, "{hits:?}");
}

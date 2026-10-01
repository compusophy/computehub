use std::cell::RefCell;
use std::rc::Rc;

use gfx::{Instance, Kind};
use ui::Key::*;
use ui::{App, AppEvent as E, Cx, FontId, Sense, Ui, UiState, WidgetId as W};
use wm::Dir;

use super::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");
const SYM_A: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-a.ttf");
/// The probe's own fill, so its content is easy to count.
const INK: Rgba = Rgba(1, 2, 3, 255);
/// Big enough that a few glyphs overflow the atlas at dpr 4.
const BIG: TextStyle = TextStyle::new(FontId::Mono, 240.0, INK);
const PORT: u16 = 8123;
/// The names the default registry knows.
const KNOWN: &str = "welcome terminal /apps/counter.app launcher long huge nan big";

/// What a probe saw: an event, or (`Err`) a frame it drew, its rect and state.
type Seen = Result<E, (RectF, UiState)>;
type Log = Rc<RefCell<Vec<(u32, Seen)>>>;

/// A scripted app: instance number (1, 2, ... as created, so window `n`
/// while every open succeeds), name, log, and whether it wants text input.
/// It logs all it sees; draws an INK button (hit 1, Click) at (10, 10),
/// 60 x 20, in its content, hit 3 (Click) over the button's right 20 px and
/// hit 2 (Text) 30 px lower; and runs `;`-separated text commands. An
/// "eager" probe connects at its first event, as a paired terminal does; a
/// "grid" probe, whose grid changes when it draws, then sends on socket 1;
/// a "busy" probe asks to redraw at every event.
struct Probe(u32, &'static str, Log, bool);

impl App for Probe {
    fn title(&self) -> String {
        match self.1 {
            "long" => "A window title far too long for any titlebar at all".into(),
            name => name.to_uppercase(),
        }
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let r = ui.rect();
        let at = |dx, dy, w| RectF::new(r.x + dx, r.y + dy, w, 20.0);
        ui.fill(at(10.0, 10.0, 60.0), 0.0, INK);
        ui.hit(W(1), at(10.0, 10.0, 60.0), Sense::Click);
        ui.hit(W(3), at(50.0, 10.0, 20.0), Sense::Click);
        ui.hit(W(2), at(10.0, 40.0, 60.0), Sense::Text);
        if self.1 == "big" {
            ui.text(r.x, r.y + 300.0, "ABCD", BIG);
        }
        self.2.borrow_mut().push((self.0, Err((r, ui.state()))));
    }

    fn event(&mut self, ev: E, cx: &mut Cx<'_>) -> bool {
        // Whether it drew since its last event; `None` before its first.
        let log = self.2.borrow();
        let drew = log.iter().rfind(|e| e.0 == self.0).map(|e| e.1.is_err());
        drop(log);
        self.2.borrow_mut().push((self.0, Ok(ev.clone())));
        let port = cx.pairing.map_or(1, |p| p.port);
        let cmds = match (&ev, self.1, drew) {
            (E::Text(t), ..) => t.as_str(),
            (_, "eager", None) => "connect",
            (_, "grid", Some(true)) => "send 1",
            _ => "",
        };
        for cmd in cmds.split(';') {
            let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
            let id = SocketId(arg.parse().unwrap_or(0));
            match verb {
                "open" => cx.open(arg),
                "float" => cx.open_floating(arg),
                "close" => cx.close_self(),
                "connect" => drop(cx.connect(port)),
                "send" => cx.send(id, b"hi".to_vec()),
                "drop" => cx.close_socket(id),
                "fonts" => cx.load_fallback_fonts(),
                "text" => self.3 = !self.3,
                "write" => cx.vfs.write("/tmp/probe", arg.as_bytes()).unwrap(),
                "now" => cx.vfs.write("/tmp/now", &cx.now_ms.to_le_bytes()).unwrap(),
                _ => {}
            }
        }
        self.1 == "busy" || matches!(ev, E::Click(_) | E::Wheel { .. } | E::Text(_))
    }

    fn wants_text_input(&self) -> bool {
        self.3
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        let names = ["launcher", "huge", "nan"];
        let i = names.iter().position(|n| *n == self.1)?;
        Some([(400.0, 300.0), (5e3, 5e3), (f32::NAN, 100.0)][i])
    }
}

/// Probes for the space-separated names in `known`; `name:alias` opens
/// `name` as a probe called `alias`.
fn registry(log: &Log, known: &'static str) -> Registry {
    let (log, n) = (log.clone(), Rc::new(RefCell::new(0)));
    Box::new(move |name| {
        let named = |k: &&str| k.split(':').next() == Some(name);
        let k = known.split(' ').find(named)?;
        *n.borrow_mut() += 1;
        let probe = Probe(*n.borrow(), k.rsplit(':').next()?, log.clone(), false);
        Some(Box::new(probe) as Box<dyn App>)
    })
}

fn fonts() -> TextSystem {
    let mut text = TextSystem::new(SANS.to_vec()).unwrap();
    text.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    text.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    text
}
/// A pairing on `port`; none for port 0.
fn pairing(port: u16) -> Option<Pairing> {
    ui::parse_pairing(&format!("node={port}&token={}", "0f".repeat(32)))
}
/// A desktop of the probes in `known`, paired on `port` unless it is 0.
fn desk_of(w: f32, h: f32, known: &'static str, port: u16) -> (Shell, Log) {
    let log = Log::default();
    let reg = registry(&log, known);
    (Shell::new(w, h, fonts(), Vfs::new(), reg, pairing(port)), log)
}
/// The default desktop, its startup report taken and its log cleared.
fn desk() -> (Shell, Log) {
    let (mut s, log) = desk_of(1280.0, 800.0, KNOWN, PORT);
    assert_eq!(s.input(Input::PointerLeave).text_input, Some(false));
    log.borrow_mut().clear();
    (s, log)
}
fn take(log: &Log) -> Vec<(u32, Seen)> {
    std::mem::take(&mut *log.borrow_mut())
}
/// The events logged since the last take.
fn evs(log: &Log) -> Vec<(u32, E)> {
    take(log).into_iter().filter_map(|(n, s)| Some((n, s.ok()?))).collect()
}
/// The frames drawn since the last take: rects and UI states.
fn frames(log: &Log) -> Vec<(u32, RectF, UiState)> {
    let drew = |(n, s): (u32, Seen)| s.err().map(|(r, st)| (n, r, st));
    take(log).into_iter().filter_map(drew).collect()
}
/// Modifiers named by letter: `a`lt, `m`eta, `s`hift, `c`trl.
fn mods(m: &str) -> Mods {
    let mut mods = Mods::default();
    [mods.shift, mods.ctrl, mods.alt, mods.meta] = ['s', 'c', 'a', 'm'].map(|c| m.contains(c));
    mods
}
fn resized(c: RectF) -> E {
    E::Resized { w: c.w, h: c.h }
}
fn ws_open(id: u32, port: u16) -> Effect {
    let url = format!("ws://127.0.0.1:{port}/");
    Effect::WsOpen { id, url }
}
/// "hi" sent on socket 1.
fn hi() -> Effect {
    let bytes = b"hi".to_vec();
    Effect::WsSend { id: 1, bytes }
}
fn rc(r: &Response) -> (bool, bool) {
    (r.redraw, r.consumed)
}
fn mid(r: RectF) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}
fn count(all: &[Instance], kind: Kind, color: Rgba) -> usize {
    let kind = kind as u8 as f32;
    let hit = |i: &&Instance| i.kind == kind && i.color == color;
    all.iter().filter(hit).count()
}
fn is_glyph(i: &Instance) -> bool {
    i.kind == Kind::Glyph as u8 as f32
}

/// Test drivers, as methods so a point can be computed from the shell in
/// the same call: `s.click(s.at(2, 20.0, 15.0))`.
impl Shell {
    fn k(&mut self, key: Key, m: &str) -> Response {
        let mods = mods(m);
        self.input(Input::Key { key, mods })
    }
    fn say(&mut self, t: &str) -> Response {
        self.input(Input::Text(t.to_string()))
    }
    fn down(&mut self, (x, y): (f32, f32)) -> Response {
        self.input(Input::PointerDown { x, y, button: 0 })
    }
    fn up(&mut self, (x, y): (f32, f32)) -> Response {
        self.input(Input::PointerUp { x, y, button: 0 })
    }
    fn move_to(&mut self, (x, y): (f32, f32)) -> Response {
        self.input(Input::PointerMove { x, y })
    }
    fn click(&mut self, at: (f32, f32)) -> Response {
        self.down(at);
        self.up(at)
    }
    fn rect_of(&self, n: u32) -> Option<Rect> {
        let layout = self.wm().layout();
        layout.iter().find(|p| p.win == WinId(n)).map(|p| p.rect)
    }
    fn content(&self, n: u32) -> RectF {
        content_rect(rectf(self.rect_of(n).unwrap()))
    }
    /// A point `(dx, dy)` into window `n`'s content.
    fn at(&self, n: u32, dx: f32, dy: f32) -> (f32, f32) {
        let c = self.content(n);
        (c.x + dx, c.y + dy)
    }
    /// Centers of window `n`'s float and close buttons.
    fn buttons(&self, n: u32) -> [(f32, f32); 2] {
        let r = rectf(self.rect_of(n).unwrap());
        title_buttons(r, WinId(n)).unwrap().map(|b| mid(b.0))
    }
    /// Center of a panel button.
    fn spot(&self, t: Target) -> (f32, f32) {
        let targets = self.panel_targets();
        mid(targets.iter().find(|p| p.1 == t).unwrap().0)
    }
    fn drawn(&mut self) -> Vec<Instance> {
        let mut list = DrawList::new();
        self.draw(&mut list);
        list.instances().to_vec()
    }
    fn names(&self) -> Vec<(u32, &str)> {
        let wins = self.host.wins().iter();
        wins.map(|w| (w.id.0, &*w.name)).collect()
    }
}

#[test]
fn startup_opens_the_apps_and_focuses_welcome() {
    let (mut s, log) = desk_of(1280.0, 800.0, KNOWN, PORT);
    let (g, n) = (s.wm().gaps(), s.wm().workspace_count());
    assert_eq!((g.outer, g.inner, n), (10, 10, 4));
    assert_eq!(s.wm().area(), Rect::new(0, 36, 1280, 764));
    assert!(s.wm().layout().iter().all(|p| !p.floating));
    let names = [(1, "welcome"), (2, "terminal"), (3, "/apps/counter.app")];
    assert_eq!(s.names(), names);
    assert_eq!((s.wm().focused(), s.clear_color()), (Some(WinId(1)), BG));
    assert_eq!(s.rect_of(1), Some(Rect::new(10, 46, 625, 744)));
    assert_eq!(s.content(1), RectF::new(11.0, 76.0, 623.0, 713.0));
    // Each app learns its focus and size before it first draws.
    let size = |n| (n, resized(s.content(n)));
    let want = [(1, E::Focus(true)), size(1), size(2), size(3)];
    assert_eq!(evs(&log), want);
    let first = s.input(Input::PointerLeave);
    assert_eq!(rc(&first), (false, false));
    assert_eq!((first.text_input, first.effects), (Some(false), vec![]));
    assert_eq!(s.input(Input::PointerLeave), Response::default());
    // Unknown names open nothing; startup's effects wait for take_effects.
    let (mut s, _) = desk_of(1280.0, 800.0, "welcome:eager /apps/counter.app:eager", 0);
    assert_eq!(s.names(), [(1, "welcome"), (2, "/apps/counter.app")]);
    assert_eq!(s.take_effects(), [ws_open(1, 1), ws_open(2, 1)]);
    assert!(s.take_effects().is_empty() && s.input(Input::PointerLeave).effects.is_empty());
    let (s, _) = desk_of(99.6, 36.4, "", 0);
    assert_eq!(s.wm().area(), Rect::new(0, 36, 100, 0));
}

#[test]
fn every_binding_reaches_the_wm() {
    let f = WinId(0); // stands for the window focused at the time
    let mut table = vec![
        (Enter, "a", Cmd::Open { floating: false }),
        (Enter, "as", Cmd::Open { floating: true }),
        (Char('f'), "m", Cmd::ToggleFloat),
        (Char('o'), "a", Cmd::ToggleOrientation),
    ];
    let dirs = [Dir::Left, Dir::Down, Dir::Up, Dir::Right];
    let keys = "hjkl".chars().map(Char).chain([Left, Down, Up, Right]);
    for (k, d) in keys.zip(dirs.into_iter().cycle()) {
        table.push((k, "a", Cmd::FocusDir(d)));
        table.push((k, "as", Cmd::MoveDir(d)));
        table.push((k, "ac", Cmd::Resize { dir: d, px: 40 }));
    }
    table.extend([
        (Char('3'), "as", Cmd::MoveToWorkspace { win: f, ws: 2 }),
        (Char('3'), "m", Cmd::SwitchWorkspace(2)),
        (Char('q'), "a", Cmd::Close(f)),
        (Char('1'), "ma", Cmd::SwitchWorkspace(0)),
        (Char('4'), "a", Cmd::SwitchWorkspace(3)),
    ]);
    let ((mut s, log), mut kinds) = (desk(), Vec::new());
    s.click(s.at(3, 300.0, 200.0));
    for (k, m, cmd) in table {
        let mut want = s.wm().clone();
        let focused = want.focused().unwrap_or(f);
        let cmd = match cmd {
            Cmd::Close(_) => Cmd::Close(focused),
            Cmd::MoveToWorkspace { ws, .. } => Cmd::MoveToWorkspace { win: focused, ws },
            c => c,
        };
        let (before, kind) = (want.state_hash(), std::mem::discriminant(&cmd));
        let _ = want.apply(cmd);
        let r = s.k(k, m);
        assert_eq!(s.wm().state_hash(), want.state_hash(), "{k:?} {m}");
        assert_eq!(rc(&r), (want.state_hash() != before, true), "{k:?} {m}");
        if r.redraw && !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    assert_eq!(kinds.len(), 9, "a binding never changed the wm");
    // Bindings never reach an app, and closed windows drop their apps.
    assert!(!evs(&log).iter().any(|e| matches!(e.1, E::Key { .. })));
    let live: usize = (0..4).map(|ws| s.wm().layout_of(ws).len()).sum();
    let wins = s.host.wins();
    assert!(wins.len() == live && wins.iter().all(|w| s.wm().workspace_of(w.id).is_some()));
}

#[test]
fn other_keys_go_to_the_focused_app() {
    let (mut s, log) = desk();
    let hash = s.wm().state_hash();
    // Unbound keys, with or without Alt and Meta, are the app's.
    let unbound = [(Escape, "a"), (Char('5'), "a"), (Char('0'), "m")];
    let more = [(Enter, "ac"), (Char('q'), "as"), (Char('v'), "ms")];
    let ctrl = [(Tab, ""), (Char('i'), "c"), (Char('a'), "c")];
    let insert = [(Insert, ""), (Insert, "c"), (Insert, "cs")];
    for (key, m) in unbound.into_iter().chain(more).chain(ctrl).chain(insert) {
        let mods = mods(m);
        assert_eq!(rc(&s.k(key, m)), (false, true), "{key:?} {m}");
        assert_eq!(evs(&log), [(1, E::Key { key, mods })], "{key:?} {m}");
    }
    // Paste keys go to the browser, as the platform leaves them; the text
    // comes after. A terminal sent Shift+Insert too would paste twice.
    let paste = [(Char('v'), "c"), (Char('v'), "cs"), (Char('v'), "m")];
    for (key, m) in paste.into_iter().chain([(Insert, "s")]) {
        assert_eq!(s.k(key, m), Response::default(), "{key:?} {m}");
    }
    assert_eq!(rc(&s.say("pasted")), (true, true));
    assert_eq!(evs(&log), [(1, E::Text("pasted".into()))]);
    // Reload and the developer tools are the browser's unless the app
    // wants text input.
    let browser = [(F(5), ""), (F(12), ""), (Char('r'), "c")];
    let more = [(Char('r'), "cs"), (Char('i'), "cs")];
    for (key, m) in browser.into_iter().chain(more) {
        let mods = mods(m);
        assert!(!s.k(key, m).consumed, "{key:?} {m}");
        assert_eq!(evs(&log), [(1, E::Key { key, mods })]);
        s.say("text");
        assert!(s.k(key, m).consumed, "{key:?} {m}");
        s.say("text");
        take(&log);
    }
    assert_eq!(s.wm().state_hash(), hash);
    // With no window focused, keys and text pass through.
    s.k(Char('2'), "a");
    take(&log);
    assert_eq!(s.k(Char('a'), ""), Response::default());
    assert_eq!(s.say("x"), Response::default());
    assert_eq!(s.say(""), Response::default());
    assert_eq!(evs(&log), []);
}

#[test]
fn focus_and_text_input_are_reported() {
    let (mut s, log) = desk();
    assert_eq!(s.say("text").text_input, Some(true));
    assert_eq!(s.say("nothing").text_input, None);
    assert_eq!(s.k(Right, "a").text_input, Some(false));
    assert_eq!(s.k(Left, "a").text_input, Some(true));
    assert_eq!(s.k(Char('2'), "a").text_input, Some(false));
    assert_eq!(s.k(Char('1'), "a").text_input, Some(true));
    assert_eq!(s.say("text").text_input, Some(false));
    take(&log);
    // Focus moves reach both apps; a workspace with none loses it.
    s.click(s.at(3, 300.0, 200.0));
    let moved = [(1, E::Focus(false)), (3, E::Focus(true))];
    assert_eq!(evs(&log)[..2], moved);
    s.k(Char('2'), "a");
    assert_eq!(evs(&log), [(3, E::Focus(false))]);
    s.k(Enter, "a");
    assert_eq!(evs(&log)[0], (4, E::Focus(true)));
    // A new screen size resizes every app, on every workspace.
    s.input(Input::Resize { w: 999.0, h: 700.0 });
    let sized = evs(&log);
    assert!(sized.len() == 4 && (1..=4).all(|n| sized.iter().any(|e| e.0 == n)));
}

#[test]
fn the_pointer_presses_clicks_and_hovers_widgets() {
    let (mut s, log) = desk();
    s.drawn();
    take(&log);
    let press = |x, y, id: Option<u32>| {
        let id = id.map(W);
        E::PointerDown { x, y, id }
    };
    // Pressing content focuses the window first; coordinates are relative.
    assert_eq!(rc(&s.down(s.at(2, 20.0, 15.0))), (true, true));
    let focus = [(1, E::Focus(false)), (2, E::Focus(true))];
    assert_eq!(evs(&log)[..2], focus);
    s.drawn();
    let st = frames(&log);
    let two = st.iter().find(|x| x.0 == 2).unwrap().2;
    let two = (two.hover, two.pressed, two.focused);
    assert_eq!(two, (Some(W(1)), Some(W(1)), true));
    assert!(st.iter().all(|x| x.0 == 2 || x.2 == UiState::default()));
    assert_eq!(rc(&s.up(s.at(2, 21.0, 16.0))), (true, true));
    assert_eq!(evs(&log), [(2, E::Click(W(1)))]);
    // The topmost hit wins; releasing elsewhere, or over a text hit, is no click.
    s.click(s.at(2, 55.0, 15.0));
    let want = [(2, press(55.0, 15.0, Some(3))), (2, E::Click(W(3)))];
    assert_eq!(evs(&log), want);
    s.down(s.at(2, 20.0, 15.0));
    s.up(s.at(2, 20.0, 35.0));
    s.click(s.at(2, 20.0, 45.0));
    s.click(s.at(2, 200.0, 200.0));
    let downs = [(20.0, 15.0), (20.0, 45.0), (200.0, 200.0)];
    let want = downs.into_iter().zip([Some(1), Some(2), None]);
    let want = want.map(|((x, y), id)| (2, press(x, y, id)));
    assert!(evs(&log).into_iter().eq(want));
    // A release over another window, or with another button, is no click.
    s.down(s.at(2, 20.0, 15.0));
    s.up(s.at(3, 20.0, 15.0));
    let (x, y) = s.at(2, 20.0, 15.0);
    s.down((x, y));
    s.input(Input::PointerUp { x, y, button: 2 });
    assert!(!evs(&log).iter().any(|e| matches!(e.1, E::Click(_))));
    // Titlebars are not content; hovering a widget redraws when it changes.
    let r = rectf(s.rect_of(3).unwrap());
    s.click((r.x + 100.0, r.y + 10.0));
    assert_eq!(evs(&log), [(2, E::Focus(false)), (3, E::Focus(true))]);
    assert!(s.move_to(s.at(3, 20.0, 15.0)).redraw);
    assert_eq!(s.app_hover, Some((WinId(3), W(1))));
    assert!(!s.move_to(s.at(3, 21.0, 15.0)).redraw);
    assert!(s.move_to(s.at(3, 55.0, 15.0)).redraw);
    assert!(s.move_to(s.at(3, 200.0, 200.0)).redraw);
    assert_eq!(s.app_hover, None);
    // The wheel goes to the window under the pointer, if any, with the
    // pointer relative to its content.
    let (x, y) = s.at(2, 50.0, 60.0);
    assert_eq!(rc(&s.input(Input::Wheel { x, y, dy: 40.0 })), (true, true));
    assert_eq!(evs(&log), [(2, E::Wheel { x: 50.0, y: 60.0, dy: 40.0 })]);
    for (x, y, dy) in [(x, 10.0, 1.0), (x, y, f32::NAN)] {
        assert_eq!(s.input(Input::Wheel { x, y, dy }), Response::default());
    }
    assert_eq!((evs(&log), s.wm().focused()), (vec![], Some(WinId(3))));
}

#[test]
fn a_frame_that_changes_a_grid_queues_what_the_app_sends() {
    let (mut s, log) = desk_of(1280.0, 800.0, "terminal:grid", 0);
    assert_eq!(s.say("connect").effects, [ws_open(1, 1)]);
    // After its first frame at a size the app hears the size again; what it
    // sends then (a terminal's RESIZE) waits for take_effects, once.
    s.drawn();
    assert_eq!(s.take_effects(), [hi()]);
    assert!(s.take_effects().is_empty() && s.input(Input::PointerLeave).effects.is_empty());
    s.input(Input::Resize { w: 900.0, h: 700.0 });
    s.drawn();
    s.drawn();
    // A response sooner than take_effects hands them out first instead.
    assert_eq!(s.say("").effects, [hi()]);
    // A new pixel ratio retells every app after its next frame.
    s.set_dpr(2.0);
    take(&log);
    s.drawn();
    s.set_dpr(2.0);
    s.drawn();
    assert_eq!(evs(&log), [(1, resized(s.content(1)))]);
}

#[test]
fn an_app_that_redraws_on_its_size_after_a_frame_is_drawn_again() {
    // As Welcome does to clamp its scroll: else the grown window's frame
    // stays stale until some unrelated event.
    let seen = |log: &Log| {
        let mine = take(log).into_iter().filter(|e| e.0 == 1);
        mine.map(|e| e.1.is_ok()).collect::<Vec<_>>()
    };
    let (mut s, log) = desk_of(1280.0, 800.0, "welcome:busy", 0);
    s.drawn();
    take(&log);
    s.input(Input::Resize { w: 900.0, h: 700.0 });
    s.drawn();
    assert_eq!(seen(&log), [true, false, true, false]); // Resized, frame, Resized, frame
    s.drawn();
    assert_eq!(seen(&log), [false]);
    // With no app asking, a frame is drawn once.
    let (mut s, log) = desk_of(1280.0, 800.0, "welcome", 0);
    take(&log);
    s.drawn();
    assert_eq!(seen(&log), [false, true]);
}

#[test]
fn apps_on_hidden_workspaces_ask_for_no_frames() {
    let (mut s, log) = desk_of(1280.0, 800.0, "welcome:busy", PORT);
    s.say("connect");
    s.k(Char('2'), "as");
    take(&log);
    // The app gets its data, but nothing on screen changed; shown, it draws.
    let (id, ev) = (1, WsEvent::Data(b"build output".to_vec()));
    let r = s.input(Input::Ws { id, ev: ev.clone() });
    assert_eq!((rc(&r), evs(&log).len()), ((false, false), 1));
    assert!(s.k(Char('2'), "a").redraw && s.input(Input::Ws { id, ev }).redraw);
}

#[test]
fn a_pairing_opens_a_terminal_that_connects_with_it() {
    let (mut s, _log) = desk_of(1280.0, 800.0, "welcome terminal:eager", 0);
    assert_eq!(s.take_effects(), [ws_open(1, 1)]);
    let r = s.set_pairing(pairing(PORT).unwrap());
    assert_eq!((r.redraw, r.effects), (true, vec![ws_open(2, PORT)]));
    let focus = s.wm().layout().into_iter().find(|p| p.focused);
    assert_eq!(focus.map(|p| (p.win, p.floating)), Some((WinId(3), false)));
    assert_eq!(s.names()[2], (3, "terminal"));
    // Terminals opened later connect with the latest pairing.
    assert_eq!(s.k(Enter, "a").effects, [ws_open(3, PORT)]);
    assert_eq!(s.set_pairing(pairing(7).unwrap()).effects, [ws_open(4, 7)]);
    assert_eq!(s.k(Enter, "as").effects, [ws_open(5, 7)]);
}

#[test]
fn ticks_set_the_clock_and_reach_every_app() {
    let (mut s, log) = desk();
    let clock = |s: &mut Shell| {
        let right = |i: &&Instance| is_glyph(i) && i.rect[0] > 1200.0;
        s.drawn().iter().filter(right).count()
    };
    assert_eq!(clock(&mut s), 0);
    let tick = |s: &mut Shell, minutes, now_ms| s.input(Input::Tick { minutes, now_ms });
    assert_eq!(rc(&tick(&mut s, 9 * 60 + 5, 1000.0)), (true, false));
    assert_eq!(clock(&mut s), 5); // "09:05"
    let at = |now_ms| E::Tick { now_ms };
    let ticks = evs(&log).into_iter().filter(|e| e.1 == at(1000.0));
    assert!(ticks.map(|e| e.0).eq([1, 2, 3]));
    assert!(!tick(&mut s, 9 * 60 + 5, f64::NAN).redraw);
    assert_eq!(evs(&log)[0], (1, at(1000.0)));
    assert!(tick(&mut s, 24 * 60 + 1, 2000.0).redraw);
    assert_eq!(s.clock, Some(1));
    // Between ticks the page clock moves by set_now, which sends no event:
    // a terminal's synchronized-output hold times out by it.
    take(&log);
    s.set_now(2500.0);
    s.set_now(f64::NAN);
    assert!(take(&log).is_empty());
    s.say("now");
    assert_eq!(s.vfs().read("/tmp/now"), Ok(&2500f64.to_le_bytes()[..]));
    s.drawn();
    assert!(frames(&log).iter().all(|f| f.2.now_ms == 2500.0));
    // Apps reach the filesystem through their context.
    s.say("write hello");
    assert_eq!(s.vfs().read("/tmp/probe"), Ok(&b"hello"[..]));
}

#[test]
fn titlebar_buttons_fire_only_on_release_over_the_same_button() {
    let (mut s, _log) = desk();
    let [float, close] = s.buttons(1);
    assert_eq!(rc(&s.down(close)), (true, true));
    assert_eq!(s.wm().focused(), Some(WinId(1)));
    s.up(float);
    s.down(close);
    s.up((300.0, 400.0));
    assert!(s.rect_of(1).is_some());
    s.say("connect");
    assert_eq!(s.click(close).effects, [Effect::WsClose { id: 1 }]);
    assert!(s.rect_of(1).is_none() && s.host.win(WinId(1)).is_none());
    // Window 2 now spans the top; its float button floats it in place.
    let ([float, close], r) = (s.buttons(2), s.rect_of(2));
    s.click(float);
    assert_eq!(s.wm().is_floating(WinId(2)), Some(true));
    assert_eq!((s.wm().focused(), s.rect_of(2)), (Some(WinId(2)), r));
    // Other buttons do nothing, but a chord's last release (button 2) disarms.
    let (x, y) = close;
    s.down(close);
    assert!(s.input(Input::PointerUp { x, y, button: 2 }).consumed);
    assert!(s.input(Input::PointerDown { x, y, button: 2 }).consumed);
    assert!(s.rect_of(2).is_some() && s.armed.is_none());
}

#[test]
fn panel_opens_apps_and_switches_workspaces() {
    let (mut s, _log) = desk();
    let launcher = s.spot(Target::Launcher);
    let terminal = s.spot(Target::Terminal);
    let ws: Vec<_> = (0..4).map(|i| s.spot(Target::Workspace(i))).collect();
    assert_eq!((launcher, terminal), ((18.0, 18.0), (50.0, 18.0)));
    // Centered: 578 + 4 * 28 + 3 * 4 = 702 = 1280 - 578.
    assert_eq!((ws[0], ws[3]), ((592.0, 18.0), (688.0, 18.0)));
    s.click(terminal);
    assert_eq!(s.wm().is_floating(WinId(4)), Some(false));
    s.click(launcher);
    assert_eq!(s.wm().is_floating(WinId(5)), Some(true));
    s.down(terminal);
    s.up(launcher);
    assert_eq!(s.names()[3..], [(4, "terminal"), (5, "launcher")]);
    s.click(ws[2]);
    assert_eq!(s.wm().active_workspace(), 2);
    s.down(ws[3]);
    s.up(ws[1]);
    assert_eq!(s.wm().active_workspace(), 2);
    // The bare panel swallows clicks.
    let hash = s.wm().state_hash();
    assert_eq!(s.hit(300.0, 10.0), Some(Target::Panel));
    s.click((300.0, 10.0));
    assert_eq!(s.wm().state_hash(), hash);
    // mod+Space focuses the launcher already open, on its workspace.
    assert!(s.k(Space, "a").redraw && s.wm().focused() == Some(WinId(5)));
    assert_eq!((s.wm().active_workspace(), s.host.wins().len()), (0, 5));
}

#[test]
fn dragging_a_floating_titlebar_moves_the_window() {
    let (mut s, _log) = desk();
    s.k(Enter, "as");
    let at = |x, y| Some(Rect::new(x, y, 853, 509));
    assert_eq!(s.rect_of(4), at(213, 163));
    s.down((253.0, 175.0));
    assert_eq!(rc(&s.move_to((303.0, 205.0))), (true, true));
    assert_eq!(s.rect_of(4), at(263, 193));
    // A chord's last release (button 2) ends the drag; the top stays below the panel.
    let (x, y) = (0.0, -500.0);
    s.move_to((x, y));
    s.input(Input::PointerUp { x, y, button: 2 });
    s.move_to((500.0, 500.0));
    assert_eq!(s.rect_of(4), at(-40, 36));
    s.down((0.0, 48.0));
    s.input(Input::PointerLeave);
    s.move_to((245.6, 172.0));
    assert_eq!(s.rect_of(4), at(206, 160));
    s.up((0.0, 0.0));
    s.move_to((900.0, 700.0));
    // Bodies and tiled titlebars focus without dragging.
    s.down((500.0, 500.0));
    s.move_to((600.0, 600.0));
    assert_eq!(s.rect_of(4), at(206, 160));
    let tiled = s.rect_of(1);
    s.down((20.0, 50.0));
    s.move_to((120.0, 150.0));
    assert_eq!((s.wm().focused(), s.rect_of(1)), (Some(WinId(1)), tiled));
    // A drag ends when its window stops floating.
    s.down((300.0, 170.0));
    assert!(s.drag.is_some());
    s.k(Char('f'), "a");
    s.move_to((400.0, 400.0));
    assert!(s.drag.is_none());
}

#[test]
fn hover_redraws_only_when_the_target_changes() {
    let (mut s, _log) = desk();
    assert_eq!(rc(&s.move_to((18.0, 18.0))), (true, true));
    assert_eq!(s.hover, Some(Target::Launcher));
    assert!(!s.move_to((20.0, 21.0)).redraw);
    assert!(s.move_to((300.0, 18.0)).redraw);
    assert!(!s.move_to((310.0, 18.0)).redraw);
    assert!(!s.move_to((300.0, 400.0)).redraw);
    assert!(s.move_to(s.buttons(2)[1]).redraw);
    assert_eq!(s.hover, Some(Target::Close(WinId(2))));
    assert_eq!(count(&s.drawn(), Kind::Fill, HOVER), 1);
    assert_eq!(rc(&s.input(Input::PointerLeave)), (true, false));
    assert_eq!(s.input(Input::PointerLeave), Response::default());
    assert_eq!(count(&s.drawn(), Kind::Fill, HOVER), 0);
    // Closing the hovered window by key moves the hover off it.
    s.move_to(s.buttons(1)[1]);
    s.k(Left, "a");
    assert_eq!(rc(&s.k(Char('q'), "a")), (true, true));
    assert_eq!(s.hover, None);
}

#[test]
fn draw_stays_on_screen_with_the_panel_on_top() {
    let (mut s, log) = desk();
    s.k(Enter, "as");
    take(&log);
    let mut list = DrawList::new();
    list.fill(RectF::new(0.0, 0.0, 5.0, 5.0), 0.0, ICON);
    s.draw(&mut list);
    let (all, shadow) = (list.instances(), Kind::Shadow as u8 as f32);
    assert_eq!(all[0].kind, shadow);
    for [x, y, w, h] in all.iter().filter(|i| i.kind != shadow).map(|i| i.rect) {
        assert!(x >= 0.0 && y >= 0.0 && x + w <= 1280.0 && y + h <= 800.0);
    }
    let bar = all.iter().position(|i| i.color == PANEL).unwrap();
    assert_eq!(all[bar].rect, [0.0, 0.0, 1280.0, 36.0]);
    assert!(all[bar..].iter().all(|i| i.rect[1] + i.rect[3] <= PANEL_H));
    let borders = [BORDER, BORDER_FOCUSED].map(|c| count(all, Kind::Border, c));
    assert_eq!(borders, [3, 1]);
    assert_eq!(count(all, Kind::Fill, ACCENT), 1);
    // A body and a content well each, and the app's own drawing.
    assert_eq!([WINDOW, INK].map(|c| count(all, Kind::Fill, c)), [8, 4]);
    let icons = [ICON, ICON_DIM].map(|c| count(all, Kind::Icon, c));
    assert_eq!(icons, [2 + 1, 3 * 2]);
    // Every app drew once into its content, clipped to it; titles are glyphs.
    let drew = frames(&log).into_iter().map(|f| (f.0, f.1));
    assert!(drew.eq([1, 2, 3, 4].map(|n| (n, s.content(n)))));
    let c = s.content(4);
    let ink = |i: &&Instance| i.color == INK && i.rect[0] == c.x + 10.0;
    let ink = all.iter().find(ink);
    assert_eq!(ink.unwrap().clip, [c.x, c.y, c.w, c.h]);
    let titles = all.iter().filter(|i| is_glyph(i) && i.rect[1] > PANEL_H);
    assert!(titles.count() > 20);
    // Occupied workspaces show brighter dots than empty ones.
    s.k(Char('2'), "as");
    let all = s.drawn();
    let dots = [ICON_DIM, ICON_DIM.with_alpha(150)].map(|c| count(&all, Kind::Fill, c));
    assert_eq!(dots, [1, 2]);
    // Tiles 30 px tall get no titlebar or content, 40 px wide no buttons.
    for (w, h, bars, wells, hits) in [(1000.0, 86.0, 0, 3, 0), (60.0, 400.0, 6, 6, 2)] {
        let (mut s, _log) = desk_of(w, h, KNOWN, PORT);
        let all = s.drawn();
        let fills = [WINDOW, TITLEBAR, TITLEBAR_FOCUSED].map(|c| count(&all, Kind::Fill, c));
        assert_eq!([fills[0], fills[1] + fills[2]], [wells, bars]);
        assert_eq!(count(&all, Kind::Icon, ICON), 1);
        assert!(s.host.wins().iter().all(|w| w.hits.len() == hits));
    }
}

#[test]
fn titles_are_cut_with_an_ellipsis() {
    let mut t = fonts();
    let style = TextStyle::new(FontId::SansBold, 13.0, TEXT);
    let mut cut = |s, room| t.ellipsize(s, style, room);
    assert_eq!(cut("Terminal", 200.0), "Terminal");
    assert_eq!(cut("wide", 1.0), "", "not even the ellipsis fits");
    let short = cut("A window title far too long", 80.0);
    assert!(short.ends_with('\u{2026}') && short.len() < 27, "{short}");
    assert!(t.measure(&short, style) <= 80.0);
    // In a window, the title stops before the buttons.
    let (mut s, _log) = desk();
    s.say("open long");
    let all = s.drawn();
    let r = rectf(s.rect_of(4).unwrap());
    let float_x = s.buttons(4)[0].0 - TITLE_BTN / 2.0;
    let bar = RectF::new(r.x, r.y, r.w, TITLEBAR_H);
    let on_bar = |i: &&Instance| is_glyph(i) && bar.contains(i.rect[0], i.rect[1]);
    let title = all.iter().filter(on_bar);
    assert!(title.map(|i| i.rect[0] + i.rect[2]).all(|e| e <= float_x));
}

#[test]
fn an_atlas_reset_draws_the_frame_again() {
    let (mut s, log) = desk();
    s.drawn();
    assert_eq!(frames(&log).len(), 3);
    s.say("open big");
    s.set_dpr(4.0);
    s.drawn();
    assert_eq!(frames(&log).len(), 2 * 4);
    assert!(!s.text_mut().take_atlas_reset());
}

#[test]
fn degenerate_input_never_panics() {
    let inf = f32::INFINITY;
    let bad = [0.0, -5.0, 20.0, 1e30, f32::NAN, inf, f32::MIN, f32::MAX];
    let finite = |i: &Instance| i.rect.iter().all(|v| v.is_finite());
    for (w, h) in bad.iter().flat_map(|&w| bad.map(|h| (w, h))) {
        let (mut s, _log) = desk_of(w, h, KNOWN, PORT);
        let a = s.wm().area();
        assert!(a.w >= 0 && a.h >= 0 && a.y == 36, "{w} {h}");
        assert!(s.drawn().iter().all(finite));
    }
    // A seeded run of mixed events, some at bad sizes and positions,
    // drawing after each.
    let ((mut s, _log), mut seed) = (desk(), 0x2545_f491_4f6c_dd1d_u64);
    let mut next = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n) as usize
    };
    let keys = [Enter, Char('q'), Char('f'), Space, Char('h'), Down, Other];
    let mods = ["", "a", "as", "ac", "m", "ms", "mc", "asc"];
    let cmds = "open terminal|float launcher|close|connect;send 1|fonts|text|float huge";
    let cmds: Vec<&str> = cmds.split('|').collect();
    let mut list = DrawList::new();
    for _ in 0..3000 {
        let (x, y) = (next(1400) as f32 - 60.0, next(900) as f32 - 60.0);
        let (x, y) = [(x, y), (bad[next(8)], bad[next(8)])][usize::from(next(8) == 0)];
        let (button, id) = (u8::from(next(4) == 0), next(6) as u32);
        let (minutes, now_ms) = (id, f64::from(x));
        let data = WsEvent::Data(vec![1]);
        match next(12) {
            0 | 1 => s.input(Input::PointerMove { x, y }),
            2 => s.input(Input::PointerDown { x, y, button }),
            3 => s.input(Input::PointerUp { x, y, button }),
            4 => s.input(Input::PointerLeave),
            5 if next(10) == 0 => s.input(Input::Resize { w: x, h: y }),
            6 => s.say(cmds[next(7)]),
            7 => s.input(Input::Wheel { x, y, dy: y }),
            8 => s.input(Input::Ws { id, ev: data }),
            9 if next(4) == 0 => s.fetched(id, Ok(SYM_A.to_vec())),
            10 => s.input(Input::Tick { minutes, now_ms }),
            _ => s.k(keys[next(7)], mods[next(8)]),
        };
        s.draw(&mut list);
        assert!(list.instances().iter().all(finite));
    }
}

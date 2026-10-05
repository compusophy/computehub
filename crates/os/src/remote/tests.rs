use super::*;
use crate::ai::{self, Ai, CHUNK};
use gfx::{DrawList, RectF};
use platform::{Ctl, Effect as Fx};
use ui::AppEvent::Resized;
use ui::kernel::{Effect as K, Kernel, wire};
use ui::{FontId, Hit, Request as R, THEMES, TextSystem, UiState};
use uiwire::{Class, Node, Span, Variant, mods};

const MONO: &[u8] = include_bytes!("../../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");

/// Studio editing a file, its kernel (isolated) and files.
struct Sys {
    r: Remote,
    k: Kernel,
    fs: Vfs,
    asked: Vec<R>,
}

impl Sys {
    /// With `start`, running: its first size (600 x 400) given, its Resize read.
    fn new(start: bool) -> Sys {
        let mut fs = Vfs::new();
        fs.mkdir("/bin").and(fs.write(STUDIO, b"#!wasm bin/studio.wasm\n")).unwrap();
        let mut k = Kernel::new();
        k.set_isolated(true);
        let argv = ["studio", "edit", "/apps/counter.app"].map(String::from).into();
        let r = Remote { types: true, ..Remote::new(STUDIO, argv, &Ai::default()) };
        let mut s = Sys { r, k, fs, asked: Vec::new() };
        if start {
            s.ev(AppEvent::Resized { w: 600.0, h: 400.0 });
            s.events();
        }
        s
    }

    /// Runs `f` with a [`Cx`], keeping what it asked for.
    fn cx<T>(&mut self, f: impl FnOnce(&mut Remote, &mut Cx<'_>) -> T) -> T {
        let mut cx = Cx::new(&mut self.fs, &mut self.k, 0.0);
        let out = f(&mut self.r, &mut cx);
        self.asked.extend(cx.take_requests());
        out
    }

    fn ev(&mut self, ev: AppEvent) -> bool {
        self.cx(|r, cx| r.event(ev, cx))
    }

    fn show(&mut self, nodes: Vec<Node>, requests: Vec<Request>) -> bool {
        let f = Frame { seq: 0, title: "T".into(), requests, nodes };
        self.cx(|r, cx| r.frame(2, &f.encode(), cx))
    }

    /// What the program reads now, an event a read.
    fn events(&mut self) -> Vec<Event> {
        let (mut out, mut n) = (Vec::new(), usize::MAX);
        while out.len() != n {
            n = out.len();
            self.k.message(&mut self.fs, 2, &[wire::EVENTS, 0, 0, 1, 0]);
            out.extend(self.k.take_effects().into_iter().filter_map(|e| match e {
                K::Reply { pid: 2, errno: 0, data } => Some(Event::decode(&data).expect("event")),
                _ => None,
            }));
        }
        out
    }

    /// One focused frame at 600 x 400 (content at the origin): the list and hits.
    fn draw(&mut self) -> (DrawList, Vec<Hit>) {
        let mut ts = TextSystem::new(crate::SANS.to_vec()).unwrap();
        ts.set_font(FontId::Mono, MONO.to_vec()).unwrap();
        let (mut list, mut hits) = (DrawList::new(), Vec::new());
        let state = UiState { focused: true, ..UiState::default() };
        let rect = RectF::new(0.0, 0.0, 600.0, 400.0);
        self.r.draw(&mut Ui::new(&mut list, &mut ts, rect, &mut hits, state, &THEMES[0]));
        (list, hits)
    }
}

fn text(t: &str) -> AppEvent {
    AppEvent::Text(t.into())
}

/// A key with the modifiers in `mods`: Ctrl, Meta and Shift as `c m s`.
fn key(key: Key, mods: &str) -> AppEvent {
    let [ctrl, meta, shift] = ['c', 'm', 's'].map(|m| mods.contains(m));
    AppEvent::Key { key, mods: Mods { ctrl, meta, shift, alt: false } }
}

fn press(id: u32) -> AppEvent {
    AppEvent::PointerDown { x: 0.0, y: 0.0, id: Some(WidgetId(id)) }
}

fn input(id: u32, value: &str) -> Node {
    Node::Input { id, value: value.into(), placeholder: "name".into() }
}

fn code(version: u32, text: &str, spans: Vec<Span>) -> Node {
    Node::Code { id: 4, version, line_numbers: true, text: text.into(), spans }
}

fn change(id: u32, version: u32, text: &str) -> Event {
    Event::Change { id, version, text: text.into() }
}

#[test]
fn names_open_studio_and_the_first_size_starts_it() {
    let open = |name: &str| open(name, &Ai::default());
    let argv = |name: &str| open(name).map(|a| (a.title(), a.icon(), a.preferred_size()));
    let studio = |title: &str| Some((title.into(), STUDIO_ICON, Some((880.0, 560.0))));
    assert_eq!(argv("studio"), studio("Studio"));
    assert_eq!(argv("studio:counter.app"), studio("Studio \u{2014} counter.app"));
    assert_eq!(argv("/tmp/x.app"), Some(("x.app".into(), APP_ICON, None)));
    assert!(["studio:", ".apps", ""].iter().all(|n| open(n).is_none()));
    assert_eq!(argv("assistant"), Some(("Assistant".into(), ASSISTANT_ICON, Some((560.0, 600.0)))));
    // About, Feedback and Files: bin/system.wasm, as their markers name it.
    let sys = |n: &str| open(n).map(|a| (a.title(), a.icon(), a.preferred_size(), a.compact()));
    let want = |i: usize, c| Some((SYSTEM[i].1.into(), SYSTEM[i].2, Some(SYSTEM[i].3), c));
    let got = [sys("about"), sys("feedback"), sys("files:~/a"), sys("editor:~/b.txt")];
    assert_eq!(got, [want(0, true), want(1, true), want(2, false), want(4, false)]);
    assert_eq!(sys("activity"), want(5, true));
    assert!(SYSTEM[2].2.glyph == Glyph::Folder && open("system").is_none());
    // Until their first frames, the windows of programs that open on a field take typing.
    let typing = |n: &str| open(n).is_some_and(|a| a.wants_text_input());
    let fields = ["studio", "studio:a.app", "assistant", "editor:~/b", "feedback", "terminal"];
    let none = ["about", "files:~/a", "welcome", "activity", "settings", "/tmp/x.app"];
    assert!(fields.iter().all(|n| typing(n)) && none.iter().all(|n| !typing(n)));
    // Before a frame: a still note, the title its own; no process yet; it takes typing.
    let mut s = Sys::new(false);
    assert!(s.r.wants_text_input() && s.k.procs().is_empty());
    let (list, hits) = s.draw();
    assert!(hits.is_empty() && !list.is_empty() && s.r.title().is_empty());
    // The first size starts it; once the worker is ready, Start carries the marker's URL.
    assert!(s.ev(AppEvent::Resized { w: 640.4, h: 1e9 }));
    assert_eq!(s.k.procs(), [(2, "studio".into(), true)]);
    assert_eq!(s.k.take_effects(), [K::Spawn { pid: 2, sab: true }]);
    s.k.message(&mut s.fs, 2, &wire::Msg::Ready { version: wire::VERSION }.encode());
    let (start, url) = (s.k.take_effects().pop(), ui::kernel::Load::Url("bin/studio.wasm".into()));
    assert!(matches!(start, Some(K::Start { pid: 2, program, .. }) if program == url));
    // The size is its first event, then the AI settings; the same size is no news, a new one is.
    [(640.0, 1e9), (500.0, 400.0)].into_iter().for_each(|(w, h)| _ = s.ev(Resized { w, h }));
    let config = Event::Config { model: ai::DEFAULT_MODEL.into() };
    let sized = [Event::Resize { w: 640, h: 65_535 }, config, Event::Resize { w: 500, h: 400 }];
    assert_eq!(s.events(), sized);
    // Frames of other pids are dropped. A first frame that does not decode is from a program
    // newer than the desktop, which says so; one after a frame is dropped.
    let f = Frame { title: "Mine".into(), ..Frame::default() }.encode();
    assert!(!s.cx(|r, cx| r.frame(3, &f, cx)) && s.r.note.is_empty());
    assert!(s.cx(|r, cx| r.frame(2, &f[1..], cx)) && s.r.note == NEWER && s.r.frame.is_none());
    assert!(s.cx(|r, cx| r.frame(2, &f, cx)) && s.r.title() == "Mine");
    assert!(!s.cx(|r, cx| r.frame(2, &f[1..], cx)) && s.r.title() == "Mine");
    // Activity runs the OS's own program, whatever its marker says. Its window may watch, end a
    // process (Studio's here, whose window a kill, 137, closes; Activity's runs on) and reset.
    let mut s = Sys::new(false);
    s.r = Remote { own: Some(OWN), ..Remote::new(STUDIO, vec!["activity".into()], &Ai::default()) };
    s.ev(AppEvent::Resized { w: 1.0, h: 1.0 });
    s.k.message(&mut s.fs, 2, &wire::Msg::Ready { version: wire::VERSION }.encode());
    let url = ui::kernel::Load::Url("bin/system.wasm".into());
    assert!(matches!(s.k.take_effects().pop(), Some(K::Start { program, .. }) if program == url));
    let mut studio = Remote::new(STUDIO, vec!["studio".into()], &Ai::default());
    studio.event(AppEvent::Resized { w: 1.0, h: 1.0 }, &mut Cx::new(&mut s.fs, &mut s.k, 0.0));
    s.show(vec![], vec![Request::Watch { on: true }, Request::End { pid: 3 }, Request::Reset]);
    s.r.ai.pump(&mut Ctl::default(), &mut s.k);
    let mut cx = Cx::new(&mut s.fs, &mut s.k, 0.0);
    assert!(!studio.event(AppEvent::Io, &mut cx) && cx.take_requests() == [R::CloseSelf]);
    assert!(s.r.ai.0.borrow().watch == Some(2) && s.k.procs() == [(2, "activity".into(), true)]);
    assert_eq!(s.asked, [R::Reset]);
    // A missing program says so and none starts; a failed one says why.
    let mut s = Sys::new(false);
    assert!(s.fs.remove(STUDIO, false).is_ok() && s.ev(AppEvent::Resized { w: 1.0, h: 1.0 }));
    assert_eq!((s.r.note.as_str(), s.k.procs().len()), ("/bin/studio: not found", 0));
    assert!(!s.r.wants_text_input());
    let mut s = Sys::new(true);
    s.k.message(&mut s.fs, 2, &wire::Msg::ConsWrite { data: b"panicked\n" }.encode());
    s.k.message(&mut s.fs, 2, &wire::Msg::Exit { status: 101 }.encode());
    assert!(s.ev(AppEvent::Io));
    let why = ("studio stopped with status 101", "panicked\n");
    assert_eq!((s.r.note.as_str(), s.r.log.as_str()), why);
    assert!(s.asked == [R::Agent(Request::Status { working: false })] && s.r.frame.is_none());
}

#[test]
fn clicks_keys_and_requests_go_through() {
    let mut s = Sys::new(true);
    let button = Node::Button { id: 1, variant: Variant::Primary, label: "Run".into() };
    let size = Request::Size { w: 300, h: 200 };
    let open = Request::Open { name: "studio:/apps/x.app".into() };
    assert!(s.show(vec![button.clone()], vec![size.clone(), open]));
    // Size only in the first frame.
    let (kind, text, context) = (String::from("idea"), String::from("more"), true);
    s.show(vec![button], vec![size, Request::Close, Request::Feedback { kind, text, context }]);
    let open = R::Open { name: "studio:/apps/x.app".into(), floating: false };
    let feedback = R::Feedback { kind: "idea".into(), text: "more".into(), context };
    assert_eq!(s.asked, [R::Size(300, 200), open, R::CloseSelf, feedback]);
    assert!(!s.ev(AppEvent::Click(WidgetId(1))) && !s.ev(AppEvent::Click(WidgetId(0))));
    assert!(s.ev(AppEvent::Focus(false)), "the window's focus, told");
    // Chords and Escape and Enter are keys; plain keys are not.
    assert!(!s.ev(key(Key::Char('s'), "c")));
    let plain = [key(Key::Char('s'), ""), key(Key::Enter, ""), key(Key::Space, "ms")];
    plain.into_iter().chain([key(Key::F(5), "")]).for_each(|ev| _ = s.ev(ev));
    let k = |key, mods, ch| Event::Key { id: 0, key, mods, ch };
    let (char, ms) = (uiwire::Key::Char, mods::META | mods::SHIFT);
    let keys = [k(char, mods::CTRL, 's'), k(uiwire::Key::Enter, 0, '\0'), k(char, ms, ' ')];
    let want = [&[Event::Click { id: 1 }, Event::Focus { on: false }][..], &keys].concat();
    assert_eq!(s.events(), want);
    // Acts go to the host, answers to the program; busy while starting or until a click is drawn.
    assert!(Sys::new(true).r.busy() && !Sys::new(false).r.busy() && s.r.busy());
    let act = Request::Act { id: 4, act: uiwire::Act::Wait { ms: 0 }.encode() };
    s.show(vec![], vec![act.clone(), Request::Status { working: true }]);
    assert_eq!(s.asked[4..], [R::Agent(act), R::Agent(Request::Status { working: true })]);
    let ev = [AppEvent::Click(WidgetId(1)), AppEvent::Agent(Event::Halt)].map(|e| s.ev(e));
    assert!(ev == [false; 2] && s.r.busy() && s.events() == [Event::Click { id: 1 }, Event::Halt]);
    assert!(s.show(vec![], vec![]) && !s.r.busy());
    // A program that asks for keys gets plain ones too, while no field has the keyboard; a
    // grid's press taps its square (a Tick out, after that one's frame); each busy till its
    // answer. The tap's Click, on what has the grid's id by then, is none.
    let grid = || vec![Node::Grid { id: 7, cols: 2, cells: vec![0; 4], texts: Vec::new() }];
    s.show(grid(), vec![Request::Keys { on: true }, Request::Timer { ms: 100 }]);
    assert!(!s.ev(key(Key::Left, "")) && s.events() == [k(uiwire::Key::Left, 0, '\0')]);
    assert!(s.r.busy() && s.show(grid(), vec![]) && !s.r.busy() && s.draw().1.len() == 1);
    [0.0, 100.0].into_iter().for_each(|now_ms| _ = s.ev(AppEvent::Tick { now_ms }));
    s.ev(AppEvent::PointerDown { x: 270.0, y: 25.0, id: Some(WidgetId(7)) });
    let again = vec![Node::Button { id: 7, variant: Variant::Normal, label: "Again".into() }];
    assert!(s.show(grid(), vec![]) && s.r.busy() && s.show(again, vec![]) && !s.r.busy());
    assert!(s.draw().1.len() == 1 && !s.ev(AppEvent::Click(WidgetId(7))));
    assert_eq!(s.events(), [Event::Tick { ms: 100 }, Event::Tap { id: 7, cell: 0 }]);
    assert_eq!((s.r.frame_in(150.0), s.r.frame_in(250.0)), (Some(50), Some(0))); // next Tick's
    // Closing says Close once; the program's exit then ends the window.
    (0..2).for_each(|_| s.cx(|r, cx| r.closing(cx)));
    assert_eq!(s.events(), [Event::Close]);
    s.k.message(&mut s.fs, 2, &wire::Msg::Exit { status: 0 }.encode());
    s.asked.clear();
    assert!([s.ev(AppEvent::Io), s.ev(AppEvent::Io)] == [false; 2] && s.asked == [R::CloseSelf]);
}

#[test]
fn a_quick_drag_across_a_busy_board_paints_every_unit_it_crossed() {
    // A canvas pad on a timer, ten units across: a press taps its unit, and the stroke goes on
    // while that tap is out, quicker than a frame's round trip; no later sample comes.
    let mut s = Sys::new(true);
    let canvas = |above: &str| {
        let label = Node::Text { id: 0, style: uiwire::Style::Title, text: above.into() };
        let pad = Node::Canvas { id: 9, w: 10, h: 10, draws: Vec::new() };
        if above.is_empty() { vec![pad] } else { vec![label, pad] }
    };
    s.show(canvas(""), vec![Request::Timer { ms: 100 }]);
    s.draw();
    s.ev(AppEvent::Tick { now_ms: 0.0 });
    let x = |s: &Sys, unit: f32| {
        let b = s.r.view.grids[0].rect;
        (b.x + b.w / 10.0 * (unit + 0.5), b.y + 1.0)
    };
    let stroke = |s: &mut Sys, units: &[f32]| {
        let (x0, y) = x(s, 0.0);
        s.ev(AppEvent::PointerDown { x: x0, y, id: Some(WidgetId(9)) });
        units.iter().for_each(|&u| assert!(!s.ev(AppEvent::Drag { x: x(s, u).0, y })));
    };
    stroke(&mut s, &[3.0, 6.0, 9.0]);
    assert_eq!(s.events(), [Event::Tap { id: 9, cell: 0 }]);
    // Answered with a label that moves the board down, the held end waits for that answer to
    // draw, then asks a frame and is tapped by it, where it was on the board, after the Tick
    // then due: each unit on the way, once, busy till all ten frames are in.
    assert!(s.show(canvas("Painted"), vec![]) && !s.r.play.held());
    s.draw();
    assert!(s.r.play.held() && s.r.frame_in(0.0) == Some(0));
    s.ev(AppEvent::Tick { now_ms: 100.0 });
    let taps = |cells: std::ops::Range<u32>| cells.map(|cell| Event::Tap { id: 9, cell });
    let ticked = [Event::Tick { ms: 100 }].into_iter().chain(taps(1..10)).collect::<Vec<_>>();
    assert!(s.events() == ticked && !s.r.play.held());
    let answer = |s: &mut Sys, n: usize| (0..n).all(|_| s.r.busy() && s.show(canvas("P"), vec![]));
    assert!(answer(&mut s, 10) && !s.r.busy());
    // A Tick that goes while a tap is out is answered after it: a drag then waits for both.
    stroke(&mut s, &[]);
    s.ev(AppEvent::Tick { now_ms: 200.0 });
    assert!(s.events() == [Event::Tap { id: 9, cell: 0 }, Event::Tick { ms: 100 }]);
    assert!(answer(&mut s, 1) && !s.r.busy());
    s.draw();
    s.ev(AppEvent::Drag { x: x(&s, 3.0).0, y: x(&s, 3.0).1 });
    assert!(s.events() == taps(1..4).collect::<Vec<_>>() && answer(&mut s, 4) && !s.r.busy());
    // Anything else sent after a stroke (Escape here) drops its held end: out of order.
    s.draw();
    stroke(&mut s, &[9.0]);
    s.ev(key(Key::Escape, ""));
    assert!(answer(&mut s, 1) && !s.r.busy());
    s.draw();
    assert!(!s.r.play.held() && s.r.frame_in(250.0) == Some(50));
    s.ev(AppEvent::Tick { now_ms: 250.0 });
    let esc = Event::Key { id: 0, key: uiwire::Key::Escape, mods: 0, ch: '\0' };
    assert_eq!(s.events(), [Event::Tap { id: 9, cell: 0 }, esc]);
}

#[test]
fn inputs_and_codes_keep_the_text_the_user_edits() {
    let mut s = Sys::new(true);
    s.show(vec![input(5, "a"), input(6, "z")], vec![]);
    assert!(s.ev(press(5)) && s.r.wants_text_input());
    assert!(s.ev(text("b\u{7}")) && !s.ev(text("\u{7}")));
    // One Change waits for a frame; the newest goes with it.
    s.ev(text("c"));
    assert_eq!(s.events(), [change(5, 1, "ab")]);
    s.show(vec![input(5, "a"), input(6, "z")], vec![]);
    assert_eq!(s.events(), [change(5, 2, "abc")]);
    // Enter submits after the waiting Change; Backspace edits.
    assert!(s.ev(key(Key::Backspace, "")) && !s.ev(key(Key::Enter, "")));
    assert_eq!(s.events(), [change(5, 3, "ab"), Event::Submit { id: 5 }]);
    // The focused input keeps its text; the others take the frame's.
    s.show(vec![input(5, "old"), input(6, "new")], vec![]);
    let texts = |s: &Sys| s.r.texts.inputs.iter().map(|i| i.1.clone()).collect::<Vec<_>>();
    assert_eq!(texts(&s), ["ab", "new"]);
    // Escape is a key and leaves the input, which then follows the frames.
    assert!(s.ev(key(Key::Escape, "")) && !s.r.wants_text_input());
    assert_eq!(s.events(), [Event::Key { id: 5, key: uiwire::Key::Escape, mods: 0, ch: '\0' }]);
    s.show(vec![input(5, "x")], vec![]);
    assert_eq!(texts(&s), ["x"]);
    // An input that leaves the frame takes the focus with it.
    assert!(s.ev(press(5)) && s.show(vec![], vec![]));
    assert!(!s.r.wants_text_input() && !s.ev(text("q")));
    // A Code: Enter echoed as text and Tab as text are one edit each.
    let kw = Span { start: 0, len: 5, class: Class::Keyword };
    s.show(vec![code(1, "state x", vec![kw])], vec![]);
    let got = |s: &Sys| (s.r.texts.codes[0].1.ed.text(), s.r.texts.codes[0].1.version);
    let gold = THEMES[0].ansi[3];
    let nums =
        |s: &mut Sys| s.draw().0.instances().iter().any(|i| i.kind == 4.0 && i.color == gold);
    let typed = [press(4), key(Key::End, ""), key(Key::Enter, ""), text("\n"), key(Key::Tab, "")];
    typed.into_iter().chain([text("\t"), text("y")]).for_each(|ev| _ = s.ev(ev));
    assert_eq!(got(&s).0, "state x\n  y");
    assert_eq!(s.events(), [change(4, 2, "state x\n")]);
    // A stale frame keeps the host's text and its spans go unused; the same version takes spans
    // (the `y`, byte 10, as a number); a newer one replaces the text.
    let num = |start| Span { start, len: 1, class: Class::Number };
    s.show(vec![code(2, "state x\n", vec![num(6)])], vec![]);
    assert_eq!(s.events(), [change(4, 4, "state x\n  y")]);
    assert_eq!(got(&s), ("state x\n  y".into(), 4));
    assert!(!nums(&mut s));
    s.show(vec![code(4, "state x\n  y", vec![kw, num(10)])], vec![]);
    assert!(nums(&mut s));
    s.show(vec![code(9, "new", vec![])], vec![]);
    assert_eq!(got(&s), ("new".into(), 9));
    // Ctrl+Enter is a key for the program, not a newline.
    s.ev(key(Key::Enter, "c"));
    let run = Event::Key { id: 4, key: uiwire::Key::Enter, mods: mods::CTRL, ch: '\0' };
    assert_eq!((s.events(), got(&s).0), (vec![run], "new".into()));
    // An Area: Enter is a new line (no echo), Ctrl+Enter a key; Close goes after the last edit.
    let area = |v: &str| Node::Area { id: 8, value: v.into(), placeholder: "".into() };
    s.show(vec![area("hi")], vec![]);
    assert!(s.ev(press(8)) && s.ev(key(Key::Enter, "")) && !s.ev(text("\n")) && s.ev(text("x")));
    assert_eq!(s.events(), [change(8, 1, "hi\n")]);
    s.ev(key(Key::Enter, "c"));
    let send = Event::Key { id: 8, key: uiwire::Key::Enter, mods: mods::CTRL, ch: '\0' };
    assert!(s.ev(text("y")) && s.cx(|r, cx| (r.closing(cx), true).1));
    assert_eq!(s.events(), [change(8, 2, "hi\nx"), send, change(8, 3, "hi\nxy"), Event::Close]);
}

#[test]
fn asks_wait_for_the_start_and_frames_move_the_keyboard() {
    // A prompt asked before the first size goes after it and the AI settings.
    let mut s = Sys::new(false);
    let ask = |t: &str| Event::Ask { text: t.into() };
    assert!(!s.ev(AppEvent::Ask("a timer".into())));
    s.ev(AppEvent::Resized { w: 600.0, h: 400.0 });
    let config = Event::Config { model: ai::DEFAULT_MODEL.into() };
    assert_eq!(s.events(), [Event::Resize { w: 600, h: 400 }, config, ask("a timer")]);
    // A frame puts the keyboard in its Input or Code; an id it lacks moves nothing.
    s.show(vec![input(5, ""), code(1, "a", vec![])], vec![Request::Focus { id: 4 }]);
    assert_eq!(s.r.texts.focus, 4);
    s.show(vec![input(5, "")], vec![Request::Focus { id: 5 }, Request::Focus { id: 9 }]);
    assert!(s.r.wants_text_input() && s.r.texts.focus == 5);
    // Once it runs, a prompt goes at once, after the Change that waits.
    ["x", "y"].into_iter().for_each(|t| _ = s.ev(text(t)));
    assert!(!s.ev(AppEvent::Ask("more".into())));
    assert_eq!(s.events(), [change(5, 1, "x"), change(5, 2, "xy"), ask("more")]);
}

#[test]
fn typing_before_the_first_frame_goes_into_the_field_it_focuses() {
    // As the page sends it: each character a key, then its text (an Enter's too, an echo).
    let typed = |s: &str| {
        let keys = s.chars().map(|c| [key(Key::Char(c), ""), text(&c.to_string())]);
        keys.collect::<Vec<_>>().concat()
    };
    let enter = [key(Key::Enter, ""), text("\n")];
    // Held while the program starts, chords too; its first frame's Input takes them as typed,
    // Enter its Submit; the next frame, nothing.
    let mut s = Sys::new(true);
    let ctrl_enter = key(Key::Enter, "c");
    let held = [typed("sn"), vec![key(Key::Backspace, "")], typed("nake"), vec![ctrl_enter]];
    let mut held = [&held.concat()[..], &enter].concat().into_iter();
    assert!(held.all(|ev| !s.ev(ev)) && s.r.wants_text_input() && s.events().is_empty());
    assert!(s.show(vec![input(5, "")], vec![Request::Focus { id: 5 }]));
    let run = Event::Key { id: 5, key: uiwire::Key::Enter, mods: mods::CTRL, ch: '\0' };
    let submit = Event::Submit { id: 5 };
    assert_eq!(s.events(), [change(5, 1, "s"), change(5, 7, "snake"), run, submit]);
    s.show(vec![input(5, "")], vec![Request::Focus { id: 5 }]);
    assert!(s.events().is_empty() && s.r.texts.inputs[0].1 == "snake");
    // Into an Area, Enter is a new line.
    let mut s = Sys::new(true);
    let area = || vec![Node::Area { id: 8, value: "".into(), placeholder: "".into() }];
    [typed("hi"), enter.to_vec(), typed("x")].concat().into_iter().for_each(|ev| _ = s.ev(ev));
    s.show(area(), vec![Request::Focus { id: 8 }]);
    s.show(area(), vec![]);
    assert_eq!(s.events(), [change(8, 1, "h"), change(8, 4, "hi\nx")]);
    // Later frames give nothing again: one between an Enter and its echo leaves one new line.
    let [down, echo] = enter.clone();
    assert!(s.ev(down) && s.show(area(), vec![]) && !s.ev(echo));
    assert_eq!(s.r.texts.areas[0].1.text, "hi\nx\n");
    // A first frame that focuses no field: Enter and the keys it asked for go as keys, text
    // nowhere, and the window takes typing no more.
    let mut s = Sys::new(true);
    [text("lost"), key(Key::Left, ""), key(Key::Enter, "")].into_iter().for_each(|ev| _ = s.ev(ev));
    let keys = vec![Request::Keys { on: true }];
    assert!(s.show(vec![input(5, "")], keys) && !s.r.wants_text_input());
    let k = |key| Event::Key { id: 0, key, mods: 0, ch: '\0' };
    assert_eq!(s.events(), [k(uiwire::Key::Left), k(uiwire::Key::Enter)]);
    s.show(vec![input(5, "")], vec![Request::Focus { id: 5 }]);
    assert!(s.events().is_empty() && s.r.texts.inputs[0].1.is_empty());
    // At most HELD bytes, and nothing after them: no Enter sends half a paste.
    let mut s = Sys::new(true);
    let full = "a".repeat(HELD);
    [text(&full), text("b"), key(Key::Enter, "")].into_iter().for_each(|ev| _ = s.ev(ev));
    s.show(vec![input(5, "")], vec![Request::Focus { id: 5 }]);
    assert_eq!(s.events(), [change(5, 1, &full)]);
    // The Terminal's console hears its keys from the start: none are held.
    let mut s = Sys::new(false);
    let term = Remote::new(TERMINAL, vec!["terminal".into()], &Ai::default());
    s.r = Remote { own: Some("bin/terminal.wasm"), tty: Some(Default::default()), ..term };
    s.ev(AppEvent::Resized { w: 1.0, h: 1.0 });
    let enter = Event::Key { id: 0, key: uiwire::Key::Enter, mods: 0, ch: '\0' };
    assert!(!s.ev(key(Key::Enter, "")) && s.events().ends_with(&[enter]));
}

#[test]
fn ai_requests_stream_back_to_the_program_that_asked() {
    let mut s = Sys::new(true);
    let end = |id, status, error: &str| Event::AiEnd { id, status, error: error.into() };
    // A frame's requests, then a pump: what the page is asked, what the program reads.
    let ask = |s: &mut Sys, requests: Vec<Request>| {
        s.show(vec![], requests);
        let mut ctl = Ctl::default();
        s.r.ai.pump(&mut ctl, &mut s.k);
        (ctl.effects().to_vec(), s.events())
    };
    let ai = |id| Request::Ai { id, body: "{}".into() };
    let config = |m: &str| Event::Config { model: m.into() };
    // A model saved goes to storage and to the program; one not on offer is the default.
    let mut ctl = Ctl::default();
    s.r.ai.set_model(&mut ctl, "zai/glm-5.3-flash");
    let stored = Fx::Store { key: ai::MODEL.into(), value: "zai/glm-5.3-flash".into() };
    assert_eq!(ctl.effects(), [stored]);
    let told = (vec![], vec![config("zai/glm-5.3-flash")]);
    assert_eq!((ask(&mut s, vec![]), s.r.ai.status().model), (told, ai::MODELS[1].into()));
    s.r.ai.set_model(&mut Ctl::default(), "openai/gpt-x");
    assert_eq!(ask(&mut s, vec![]).1, [config(ai::DEFAULT_MODEL)]);
    // Two at a time, to the one endpoint with no key, the chosen model first (a body's own, after
    // it, wins); the body back in pieces, then the end.
    let headers = vec![("Content-Type", "application/json".to_string())];
    let sent = |id, body: &str| Fx::Stream {
        id,
        url: ai::URL.into(),
        headers: headers.clone(),
        body: body.into(),
    };
    let stream = |id| sent(id, &[r#"{"model":""#, ai::DEFAULT_MODEL, r#""}"#].concat());
    let busy = (vec![stream(1), stream(2)], vec![end(3, 0, "busy")]);
    assert_eq!(ask(&mut s, vec![ai(1), ai(2), ai(3)]), busy);
    let mut own = Sys::new(true);
    let body = r#"{"model":"m","messages":[]}"#;
    let chosen = [r#"{"model":""#, ai::DEFAULT_MODEL, r#"","model":"m","messages":[]}"#].concat();
    assert_eq!(ask(&mut own, vec![Request::Ai { id: 1, body: body.into() }]).0, [sent(1, &chosen)]);
    let big = [vec![b'x'; CHUNK - 1], "\u{e9}".into()].concat();
    let ended = platform::Event::StreamEnd { id: 1, status: 429, error: "".into() };
    let chunk = platform::Event::Chunk { id: 1, data: big.clone() };
    [chunk, ended.clone(), ended].into_iter().for_each(|ev| s.r.ai.heard(&mut s.k, ev));
    let data = |data: &[u8]| Event::AiData { id: 1, data: data.to_vec() };
    assert_eq!(ask(&mut s, vec![]).1, [data(&big[..CHUNK]), data(&big[CHUNK..]), end(1, 429, "")]);
    // Watch, End and Reset from any window but the OS's own are dropped: never a Close to the hub.
    let no = vec![Request::Watch { on: true }, Request::End { pid: 2 }, Request::Reset];
    assert!(ask(&mut s, no).0.is_empty() && s.r.ai.0.borrow().watch.is_none());
    assert!(s.asked.is_empty());
    // A cancel aborts the stream and ends the request at once.
    let cancel = vec![Request::AiCancel { id: 4 }, Request::AiCancel { id: 2 }];
    assert_eq!(ask(&mut s, cancel), (vec![Fx::Abort(2)], vec![end(2, 0, "cancelled")]));
    // Closing aborts the rest, unanswered.
    assert_eq!(ask(&mut s, vec![ai(5)]), (vec![stream(3)], vec![]));
    s.cx(|r, cx| r.closing(cx));
    assert_eq!(ask(&mut s, vec![]), (vec![Fx::Abort(3)], vec![Event::Close]));
    // Counted: 3 asked (not the busy one), 1 failed, 2 with no receipt (cancelled, closed); then
    // a receipt a chunk boundary split, after more than the tail keeps.
    assert_eq!(s.r.ai.0.borrow().counts, [3, 1, 2, 0, 0, 0]);
    let b = [vec![b'x'; 99], b"\n: receipt in=1200 out=30 microusd=1812\n\n".to_vec()].concat();
    let mut s = Sys::new(true);
    ask(&mut s, vec![ai(1)]);
    let chunk = |d: &[u8]| platform::Event::Chunk { id: 1, data: d.to_vec() };
    let done = platform::Event::StreamEnd { id: 1, status: 200, error: "".into() };
    [chunk(&b[..120]), chunk(&b[120..]), done].into_iter().for_each(|e| s.r.ai.heard(&mut s.k, e));
    assert_eq!(s.r.ai.0.borrow().counts, [1, 0, 0, 1200, 30, 1812]);
    // A process that ended (Ctrl+C) stops its request at the next pump, before a byte came.
    assert_eq!(ask(&mut s, vec![ai(2)]).0, [stream(2)]);
    s.k.kill(2, wire::INTERRUPTED);
    assert_eq!(ask(&mut s, vec![]), (vec![Fx::Abort(2)], vec![]));
}

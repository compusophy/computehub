use super::*;
use crate::ai::{self, Ai, CHUNK};
use gfx::DrawList;
use platform::{Ctl, Effect as Fx};
use ui::kernel::{Effect as K, Kernel, wire};
use ui::{FontId, Hit, Request as R, THEMES, UiState};
use uiwire::{Class, Span, mods};

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
        let r = Remote::new(STUDIO, argv, &Ai::default());
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
    assert!(["studio:", "terminal", ".apps", ""].iter().all(|n| open(n).is_none()));
    let assistant = Some(("Assistant".into(), ASSISTANT_ICON, Some((560.0, 600.0))));
    assert_eq!(argv("assistant"), assistant);
    // Before a frame: a still note, the title its own; no process yet.
    let mut s = Sys::new(false);
    assert!(!s.r.wants_text_input() && s.k.procs().is_empty());
    let (list, hits) = s.draw();
    assert!(hits.is_empty() && !list.is_empty() && s.r.title().is_empty());
    // The first size starts it; once the worker is ready, Start carries the marker's URL.
    assert!(s.ev(AppEvent::Resized { w: 640.4, h: 1e9 }));
    assert_eq!(s.k.procs(), [(2, "studio".into(), true)]);
    assert_eq!(s.k.take_effects(), [K::Spawn { pid: 2, sab: true }]);
    s.k.message(&mut s.fs, 2, &wire::Msg::Ready { version: wire::VERSION }.encode());
    let (start, url) = (s.k.take_effects().pop(), ui::kernel::Load::Url("bin/studio.wasm".into()));
    assert!(matches!(start, Some(K::Start { pid: 2, program, .. }) if program == url));
    // The size is its first event, the AI settings next; the same size again is not news,
    // a new one is.
    s.ev(AppEvent::Resized { w: 640.0, h: 1e9 });
    s.ev(AppEvent::Resized { w: 500.0, h: 400.0 });
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
    // A missing program says so and none starts; a failed one says why.
    let mut s = Sys::new(false);
    s.fs.remove(STUDIO, false).unwrap();
    s.ev(AppEvent::Resized { w: 1.0, h: 1.0 });
    assert_eq!((s.r.note.as_str(), s.k.procs().len()), ("/bin/studio: not found", 0));
    let mut s = Sys::new(true);
    s.k.message(&mut s.fs, 2, &wire::Msg::ConsWrite { data: b"panicked\n" }.encode());
    s.k.message(&mut s.fs, 2, &wire::Msg::Exit { status: 101 }.encode());
    assert!(s.ev(AppEvent::Io));
    let why = ("studio stopped with status 101", "panicked\n");
    assert_eq!((s.r.note.as_str(), s.r.log.as_str()), why);
    assert!(s.asked.is_empty() && s.r.frame.is_none());
}

#[test]
fn clicks_keys_and_requests_go_through() {
    let mut s = Sys::new(true);
    let button = Node::Button { id: 1, variant: Variant::Primary, label: "Run".into() };
    let size = Request::Size { w: 300, h: 200 };
    let open = Request::Open { name: "studio:/apps/x.app".into() };
    assert!(s.show(vec![button.clone()], vec![size.clone(), open]));
    // Size only in the first frame.
    s.show(vec![button], vec![size, Request::Close]);
    let open = R::Open { name: "studio:/apps/x.app".into(), floating: false };
    assert_eq!(s.asked, [R::Size(300, 200), open, R::CloseSelf]);
    assert!(!s.ev(AppEvent::Click(WidgetId(1))) && !s.ev(AppEvent::Click(WidgetId(0))));
    // Chords and Escape and Enter are keys; plain keys are not.
    assert!(!s.ev(key(Key::Char('s'), "c")));
    for ev in [key(Key::Char('s'), ""), key(Key::Enter, ""), key(Key::Space, "ms")] {
        s.ev(ev);
    }
    s.ev(key(Key::F(5), "c"));
    let k = |key, mods, ch| Event::Key { id: 0, key, mods, ch };
    let (char, ms) = (uiwire::Key::Char, mods::META | mods::SHIFT);
    let keys = [k(char, mods::CTRL, 's'), k(uiwire::Key::Enter, 0, '\0'), k(char, ms, ' ')];
    assert_eq!(s.events(), [&[Event::Click { id: 1 }][..], &keys].concat());
    // Closing says Close once; the program's exit then ends the window.
    (0..2).for_each(|_| s.cx(|r, cx| r.closing(cx)));
    assert_eq!(s.events(), [Event::Close]);
    s.k.message(&mut s.fs, 2, &wire::Msg::Exit { status: 0 }.encode());
    s.asked.clear();
    assert!(!s.ev(AppEvent::Io));
    assert_eq!(s.asked, [R::CloseSelf]);
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
    s.ev(press(5));
    s.show(vec![], vec![]);
    assert!(!s.r.wants_text_input() && !s.ev(text("q")));

    // A Code: Enter echoed as text and Tab as text are one edit each.
    let kw = Span { start: 0, len: 5, class: Class::Keyword };
    s.show(vec![code(1, "state x", vec![kw])], vec![]);
    let got = |s: &Sys| (s.r.texts.codes[0].1.ed.text(), s.r.texts.codes[0].1.version);
    let gold = THEMES[0].ansi[3];
    let nums =
        |s: &mut Sys| s.draw().0.instances().iter().any(|i| i.kind == 4.0 && i.color == gold);
    s.ev(press(4));
    s.ev(key(Key::End, ""));
    for ev in [key(Key::Enter, ""), text("\n"), key(Key::Tab, ""), text("\t"), text("y")] {
        s.ev(ev);
    }
    assert_eq!(got(&s).0, "state x\n  y");
    assert_eq!(s.events(), [change(4, 2, "state x\n")]);
    // A stale frame keeps the host's text and its spans go unused; the same
    // version takes spans (the `y`, byte 10, as a number); a newer one
    // replaces the text.
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
}

#[test]
fn trees_draw_with_the_toolkit_and_fills_take_the_rest() {
    let mut s = Sys::new(true);
    let button = |id, variant| Node::Button { id, variant, label: "Go".into() };
    let note = Node::Text { id: 0, style: Style::Small, text: "a note".into() };
    let row = vec![button(1, Variant::Primary), button(2, Variant::Danger), note];
    let row = Node::Row { id: 0, gap: 8, children: row };
    let fill = Node::Fill { id: 0, children: vec![code(1, "a\nb", vec![])] };
    let card = Node::Card { id: 0, children: vec![input(5, "v"), Node::Separator] };
    let item = Node::Item { id: 9, text: "E1 2:3 bad".into(), detail: "x".into(), selected: true };
    s.show(vec![row, fill, card, Node::Spacer { px: 4 }, item], vec![]);
    let (_, hits) = s.draw();
    let at = |id| hits.iter().find(|h| h.id == WidgetId(id)).copied().expect("a hit");
    let (go, danger, field, code, row) = (at(1), at(2), at(5), at(4), at(9));
    let senses = (go.sense, field.sense, code.sense, row.sense);
    assert_eq!(senses, (Sense::Click, Sense::Text, Sense::Text, Sense::Click));
    assert_eq!((go.rect.x, go.rect.y, go.rect.h), (PAD, PAD, BUTTON_H));
    assert!(danger.rect.x > go.rect.x + go.rect.w && danger.rect.y == go.rect.y);
    // The Fill's Code takes what the others leave: the item ends at the bottom.
    assert_eq!(row.rect.y + row.rect.h, 400.0 - PAD);
    assert!(code.rect.y > PAD + BUTTON_H && code.rect.h > 100.0);
    assert_eq!(field.rect.x, PAD + CARD_PAD);
    // Too tall to fit: the wheel scrolls the window.
    s.show(vec![Node::Spacer { px: 900 }, input(5, "")], vec![]);
    s.draw();
    assert!(s.ev(AppEvent::Wheel { x: 10.0, y: 10.0, dy: 2000.0 }));
    assert_eq!(s.draw().1[0].rect.y, 400.0 - PAD - FIELD_H);
    assert!(!s.ev(AppEvent::Wheel { x: 10.0, y: 10.0, dy: 5.0 }));
    // Following (the Assistant), a view at the bottom stays there as the content grows.
    s.r.follow = true;
    s.show(vec![Node::Spacer { px: 1500 }, input(5, "")], vec![]);
    assert_eq!(s.draw().1[0].rect.y, 400.0 - PAD - FIELD_H);
}

#[test]
fn panes_take_their_width_chips_are_small_and_fills_match_a_taller_sibling() {
    let mut s = Sys::new(true);
    // Studio wide, on code: a Fill beside a taller Pane still reaches the bottom.
    let chip = Node::Button { id: 7, variant: Variant::Chip, label: "a dice roller".into() };
    let pane = Node::Pane { id: 0, w: 200, children: vec![Node::Spacer { px: 300 }, chip] };
    let fill = Node::Fill { id: 0, children: vec![code(1, "a", vec![])] };
    let main = Node::Col { id: 0, gap: 8, children: vec![fill] };
    s.show(vec![Node::Row { id: 0, gap: 16, children: vec![main, pane] }], vec![]);
    let (_, hits) = s.draw();
    let at = |id| hits.iter().find(|h| h.id == WidgetId(id)).copied().expect("a hit");
    let (code, chip) = (at(4), at(7));
    let content = 600.0 - 2.0 * PAD;
    assert!((code.rect.y + code.rect.h - (400.0 - PAD)).abs() < 1.0, "{code:?}");
    assert!((code.rect.w - (content - 216.0)).abs() < 1.0, "{code:?}");
    assert_eq!(
        (chip.rect.x, chip.rect.y, chip.rect.h),
        (PAD + content - 200.0, PAD + 308.0, CHIP_H)
    );
    assert!(chip.sense == Sense::Click && chip.rect.w < 120.0);
    // Room either side centers a Pane; one wider than its Row is cut to it.
    let pane = |w| Node::Pane { id: 0, w, children: vec![input(5, "")] };
    let flex = || Node::Col { id: 0, gap: 0, children: vec![] };
    s.show(vec![Node::Row { id: 0, gap: 0, children: vec![flex(), pane(200), flex()] }], vec![]);
    let field = s.draw().1[0].rect;
    assert_eq!((field.x, field.w), (PAD + (content - 200.0) / 2.0, 200.0));
    s.show(vec![pane(900)], vec![]);
    assert_eq!(s.draw().1[0].rect.w, content);
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
    s.ev(text("x"));
    s.ev(text("y"));
    assert!(!s.ev(AppEvent::Ask("more".into())));
    assert_eq!(s.events(), [change(5, 1, "x"), change(5, 2, "xy"), ask("more")]);
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
    // Two at a time, to the one endpoint with no key; the body back in pieces, then the end.
    let headers = vec![("Content-Type", "application/json".to_string())];
    let stream =
        |id| Fx::Stream { id, url: ai::URL.into(), headers: headers.clone(), body: b"{}".into() };
    let busy = (vec![stream(1), stream(2)], vec![end(3, 0, "busy")]);
    assert_eq!(ask(&mut s, vec![ai(1), ai(2), ai(3)]), busy);
    let big = [vec![b'x'; CHUNK - 1], "\u{e9}".into()].concat();
    let ended = platform::Event::StreamEnd { id: 1, status: 429, error: "".into() };
    let chunk = platform::Event::Chunk { id: 1, data: big.clone() };
    [chunk, ended.clone(), ended].into_iter().for_each(|ev| s.r.ai.heard(&mut s.k, ev));
    let data = |data: &[u8]| Event::AiData { id: 1, data: data.to_vec() };
    assert_eq!(ask(&mut s, vec![]).1, [data(&big[..CHUNK]), data(&big[CHUNK..]), end(1, 429, "")]);
    // A cancel aborts the stream and ends the request at once.
    let cancel = vec![Request::AiCancel { id: 4 }, Request::AiCancel { id: 2 }];
    assert_eq!(ask(&mut s, cancel), (vec![Fx::Abort(2)], vec![end(2, 0, "cancelled")]));
    // Closing aborts the rest, unanswered.
    assert_eq!(ask(&mut s, vec![ai(5)]), (vec![stream(3)], vec![]));
    s.cx(|r, cx| r.closing(cx));
    assert_eq!(ask(&mut s, vec![]), (vec![Fx::Abort(3)], vec![Event::Close]));
}

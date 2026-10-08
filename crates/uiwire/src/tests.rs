use std::fmt::Debug;
use std::io::{self, Cursor, ErrorKind, Read};

use crate::client::{self, Client};
use crate::*;

fn text(style: Style, s: &str) -> Node {
    Node::Text { id: 0, style, text: s.into() }
}

fn col(children: Vec<Node>) -> Node {
    Node::Col { id: 0, gap: 0, children }
}

fn code(text: &str, spans: &[(u32, u32)]) -> Node {
    let spans = spans.iter().map(|&(start, len)| Span { start, len, class: Class::Name });
    Node::Code { id: 9, version: 1, line_numbers: false, text: text.into(), spans: spans.collect() }
}

/// A frame with every request and node kind, nested.
fn sample() -> Frame {
    let span = |start, len, class| Span { start, len, class };
    let spans =
        vec![span(0, 3, Class::Keyword), span(4, 2, Class::Name), span(9, 1, Class::Number)];
    let spans = [spans, vec![span(10, 1, Class::Error)]].concat();
    let title = Node::Text { id: 2, style: Style::Title, text: "héllo".into() };
    let row = vec![title, Node::Button { id: 3, variant: Variant::Danger, label: "Run".into() }];
    let item = Node::Item { id: 6, text: "a".into(), detail: "b".into(), selected: true };
    let chip = Node::Button { id: 10, variant: Variant::Chip, label: "a dice roller".into() };
    let code =
        Node::Code { id: 7, version: 3, line_numbers: true, text: "let é = 1;".into(), spans };
    let children = vec![
        Node::Row { id: 0, gap: 4, children: row },
        Node::Input { id: 4, value: "abc".into(), placeholder: "name".into() },
        Node::Separator,
        Node::Spacer { px: 12 },
        Node::Card { id: 5, children: vec![item] },
        Node::Fill { id: 0, children: vec![code] },
        Node::Pane { id: 8, w: 300, children: vec![chip] },
        Node::Glyph { glyph: 6, size: 89 },
        Node::Entry {
            id: 11,
            glyph: 6,
            hue: 0x60a5fa,
            text: "apps".into(),
            detail: "".into(),
            more: true,
        },
        Node::Toggle { id: 12, on: true, label: "Include what\u{2019}s open".into() },
        Node::Area { id: 13, value: "a\nb".into(), placeholder: "What happened?".into() },
        Node::Center { id: 14, gap: 5, children: vec![text(Style::Accent, "compusophy")] },
        Node::Scroll { id: 15, children: vec![Node::Glyph { glyph: REVEAL, size: 144 }] },
        Node::Strip { id: 0, gap: 2, children: vec![text(Style::Display, "~")] },
    ];
    let requests = vec![Request::Open { name: "/apps/clock.app".into() }, Request::Close];
    let ai = vec![
        Request::Ai { id: 3, body: "{\"stream\":true}".into() },
        Request::AiCancel { id: 3 },
        Request::Focus { id: 4 },
        Request::Feedback {
            kind: "idea".into(),
            text: "more themes\nplease".into(),
            context: true,
        },
    ];
    Frame {
        seq: 7,
        title: "Studio é".into(),
        requests: [requests, vec![Request::Size { w: 640, h: 480 }], ai].concat(),
        nodes: vec![Node::Col { id: 1, gap: 8, children }, text(Style::Success, "")],
    }
}

fn events() -> Vec<Event> {
    vec![
        Event::Click { id: 3 },
        Event::Change { id: 7, version: 4, text: "let é = 2;\n".into() },
        Event::Key { id: 7, key: Key::Char, mods: mods::CTRL | mods::SHIFT, ch: 'é' },
        Event::Key { id: 0, key: Key::Escape, mods: 0, ch: '\0' },
        Event::Resize { w: 800, h: 600 },
        Event::Close,
        Event::Submit { id: 4 },
        Event::Config { model: "zai/glm-5.3".into() },
        Event::AiData { id: 3, data: vec![b'd', 0xC3, 0xFF] },
        Event::AiEnd { id: 3, status: 429, error: "".into() },
        Event::Ask { text: "a dice roller".into() },
        Event::Focus { on: true },
        Event::Focus { on: false },
        Event::Prefs { reports: true, grain: false, kept: true },
        Event::Face { face: 3 },
        Event::Output { data: b"\x1b[1mhi\r\n".to_vec() },
        Event::Text { text: "ls -l".into() },
        Event::Wheel { dy: -34 },
        Event::Ended { status: 127 },
        Event::Key { id: 0, key: Key::F, mods: 0, ch: '\u{c}' },
        Event::Pool { data: vec![1, 2] },
        Event::Done { index: 7, node: 1, out: "10 ab 0121".into() },
    ]
}

/// `v` round-trips, but no prefix of its bytes decodes, nor its bytes plus one.
fn strict<T: PartialEq + Debug>(v: &T, encode: fn(&T) -> Vec<u8>, decode: fn(&[u8]) -> Option<T>) {
    let mut bytes = encode(v);
    assert_eq!(decode(&bytes).as_ref(), Some(v));
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end]).is_none(), "{end} of {bytes:?}");
    }
    bytes.push(0);
    assert!(decode(&bytes).is_none());
}

#[test]
fn every_kind_round_trips_and_nothing_else_decodes() {
    let frame = sample();
    assert_eq!(frame.nodes[0].count(), 23);
    [frame.clone(), Frame::default()].iter().for_each(|f| strict(f, Frame::encode, Frame::decode));
    let nodes = frame.nodes.iter().chain(frame.nodes[0].children());
    nodes.for_each(|n| strict(n, Node::encode, Node::decode));
    frame.requests.iter().for_each(|r| strict(r, Request::encode, Request::decode));
    events().iter().for_each(|e| strict(e, Event::encode, Event::decode));
    // The OS's own: its pages, themes, choices, switches and links, and a preference set.
    let own = [
        Node::Pages { id: 1, on: 2, labels: "Appearance\nAI\nPrivacy".into() },
        Node::Themes { id: 10 },
        Node::Faces { id: 50, on: 4 },
        Node::Screen { id: 1, cols: 2, rows: 1, cursor: Some((0, 1)), cells: vec![7; 26] },
        Node::Screen { id: 1, cols: 1, rows: 1, cursor: None, cells: vec![0; 13] },
        Node::Choice { id: 20, on: true, text: "GLM 5.3\nbest answers".into() },
        Node::Switch { id: 30, on: false, label: "Living grain".into() },
        Node::Button { id: 31, variant: Variant::Link, label: "Send feedback".into() },
        Node::Chart { id: 0, hue: 11, h: 72, values: vec![UNKNOWN, 0, 500, 1000] },
        Node::Meter { id: 3, hue: 4, value: 1000 },
        Node::Columns { id: 40, on: 2, labels: "Name\tCPU\tMemory".into() },
    ];
    own.iter().for_each(|n| strict(n, Node::encode, Node::decode));
    let pref = Request::Pref { key: "grain".into(), value: "off".into() };
    let tty = [
        Request::Tty { cols: 80, rows: 24 },
        Request::Input { data: b"ls\r".to_vec() },
        Request::Pair { code: "K7000".into() },
        Request::Measure,
        Request::Job { name: "fractal".into(), chunks: vec!["0 0 -0.6 0 3".into(), String::new()] },
    ];
    [pref, Request::Reset]
        .iter()
        .chain(&tty)
        .for_each(|r| strict(r, Request::encode, Request::decode));
    // Past a value's 1000, the canvas colors or a Chart's sizes, nothing decodes, nor checks.
    let chart = |hue, h, values: Vec<u16>| Node::Chart { id: 0, hue, h, values };
    let bad = [
        chart(12, 72, vec![]),
        chart(1, 15, vec![]),
        chart(1, 481, vec![]),
        chart(1, 72, vec![1001]),
    ];
    let long = chart(1, 72, vec![0; CHART_POINTS + 1]);
    let meters =
        [Node::Meter { id: 0, hue: 12, value: 0 }, Node::Meter { id: 0, hue: 1, value: 1001 }];
    for n in bad.iter().chain([&long]).chain(&meters) {
        let frame = Frame { nodes: vec![n.clone()], ..Frame::default() };
        assert!(Node::decode(&n.encode()).is_none() && frame.encode_checked().is_none(), "{n:?}");
    }
}

#[test]
fn codes_and_layout_are_as_documented() {
    // How many bytes decode, each to the code it is.
    let n = |f: &dyn Fn(u8) -> Option<u8>| {
        (0..=255).filter(|&n| f(n).inspect(|&code| assert_eq!(code, n)).is_some()).count()
    };
    let style = n(&|n| Style::from_u8(n).map(|v| v as u8));
    let variant = n(&|n| Variant::from_u8(n).map(|v| v as u8));
    let (class, key) =
        (n(&|n| Class::from_u8(n).map(|v| v as u8)), n(&|n| Key::from_u8(n).map(|v| v as u8)));
    assert_eq!(([style, variant, class, key], Key::from_u8(0)), ([12, 7, 8, 16], None));
    // Little-endian, in field order.
    let (requests, button) = (vec![Request::Size { w: 0x0506, h: 7 }], "ok".into());
    let nodes = vec![Node::Button { id: 9, variant: Variant::Primary, label: button }];
    let frame = Frame { seq: 0x0102_0304, title: "T".into(), requests, nodes };
    let want = [
        1, 4, 3, 2, 1, 1, 0, 0, 0, b'T', 1, 0, 3, 6, 5, 7, 0, 1, 0, 0, 0, 4, 9, 0, 0, 0, 0, 0, 1,
        2, 0, 0, 0, b'o', b'k',
    ];
    assert_eq!(frame.encode(), want);
    let key = Event::Key { id: 2, key: Key::Enter, mods: mods::ALT, ch: 'A' };
    assert_eq!(key.encode(), [3, 2, 0, 0, 0, 1, 4, 65, 0, 0, 0]);
    // AI bytes need not be UTF-8 (a chunk may split a character).
    let data = Event::AiData { id: 1, data: vec![0xC3] };
    assert_eq!(data.encode(), [8, 1, 0, 0, 0, 1, 0, 0, 0, 0xC3]);
    assert_eq!(Event::Ask { text: "hi".into() }.encode(), [10, 2, 0, 0, 0, b'h', b'i']);
}

#[test]
fn caps_hold() {
    let frame = |nodes| Frame { nodes, ..Frame::default() }.encode();
    let many = |n| Frame::decode(&frame(vec![Node::Separator; n]));
    assert!(many(MAX_NODES).is_some() && many(MAX_NODES + 1).is_none());
    let mut deep = Node::Separator;
    for _ in 1..MAX_DEPTH {
        deep = col(vec![deep]);
    }
    let fits = |n: &Node| {
        [Node::decode(&n.encode()).is_some(), Frame::decode(&frame(vec![n.clone()])).is_some()]
    };
    assert_eq!([fits(&deep), fits(&col(vec![deep]))], [[true; 2], [false; 2]]);
    let big = |n| frame(vec![text(Style::Body, &"x".repeat(n))]);
    let base = big(0).len();
    let fits = |n| Frame::decode(&big(MAX_FRAME - base + n)).is_some();
    assert!(big(MAX_FRAME - base).len() == MAX_FRAME && fits(0) && !fits(1));
    let change = Event::Change { id: 1, version: 1, text: "x".repeat(MAX_FRAME) };
    assert!(Event::decode(&change.encode()).is_none());
}

#[test]
fn a_checked_encoding_is_exactly_one_that_decodes() {
    // Programs check their frames without the decoder: it must agree with it, and the bytes
    // are the encoding.
    let frame = |nodes| Frame { nodes, ..Frame::default() };
    let mut deep = Node::Separator;
    for _ in 0..MAX_DEPTH {
        deep = col(vec![deep]);
    }
    let base = frame(vec![]).encode().len() + text(Style::Body, "").encode().len();
    let spans: [&[(u32, u32)]; 7] = [
        &[(0, 1), (1, 2), (3, 0)],
        &[(0, 4)],
        &[(2, 1)],
        &[(1, 2), (0, 1)],
        &[(0, 3), (1, 2)],
        &[(u32::MAX, 2)],
        &[],
    ];
    let mut frames = vec![sample(), Frame::default(), frame(vec![deep.children()[0].clone()])];
    frames.extend([frame(vec![deep]), frame(vec![Node::Separator; MAX_NODES + 1])]);
    frames.extend(
        [MAX_FRAME - base, MAX_FRAME - base + 1]
            .map(|n| frame(vec![text(Style::Body, &"x".repeat(n))])),
    );
    frames.extend(spans.map(|s| frame(vec![col(vec![code("aé", s)])])));
    let many = |n| Frame { requests: vec![Request::Close; n], ..Frame::default() };
    frames.extend([many(usize::from(u16::MAX)), many(usize::from(u16::MAX) + 1)]);
    let mut decoded = 0;
    for f in &frames {
        let checked = f.encode_checked();
        assert_eq!(checked.is_some(), Frame::decode(&f.encode()).is_some(), "{:?}", f.nodes.len());
        decoded += usize::from(checked.is_some_and(|b| b == f.encode()));
    }
    assert_eq!(decoded, 7);
}

/// `bytes` with byte `at` set to `to`.
fn set(mut bytes: Vec<u8>, at: usize, to: u8) -> Vec<u8> {
    bytes[at] = to;
    bytes
}

#[test]
fn malformations_fail() {
    let ok = sample().encode();
    // The version; not UTF-8 in the title; the node count, one off either way.
    let feedback = 1 + (4 + 4) + (4 + 18) + 1;
    let at = 1 + 4 + 4 + "Studio é".len() + 2 + (1 + 4 + 15) + 1 + (1 + 4) + (9 + 15) + 5 + 5;
    let at = at + feedback;
    assert_eq!(ok[at..at + 4], [24, 0, 0, 0]);
    for (i, to) in [(0, 2), (13, 0xFF), (at, 23), (at, 25)] {
        assert!(Frame::decode(&set(ok.clone(), i, to)).is_none(), "{i}");
    }
    // A leaf with a child; an id on a Separator, Spacer or Glyph; unknown kinds.
    let leaf = [set(text(Style::Body, "a").encode(), 5, 1), Node::Separator.encode()].concat();
    assert!(Node::decode(&leaf).is_none());
    assert_eq!(Node::decode(&[7, 0, 0, 0, 0, 0, 0]), Some(Node::Separator));
    assert!(Node::decode(&[7, 1, 0, 0, 0, 0, 0]).is_none());
    assert!(Node::decode(&[8, 1, 0, 0, 0, 0, 0, 5, 0]).is_none());
    assert!(Node::decode(&[13, 1, 0, 0, 0, 0, 0, 6, 89, 0]).is_none());
    assert_eq!(
        Node::decode(&[13, 0, 0, 0, 0, 0, 0, 6, 89, 0]),
        Some(Node::Glyph { glyph: 6, size: 89 })
    );
    assert!([0, 21, 255].iter().all(|&kind| Node::decode(&[kind, 0, 0, 0, 0, 0, 0]).is_none()));
    // Codes: style, variant, selected, line-number and on flags, span class.
    assert!(Node::decode(&[3, 0, 0, 0, 0, 0, 0, 12, 0, 0, 0, 0]).is_none());
    assert!(Node::decode(&[4, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0]).is_none());
    assert!(Node::decode(&[15, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0]).is_none());
    assert!(Node::decode(&[10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]).is_none());
    assert!(Node::decode(&set(code("ab", &[]).encode(), 11, 2)).is_none());
    let class = code("ab", &[(0, 1)]).encode();
    assert!(Node::decode(&set(class.clone(), class.len() - 1, 8)).is_none());

    // Spans: in order, apart, inside the text, on char boundaries.
    assert!(Node::decode(&code("aé", &[(0, 1), (1, 2), (3, 0)]).encode()).is_some());
    for spans in [&[(0, 4)][..], &[(2, 1)], &[(1, 1)], &[(1, 2), (0, 1)], &[(0, 3), (1, 2)]] {
        assert!(Node::decode(&code("aé", spans).encode()).is_none(), "{spans:?}");
    }
    assert!(Node::decode(&code("a", &[(u32::MAX, 2)]).encode()).is_none());
    let lie = code("a", &[]).encode();
    assert!(Node::decode(&[&lie[..lie.len() - 4], &[0xFF; 4]].concat()).is_none());

    // Events: key code, modifier bits, char, kind.
    let key = |k: u8, m: u8, ch: u32| {
        let mut b = vec![3, 0, 0, 0, 0, k, m];
        b.extend(ch.to_le_bytes());
        Event::decode(&b)
    };
    assert!(key(8, 15, 'x' as u32).is_some());
    assert!(key(0, 0, 0).is_none() && key(17, 0, 0).is_none());
    assert!(key(1, 16, 0).is_none() && key(1, 0, 0xD800).is_none());
    assert!(Event::decode(&[17]).is_none() && Request::decode(&[14, 0, 0, 0, 0]).is_none());
    assert!(Event::decode(&[11, 2]).is_none() && Event::decode(&[11, 1]).is_some());
}

/// Every act, the overlay's status, an Acted carrying `scene()`, and Halt.
fn agent() -> (Vec<Request>, Vec<Event>) {
    let acts = [
        Act::Wait { ms: 5000 },
        Act::Click { win: 2, id: 31 },
        Act::Type { win: 2, id: 4, text: "h\u{e9}llo".into(), submit: true },
        Act::Key { win: 0, code: "KeyS".into(), mods: mods::CTRL | mods::SHIFT },
        Act::Scroll { win: 3, id: 0, dy: -3000 },
        Act::Open { name: "settings".into() },
        Act::Window { win: 2, op: WinOp::Restore },
        Act::Theme { name: "Dawn".into() },
        Act::Tap { win: 2, id: 1 << 30, cell: 199 },
    ];
    acts.iter().for_each(|a| strict(a, Act::encode, Act::decode));
    let ask = |(id, act): (u32, Act)| Request::Act { id, act: act.encode() };
    let mut requests: Vec<_> = (1..).zip(acts).map(ask).collect();
    requests.extend([Request::Status { working: true }, Request::Status { working: false }]);
    requests
        .extend([Request::Yield { win: 2, hide: true }, Request::Yield { win: 3, hide: false }]);
    let scene = scene().encode();
    let acted = Event::Acted { id: 9, code: acted::OFF_SCREEN, note: "e9".into(), scene };
    (requests, vec![acted, Event::Halt])
}

/// A shown window with a hit of each kind, a mark and a run; a minimized one.
fn scene() -> scene::Scene {
    use scene::*;
    let hits = vec![Hit { id: 30, sense: 2, rect: [10, 20, -5, 400] }, Hit::default()];
    let marks = vec![Mark { id: 30, role: 3, flags: 2, value: "on".into() }];
    let runs = vec![Run { rect: [1, 2, 3, i16::MIN], text: "Send error reports".into() }];
    let (app, title) = ("settings".into(), "Settings".into());
    let shown = Win { id: 2, app, title, rect: [1, 2, 3, 4], state: 3, hits, marks, runs };
    let min = Win { id: 1, app: "terminal".into(), state: state::MIN, ..Win::default() };
    let (apps, wins) = (vec!["settings".into(), "files".into()], vec![shown, min]);
    Scene { w: 1440, h: 900, touch: false, theme: "Dawn".into(), focus: 2, apps, wins }
}

#[test]
fn acts_and_scenes_round_trip_strictly() {
    let (requests, acted) = agent();
    requests.iter().for_each(|r| strict(r, Request::encode, Request::decode));
    acted.iter().for_each(|e| strict(e, Event::encode, Event::decode));
    let scenes = [scene(), scene::Scene::default()];
    scenes.iter().for_each(|s| strict(s, scene::Scene::encode, scene::Scene::decode));
    // The kinds after the last, and their fields in order.
    let click = Request::Act { id: 1, act: Act::Click { win: 2, id: 3 }.encode() };
    assert_eq!(click.encode(), [8, 1, 0, 0, 0, 9, 0, 0, 0, 2, 2, 0, 0, 0, 3, 0, 0, 0]);
    let scroll = Act::Scroll { win: 1, id: 0, dy: -2 };
    assert_eq!(scroll.encode(), [5, 1, 0, 0, 0, 0, 0, 0, 0, 254, 255]);
    // A request whose bytes are no act never decodes.
    assert!(Request::decode(&[8, 1, 0, 0, 0, 1, 0, 0, 0, 9]).is_none());
    let (status, aside) =
        (Request::Status { working: true }, Request::Yield { win: 2, hide: true });
    let codes = [status.encode(), aside.encode(), Event::Halt.encode()];
    assert_eq!(codes, [&[9, 1][..], &[18, 2, 0, 0, 0, 1], &[13]]);
    assert!(Request::decode(&[18, 2, 0, 0, 0, 2]).is_none());
    // The overlay's alone: its acts, its status, stepping aside.
    let others = [Request::Close, Request::Reset, Request::Keys { on: true }];
    assert!([click.clone(), status, aside].iter().all(Request::overlay));
    assert!(!others.iter().any(Request::overlay));
    let acted = Event::Acted { id: 1, code: 916, note: "".into(), scene: vec![7] };
    assert_eq!(acted.encode(), [12, 1, 0, 0, 0, 0x94, 3, 0, 0, 0, 0, 1, 0, 0, 0, 7]);
    let ops = (0..=255).filter(|&n| WinOp::from_u8(n).is_some_and(|o| o as u8 == n)).count();
    assert_eq!(ops, 5);
    // Unknown acts and window ops, modifier bits past ALL, a bool that is not 0 or 1.
    let act = Act::decode;
    assert!(act(&[0]).is_none() && act(&[9]).is_none() && act(&[1, 1, 0]).is_some());
    assert!(act(&[7, 1, 0, 0, 0, 5]).is_none() && act(&[7, 1, 0, 0, 0, 4]).is_some());
    assert!(act(&[4, 0, 0, 0, 0, 0, 0, 0, 0, 16]).is_none());
    assert!(act(&[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]).is_none());
    // A window's state past snapped, a hit's sense past scroll.
    let bytes = scene().encode();
    let at = 2 + 2 + (4 + 4) + 4 + 4 + (4 + 8) + (4 + 5) + 4 + 4 + (4 + 8) + (4 + 8) + 8;
    assert_eq!((bytes[at], bytes[at + 1 + 4 + 4]), (3, 2));
    assert!(scene::Scene::decode(&set(bytes.clone(), at, 4)).is_none());
    assert!(scene::Scene::decode(&set(bytes.clone(), at + 9, 3)).is_none());
    assert!(scene::Scene::decode(&set(bytes.clone(), at + 9, 0)).is_some());
    // A touch screen's flag comes last, a 1 only when set, so a scene with none (an older
    // desktop's) reads as no touch screen; anything else there is malformed.
    let touch = scene::Scene { touch: true, ..scene() };
    let with = touch.encode();
    assert!(with[..bytes.len()] == bytes[..] && with[bytes.len()..] == [1]);
    assert_eq!(scene::Scene::decode(&with), Some(touch));
    let bad = [set(with.clone(), bytes.len(), 0), [&with[..], &[1]].concat()];
    assert!(bad.iter().all(|b| scene::Scene::decode(b).is_none()));
}

/// A grid with a handler and texts, and one without either.
fn grids() -> [Node; 2] {
    let texts = ["x", "", "\u{e9}", "o"].map(String::from).to_vec();
    let tap = Node::Grid { id: 1 << 30, cols: 2, cells: vec![0, 1, 8, 3], texts };
    [tap, Node::Grid { id: 0, cols: 10, cells: vec![0; 200], texts: Vec::new() }]
}

#[test]
fn grids_timers_ticks_keys_and_taps_round_trip_strictly() {
    let grids = grids();
    grids.iter().for_each(|g| strict(g, Node::encode, Node::decode));
    let frame = Frame { nodes: vec![col(grids.to_vec())], ..Frame::default() };
    assert_eq!(frame.encode_checked(), Some(frame.encode()));
    let requests =
        [Request::Timer { ms: 0 }, Request::Timer { ms: 150 }, Request::Keys { on: true }];
    requests.iter().for_each(|r| strict(r, Request::encode, Request::decode));
    let events = [Event::Tick { ms: 16 }, Event::Tap { id: 1 << 30, cell: 37 }];
    events.iter().for_each(|e| strict(e, Event::encode, Event::decode));
    // The codes allocated, after the last, fields in order.
    assert_eq!(requests[1].encode(), [10, 150, 0, 0, 0]);
    assert_eq!(requests[2].encode(), [11, 1]);
    assert_eq!(events[0].encode(), [14, 16, 0, 0, 0]);
    assert_eq!(events[1].encode(), [15, 0, 0, 0, 64, 37, 0, 0, 0]);
    let small = Node::Grid { id: 3, cols: 2, cells: vec![1, 2], texts: Vec::new() };
    assert_eq!(small.encode(), [20, 3, 0, 0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 1, 2, 0, 0, 0, 0]);
    assert_eq!(
        Act::Tap { win: 1, id: 2, cell: 3 }.encode(),
        [9, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]
    );
    // No columns, a square past 8, texts not one per square: neither checked nor decoded.
    let bad = [
        Node::Grid { id: 0, cols: 0, cells: vec![1], texts: Vec::new() },
        Node::Grid { id: 0, cols: 1, cells: vec![9], texts: Vec::new() },
        Node::Grid { id: 0, cols: 1, cells: vec![1, 2], texts: vec!["a".into()] },
    ];
    for g in bad {
        let frame = Frame { nodes: vec![g.clone()], ..Frame::default() };
        assert!(Node::decode(&g.encode()).is_none() && frame.encode_checked().is_none(), "{g:?}");
    }
    // A text count past what the bytes could hold, and a grid with a child.
    let mut lie = small.encode();
    lie.splice(15..19, [0xFF; 4]);
    assert!(Node::decode(&lie).is_none());
    assert!(Node::decode(&set(small.encode(), 5, 1)).is_none());
}

fn draw(shape: Shape, color: u8, at: [i16; 5], text: &str) -> Draw {
    Draw { shape, color, at, text: text.into() }
}

/// A canvas with a handler and every shape, and one without either.
fn canvases() -> [Node; 2] {
    let draws = vec![
        draw(Shape::Rect, 1, [0, 0, 160, 8, 0], ""),
        draw(Shape::Circle, 3, [-5, 200, 2, 0, 0], ""),
        draw(Shape::Ring, 4, [150, 150, 30, 8, 0], ""),
        draw(Shape::Line, 10, [100, 8, -100, 291, 3], ""),
        draw(Shape::Text, 11, [80, 4, 6, 0, 0], "Score 7 \u{e9}"),
        draw(Shape::Sprite, 0, [8, 12, 2, 0, 0], "..5..\n.555."),
        draw(Shape::Pixels, 0, [-4, 20, 3, 10, 0], "112.b."),
    ];
    let tap = Node::Canvas { id: 1 << 30, w: 300, h: 300, draws };
    [tap, Node::Canvas { id: 0, w: 1, h: MAX_SIDE, draws: Vec::new() }]
}

#[test]
fn canvases_round_trip_every_shape_and_decode_strictly() {
    let canvases = canvases();
    canvases.iter().for_each(|c| strict(c, Node::encode, Node::decode));
    let frame = Frame { nodes: vec![col(canvases.to_vec())], ..Frame::default() };
    assert_eq!(frame.encode_checked(), Some(frame.encode()));
    let Node::Canvas { draws, .. } = &canvases[0] else { unreachable!() };
    assert_eq!(draws.iter().map(Draw::ink).collect::<Vec<_>>(), [1, 1, 1, 1, 10, 5, 4]);
    // Its code after the last; its size, its draws' count, and each draw's shape, color and
    // slots, then a string for a Text or a Sprite only.
    let line = draw(Shape::Line, 9, [1, -2, 3, 4, 5], "");
    let one =
        Node::Canvas { id: 2, w: 3, h: 4, draws: vec![line, draw(Shape::Text, 0, [0; 5], "")] };
    let at = |v: [i16; 5]| v.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    let head = [21, 2, 0, 0, 0, 0, 0, 3, 0, 4, 0, 2, 0, 0, 0];
    let want = [&head[..], &[3, 9], &at([1, -2, 3, 4, 5]), &[4, 0], &at([0; 5]), &[0; 4]].concat();
    assert_eq!(one.encode(), want);
    // No shape 7, a count past what the bytes hold, nor a child.
    assert!(Node::decode(&set(one.encode(), 15, 7)).is_none());
    assert!(Node::decode(&set(one.encode(), 14, 1)).is_none());
    assert!(Node::decode(&set(one.encode(), 5, 1)).is_none());
    // As many ink as a canvas holds, and one more.
    let sprite = |n| draw(Shape::Sprite, 0, [0, 0, 1, 0, 0], &"7".repeat(n));
    let full = Node::Canvas { id: 0, w: 9, h: 9, draws: vec![sprite(MAX_INK - 1)] };
    strict(&full, Node::encode, Node::decode);
    // A side of 0 or past the most, a color past 11, a sprite's color, a slot past those its
    // shape uses, a negative size, a text on a Rect, a Text of two lines, too much ink: neither
    // checked nor decoded as they are.
    let canvas = |w, h, d: Draw| Node::Canvas { id: 1, w, h, draws: vec![d] };
    let rect = |color, at| draw(Shape::Rect, color, at, "");
    let bad = [
        canvas(0, 9, rect(1, [0; 5])),
        canvas(9, MAX_SIDE + 1, rect(1, [0; 5])),
        canvas(9, 9, rect(12, [0; 5])),
        canvas(9, 9, draw(Shape::Sprite, 1, [0; 5], "1")),
        canvas(9, 9, rect(1, [0, 0, 1, 1, 1])),
        canvas(9, 9, draw(Shape::Circle, 1, [0, 0, 0, 1, 0], "")),
        canvas(9, 9, rect(1, [0, 0, -1, 1, 0])),
        canvas(9, 9, draw(Shape::Line, 1, [0, 0, 1, 1, -3], "")),
        canvas(9, 9, draw(Shape::Rect, 1, [0; 5], "x")),
        canvas(9, 9, draw(Shape::Text, 1, [0; 5], "a\nb")),
        canvas(9, 9, sprite(MAX_INK)),
    ];
    for c in bad {
        let frame = Frame { nodes: vec![c.clone()], ..Frame::default() };
        let back = Node::decode(&c.encode());
        assert!(back != Some(c.clone()) && frame.encode_checked().is_none(), "{c:?}");
    }
}

#[test]
fn pixels_are_whole_rows_of_paint_and_count_their_runs() {
    let px = |at: [i16; 5], cells: &str| draw(Shape::Pixels, 0, at, cells);
    let canvas = |draws| Node::Canvas { id: 0, w: 64, h: 64, draws };
    // Its code after Sprite's, its cells a string as a Sprite's rows are.
    let at = [1, 0, 2, 0, 2, 0, 3, 0, 0, 0];
    let want = [&[21, 0, 0, 0, 0, 0, 0, 64, 0, 64, 0, 1, 0, 0, 0, 6, 0][..], &at, &[2, 0, 0, 0]];
    assert_eq!(
        canvas(vec![px([1, 2, 2, 3, 0], "0b")]).encode(),
        [&want.concat()[..], b"0b"].concat()
    );
    // A run of one color is one ink, however long; none shows through and costs nothing.
    let (full, side) = (|c: &str| c.repeat(PIXELS_SIDE * PIXELS_SIDE), PIXELS_SIDE as i16);
    assert_eq!(px([0, 0, side, 1, 0], &full("a")).ink(), 1 + PIXELS_SIDE);
    assert_eq!(px([0, 0, 2, 1, 0], "1..1").ink(), 3);
    assert_eq!(px([0, 0, 4, 1, 0], "....").ink(), 1);
    // Four full boards a canvas holds, and an empty one; a cell more is too many.
    let board = px([0, 0, side, 1, 0], &full("."));
    let four = canvas(vec![board; MAX_PIXELS / (PIXELS_SIDE * PIXELS_SIDE)]);
    strict(&four, Node::encode, Node::decode);
    strict(&canvas(vec![px([0, 0, 1, 0, 0], "")]), Node::encode, Node::decode);
    let Node::Canvas { mut draws, .. } = four else { unreachable!() };
    draws.push(px([0, 0, 1, 1, 0], "1"));
    // A color, no or too many cells a row, rows not whole or too many, a char not paint, a slot
    // past those it uses, a negative side, too many cells in all: neither checked nor decoded.
    let bad = [
        draw(Shape::Pixels, 1, [0, 0, 1, 1, 0], "1"),
        px([0, 0, 0, 1, 0], ""),
        px([0, 0, side + 1, 1, 0], &".".repeat(PIXELS_SIDE + 1)),
        px([0, 0, 2, 1, 0], "123"),
        px([0, 0, 1, 1, 0], &".".repeat(PIXELS_SIDE + 1)),
        px([0, 0, 1, 1, 0], "c"),
        px([0, 0, 2, 1, 0], "1\n"),
        px([0, 0, 1, 1, 1], "1"),
        px([0, 0, 1, -1, 0], "1"),
    ];
    for c in bad.map(|d| canvas(vec![d])).into_iter().chain([canvas(draws)]) {
        let frame = Frame { nodes: vec![c.clone()], ..Frame::default() };
        let back = Node::decode(&c.encode());
        assert!(back != Some(c.clone()) && frame.encode_checked().is_none(), "{c:?}");
    }
}

#[test]
fn fuzzed_decodes_never_panic_and_stay_canonical() {
    let mut corpus = vec![sample().encode(), Frame::default().encode(), scene().encode()];
    corpus.extend(grids().iter().chain(&canvases()).map(Node::encode));
    corpus.extend([Request::Timer { ms: 7 }.encode(), Event::Tap { id: 1, cell: 2 }.encode()]);
    let (requests, acted) = agent();
    corpus.extend(requests.iter().map(Request::encode).chain(acted.iter().map(Event::encode)));
    corpus.extend(events().iter().map(Event::encode));
    corpus.extend(sample().requests.iter().map(Request::encode));
    corpus.extend(sample().nodes.iter().map(Node::encode));
    // xorshift64*, a number below `n`: a fixed seed keeps the fuzz deterministic.
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut below = |n: usize| {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        (seed.wrapping_mul(0x2545_F491_4F6C_DD1D) % n.max(1) as u64) as usize
    };
    let mut decoded = 0;
    for _ in 0..10_000 {
        let mut input = if below(8) == 0 {
            (0..below(64)).map(|_| below(256) as u8).collect()
        } else {
            corpus[below(corpus.len())].clone()
        };
        for _ in 0..=below(4) {
            let at = below(input.len());
            match below(5) {
                0 if !input.is_empty() => input[at] ^= 1u8 << below(8),
                1 if !input.is_empty() => input[at] = below(256) as u8,
                2 => input.truncate(at),
                3 => input.insert(at, below(256) as u8),
                _ if !input.is_empty() => input[at] = [0, 1, 0xFF][below(3)],
                _ => {}
            }
        }
        let again =
            [Frame::decode(&input).map(|v| v.encode()), Event::decode(&input).map(|v| v.encode())];
        let more =
            [Request::decode(&input).map(|v| v.encode()), Node::decode(&input).map(|v| v.encode())];
        let scene = scene::Scene::decode(&input).map(|v| v.encode());
        for bytes in again.into_iter().chain(more).chain([scene]).flatten() {
            assert_eq!(bytes, input);
            decoded += 1;
        }
    }
    assert!(decoded > 100, "the fuzz should reach valid inputs too: {decoded}");
}

#[test]
fn client_reads_events_and_writes_frames() {
    let (resize, click) = (Event::Resize { w: 3, h: 4 }, Event::Click { id: 1 });
    // An event over 64 KiB comes in 64 KiB parts; a short part ends one. A chain of
    // cursors hands out one chunk per read, as /dev/events does.
    let long = Event::Change { id: 1, version: 2, text: "x".repeat(70_000) };
    let parts: Vec<_> = long.encode().chunks(client::PART).map(<[u8]>::to_vec).collect();
    let chunks = [vec![resize.encode()], parts, vec![click.encode(), vec![9; 70]]].concat();
    let events = chunks.into_iter().map(Cursor::new);
    let events = events.fold(Box::new(io::empty()) as Box<dyn Read>, |r, c| Box::new(r.chain(c)));
    let mut out = Vec::new();
    let mut client = Client::new(events, &mut out);
    assert_eq!(client.next_event().unwrap(), resize);
    assert_eq!(client.next_event().unwrap(), long);
    assert_eq!(client.next_event().unwrap(), click);
    assert_eq!(client.next_event().unwrap_err().kind(), ErrorKind::InvalidData);
    assert_eq!(client.next_event().unwrap_err().kind(), ErrorKind::UnexpectedEof);
    client.show(&sample()).unwrap();
    // A frame that breaks a cap (its size, or a node's: checked without decoding) is never sent.
    let titled = |n| Frame { title: "t".repeat(n), ..Frame::default() };
    let at = MAX_FRAME - titled(0).encode().len();
    assert_eq!(client.show(&titled(at + 1)).unwrap_err().kind(), ErrorKind::InvalidInput);
    let bad = Frame { nodes: vec![code("a", &[(0, 2)])], ..Frame::default() };
    assert_eq!(client.show(&bad).unwrap_err().kind(), ErrorKind::InvalidInput);
    drop(client);
    assert_eq!(out, sample().encode());
    let mut sent = Vec::new();
    Client::new(io::empty(), &mut sent).show(&titled(at)).unwrap();
    assert_eq!(sent.len(), MAX_FRAME);

    // A sink that takes 8 bytes of the frame.
    let mut small = Client::new(io::empty(), Cursor::new([0; 8]));
    assert_eq!(small.show(&sample()).unwrap_err().kind(), ErrorKind::WriteZero);
}

/// A sample of a desktop running Files (idle, window 3) and spin (in a Terminal, window 2).
fn stats() -> stat::Stats {
    use stat::*;
    let argv = |s: &str| s.split(' ').map(String::from).collect();
    let files = Proc { pid: 4, window: 3, state: IDLE, argv: argv("files ~"), counts: vec![9] };
    let spin = Proc { pid: 6, window: 2, state: RUNS, argv: argv("spin 9\u{e9}"), counts: vec![0] };
    let meters = vec![(4, vec![120, 2048]), (6, vec![u32::MAX, 1024])];
    let (loud, quiet) = ((0..LOUD as u32).collect(), vec![1, 2, 3, 4_000, 18_000]);
    Stats { at: 70_000, loud, procs: vec![files, spin], meters, quiet, own: vec![12, 9_000] }
}

#[test]
fn watch_end_and_stats_keep_their_codes() {
    let (watch, end) = (Request::Watch { on: true }, Request::End { pid: 0x0102_0304 });
    for r in [&watch, &Request::Watch { on: false }, &end] {
        strict(r, Request::encode, Request::decode);
    }
    let ev = Event::Stats { data: stats().encode() };
    strict(&ev, Event::encode, Event::decode);
    assert_eq!((watch.encode(), end.encode()), (vec![12, 1], vec![13, 4, 3, 2, 1]));
    assert_eq!(Event::Stats { data: vec![7] }.encode(), [16, 1, 0, 0, 0, 7]);
    assert!(Request::decode(&[12, 2]).is_none());
}

#[test]
fn stats_round_trip_and_decode_strictly() {
    use stat::*;
    let s = stats();
    strict(&s, Stats::encode, Stats::decode);
    strict(&Stats::default(), Stats::encode, Stats::decode);
    assert_eq!((s.meters_of(6), s.meters_of(5)), (&[u32::MAX, 1024][..], &[][..]));
    // The layout: version, at, the loud counts, the table's rows.
    let b = s.encode();
    assert_eq!(b[..9], [VERSION, 0x70, 0x11, 1, 0, LOUD as u8, 0, 0, 0]);
    let table = 5 + 2 + 4 * LOUD;
    assert_eq!(b[table..table + 6], [2, 0, 4, 0, 0, 0]);
    // Another version, a state past ENDED, a word not UTF-8, more words than it has, fewer rows
    // than it has.
    let row = table + 2;
    for (at, to) in [(0, 2), (row + 8, 3), (row + 15, 0xFF), (row + 9, 9), (table, 1)] {
        assert!(Stats::decode(&set(b.clone(), at, to)).is_none(), "{at} = {to}");
    }
    // Counts past the bytes left; a row with counts this reader does not know keeps them.
    let lie = Stats { loud: vec![0; 3], ..Stats::default() }.encode();
    assert!(Stats::decode(&set(lie, 5, 4)).is_none());
    let mut more = s.clone();
    more.procs[0].counts = vec![9, 7, 7];
    assert_eq!(Stats::decode(&more.encode()).unwrap().procs[0].counts[DRAWS], 9);
}

#[test]
fn pace_is_silent_at_rest_and_samples_at_most_once_a_second() {
    use stat::{GAP_MS, Pace};
    let mut p = Pace::default();
    // Unstirred: nothing due, no wake, however long.
    assert!(!p.due(0) && !p.due(1 << 40) && p.wait(5).is_none());
    // Stirs at 0, 300 and 900: one sample at 0, a wake for 1,000.
    p.stir = true;
    assert!(p.due(0) && p.took(0, 1, false));
    for t in [300, 900] {
        p.stir = true;
        assert_eq!((p.due(t), p.wait(t)), (false, Some((GAP_MS - t) as u32)));
    }
    // The change is posted, then once more as it rests (unchanged), then nothing.
    assert!(p.due(1000) && p.took(1000, 2, false));
    assert_eq!(p.wait(1000), Some(1000));
    assert!(p.due(2000) && p.took(2000, 2, false));
    assert!(!p.due(9000) && p.wait(9000).is_none());
    // What differs only in quiet parts hashes alike: a stirred look posts nothing.
    let quiet = stat::hash(b"loud");
    let mut p = Pace::default();
    p.stir = true;
    p.took(0, quiet, false);
    p.took(1000, quiet, false);
    p.stir = true;
    assert!(p.due(2000) && !p.took(2000, quiet, false) && p.wait(2000).is_none());
}

#[test]
fn pace_follows_a_hot_process_and_a_new_watcher_at_once() {
    use stat::Pace;
    let mut p = Pace::default();
    p.stir = true;
    assert!(p.took(0, 5, true));
    // Hot: one a second with no stir, posted while its time moves, until it stops running.
    for (t, h) in [(1000, 6), (2000, 7)] {
        assert!(p.due(t) && p.took(t, h, true) && p.wait(t) == Some(1000));
    }
    assert!(p.due(3000) && p.took(3000, 8, false)); // Ended: changed, cold.
    assert!(p.due(4000) && p.took(4000, 8, false) && !p.due(5000)); // Once more, then rest.
    // A new watcher hears at once, though a sample was just taken and nothing changed.
    p.fresh = true;
    assert!(p.due(4001) && p.took(4001, 8, false) && p.wait(4001) == Some(1000));
    assert_ne!(stat::hash(b"a"), stat::hash(b"b"));
    assert_eq!(stat::hash(b""), 0xcbf2_9ce4_8422_2325);
}

#[test]
fn a_receipt_is_read_whole_from_the_end_of_a_stream() {
    use stat::receipt;
    let end = b"data: [DONE]\n\n\n: receipt in=1200 out=30 microusd=1812\n\n";
    assert_eq!(receipt(end), Some([1200, 30, 1812]));
    // The last one counts, of up to 9 digits each. Cut short, garbled, out of order, with more
    // words, or only quoted inside data: none.
    let two = [&b"\n: receipt in=1 out=1 microusd=1\n"[..], &end[..]].concat();
    assert_eq!(receipt(&two), Some([1200, 30, 1812]));
    let big = b"\n: receipt in=999999999 out=0 microusd=0";
    assert_eq!(receipt(big), Some([999_999_999, 0, 0]));
    #[rustfmt::skip]
    let bad: [&[u8]; 8] = [&end[..end.len() - 12], b"\n: receipt in=1 out=2 microusd=3 x",
        b"\n: receipt in=1", b"\n: receipt in=1000000000 out=0 microusd=0",
        b"\n: receipt in= out=1 microusd=1", b"\n: receipt out=1 in=1 microusd=1",
        b"\n: receipt in=-1 out=1 microusd=1", b"data: {\"x\":\": receipt in=1 out=1 microusd=1\"}"];
    for b in bad {
        assert_eq!(receipt(b), None, "{:?}", String::from_utf8_lossy(b));
    }
}

#[test]
fn the_meshs_snapshots_and_frames_come_back_whole() {
    use crate::pool::{Device, Job, Snap, VERSION};
    use crate::relay::{Frame, HEAD};
    let d = |name: &str| Device {
        name: name.into(),
        cores: 16,
        units: 1 << 40,
        tx: 9,
        ..Device::default()
    };
    let job = Job {
        name: "fractal".into(),
        mine: true,
        total: 144,
        done: 3,
        busy: 7,
        per: vec![2, 1],
        ..Job::default()
    };
    let snap = Snap {
        at: 5,
        pairing: "Linking".into(),
        code: "K7".into(),
        devices: vec![d("a"), d("b")],
        job: Some(job),
    };
    let bytes = snap.encode();
    assert_eq!((bytes[0], Snap::decode(&bytes)), (VERSION, Some(snap.clone())));
    assert_eq!(Snap::decode(&[&bytes[..], &[0]].concat()), None, "trailing");
    assert_eq!(Snap::decode(&bytes[..bytes.len() - 1]), None, "short");
    assert_eq!(Snap::decode(&[&[VERSION + 1], &bytes[1..]].concat()), None, "another version");
    // Frames: none until one is whole, then each in turn with the bytes it took.
    let (a, b) = (Frame { op: 3, a: 7, b: 200, now: 9, data: b"v=0".to_vec() }, Frame::default());
    let mut out = Vec::new();
    a.put(&mut out);
    b.put(&mut out);
    assert_eq!(out.len(), 2 * HEAD + 3);
    assert_eq!(Frame::take(&out[..HEAD + 2]), None);
    assert_eq!(Frame::take(&out), Some((a, HEAD + 3)));
    assert_eq!(Frame::take(&out[HEAD + 3..]), Some((b, HEAD)));
}

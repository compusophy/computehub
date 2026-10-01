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
    let code =
        Node::Code { id: 7, version: 3, line_numbers: true, text: "let é = 1;".into(), spans };
    let children = vec![
        Node::Row { id: 0, gap: 4, children: row },
        Node::Input { id: 4, value: "abc".into(), placeholder: "name".into() },
        Node::Separator,
        Node::Spacer { px: 12 },
        Node::Card { id: 5, children: vec![item] },
        Node::Fill { id: 0, children: vec![code] },
    ];
    let requests = vec![Request::Open { name: "/apps/clock.app".into() }, Request::Close];
    let ai =
        vec![Request::Ai { id: 3, body: "{\"stream\":true}".into() }, Request::AiCancel { id: 3 }];
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
    assert_eq!(frame.nodes[0].count(), 11);
    [frame.clone(), Frame::default()].iter().for_each(|f| strict(f, Frame::encode, Frame::decode));
    let nodes = frame.nodes.iter().chain(frame.nodes[0].children());
    nodes.for_each(|n| strict(n, Node::encode, Node::decode));
    frame.requests.iter().for_each(|r| strict(r, Request::encode, Request::decode));
    events().iter().for_each(|e| strict(e, Event::encode, Event::decode));
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
    assert_eq!(([style, variant, class, key], Key::from_u8(0)), ([9, 3, 8, 8], None));
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

/// `bytes` with byte `at` set to `to`.
fn set(mut bytes: Vec<u8>, at: usize, to: u8) -> Vec<u8> {
    bytes[at] = to;
    bytes
}

#[test]
fn malformations_fail() {
    let ok = sample().encode();
    // The version; not UTF-8 in the title; the node count, one off either way.
    let at = 1 + 4 + 4 + "Studio é".len() + 2 + (1 + 4 + 15) + 1 + (1 + 4) + (9 + 15) + 5;
    assert_eq!(ok[at..at + 4], [12, 0, 0, 0]);
    for (i, to) in [(0, 2), (13, 0xFF), (at, 11), (at, 13)] {
        assert!(Frame::decode(&set(ok.clone(), i, to)).is_none(), "{i}");
    }
    // A leaf with a child; an id on a Separator or Spacer; unknown kinds.
    let leaf = [set(text(Style::Body, "a").encode(), 5, 1), Node::Separator.encode()].concat();
    assert!(Node::decode(&leaf).is_none());
    assert_eq!(Node::decode(&[7, 0, 0, 0, 0, 0, 0]), Some(Node::Separator));
    assert!(Node::decode(&[7, 1, 0, 0, 0, 0, 0]).is_none());
    assert!(Node::decode(&[8, 1, 0, 0, 0, 0, 0, 5, 0]).is_none());
    assert!([0, 12, 255].iter().all(|&kind| Node::decode(&[kind, 0, 0, 0, 0, 0, 0]).is_none()));
    // Codes: style, variant, selected, line-number flags, span class.
    assert!(Node::decode(&[3, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0]).is_none());
    assert!(Node::decode(&[4, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0]).is_none());
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
    assert!(key(0, 0, 0).is_none() && key(9, 0, 0).is_none());
    assert!(key(1, 16, 0).is_none() && key(1, 0, 0xD800).is_none());
    assert!(Event::decode(&[10]).is_none() && Request::decode(&[6]).is_none());
}

#[test]
fn fuzzed_decodes_never_panic_and_stay_canonical() {
    let mut corpus = vec![sample().encode(), Frame::default().encode()];
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
        for bytes in again.into_iter().chain(more).flatten() {
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
    let bad = Frame { nodes: vec![code("a", &[(0, 2)])], ..Frame::default() };
    assert_eq!(client.show(&bad).unwrap_err().kind(), ErrorKind::InvalidInput);
    drop(client);
    assert_eq!(out, sample().encode());

    // A sink that takes 8 bytes of the frame.
    let mut small = Client::new(io::empty(), Cursor::new([0; 8]));
    assert_eq!(small.show(&sample()).unwrap_err().kind(), ErrorKind::WriteZero);
}

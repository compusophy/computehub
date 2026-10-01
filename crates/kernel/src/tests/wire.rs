use super::Rng;
use crate::wire::*;

fn start() -> Start {
    Start {
        role: Role::Process,
        pid: 7,
        tty: Some((80, 24)),
        stdout: Stdout::Console,
        cwd: "/tmp/x".into(),
        roots: vec!["/".into()],
        argv: vec!["hello".into(), "a b".into(), "é".into()],
        env: vec!["K=V".into()],
    }
}

fn msgs() -> Vec<Msg<'static>> {
    vec![
        Msg::Ready { version: VERSION },
        Msg::Open { oflags: O_CREAT | O_EXCL, path: "/tmp/a" },
        Msg::Read { off: 1 << 40, max: 65_536, path: "/tmp/a" },
        Msg::Write { off: APPEND, path: "/tmp/a", data: b"hi" },
        Msg::List { skip: 3, path: "/" },
        Msg::Mkdir { path: "/tmp/d" },
        Msg::Remove { kind: KIND_DIR, path: "/tmp/d" },
        Msg::Rename { from: "/a", to: "/b" },
        Msg::SetLen { len: 9, path: "/a" },
        Msg::ConsBell,
        Msg::ConsWrite { data: b"out\n" },
        Msg::ConsRead { max: 4096 },
        Msg::ConsMode { bits: MODE_RAW | MODE_NOECHO },
        Msg::HomeState { state: HOME_SAVED, note: "saved" },
        Msg::Exit { status: -1 },
        Msg::Reply { errno: ENOENT, data: b"\x01" },
        Msg::Save,
    ]
}

#[test]
fn start_round_trips_and_rejects_every_truncation() {
    let home = Start {
        role: Role::Home,
        pid: HOME_PID,
        tty: None,
        stdout: Stdout::File { path: "/tmp/out".into(), append: true },
        cwd: "/home".into(),
        roots: vec!["/home".into()],
        argv: vec![],
        env: vec![],
    };
    let truncated = Start { stdout: Stdout::File { path: "/x".into(), append: false }, ..start() };
    for s in [start(), home, truncated] {
        let b = s.encode();
        assert_eq!(Start::decode(&b), Some(s));
        for n in 0..b.len() {
            assert_eq!(Start::decode(&b[..n]), None, "prefix {n}");
        }
        assert_eq!(Start::decode(&[b.as_slice(), &[0]].concat()), None);
    }
    assert_eq!(start().encode()[..12], [VERSION, 0, 7, 0, 0, 0, 1, 80, 0, 24, 0, 0]);
}

#[test]
fn start_rejects_bad_fields() {
    let b = start().encode();
    let with = |i: usize, v: u8| {
        let mut b = b.clone();
        b[i] = v;
        Start::decode(&b)
    };
    // version, role, flags, stdout kind.
    assert_eq!([with(0, 2), with(1, 2), with(6, 2), with(11, 3)], [None, None, None, None]);
    let console_with_path =
        Start { stdout: Stdout::File { path: "/x".into(), append: false }, ..start() }.encode();
    let mut bad = console_with_path.clone();
    bad[11] = 0;
    assert_eq!(Start::decode(&bad), None);
    // From the cwd on: "/a", or two bytes that are not UTF-8, then no lists.
    let cwd = |c: [u8; 2]| Start::decode(&[&b[..14], &[2, 0], &c, &[0; 6]].concat());
    assert_eq!(cwd(*b"/a").map(|s| s.cwd), Some("/a".into()));
    assert_eq!(cwd([0xFF, 0xFE]), None);
}

#[test]
fn messages_round_trip_and_truncations_never_decode_as_them() {
    for m in msgs() {
        let b = m.encode();
        assert_eq!(Msg::decode(&b), Some(m));
        let rest = matches!(m, Msg::Write { .. } | Msg::ConsWrite { .. } | Msg::Reply { .. });
        for n in 0..b.len() {
            let got = Msg::decode(&b[..n]);
            assert!(got != Some(m) && (rest || got.is_none()), "{m:?} prefix {n}");
        }
        if !rest {
            assert_eq!(Msg::decode(&[b.as_slice(), &[0]].concat()), None, "{m:?} + 1");
        }
    }
    assert_eq!(Msg::encode(&Msg::Exit { status: 130 }), [EXIT, 130, 0, 0, 0]);
    assert_eq!(Msg::encode(&Msg::Open { oflags: 0, path: "/a" }), [OPEN, 0, 2, 0, b'/', b'a']);
}

#[test]
fn bad_messages_decode_to_none() {
    for b in [&[][..], &[DRAW], &[EVENTS], &[0x7F], &[0xFF], &[MKDIR, 2, 0, 0xC3, 0x28]] {
        assert_eq!(Msg::decode(b), None, "{b:?}");
    }
    let path = "/a";
    let body = |n| Msg::Write { off: 0, path, data: &vec![7; n] }.encode();
    assert!(Msg::decode(&body(MAX_PAYLOAD)).is_some());
    assert_eq!(Msg::decode(&body(MAX_PAYLOAD + 1)), None);
}

#[test]
fn readers_stop_at_the_end_and_writers_bound_strings() {
    let mut r = Reader(&[1, 2, 3]);
    assert_eq!(
        (r.u32(), r.u16(), r.u16(), r.u8(), r.end()),
        (None, Some(513), None, Some(3), Some(()))
    );
    assert_eq!((r.take(1), r.rest(), r.str()), (None, &[][..], None));
    let long = "é".repeat(40_000);
    let b = Writer::default().str(&long).done();
    assert_eq!(&b[..2], 65_534u16.to_le_bytes());
    assert_eq!(Reader(&b).str().map(str::len), Some(65_534));
    let mut over = start();
    over.argv = vec!["x".repeat(70_000)];
    assert!(over.encode().len() > MAX_START);
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let valid: Vec<Vec<u8>> = msgs().iter().map(Msg::encode).chain([start().encode()]).collect();
    for i in 0..10_000 {
        let len = (rng.next() % 64) as usize;
        let mut b = rng.bytes(len);
        if i % 2 == 0 {
            // Mutate a valid message, so decoders get past the first byte.
            b = valid[i / 2 % valid.len()].clone();
            let at = rng.next() as usize % b.len();
            b[at] = rng.next() as u8;
        }
        let _ = (Msg::decode(&b), Start::decode(&b));
    }
}

#[test]
fn layout_and_errnos_hold() {
    assert_eq!((PAYLOAD_AT as usize + MAX_PAYLOAD) as u32, RING_AT);
    assert_eq!(RING_AT + RING_BYTES, SAB_BYTES);
    assert_eq!(MEM_PAGES * 65_536, 256 << 20);
    use vfs::VfsError::*;
    let all = [NotFound, NotADir, IsADir, Exists, NotEmpty, InvalidPath, NoSpace];
    assert_eq!(all.map(errno), [44, 54, 31, 20, 55, 28, 51]);
}

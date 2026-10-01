use super::Rng;
use crate::wire::*;

fn start() -> Start {
    let (stdout, cwd, roots) = (Stdout::Console, "/tmp/x".into(), vec!["/".into()]);
    let (argv, env) = (["hello", "a b", "é"].map(String::from).to_vec(), vec!["K=V".into()]);
    Start { role: Role::Process, pid: 7, tty: Some((80, 24)), stdout, cwd, roots, argv, env }
}
#[rustfmt::skip]
fn msgs() -> [Msg<'static>; 17] {
    [Msg::Ready { version: VERSION }, Msg::Open { oflags: O_CREAT | O_EXCL, path: "/tmp/a" },
        Msg::Read { off: 1 << 40, max: 65_536, path: "/tmp/a" }, Msg::List { skip: 3, path: "/" },
        Msg::Write { off: APPEND, path: "/tmp/a", data: b"hi" }, Msg::Mkdir { path: "/tmp/d" },
        Msg::Remove { kind: KIND_DIR, path: "/tmp/d" }, Msg::Rename { from: "/a", to: "/b" },
        Msg::SetLen { len: 9, path: "/a" }, Msg::ConsBell, Msg::ConsWrite { data: b"out\n" },
        Msg::ConsRead { max: 4096 }, Msg::ConsMode { bits: MODE_RAW | MODE_NOECHO },
        Msg::HomeState { state: HOME_SAVED, note: "saved" }, Msg::Exit { status: -1 },
        Msg::Reply { errno: ENOENT, data: b"\x01" }, Msg::Save]
}

#[test]
fn start_round_trips_and_rejects_every_truncation_and_bad_field() {
    let stdout = Stdout::File { path: "/tmp/out".into(), append: true };
    let (role, pid, cwd, roots) = (Role::Home, HOME_PID, "/home".into(), vec!["/home".into()]);
    let home = Start { role, pid, tty: None, stdout, cwd, roots, argv: vec![], env: vec![] };
    let truncated = Start { stdout: Stdout::File { path: "/x".into(), append: false }, ..start() };
    for s in [start(), home, truncated] {
        let b = s.encode();
        assert_eq!(Start::decode(&b), Some(s));
        (0..b.len()).for_each(|n| assert_eq!(Start::decode(&b[..n]), None, "prefix {n}"));
        assert_eq!(Start::decode(&[b.as_slice(), &[0]].concat()), None);
    }
    assert_eq!(start().encode()[..12], [VERSION, 0, 7, 0, 0, 0, 1, 80, 0, 24, 0, 0]);
    let b = start().encode();
    let f = Start { stdout: Stdout::File { path: "/x".into(), append: false }, ..start() }.encode();
    let with = |b: &[u8], i: usize, v: u8| Start::decode(&[&b[..i], &[v], &b[i + 1..]].concat());
    // version, role, flags, stdout kind; console stdout (0) with a path.
    let bad = [with(&b, 0, 2), with(&b, 1, 2), with(&b, 6, 2), with(&b, 11, 3), with(&f, 11, 0)];
    assert_eq!(bad, [None, None, None, None, None]);
    // From the cwd on: "/a", or two bytes that are not UTF-8, then no lists.
    let cwd = |c: [u8; 2]| Start::decode(&[&b[..14], &[2, 0], &c, &[0; 6]].concat());
    assert_eq!((cwd(*b"/a").map(|s| s.cwd), cwd([0xFF, 0xFE])), (Some("/a".into()), None));
}

#[test]
fn messages_round_trip_and_truncated_or_bad_ones_never_decode_as_them() {
    for m in msgs() {
        let b = m.encode();
        assert_eq!(Msg::decode(&b), Some(m));
        let rest = matches!(m, Msg::Write { .. } | Msg::ConsWrite { .. } | Msg::Reply { .. });
        for n in 0..b.len() {
            let got = Msg::decode(&b[..n]);
            assert!(got != Some(m) && (rest || got.is_none()), "{m:?} prefix {n}");
        }
        assert!(rest || Msg::decode(&[b.as_slice(), &[0]].concat()).is_none(), "{m:?} + 1");
    }
    assert_eq!(Msg::encode(&Msg::Exit { status: 130 }), [EXIT, 130, 0, 0, 0]);
    assert_eq!(Msg::encode(&Msg::Open { oflags: 0, path: "/a" }), [OPEN, 0, 2, 0, b'/', b'a']);
    for b in [&[][..], &[DRAW], &[EVENTS], &[0x7F], &[0xFF], &[MKDIR, 2, 0, 0xC3, 0x28]] {
        assert_eq!(Msg::decode(b), None, "{b:?}");
    }
    let body = |n| Msg::Write { off: 0, path: "/a", data: &vec![7; n] }.encode();
    assert!(Msg::decode(&body(MAX_PAYLOAD)).is_some());
    assert_eq!(Msg::decode(&body(MAX_PAYLOAD + 1)), None);
}

#[test]
fn readers_stop_at_the_end_and_writers_bound_strings() {
    let mut r = Reader(&[1, 2, 3]);
    let got = (r.u32(), r.u16(), r.u16(), r.u8(), r.end());
    assert_eq!(got, (None, Some(513), None, Some(3), Some(())));
    assert_eq!((r.take(1), r.rest(), r.str()), (None, &[][..], None));
    let long = "é".repeat(40_000);
    let b = Writer::default().str(&long).done();
    assert_eq!(&b[..2], 65_534u16.to_le_bytes());
    assert_eq!(Reader(&b).str().map(str::len), Some(65_534));
    assert!(Start { argv: vec!["x".repeat(70_000)], ..start() }.encode().len() > MAX_START);
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
fn layout_errnos_and_fnv64_hold() {
    assert_eq!((PAYLOAD_AT as usize + MAX_PAYLOAD) as u32, RING_AT);
    assert_eq!((RING_AT + RING_BYTES, MEM_PAGES * 65_536), (SAB_BYTES, 256 << 20));
    use vfs::VfsError::*;
    let all = [NotFound, NotADir, IsADir, Exists, NotEmpty, InvalidPath, NoSpace];
    assert_eq!(all.map(errno), [44, 54, 31, 20, 55, 28, 51]);
    let fnv = [b"", &b"a"[..], b"foobar"].map(crate::snap::fnv64);
    assert_eq!(fnv, [0xcbf2_9ce4_8422_2325, 0xaf63_dc4c_8601_ec8c, 0x8594_4171_f739_67e8]);
}

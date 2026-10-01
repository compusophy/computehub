use super::*;

/// One reported event; CSI and ESC rebuilt in canonical form (`?2026$p`).
#[derive(Debug, PartialEq, Eq)]
#[rustfmt::skip]
enum Ev { Text(String), Exec(u8), Csi(String), Esc(String), Osc(Vec<Vec<u8>>) }

/// Records events and asserts the documented limits on every one.
#[derive(Default)]
struct Rec(Vec<Ev>);

impl Perform for Rec {
    fn print(&mut self, c: char) {
        match self.0.last_mut() {
            Some(Ev::Text(s)) => s.push(c),
            _ => self.0.push(Ev::Text(c.into())),
        }
    }
    fn execute(&mut self, byte: u8) {
        assert!(byte < 0x20 && byte != 0x1B);
        self.0.push(Ev::Exec(byte));
    }
    fn csi(&mut self, params: &Params, inter: &[u8], private: Option<u8>, action: u8) {
        assert!(params.len() <= MAX_PARAMS && inter.len() <= MAX_INTERMEDIATES);
        assert!(private.is_none_or(|p| b"<=>?".contains(&p)));
        assert!((0x40..=0x7E).contains(&action));
        let mut s: String = private.map(char::from).into_iter().collect();
        for (i, group) in params.iter().enumerate() {
            assert!((1..=MAX_SUBPARAMS).contains(&group.len()));
            assert_eq!((params.get(i), params.sub(i)), (group[0], group));
            for (j, v) in group.iter().enumerate() {
                s.extend((i + j > 0).then_some(if j == 0 { ';' } else { ':' }));
                s.extend(v.map(|v| v.to_string()));
            }
        }
        s.extend(inter.iter().chain([&action]).map(|&b| char::from(b)));
        self.0.push(Ev::Csi(s));
    }
    fn esc(&mut self, inter: &[u8], byte: u8) {
        assert!(inter.len() <= MAX_INTERMEDIATES && (0x30..=0x7E).contains(&byte));
        let s = inter.iter().chain([&byte]).map(|&b| char::from(b));
        self.0.push(Ev::Esc(s.collect()));
    }
    fn osc(&mut self, params: &[&[u8]]) {
        assert!((1..=MAX_OSC_PARAMS).contains(&params.len()));
        assert!(params.iter().map(|p| p.len() + 1).sum::<usize>() <= MAX_OSC + 1);
        self.0.push(Ev::Osc(params.iter().map(|p| p.to_vec()).collect()));
    }
}

fn events(chunks: &[&[u8]]) -> Vec<Ev> {
    let (mut parser, mut rec) = (Parser::default(), Rec::default());
    chunks.iter().for_each(|chunk| parser.advance(chunk, &mut rec));
    rec.0
}

/// Events as text: controls as `^J`, sequences in braces without the ESC
/// (`{[1;2H}`, `{(B}`, `{]0|title}` with OSC parts joined by `|`), U+FFFD as `~`.
fn log(bytes: &[u8]) -> String {
    let mut out = String::new();
    for ev in events(&[bytes]) {
        match ev {
            Ev::Text(s) => out += &s.replace('\u{FFFD}', "~"),
            Ev::Exec(b) => out.extend(['^', char::from(b + 0x40)]),
            Ev::Csi(s) => out += &format!("{{[{s}}}"),
            Ev::Esc(s) => out += &format!("{{{s}}}"),
            Ev::Osc(parts) => {
                let parts: Vec<_> = parts.iter().map(|p| String::from_utf8_lossy(p)).collect();
                out += &format!("{{]{}}}", parts.join("|"));
            }
        }
    }
    out
}

#[test]
fn logs() {
    let cases: &[(&[u8], &str)] = &[
        (b"hi\r\n\x07\x08\tx\x7f\x00y", "hi^M^J^G^H^Ix^@y"),
        // Invalid UTF-8: overlong, surrogate, too high, cut short.
        (b"\x80\xC0\x80\xE0\x80\x80\xED\xA0\x80", "~~~~~~~~~"),
        (b"\xF4\x90\x80\x80\xF5\xFF", "~~~~~~"),
        (b"\xE2\x82x\xE2\x18\xF0\x9F\x98\x1b[m", "~x~^X~{[m}"),
        // C1 bytes are UTF-8 continuation bytes, never controls.
        (b"\x9b31m\x9d0;x\x07\xC2\x9b", "~31m~0;x^G\u{9b}"),
        // CSI parameters, private markers, intermediates, sub-parameters.
        (b"\x1b[1;2H\x1b[H\x1b[;5H\x1b[5;H", "{[1;2H}{[H}{[;5H}{[5;H}"),
        (b"\x1b[;m\x1b[007m\x1b[?25l\x1b[>c", "{[;m}{[7m}{[?25l}{[>c}"),
        (b"\x1b[<u\x1b[=1;1u\x1b[?2026$p", "{[<u}{[=1;1u}{[?2026$p}"),
        (b"\x1b[2 q\x1b[1$\"p\x1b[:m", "{[2 q}{[1$\"p}{[:m}"),
        (b"\x1b[38:2::1:2:3m\x1b[1;4:3;58:5:9m", "{[38:2::1:2:3m}{[1;4:3;58:5:9m}"),
        (b"\x1b[1:2:3:4:5:6:7:8:9:10;7:1:2:3:4:5:6:7:8:9m", "{[1:2:3:4:5:6:7:8;7:1:2:3:4:5:6:7m}"),
        // Dropped: a marker after a param, a param after an intermediate, too
        // many intermediates, non-ASCII. DEL is ignored.
        (b"\x1b[1?hA\x1b[ 1mB\x1b[1$$$pC\x1b[1\xC3\xA9mD\x1b[1\x7fm", "ABCD{[1m}"),
        // C0 controls execute inside sequences, but not strings.
        (b"\x1b[1\n;2\rH\x1b(\x08B\x1b[1?\x07h\x1b]0;a\nb\x07", "^J^M{[1;2H}^H{(B}^G{]0|ab}"),
        // ESC dispatches, and always restarts; CAN and SUB abort.
        (b"\x1b7\x1b(B\x1b#8\x1b\\\x1bc\x1b(((B\x1b\xC3\xA9", "{7}{(B}{#8}{\\}{c}\u{e9}"),
        (b"\x1b(\xC3\xA9\x1b[1;\x1b[2m\x1b[?1\x1b7\x1b(\x1b\x1b[m", "\u{e9}{[2m}{7}{[m}"),
        (b"\x1b[1;2\x18m\x1b(\x1aB\x1b]0;t\x1a\x07", "^Xm^ZB^Z^G"),
        (b"\x1bPq#0\x18x\x1b\x18\\", "^Xx^X\\"),
        // OSC terminators and splitting.
        (b"\x1b]0;title\x07\x1b]2;t\x1b\\x\x1b]\x07", "{]0|title}{]2|t}x{]}"),
        (b"\x1b]8;id=1;http://a/?q;2\x1b\\go", "{]8|id=1|http://a/?q|2}go"),
        (b"\x1b]0;h\xC3\xA9\x07\x1b]8;;\x07\x1b]0;a\x1b[m", "{]0|h\u{e9}}{]8||}{]0|a}{[m}"),
        // DCS, SOS, PM and APC are ignored.
        (b"\x1bP1$r0m\x1b\\A\x1bP+q544e\x1b\\\x1bXsos\x1b\\\x1b^pm\x1b\\B", "AB"),
        (b"\x1b_Gf=1;AAAA\x07still\x1b\\C\x1bPq#0\x1b[1m", "C{[1m}"),
    ];
    for &(bytes, want) in cases {
        assert_eq!(log(bytes), want, "{bytes:?}");
    }
}

#[test]
fn chunking_never_changes_the_result() {
    let s = "aé€😀z";
    let b = s.as_bytes();
    for i in 0..=b.len() {
        for j in i..=b.len() {
            assert_eq!(events(&[&b[..i], &b[i..j], &b[j..]]), [Ev::Text(s.into())], "{i} {j}");
        }
    }
    let bytewise = |b: &'static [u8]| events(&b.chunks(1).collect::<Vec<_>>());
    assert_eq!(bytewise(s.as_bytes()), [Ev::Text(s.into())]);
    let parts = [&b"52"[..], b"c", b"aGk="].map(<[u8]>::to_vec);
    assert_eq!(bytewise(b"\x1b]52;c;aGk=\x1b\\"), [Ev::Osc(parts.into())]);
    let sample: &[u8] = b"a\x1b[?2026h\x1b]8;;http://x\x1b\\link\x1b]8;;\x07\x1b[38:2::1:2:3m\
        \xF0\x9F\x98\x80\x1bP+q\x1b\\\x1b(0q\x1b[2 q\xE2\x82";
    assert_eq!(bytewise(sample), events(&[sample]));
    let want = "a{[?2026h}{]8||http://x}link{]8||}{[38:2::1:2:3m}😀{(0}q{[2 q}";
    assert_eq!(log(sample), want);
}

#[test]
fn params_api() {
    struct Grab(Params);
    impl Perform for Grab {
        fn csi(&mut self, params: &Params, _: &[u8], _: Option<u8>, _: u8) {
            self.0 = params.clone();
        }
    }
    let mut grab = Grab(Params::default());
    assert!(grab.0.is_empty() && grab.0.get(0).is_none() && grab.0.sub(0).is_empty());
    Parser::default().advance(b"\x1b[4:3;;99999999m", &mut grab);
    let p = grab.0;
    assert_eq!((p.len(), p.is_empty(), p.iter().count()), (3, false, 3));
    let (n, big) = (None, Some(u16::MAX));
    assert_eq!([p.get(0), p.get(1), p.get(2), p.get(3)], [Some(4), n, big, n]);
    assert_eq!([p.sub(0), p.sub(1), p.sub(3)], [&[Some(4), Some(3)][..], &[n], &[]]);
    assert_eq!(format!("{p:?}"), "[[Some(4), Some(3)], [None], [Some(65535)]]");
}

#[test]
fn limits_saturate_drop_and_truncate() {
    let nines = "9".repeat(1000);
    assert_eq!(log(format!("\x1b[{nines};65535;65536m").as_bytes()), "{[65535;65535;65535m}");
    let n40: Vec<String> = (0..40).map(|i| i.to_string()).collect();
    let n32 = n40[..32].join(";");
    assert_eq!(log(format!("\x1b[{}m", n40.join(";")).as_bytes()), format!("{{[{n32}m}}"));
    assert_eq!(log(format!("\x1b[{}:7:7;x", n40.join(";")).as_bytes()), format!("{{[{n32}x}}"));
    // At most MAX_OSC_PARAMS parts, the last keeping the rest, of MAX_OSC bytes.
    let want = format!("{{]{}|{}}}", n40[..15].join("|"), n40[15..20].join(";"));
    assert_eq!(log(format!("\x1b]{}\x07", n40[..20].join(";")).as_bytes()), want);
    let evs = log(format!("\x1b]0;{}\x07ok", "a".repeat(10_000)).as_bytes());
    assert_eq!(evs, format!("{{]0|{}}}ok", "a".repeat(MAX_OSC - 2)));
}

#[test]
fn fuzz_lcg_never_panics_and_ignores_chunking() {
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (x >> 33) as u32
    };
    let pick: &[u8] = b"\x1b\x1b\x1b[[]];;::?>=<0123456789m$ \"\x07\x18\x1a\\P_X^\
        \xE2\x82\xAC\xF0\x9F\xC2\x80\xFF\x7F\n";
    let bytes: Vec<u8> = (0..100_000)
        .map(|_| match next() {
            r if r % 4 == 0 => (r >> 8) as u8,
            r => pick[(r >> 8) as usize % pick.len()],
        })
        .collect();
    let (mut chunks, mut rest) = (Vec::new(), &bytes[..]);
    while !rest.is_empty() {
        let (chunk, tail) = rest.split_at(((next() % 64) as usize + 1).min(rest.len()));
        chunks.push(chunk);
        rest = tail;
    }
    let whole = events(&[&bytes]);
    assert_eq!(events(&chunks), whole);
    #[rustfmt::skip]
    let kinds: [fn(&Ev) -> bool; 5] = [
        |e| matches!(e, Ev::Text(_)), |e| matches!(e, Ev::Exec(_)), |e| matches!(e, Ev::Esc(_)),
        |e| matches!(e, Ev::Csi(s) if s.contains(':')), |e| matches!(e, Ev::Osc(p) if p.len() > 1),
    ];
    assert_eq!(kinds.map(|k| whole.iter().any(k)), [true; 5]);
}

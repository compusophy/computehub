use super::*;

/// One reported event. CSI and ESC are kept in their canonical byte form
/// (`?2026$p`, `(B`), rebuilt from what the parser reported.
#[derive(Debug, PartialEq, Eq)]
enum Ev {
    Text(String),
    Exec(u8),
    Csi(String),
    Esc(String),
    Osc(Vec<Vec<u8>>),
}

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
                match (i, j) {
                    (0, 0) => {}
                    (_, 0) => s.push(';'),
                    _ => s.push(':'),
                }
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
    let (mut parser, mut rec) = (Parser::new(), Rec::default());
    for chunk in chunks {
        parser.advance(chunk, &mut rec);
    }
    rec.0
}

/// Events as text: controls in caret notation (`^J`), and each sequence in
/// braces, ESC dropped: `{[1;2H}`, `{(B}`, and `{]0|title}` with the OSC's
/// parts joined by `|`.
fn log(bytes: &[u8]) -> String {
    let mut out = String::new();
    for ev in events(&[bytes]) {
        match ev {
            Ev::Text(s) => out += &s,
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

/// `~` stands for U+FFFD.
fn r(s: &str) -> String {
    s.replace('~', "\u{FFFD}")
}

#[test]
fn ascii_and_c0() {
    assert_eq!(log(b"hi\r\n\x07\x08\tx\x7f\x00y"), "hi^M^J^G^H^Ix^@y");
}

#[test]
fn utf8_split_anywhere() {
    let s = "aé€😀z";
    let b = s.as_bytes();
    for i in 0..=b.len() {
        for j in i..=b.len() {
            let evs = events(&[&b[..i], &b[i..j], &b[j..]]);
            assert_eq!(evs, [Ev::Text(s.into())], "{i} {j}");
        }
    }
    let bytewise: Vec<&[u8]> = b.chunks(1).collect();
    assert_eq!(events(&bytewise), [Ev::Text(s.into())]);
}

#[test]
fn utf8_invalid_yields_replacement() {
    assert_eq!(log(b"\x80\xC0\x80\xE0\x80\x80"), r("~~~~~~")); // overlong
    assert_eq!(log(b"\xED\xA0\x80"), r("~~~")); // surrogate
    assert_eq!(log(b"\xF4\x90\x80\x80\xF5\xFF"), r("~~~~~~")); // too high
    assert_eq!(log(b"\xE2\x82x\xE2\x18"), r("~x~^X")); // cut short
    assert_eq!(log(b"\xF0\x9F\x98\x1b[m"), r("~{[m}"));
    // C1 bytes are UTF-8 continuation bytes, never controls.
    assert_eq!(log(b"\x9b31m\x9d0;x\x07\xC2\x9b"), r("~31m~0;x^G\u{9b}"));
}

#[test]
fn csi_params_markers_intermediates() {
    let evs = log(b"\x1b[1;2H\x1b[H\x1b[;5H\x1b[5;H\x1b[;m\x1b[007m");
    assert_eq!(evs, "{[1;2H}{[H}{[;5H}{[5;H}{[;m}{[7m}");
    let evs = log(b"\x1b[?25l\x1b[>c\x1b[<u\x1b[=1;1u\x1b[?2026$p\x1b[2 q\x1b[1$\"p");
    assert_eq!(evs, "{[?25l}{[>c}{[<u}{[=1;1u}{[?2026$p}{[2 q}{[1$\"p}");
    let evs = log(b"\x1b[38:2::1:2:3m\x1b[1;4:3;58:5:9m\x1b[:m");
    assert_eq!(evs, "{[38:2::1:2:3m}{[1;4:3;58:5:9m}{[:m}");
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
    Parser::new().advance(b"\x1b[4:3;;99999999m", &mut grab);
    let p = grab.0;
    assert_eq!((p.len(), p.is_empty(), p.iter().count()), (3, false, 3));
    let (n, big) = (None, Some(u16::MAX));
    assert_eq!([p.get(0), p.get(1), p.get(2), p.get(3)], [Some(4), n, big, n]);
    assert_eq!([p.sub(0), p.sub(1), p.sub(3)], [&[Some(4), Some(3)][..], &[n], &[]]);
    assert_eq!(format!("{p:?}"), "[[Some(4), Some(3)], [None], [Some(65535)]]");
}

#[test]
fn csi_saturates_and_drops_overflow() {
    let nines = "9".repeat(1000);
    let evs = log(format!("\x1b[{nines};65535;65536m").as_bytes());
    assert_eq!(evs, "{[65535;65535;65535m}");
    let n40: Vec<String> = (0..40).map(|i| i.to_string()).collect();
    let n32 = n40[..32].join(";");
    let evs = log(format!("\x1b[{}m", n40.join(";")).as_bytes());
    assert_eq!(evs, format!("{{[{n32}m}}"));
    let evs = log(format!("\x1b[{}:7:7;x", n40.join(";")).as_bytes());
    assert_eq!(evs, format!("{{[{n32}x}}"));
    let evs = log(b"\x1b[1:2:3:4:5:6:7:8:9:10;7:1:2:3:4:5:6:7:8:9m");
    assert_eq!(evs, "{[1:2:3:4:5:6:7:8;7:1:2:3:4:5:6:7m}");
}

#[test]
fn csi_malformed_is_consumed_not_dispatched() {
    // A marker after a param, a param after an intermediate, too many
    // intermediates, and non-ASCII bytes.
    assert_eq!(log(b"\x1b[1?hA\x1b[ 1mB\x1b[1$$$pC\x1b[1\xC3\xA9mD"), "ABCD");
    assert_eq!(log(b"\x1b[1\x7fm"), "{[1m}"); // DEL is ignored
}

#[test]
fn c0_executes_inside_sequences() {
    let evs = log(b"\x1b[1\n;2\rH\x1b(\x08B\x1b[1?\x07h\x1b]0;a\nb\x07");
    assert_eq!(evs, "^J^M{[1;2H}^H{(B}^G{]0|ab}");
}

#[test]
fn esc_dispatch_and_restart() {
    assert_eq!(log(b"\x1b7\x1b(B\x1b#8\x1b\\\x1bc\x1b(((B"), "{7}{(B}{#8}{\\}{c}");
    assert_eq!(log(b"\x1b\xC3\xA9\x1b(\xC3\xA9"), "éé");
    assert_eq!(log(b"\x1b[1;\x1b[2m\x1b[?1\x1b7\x1b(\x1b\x1b[m"), "{[2m}{7}{[m}");
}

#[test]
fn can_and_sub_abort() {
    let evs = log(b"\x1b[1;2\x18m\x1b(\x1aB\x1b]0;t\x1a\x07\x1bPq#0\x18x\x1b\x18\\");
    assert_eq!(evs, "^Xm^ZB^Z^G^Xx^X\\");
}

#[test]
fn osc_terminators_and_splitting() {
    let evs = log(b"\x1b]0;title\x07\x1b]2;t\x1b\\x\x1b]\x07\x1b]0;h\xC3\xA9\x07");
    assert_eq!(evs, "{]0|title}{]2|t}x{]}{]0|hé}");
    let evs = log(b"\x1b]8;id=1;http://a/?q;2\x1b\\go\x1b]8;;\x07\x1b]0;a\x1b[m");
    assert_eq!(evs, "{]8|id=1|http://a/?q|2}go{]8||}{]0|a}{[m}");
    // At most MAX_OSC_PARAMS parts; the last keeps the rest.
    let n20: Vec<String> = (0..20).map(|i| i.to_string()).collect();
    let want = format!("{{]{}|{}}}", n20[..15].join("|"), n20[15..].join(";"));
    assert_eq!(log(format!("\x1b]{}\x07", n20.join(";")).as_bytes()), want);
    let bytewise: Vec<&[u8]> = b"\x1b]52;c;aGk=\x1b\\".chunks(1).collect();
    let parts = [&b"52"[..], b"c", b"aGk="].map(<[u8]>::to_vec);
    assert_eq!(events(&bytewise), [Ev::Osc(parts.into())]);
}

#[test]
fn osc_overflow_is_truncated() {
    let evs = log(format!("\x1b]0;{}\x07ok", "a".repeat(10_000)).as_bytes());
    assert_eq!(evs, format!("{{]0|{}}}ok", "a".repeat(MAX_OSC - 2)));
}

#[test]
fn dcs_sos_pm_apc_are_ignored() {
    let evs = log(b"\x1bP1$r0m\x1b\\A\x1bP+q544e\x1b\\\x1bXsos\x1b\\\x1b^pm\x1b\\B");
    assert_eq!(evs, "AB");
    let evs = log(b"\x1b_Gf=1;AAAA\x07still\x1b\\C\x1bPq#0\x1b[1m");
    assert_eq!(evs, "C{[1m}");
}

#[test]
fn chunking_never_changes_the_result() {
    let sample: &[u8] = b"a\x1b[?2026h\x1b]8;;http://x\x1b\\link\x1b]8;;\x07\x1b[38:2::1:2:3m\
        \xF0\x9F\x98\x80\x1bP+q\x1b\\\x1b(0q\x1b[2 q\xE2\x82";
    let bytewise: Vec<&[u8]> = sample.chunks(1).collect();
    assert_eq!(events(&bytewise), events(&[sample]));
    let want = "a{[?2026h}{]8||http://x}link{]8||}{[38:2::1:2:3m}😀{(0}q{[2 q}";
    assert_eq!(log(sample), want);
}

#[test]
fn fuzz_lcg_never_panics_and_ignores_chunking() {
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        x = x.wrapping_mul(6_364_136_223_846_793_005);
        x = x.wrapping_add(1_442_695_040_888_963_407);
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
    let mut chunks = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let end = (at + (next() % 64) as usize + 1).min(bytes.len());
        chunks.push(&bytes[at..end]);
        at = end;
    }
    let whole = events(&[&bytes]);
    assert_eq!(events(&chunks), whole);
    let kinds = [
        whole.iter().any(|e| matches!(e, Ev::Text(_))),
        whole.iter().any(|e| matches!(e, Ev::Exec(_))),
        whole.iter().any(|e| matches!(e, Ev::Csi(s) if s.contains(':'))),
        whole.iter().any(|e| matches!(e, Ev::Esc(_))),
        whole.iter().any(|e| matches!(e, Ev::Osc(p) if p.len() > 1)),
    ];
    assert_eq!(kinds, [true; 5]);
}

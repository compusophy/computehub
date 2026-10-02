use super::*;
use crate::{Glyph, Point, outline as glyph};
use font::{Bitmap, render_outline};

/// The design's examples: the shots' (snake, todo) and what the card steers a model toward.
#[rustfmt::skip]
const EXAMPLES: [(&str, &str); 14] = [
    ("tic-tac-toe", "line 2 7 10 15 line 10 7 2 15 ring 17 11 4"),
    ("snake", "line 4 20 4 13 11 13 11 20 19 20 19 9 dot 19 6 3 dot 7 5 2"),
    ("todo", "line 3 7 6 10 11 4 line 14 7 21 7 loop 4 14 9 14 9 19 4 19 line 14 17 21 17"),
    ("tetris", "fill 2 6 8 6 8 12 2 12 fill 9 6 15 6 15 12 9 12 fill 16 6 22 6 22 12 16 12 \
        fill 9 13 15 13 15 19 9 19"),
    ("pomodoro", "ring 12 13 8 line 12 9 12 13 15 15 line 10 2 14 2"),
    ("timer", "arc 12 13 8 0 270 line 12 13 12 7 line 9 2 15 2"),
    ("dice", "loop 3 3 21 3 21 21 3 21 dot 8 8 2 dot 12 12 2 dot 16 16 2"),
    ("drawing pad", "loop 3 21 4 16 16 4 20 8 8 20 line 14 6 18 10"),
    ("habit tracker", "loop 3 3 21 3 21 21 3 21 line 7 12 11 16 17 8"),
    ("tip calculator", "ring 7 7 3 ring 17 17 3 line 19 4 5 20"),
    ("music", "line 9 18 9 5 19 3 19 15 dot 6 18 3 dot 16 15 3"),
    ("favorites", "fill 12 2 15 9 22 9 16 14 18 21 12 17 6 21 8 14 2 9 9 9"),
    ("clock", "ring 12 12 9 line 12 7 12 12 16 14"),
    ("notes", "loop 5 3 19 3 19 21 5 21 line 8 8 16 8 line 8 12 16 12 line 8 16 13 16"),
];

fn made(text: &str) -> Made {
    Made::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{text}: {e:?}"))
}

fn raster(o: &[Vec<Point>], px: i32) -> Bitmap {
    let mut b = Bitmap::default();
    render_outline(o, px as f32 / 1000.0, &mut b).unwrap();
    b
}

fn drawn(m: &Made, px: i32) -> Bitmap {
    let mut o = vec![vec![pt(0.0, 0.0)]];
    m.outline(&mut o);
    raster(&o, px)
}

/// Whether the bitmap's ink lies within `slack` px of the box (x 0..px, y -px..0).
fn inside(b: &Bitmap, px: i32, slack: i32) -> bool {
    let (x0, y0) = (b.left, b.top);
    let (x1, y1) = (x0 + b.w as i32, y0 + b.h as i32);
    b.w == 0 || x0 >= -slack && y0 >= -px - slack && x1 <= px + slack && y1 <= slack
}

fn ink(b: &Bitmap) -> u32 {
    b.data.iter().map(|&c| u32::from(c)).sum::<u32>() / 255
}

#[test]
fn made_icons_parse_the_documented_forms() {
    for (name, text) in EXAMPLES {
        let m = made(text);
        assert!(text.len() <= 101 && m.len > 0, "{name}");
        // Written another way, the same icon: one hash, one value.
        let other = text.replacen(' ', ",", 3).replacen(' ', " ;\t", 2);
        assert_eq!(made(&other), m, "{name}: {other}");
        assert_eq!(made(&[" ", text, " ;"].concat()), m, "{name}");
    }
    let all = EXAMPLES.map(|e| made(e.1).hash());
    assert!(all.iter().enumerate().all(|(i, h)| !all[..i].contains(h)));
    // The edges of the grid are allowed, and the smallest circles.
    for ok in
        ["line 0 0 24 24", "ring 12 12 12", "dot 1 23 1", "arc 2 2 2 360 0", "fill 0 0 9 0 0 9"]
    {
        made(ok);
    }
    // Codes, E0931 to E0937, in order.
    let err = |why| IconError { at: 0, why };
    assert_eq!((err(Why::Text).code(), err(Why::Empty).code()), (931, 937));
}

#[test]
fn made_icons_refuse_what_they_cannot_draw() {
    use Why::*;
    let dots = "dot 12 12 3 ".repeat(17);
    let lines = "line 1 1 2 2 3 3 4 4 5 5 6 6 7 7 8 8 ".repeat(5);
    let long = ["dot 12 12 3", &" ".repeat(390)].concat();
    #[rustfmt::skip]
    let table: [(&str, Why, u16); 27] = [
        ("", Empty, 0), (" ,; \t", Empty, 0),
        ("Line 1 1 5 5", Word, 0), ("L 1 1 5 5", Word, 0), ("C 12 12 3", Word, 0),
        ("line 1.5 1 5 5", Word, 5), ("line -1 1 5 5", Word, 5), ("rect 2 2 9 9", Word, 0),
        ("line 1 1 5 5 \u{e9}", Word, 13), ("dot 12 12 3\nline 1 1 5 5", Word, 10),
        ("3 line 1 1 5 5", Number, 0), ("line 1 1 5 5 1000", Number, 13),
        ("line 0001 1 5 5", Number, 5),
        ("line 1 1", Count, 0), ("line 1 1 5", Count, 0), ("loop 1 1 5 5", Count, 0),
        ("ring 12 12", Count, 0), ("dot 1 1 1 line 1 1", Count, 10), ("arc 12 12 6 0", Count, 0),
        ("line 1 1 25 5", Range, 0), ("ring 12 12 1", Range, 0), ("ring 2 12 5", Range, 0),
        ("dot 12 12 0", Range, 0), ("arc 12 12 6 0 361", Range, 0),
        (&dots, Many, 192), (&lines, Many, 4 * 37 + 5), (&long, Text, 0),
    ];
    for (text, why, at) in table {
        assert_eq!(Made::parse(text.as_bytes()), Err(IconError { at, why }), "{text:?}");
    }
    // A shape of 12 points takes 24 numbers; of 13, too many.
    let pts = |n: usize| ["line", &" 5 5".repeat(n)].concat();
    assert!(Made::parse(pts(12).as_bytes()).is_ok());
    assert_eq!(Made::parse(pts(13).as_bytes()).map_err(|e| e.why), Err(Count));
}

#[test]
fn a_header_is_read_only_from_the_leading_comments() {
    let icon = Some(&b" dot 12 12 3"[..]);
    for file in [
        "// Tic-tac-toe: x.\n// icon: dot 12 12 3\nstate n = 0;\n",
        "// A\r\n// icon: dot 12 12 3\r\nlabel 1;",
        "\n\n// A\n\n// icon: dot 12 12 3\n\n",
        "  // A\n   // icon: dot 12 12 3  \nlabel 1;",
        "// icon: dot 12 12 3",
        "// icon: dot 12 12 3\n// icon: ring 12 12 5\n",
    ] {
        assert_eq!(header(file.as_bytes()), icon, "{file:?}");
    }
    let wide = ["// ", &"a".repeat(5000), "\n// icon: dot 12 12 3\n"].concat();
    // Cut by the head: the line is never read, so never as `dot 1`.
    let cut =
        ["// ", &"a".repeat(HEAD - 14), "\n// icon: dot 12 12 3 dot 1 1 1\nlabel 1;\n"].concat();
    assert!(cut.find("icon").unwrap() < HEAD && cut.find("dot 1 1").unwrap() > HEAD);
    for file in [
        "// A\nstate n = 0;\n// icon: dot 12 12 3\n",
        "/* A */\n// icon: dot 12 12 3\n",
        "//icon: dot 12 12 3\n",
        "// Icon: dot 12 12 3\n",
        "label 1; // icon: dot 12 12 3\n",
        &wide,
        &cut,
    ] {
        assert_eq!(header(file.as_bytes()), None, "{file:?}");
    }
    // Read whole or not at all.
    assert_eq!(read(b"// A\n// icon: dot 12 12 3\n"), Made::parse(b"dot 12 12 3").ok());
    assert_eq!(read(b"// A\n// icon: dot 12 12 3 L\n"), None);
}

/// A deterministic xorshift.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x % n
    }
}

/// A shape as a model might write it, now and then wrong.
fn shape(r: &mut Rng, out: &mut String) {
    let k = r.below(6) as usize;
    let mut n = [2 + 2 * r.below(12), 4 + 2 * r.below(11), 6 + 2 * r.below(10), 3, 3, 5][k];
    if r.below(12) == 0 {
        n += 1;
    }
    out.push_str(core::str::from_utf8(WORDS[k]).unwrap());
    for i in 0..n {
        out.push_str([" ", ",", " ; ", "\t", "  "][r.below(5) as usize]);
        let v = match (r.below(60), k >= 3, i) {
            (0, ..) => r.below(1000),
            (1, ..) => 25 + r.below(4),
            (_, true, 2) => 1 + r.below(4),
            (_, true, 0 | 1) => 4 + r.below(17),
            (_, true, _) => r.below(361),
            _ => r.below(25),
        };
        out.push_str(&v.to_string());
    }
}

/// An accepted icon's outline is finite and bounded, and draws within a unit of its box.
fn bounded(m: &Made, px: i32) {
    let mut o = Vec::new();
    m.outline(&mut o);
    let pts: usize = o.iter().map(Vec::len).sum();
    let finite = o.iter().flatten().all(|p| p.x.is_finite() && p.y.is_finite());
    assert!(finite && o.len() <= MAX_NUMBERS && pts <= 1400, "{m:?}: {} {pts}", o.len());
    // One grid unit is 40 box units.
    let b = raster(&o, px);
    assert!(inside(&b, px, (px * 40 + 999) / 1000), "{m:?}");
}

#[test]
fn made_icons_stay_on_their_plate() {
    let (mut r, mut ok) = (Rng(0x9e37_79b9_7f4a_7c15), 0);
    for case in 0..20_000 {
        let mut text = String::new();
        for _ in 0..1 + r.below(if case % 50 == 0 { 20 } else { 4 }) {
            shape(&mut r, &mut text);
            text.push(' ');
        }
        if let Ok(m) = Made::parse(text.as_bytes()) {
            ok += 1;
            // Every one checked; every tenth drawn too.
            if ok % 10 == 0 {
                bounded(&m, 90);
            } else {
                let mut o = Vec::new();
                m.outline(&mut o);
                assert!(o.iter().flatten().all(|p| p.x.is_finite() && p.y.is_finite()));
            }
        }
    }
    assert!(ok > 4_000, "{ok} of 20000 read");
    // Soups of tokens, and of any bytes at all: none panics, and what reads is bounded.
    #[rustfmt::skip]
    let tokens = ["line", "loop", "fill", "ring", "dot", "arc", "2", "12", "22", "3", "5", "8",
        "0", "24", "360", "25", "999", "1000", "-1", "1.5", "L", "\u{e9}", "\n", "//"];
    // Mostly numbers, now and then a word, rarely junk; a word first.
    let mut token = |i: usize| match (i, r.below(30)) {
        (0, k) | (_, k @ 0..=4) => tokens[k as usize % 6],
        (_, 5..=27) => tokens[6 + r.below(9) as usize],
        _ => tokens[15 + r.below(9) as usize],
    };
    let mut soups = Vec::new();
    for case in 0..20_000 {
        let n = if case % 100 == 0 { 200 } else { 16 };
        let soup: String = (0..n).flat_map(|i| [token(i), [" ", ";", ","][i % 3]]).collect();
        soups.push(soup.into_bytes());
    }
    let mut kept = 0;
    for (case, soup) in soups.iter_mut().enumerate() {
        if case % 3 == 0 {
            soup.iter_mut().for_each(|b| *b = r.below(256) as u8);
        }
        let n = r.below(soup.len() as u64 + 1) as usize;
        let soup = &soup[..n];
        if let Ok(m) = Made::parse(soup) {
            kept += 1;
            bounded(&m, 57);
        }
        _ = header(soup);
    }
    assert!(kept > 50, "{kept} soups read");
    // Degenerate shapes draw bounded, never refused: a point, a reversal, a flat or crossed fill,
    // an arc all the way round; the worst legal outline too.
    for text in [
        "line 1 1 1 1",
        "line 2 2 20 20 2 2",
        "fill 2 2 12 12 22 22",
        "fill 2 2 22 22 22 2 2 22",
        "arc 12 12 6 90 90",
        "loop 2 2 2 2 2 2",
        "ring 12 12 12 ring 12 12 12 ring 12 12 12 ring 12 12 12 ring 12 12 12 ring 12 12 12 \
         ring 12 12 12 ring 12 12 12 ring 12 12 12 ring 12 12 12 ring 12 12 12 ring 12 12 12 \
         ring 12 12 12 ring 12 12 12 ring 12 12 12 loop 2 2 22 2 22 22 2 22 12 12 2 22 12 2",
    ] {
        bounded(&made(text), 90);
    }
}

#[test]
fn a_made_x_has_the_close_glyphs_weight() {
    let mut o = Vec::new();
    glyph(Glyph::Close, &mut o);
    let close = raster(&o, 34);
    let x = drawn(&made("line 5 5 19 19 line 5 19 19 5"), 34);
    assert_eq!((x.w, x.h, x.left, x.top), (close.w, close.h, close.left, close.top));
    let (a, b) = (ink(&x) as f32, ink(&close) as f32);
    assert!((a / b - 1.0).abs() < 0.03, "{a} {b}");
}

#[test]
fn every_example_differs_and_has_ink() {
    for px in [19, 30, 59] {
        let all = EXAMPLES.map(|e| drawn(&made(e.1), px));
        for (i, b) in all.iter().enumerate() {
            let name = EXAMPLES[i].0;
            assert!(inside(b, px, 1) && ink(b) as i32 >= px * px / 12, "{name} at {px}");
            assert!(b.data.iter().any(|&c| c >= 192), "{name} at {px}: some ink solid");
            assert!(all[..i].iter().all(|a| a.data != b.data), "{name} at {px}");
        }
    }
}

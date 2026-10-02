use super::*;

const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const SEMI: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");
const SYM_A: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-a.ttf");
const SYM_B: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-b.ttf");

fn font(bytes: &[u8]) -> Font {
    Font::parse(bytes.to_vec()).unwrap()
}

fn ids(f: &Font, s: &str) -> Vec<Option<u16>> {
    s.chars().map(|c| f.glyph_index(c)).collect()
}

fn draw(f: &Font, c: char, px: f32) -> Bitmap {
    let mut b = Bitmap::default();
    f.rasterize(f.glyph_index(c).unwrap(), px, &mut b).unwrap();
    assert_eq!(b.data.len(), (b.w * b.h) as usize);
    b
}

fn at(b: &Bitmap, x: u32, y: u32) -> u8 {
    b.data[(y * b.w + x) as usize]
}

#[test]
fn reads_the_real_fonts() {
    // (font, units per em, ascender, descender, glyph count)
    let cases = [(INTER, 2048, 1984, -494, 121), (SEMI, 2048, 1984, -494, 121)];
    let more = [(MONO, 1000, 1020, -300, 461), (SYM_A, 1000, 1069, -630, 569)];
    let last = (SYM_B, 1000, 1480, -570, 104);
    for (bytes, upem, asc, desc, glyphs) in cases.into_iter().chain(more).chain([last]) {
        let f = font(bytes);
        assert_eq!((f.metrics(), f.num_glyphs), ([upem, asc, desc, 0], glyphs));
    }
    let (inter, mono) = (font(INTER), font(MONO));
    assert_eq!(ids(&inter, "A ─"), [Some(1), Some(109), None]);
    assert_eq!(ids(&mono, "A ─\u{10FFFF}\u{FFFF}"), [Some(1), Some(180), Some(355), None, None]);
    assert_eq!(ids(&font(SYM_A), "✻"), [Some(229)]);
    assert_eq!(ids(&font(SYM_B), "⎿A"), [Some(93), None]);
    assert_eq!((inter.advance(1), inter.advance(109)), (1413, 576));
    for c in "A ─éi".chars() {
        assert_eq!(mono.advance(mono.glyph_index(c).unwrap()), 600, "{c}");
    }
    // JetBrains Mono has 455 metrics for 461 glyphs: the rest reuse the last.
    assert_eq!(mono.advance(460), mono.advance(454));
    assert_eq!((font(SYM_A).advance(229), font(SYM_B).advance(93)), (750, 699));
    let e_acute = mono.glyph_index('é').unwrap();
    let raw = &mono.data[mono.glyph_range(e_acute).unwrap()];
    assert!(i16_at(raw, 0).unwrap() < 0, "é is a composite");
    let (mut o, mut accent) = (Vec::new(), Vec::new());
    mono.outline(e_acute, &mut o).unwrap();
    let points: usize = o.iter().map(Vec::len).sum();
    assert_eq!((o.len(), points, o[0][0]), (3, 40, Point { x: 300.0, y: -10.0, on: true }));
    // The accent is glyph 437 placed 30 units right.
    mono.outline(437, &mut accent).unwrap();
    let shift = |c: &[Point], dx: f32| c.iter().map(|p| (p.x + dx, p.y, p.on)).collect::<Vec<_>>();
    assert_eq!(shift(&o[2], 0.0), shift(&accent[0], 30.0));
    let err = |d: &[u8]| Font::parse(d.to_vec()).unwrap_err();
    let headers = [&b""[..], b"OTTO\0\0\0\0", b"ttcf\0\0\0\0", b"true"];
    assert_eq!(headers.map(err), [FontError::NotTrueType; 4]);
    let mut no_glyf = INTER.to_vec();
    let glyf = (12..).step_by(16).find(|&r| &INTER[r..r + 4] == b"glyf").unwrap();
    no_glyf[glyf..glyf + 4].copy_from_slice(b"gly_");
    assert_eq!(err(&no_glyf), FontError::MissingTable(*b"glyf"));
    for cut in [11, 100, INTER.len() / 2] {
        assert!(Font::parse(INTER[..cut].to_vec()).is_err(), "cut at {cut}");
    }
    assert_eq!(FontError::Malformed(*b"loca").to_string(), "malformed `loca` table");
    assert_eq!(FontError::NoGlyph(65_535).to_string(), "no glyph 65535");
}

#[test]
fn rasterizes_real_glyphs() {
    for f in [font(INTER), font(MONO)] {
        // An o at 16 px is a ring.
        let b = draw(&f, 'o', 16.0);
        let (cx, cy) = (b.w / 2, b.h / 2);
        assert!(at(&b, cx, cy) < 40, "a hole in the middle");
        let row = |xs: Range<u32>| xs.map(|x| at(&b, x, cy)).max().unwrap();
        let col = |ys: Range<u32>| ys.map(|y| at(&b, cx, y)).max().unwrap();
        assert!(row(0..cx) > 180 && row(cx + 1..b.w) > 180, "side strokes");
        assert!(col(0..cy) > 180 && col(cy + 1..b.h) > 180, "top and bottom");
    }
    // Inter's I is the box x 180..370, y 0..1490: at 20 px x 1.76..3.61 and
    // 14.55 px tall, so coverage 0.24, 1 and 0.61 across, 0.55 on top.
    let b = draw(&font(INTER), 'I', 20.0);
    assert_eq!((b.left, b.top, b.w, b.h), (1, -15, 3, 15));
    for y in 1..b.h {
        assert_eq!(at(&b, 1, y), 255, "row {y}");
        assert!(at(&b, 0, y).abs_diff(62) <= 2 && at(&b, 2, y).abs_diff(156) <= 2);
    }
    assert!(at(&b, 1, 0).abs_diff(141) <= 2);
    // Empty glyphs and bad sizes leave an empty bitmap.
    let inter = font(INTER);
    let mut b = draw(&inter, 'A', 12.0);
    inter.rasterize(109, 12.0, &mut b).unwrap();
    assert_eq!((b.w, b.h, b.data.len()), (0, 0, 0));
    for px in [0.0, -3.0, f32::NAN, f32::INFINITY, 1e6] {
        assert_eq!(inter.rasterize(1, px, &mut b), Err(FontError::BadSize));
        assert!(b.data.is_empty());
    }
    assert_eq!(inter.rasterize(121, 12.0, &mut b), Err(FontError::NoGlyph(121)));
    // Any outline: a 4-unit square with a reversed 2-unit hole, and an overlapping
    // same-way square that clamps instead of cancelling; at 2 px a unit.
    let sq = |x: f32, y: f32, s: f32| {
        let p = |x, y| Point { x, y, on: true };
        vec![p(x, y), p(x + s, y), p(x + s, y + s), p(x, y + s)]
    };
    let hole: Vec<Point> = sq(1.0, 1.0, 2.0).into_iter().rev().collect();
    render_outline(&[sq(0.0, 0.0, 4.0), hole, sq(2.0, 0.0, 4.0)], 2.0, &mut b).unwrap();
    assert_eq!((b.left, b.top, b.w, b.h), (0, -8, 12, 8));
    let row: Vec<u8> = (0..12).map(|x| at(&b, x, 3)).collect();
    assert_eq!(row, [255, 255, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255]);
    assert_eq!(render_outline(&[], f32::NAN, &mut b), Err(FontError::BadSize));
}

/// Big-endian words, for a tiny synthetic font that covers what the real
/// ones don't: format 12, every composite transform, limits, exact coverage.
fn words(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|&w| (w as u16).to_be_bytes()).collect()
}

/// A simple glyph of axis-aligned rectangles `(x0, y0, x1, y1)`; each is
/// clockwise (y up) unless x0 > x1, which reverses it.
fn rects(rs: &[(i32, i32, i32, i32)]) -> Vec<u8> {
    let ends: Vec<i32> = (0..rs.len() as i32).map(|i| 4 * i + 3).collect();
    let mut g = [words(&[rs.len() as i32, 0, 0, 0, 0]), words(&ends), words(&[0])].concat();
    g.extend(vec![1u8; 4 * rs.len()]);
    let pts: Vec<(i32, i32)> =
        rs.iter().flat_map(|&(x0, y0, x1, y1)| [(x0, y0), (x0, y1), (x1, y1), (x1, y0)]).collect();
    for axis in 0..2 {
        let mut prev = 0;
        for &(x, y) in &pts {
            g.extend(words(&[[x, y][axis] - prev]));
            prev = [x, y][axis];
        }
    }
    g
}

/// A composite glyph: `(flags, glyph, args, transform words)` per component,
/// args as words; MORE_COMPONENTS is set for all but the last.
type Part<'a> = (u16, u16, [i32; 2], &'a [i32]);

fn composite(parts: &[Part]) -> Vec<u8> {
    let mut g = words(&[-1, 0, 0, 0, 0]);
    for (i, &(flags, glyph, args, xf)) in parts.iter().enumerate() {
        let more = if i + 1 < parts.len() { 0x20 } else { 0 };
        let (flags, glyph) = (i32::from(flags | more | 0x01), i32::from(glyph));
        g.extend(words(&[&[flags, glyph, args[0], args[1]][..], xf].concat()));
    }
    g
}

fn synth(glyphs: &[Vec<u8>], cmap_sub: &[u8], enc: u16) -> Font {
    let n = glyphs.len() as i32;
    let magic = words(&[1, 0, 0, 0, 0, 0, 0x5F0F, 0x3CF5, 0, 1000]);
    let head = [magic, vec![0; 30], words(&[1, 0])].concat();
    let hhea = [words(&[1, 0, 800, -200, 0]), vec![0; 24], words(&[n])].concat();
    let cmap = [words(&[0, 1, 3, i32::from(enc), 0, 12]), cmap_sub.to_vec()].concat();
    let hmtx = words(&glyphs.iter().flat_map(|_| [500, 0]).collect::<Vec<_>>());
    let (mut loca, mut glyf) = (Vec::new(), Vec::new());
    for g in glyphs.iter().chain([&vec![]]) {
        loca.extend((glyf.len() as u32).to_be_bytes());
        glyf.extend(g);
    }
    let maxp = words(&[0, 0x5000, n]);
    let tables = [(b"cmap", cmap), (b"glyf", glyf), (b"head", head), (b"hhea", hhea)];
    let tables = tables.into_iter().chain([(b"hmtx", hmtx), (b"loca", loca), (b"maxp", maxp)]);
    let (mut dir, mut body) = ([words(&[1, 0, 7]), vec![0; 6]].concat(), Vec::new());
    for (tag, t) in tables {
        let off = (12 + 16 * 7 + body.len()) as u32;
        dir.extend([*tag, [0; 4], off.to_be_bytes(), (t.len() as u32).to_be_bytes()].concat());
        body.extend(t);
    }
    Font::parse([dir, body].concat()).unwrap()
}

/// One segment mapping 'A'.. to glyph 1.., plus the 0xFFFF sentinel.
fn fmt4_identity() -> Vec<u8> {
    let (ends, starts, deltas) = ([0x5A, 0xFFFF], [0x41, 0xFFFF], [-0x40, 1]);
    words(&[&[4, 32, 0, 4, 0, 0, 0][..], &ends, &[0], &starts, &deltas, &[0, 0]].concat())
}

#[test]
fn synthetic_fonts() {
    let groups = [(0x41, 0x43, 1), (0x1F600, 0x1F602, 4)];
    let mut sub = words(&[12, 0, 0, 16 + 12 * 2, 0, 0, 0, 2]);
    for (s, e, g) in groups {
        sub.extend([s, e, g].iter().flat_map(|v: &u32| v.to_be_bytes()));
    }
    let f = synth(&vec![rects(&[(0, 0, 100, 100)]); 7], &sub, 10);
    let want = [Some(1), Some(3), None, Some(4), Some(6), None, None];
    assert_eq!(ids(&f, "ACD\u{1F600}\u{1F602}\u{1F603}\u{1F5FF}"), want);
    let mut glyphs = vec![rects(&[]), rects(&[(0, 0, 100, 100)])];
    let parts: [&[Part]; 7] = [
        &[(0x0A, 1, [200, 50], &[8192])],     // 2: scale 0.5, then offset
        &[(0x42, 1, [0, 0], &[24576, 8192])], // 3: x 1.5, y 0.5
        &[(0x82, 1, [0, 0], &[0, 16384, -16384, 0])], // 4: 2x2, a quarter turn
        &[(0x00, 1, [3, 7], &[])],            // 5: point matching: offset 0
        &[(0x02, 1, [0, 0], &[]), (0x0A, 1, [200, 50], &[8192])], // 6: two parts
        &[(0x080A, 1, [200, 50], &[8192])],   // 7: scaled offset
        &[(0x02, 8, [0, 0], &[])],            // 8: refers to itself
    ];
    glyphs.extend(parts.iter().map(|p| composite(p)));
    // 9..=17: a chain, each glyph wrapping the one before; 9 wraps glyph 1.
    for k in 9..=17 {
        glyphs.push(composite(&[(0x02, if k == 9 { 1 } else { k - 1 }, [1, 0], &[])]));
    }
    let f = synth(&glyphs, &fmt4_identity(), 1);
    assert_eq!(ids(&f, "AB@"), [Some(1), Some(2), None]);
    let mut o = Vec::new();
    let mut outline = |g: u16| {
        let r = f.outline(g, &mut o);
        let ps = o.iter().flatten();
        let bbox = ps.fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |(a, b, c, d), p| {
            (a.min(p.x), b.min(p.y), c.max(p.x), d.max(p.y))
        });
        r.map(|()| (o.len(), bbox))
    };
    assert_eq!(outline(0), Ok((0, (f32::MAX, f32::MAX, f32::MIN, f32::MIN))));
    assert_eq!(outline(2), Ok((1, (200.0, 50.0, 250.0, 100.0))));
    assert_eq!(outline(3), Ok((1, (0.0, 0.0, 150.0, 50.0))));
    assert_eq!(outline(4), Ok((1, (-100.0, 0.0, 0.0, 100.0))));
    assert_eq!(outline(5), Ok((1, (0.0, 0.0, 100.0, 100.0))));
    assert_eq!(outline(6), Ok((2, (0.0, 0.0, 250.0, 100.0))));
    assert_eq!(outline(7), Ok((1, (100.0, 25.0, 150.0, 75.0))));
    assert_eq!(outline(8), Err(FontError::TooComplex));
    // Glyph 16 nests 8 composite levels (fine); 17 nests 9 (too deep).
    assert_eq!(outline(16), Ok((1, (8.0, 0.0, 108.0, 100.0))));
    assert_eq!(outline(17), Err(FontError::TooComplex));
    assert_eq!(outline(18), Err(FontError::NoGlyph(18)));
    assert!(o.is_empty());
    let square = (0, 0, 1000, 1000);
    let glyphs = [rects(&[]), rects(&[(0, 0, 500, 500)]), rects(&[(50, 0, 950, 500)])];
    let more = [rects(&[square, (400, 400, 600, 600)]), rects(&[square, (600, 400, 400, 600)])];
    let f = synth(&[&glyphs[..], &more].concat(), &fmt4_identity(), 1);
    let mut b = Bitmap::default();
    // A pixel-aligned square is solid; half-pixel edges are half covered.
    f.rasterize(1, 10.0, &mut b).unwrap();
    assert_eq!((b.left, b.top, b.w, b.h), (0, -5, 5, 5));
    assert!(b.data.iter().all(|&c| c == 255));
    f.rasterize(2, 10.0, &mut b).unwrap();
    assert_eq!((b.left, b.w), (0, 10));
    for y in 0..b.h {
        assert_eq!((at(&b, 0, y), at(&b, 5, y), at(&b, 9, y)), (128, 255, 128));
    }
    // A same-direction overlap clamps to full; an opposite one is a hole.
    f.rasterize(3, 10.0, &mut b).unwrap();
    assert!(b.data.iter().all(|&c| c == 255));
    f.rasterize(4, 10.0, &mut b).unwrap();
    let row: Vec<u8> = (0..b.w).map(|x| at(&b, x, 5)).collect();
    assert_eq!(row, [255, 255, 255, 255, 0, 0, 255, 255, 255, 255]);
}

/// 2,000 mutated real fonts: parsing, mapping, outlining and rasterizing may
/// fail but never panic (debug builds also trap integer overflow).
#[test]
fn fuzz_never_panics() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize
    };
    let sources = [font(INTER), font(MONO), font(SYM_B)];
    let (mut outline, mut bitmap) = (Vec::new(), Bitmap::default());
    let mut parsed = 0;
    for i in 0..2000 {
        let src = &sources[i % sources.len()];
        let mut data = src.data.clone();
        let focus = next() as u16 % src.num_glyphs;
        let glyph = src.glyph_range(focus).unwrap();
        // Where to mutate: anywhere, the directory and head, the cmap
        // subtable, one glyph's outline (or its loca entry), or truncate.
        let region = match next() % 5 {
            0 => 0..data.len(),
            1 => 0..256,
            2 => src.cmap.sub.start..src.cmap.sub.start + 256,
            3 if !glyph.is_empty() => glyph,
            3 => src.loca + 2 * usize::from(focus)..src.loca + 2 * usize::from(focus) + 4,
            _ => {
                data.truncate(next() % data.len());
                0..0
            }
        };
        for _ in 0..if region.is_empty() { 0 } else { 1 + next() % 6 } {
            let at = region.start + next() % region.len();
            if let Some(b) = data.get_mut(at) {
                *b = next() as u8;
            }
        }
        let Ok(f) = Font::parse(data) else { continue };
        parsed += 1;
        let mut glyphs = vec![focus, next() as u16];
        let chars: Vec<char> =
            (0..16).filter_map(|_| char::from_u32(next() as u32 % 0x30000)).collect();
        for c in (' '..='~').chain(chars) {
            let g = f.glyph_index(c);
            if next() % 16 == 0 {
                glyphs.extend(g);
            }
        }
        for g in glyphs {
            let _ = (f.advance(g), f.outline(g, &mut outline));
            let px = [4.0, 11.5, 16.0, 33.3][next() % 4];
            if f.rasterize(g, px, &mut bitmap).is_ok() {
                assert_eq!(bitmap.data.len(), (bitmap.w * bitmap.h) as usize);
            }
        }
    }
    assert!(parsed > 1000, "most mutants still parse: {parsed}");
}

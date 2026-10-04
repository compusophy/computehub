use super::*;
use font::{Bitmap, render_outline};

fn draw(g: Glyph, px: i32) -> Bitmap {
    let mut o = Vec::new();
    outline(g, &mut o);
    raster(&o, px)
}

fn raster(o: &[Vec<Point>], px: i32) -> Bitmap {
    let mut b = Bitmap::default();
    render_outline(o, px as f32 / 1000.0, &mut b).unwrap();
    b
}

/// Ink only in the box (x 0..px right of the origin, y 0..px above it), some of it solid; its
/// whole pixels of ink.
fn boxed(b: &Bitmap, px: i32) -> Option<i32> {
    let w = b.w as i32;
    let at = |i: usize| (b.left + i as i32 % w, b.top + i as i32 / w);
    let out = |(x, y): (i32, i32)| x < 0 || x >= px || y < -px || y >= 0;
    let stray = (0..b.data.len()).any(|i| b.data[i] > 0 && out(at(i)));
    let solid = b.data.iter().any(|&c| c >= 192);
    (!stray && solid).then(|| b.data.iter().map(|&c| i32::from(c)).sum::<i32>() / 255)
}

#[test]
fn every_glyph_draws_inside_its_box() {
    for g in Glyph::ALL {
        for px in [13, 21, 34, 55] {
            let ink = boxed(&draw(g, px), px);
            assert!(ink.is_some_and(|ink| ink >= px / 2), "{g:?} at {px}: {ink:?} px of ink");
        }
    }
    assert!(Glyph::ALL.iter().enumerate().all(|(i, &g)| g as usize == i));
    // The mark's center is a dot, as is the ring's; the cog's center a hole.
    let mid = |b: Bitmap| b.data[(b.h / 2 * b.w + b.w / 2) as usize];
    assert_eq!([Glyph::Mark, Glyph::Apps, Glyph::Cog].map(|g| mid(draw(g, 55))), [255, 255, 0]);
}

#[test]
fn the_mark_is_rings_of_fibonacci_dots() {
    #[rustfmt::skip]
    let want = [(8, 190.74, 49.49), (13, 308.62, 30.59), (21, 381.49, 18.90), (34, 426.49, 11.68),
        (55, 454.36, 7.22), (89, 471.49, 4.46), (144, 482.14, 2.76)];
    for (got, want) in rings().zip(want) {
        let near = (got.1 - want.1).abs() <= 0.05 && (got.2 - want.2).abs() <= 0.05;
        assert!(got.0 == want.0 && near, "{got:?} for {want:?}");
    }
    let mut o = Vec::new();
    outline(Glyph::Mark, &mut o);
    // 365 dots, all ink (no disc behind them): the center, then each ring clockwise from the top.
    assert_eq!(o.len(), 365);
    assert!(o.iter().all(|c| area(c) > 0.0));
    let center = |c: &[Point]| {
        let on: Vec<&Point> = c.iter().filter(|p| p.on).collect();
        let n = on.len() as f32;
        (on.iter().map(|p| p.x).sum::<f32>() / n, on.iter().map(|p| p.y).sum::<f32>() / n)
    };
    let near =
        |(x, y): (f32, f32), (u, v): (f32, f32)| (x - u).abs() < 0.05 && (y - v).abs() < 0.05;
    assert!(near(center(&o[0]), C) && near(center(&o[1]), (500.0, 690.74)));
    assert!(near(center(&o[2]), (634.87, 634.87)) && near(center(&o[364]), (478.97, 981.68)));
}

/// Face `n`'s dots.
fn dots(n: u8) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    face(n, |x, y| out.push((x, y)));
    out
}

#[test]
fn a_face_holds_its_count_of_dots_upright_apart_and_inside_its_ring() {
    for n in 0..FACES {
        let dots = dots(n);
        assert_eq!(dots.len(), usize::from(n));
        // Mirrored left to right, and clear of the ring by more than they are of each other.
        for &(x, y) in &dots {
            assert!(dots.iter().any(|d| (d.0 + x).abs() < 1e-4 && (d.1 - y).abs() < 1e-4));
            assert!((x * x + y * y).sqrt() + 2.5 * FACE_DOT < 1.0, "{n}: {x} {y}");
        }
        // No two touch: a dot's width between them at least.
        for (i, a) in dots.iter().enumerate() {
            for b in &dots[i + 1..] {
                let apart = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                assert!(apart >= 3.0 * FACE_DOT, "{n}: {a:?} {b:?}");
            }
        }
    }
    // One is the middle; two side by side; three an apex up; seven the middle in six.
    assert_eq!(dots(1), [(0.0, 0.0)]);
    let two = dots(2);
    assert!(two.iter().all(|d| d.1.abs() < 1e-4) && (two[0].0 + two[1].0).abs() < 1e-4);
    assert!(dots(3)[0].0.abs() < 1e-4 && dots(3)[0].1 < 0.0);
    assert_eq!((dots(7)[0], dots(200).len()), ((0.0, 0.0), usize::from(FACES - 1)));
}

#[test]
fn sigils_are_one_shape_per_name_and_differ_between_names() {
    let fnv = |s: &str| {
        s.bytes().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619))
    };
    let drawn = |name: &str, px: i32| {
        let mut o = vec![vec![pt(0.0, 0.0)]];
        sigil(fnv(name), &mut o);
        let b = raster(&o, px);
        (b.w, b.h, b.left, b.top, b.data)
    };
    // Six names, six shapes, at every size; the same name always the same raster.
    let names = ["clock.app", "notes.app", "tetris.app", "todo.app", "paint.app", "timer.app"];
    for px in [21, 34, 55] {
        let all = names.map(|n| drawn(n, px));
        for (i, a) in all.iter().enumerate() {
            assert_eq!(a, &drawn(names[i], px));
            assert!(all[..i].iter().all(|b| b != a), "{} at {px}", names[i]);
        }
    }
    // Every seed draws inside its box with ink to spare; 64 names make at least 28 shapes.
    let mut seen = Vec::new();
    for i in 0..64 {
        let mut o = Vec::new();
        sigil(fnv(&["app", &i.to_string()].concat()), &mut o);
        let b = raster(&o, 34);
        assert!(boxed(&b, 34).is_some_and(|ink| ink >= 100), "app{i}");
        if !seen.contains(&b.data) {
            seen.push(b.data);
        }
    }
    assert!(seen.len() >= 28, "{} shapes", seen.len());
}

#[test]
fn sin_and_cos_are_the_platform_s_to_an_ulp() {
    // On wasm32 they are bit for bit f32::sin and cos (musl's) for every |x| < 2^28 π/2; here,
    // against the host's libm, within an ulp: a glyph's point moves under 1e-4 of a box unit.
    let near = |x: f32| (sin(x) - x.sin()).abs() <= 1.2e-7 && (cos(x) - x.cos()).abs() <= 1.2e-7;
    assert!((0..=400_000).map(|i| -40.0 + i as f32 * 0.000_2).all(near));
    assert!([1e3, -5e4, 1e6, 4e8].into_iter().all(near));
    // Below 2^-12, x and 1 exactly (the sign of zero kept); past 2^28 π/2, and for NaN, NaN.
    assert!(sin(-0.0).to_bits() == (-0.0f32).to_bits() && sin(2e-4) == 2e-4 && cos(-2e-4) == 1.0);
    assert!([5e8, f32::INFINITY, f32::NAN].iter().all(|&x| sin(x).is_nan() && cos(-x).is_nan()));
}

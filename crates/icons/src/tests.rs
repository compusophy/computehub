use super::*;
use font::{Bitmap, render_outline};

fn draw(g: Glyph, px: i32) -> Bitmap {
    let (mut o, mut b) = (Vec::new(), Bitmap::default());
    outline(g, &mut o);
    render_outline(&o, px as f32 / 1000.0, &mut b).unwrap();
    b
}

#[test]
fn every_glyph_draws_inside_its_box() {
    for g in Glyph::ALL {
        for px in [13, 21, 34, 55] {
            // Ink only in the box (x 0..px right of the origin, y 0..px above it), solid somewhere.
            let b = draw(g, px);
            let w = b.w as i32;
            let at = |i: usize| (b.left + i as i32 % w, b.top + i as i32 / w);
            let out = |(x, y): (i32, i32)| x < 0 || x >= px || y < -px || y >= 0;
            let stray = (0..b.data.len()).any(|i| b.data[i] > 0 && out(at(i)));
            let ink = b.data.iter().map(|&c| i32::from(c)).sum::<i32>() / 255;
            let solid = b.data.iter().any(|&c| c >= 192);
            assert!(!stray && solid && ink >= px / 2, "{g:?} at {px}: {ink} px of ink");
        }
    }
    assert!(Glyph::ALL.iter().enumerate().all(|(i, &g)| g as usize == i));
    // The mark's center is a dot, as is the launcher's; the cog's center a hole.
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

use gfx::{DrawList, Instance, Kind, Sem};
use ui::{THEMES, TextSystem, UiState};

use super::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
/// Where the canvases below are drawn from.
const O: f32 = 20.0;

/// `draws` on canvas 1 of `units` in `r` at `dpr` device px a px in theme `t`: its fills, what
/// it noted, and where its units are.
fn drawn(
    t: &Theme,
    dpr: f32,
    r: RectF,
    units: (u16, u16),
    draws: &[Draw],
) -> (DrawList, Sem, RectF) {
    let mut out = Vec::new();
    for mut list in [DrawList::new(), DrawList::recording()] {
        let (mut ts, mut hits) = (TextSystem::new(SANS.to_vec()).unwrap(), Vec::new());
        ts.set_dpr(dpr);
        let ui = Ui::new(&mut list, &mut ts, r, &mut hits, UiState::default(), t);
        let at = draw(&mut { ui }, r, 1, units, draws);
        out.push((list, at));
    }
    let ((mut noted, _), (list, at)) = (out.pop().unwrap(), out.pop().unwrap());
    (list, noted.take_sem().unwrap(), at)
}

fn of(list: &DrawList, kind: Kind) -> Vec<&Instance> {
    list.instances().iter().filter(|i| i.kind == kind as u8 as f32).collect()
}

/// A draw of `shape` in `color` at `at`, with `text`.
fn shape(shape: Shape, color: u8, at: [i16; 5], text: &str) -> Draw {
    Draw { shape, color, at, text: text.into() }
}

#[test]
fn colors_come_from_the_theme() {
    for t in &THEMES {
        let draws: Vec<_> =
            (0..12).map(|c| shape(Shape::Rect, c, [c.into(), 0, 1, 1, 0], "")).collect();
        let (list, ..) = drawn(t, 1.0, RectF::new(O, O, 120.0, 10.0), (12, 1), &draws);
        let silver = if t.dark { t.ansi[7] } else { t.text_faint };
        let mut want = vec![t.surface_lo];
        want.extend(t.ansi[1..7].iter().copied().chain([silver, t.ansi[8]]));
        want.extend([t.text, t.text_dim, t.accent]);
        let got: Vec<Rgba> = of(&list, Kind::Fill).iter().skip(1).map(|f| f.color).collect();
        assert_eq!(got, want, "{}", t.name);
        assert!((0..12).all(|c| ink(t, c) == want[usize::from(c)]));
        // A Grid's square shares them, its text in the base where it is a color.
        assert_eq!((square(t, 0), square(t, 4)), ((t.surface_lo, t.text), (t.ansi[4], t.base)));
    }
}

#[test]
fn shapes_rings_texts_and_sprites_draw_scaled() {
    let t = &THEMES[1];
    let draws = vec![
        shape(Shape::Ring, 4, [50, 25, 10, 2, 0], ""),
        shape(Shape::Circle, 3, [50, 25, 4, 0, 0], ""),
        shape(Shape::Text, 11, [50, 40, 3, 0, 0], "Score 7"),
        shape(Shape::Sprite, 0, [0, 40, 2, 0, 0], "11.2\n.3a"),
        // Nothing of no size; a ring of no width.
        shape(Shape::Rect, 5, [0, 0, 0, 5, 0], ""),
        shape(Shape::Circle, 5, [1, 1, 0, 0, 0], ""),
        shape(Shape::Ring, 5, [1, 1, 4, 0, 0], ""),
        shape(Shape::Text, 5, [1, 1, 0, 0, 0], "x"),
    ];
    // 100 x 50 units in 560 x 280: 5.6 px a unit.
    let (list, sem, at) = drawn(t, 1.0, RectF::new(O, O, 600.0, 280.0), (100, 50), &draws);
    assert_eq!(at, RectF::new(O + 20.0, O, 560.0, 280.0), "centered across");
    let (cx, cy) = (O + 20.0 + 50.5 * 5.6, O + 25.5 * 5.6);
    let ring = of(&list, Kind::Border).into_iter().find(|b| b.color == t.ansi[4]).unwrap();
    assert_eq!((ring.radius, ring.p0), (56.0, 11.2));
    assert_eq!(ring.rect, [cx - 56.0, cy - 56.0, 112.0, 112.0]);
    let disc = of(&list, Kind::Fill).into_iter().find(|f| f.color == t.ansi[3]).unwrap();
    assert_eq!((disc.radius, disc.rect[2]), (4.0 * 5.6, 8.0 * 5.6));
    assert!(list.instances().iter().all(|i| i.color != t.ansi[5]));
    // 3 units of 5.6 px is 16.8 px: set at 16, in the boot's font, read where it shows.
    assert!(of(&list, Kind::Glyph).iter().filter(|g| g.color == t.accent).count() >= 6);
    let run = sem.runs.iter().find(|r| r.text == "Score 7").unwrap();
    assert_eq!(run.rect.h, 1.25 * 16.0);
    // A sprite's squares, two units a side: a run of one color a fill; its `a` shows through.
    let low = |f: &&Instance| f.rect[1] >= O + 220.0;
    let sprite: Vec<_> = of(&list, Kind::Fill).into_iter().filter(low).collect();
    let colors: Vec<Rgba> = sprite.iter().map(|f| f.color).collect();
    assert_eq!(colors, [t.ansi[1], t.ansi[2], t.ansi[3]]);
    assert_eq!(sprite[0].rect, [O + 20.0, O + 224.0, 22.0, 11.0]);
    assert_eq!((sprite[2].rect[0], sprite[2].rect[1]), (O + 31.0, O + 235.0));
}

#[test]
fn a_text_whose_point_is_on_the_canvas_stays_inside_it() {
    // 160 x 120 units in 320 x 240, 2 px a unit, texts 8 units tall (set at 16 px, so 4 px in):
    // a score at the left edge, balls at the right, a line on the bottom row, one wider than the
    // canvas, and one whose point is off it.
    let hud = |x, y, text| shape(Shape::Text, 9, [x, y, 8, 0, 0], text);
    let long = "a line of text far too long for this canvas, cut at its right";
    let draws =
        [hud(2, 4, "Score 0"), hud(158, 4, "Balls 3"), hud(80, 119, "Over"), hud(80, 60, long)];
    let r = RectF::new(O, O, 320.0, 240.0);
    let (list, sem, at) = drawn(&THEMES[0], 1.0, r, (160, 120), &draws);
    let run = |text: &str| sem.runs.iter().find(|r| r.text == text).map(|r| r.rect);
    let (score, balls, over) = (run("Score 0").unwrap(), run("Balls 3").unwrap(), run("Over"));
    // Its capitals' top 4 px in too (at y 4 their middle is 9 px down, half of 11.6 px up).
    assert_eq!((score.x, score.y + 16.0), (at.x + 4.0, (at.y + 4.0 + 0.727 * 16.0).round()));
    assert!((balls.x + balls.w - (at.x + at.w - 4.0)).abs() <= 1.0, "{balls:?}");
    assert!(over.is_some_and(|o| o.y + 16.0 <= at.y + at.h - 4.0 + 0.5), "{over:?}");
    assert_eq!(run(long).map(|l| l.x), Some(at.x + 4.0), "wider: from the left");
    // Every glyph inside the canvas; a point off it draws where it is, unseen.
    let off = drawn(&THEMES[0], 1.0, r, (160, 120), &[hud(200, 60, "Off")]).1;
    assert!(off.runs.is_empty());
    let inside = |g: &&Instance| g.rect[0] >= at.x && g.rect[0] + g.rect[2] <= at.x + at.w;
    assert!(of(&list, Kind::Glyph).iter().filter(|g| g.rect[1] < O + 30.0).all(inside));
}

#[test]
fn pixels_fill_a_run_of_a_color_once_on_device_pixels() {
    let t = &THEMES[0];
    // Four cells a row, three rows, each 5 units: `.` shows through, `b` is the accent.
    let px = [shape(Shape::Pixels, 0, [2, 3, 4, 5, 0], "1122..b.3333")];
    // 30 units in 97 px: a unit is no whole number of px.
    let r = RectF::new(O, O, 130.0, 97.0);
    for dpr in [1.0, 2.0, 3.5] {
        let (list, sem, at) = drawn(t, dpr, r, (40, 30), &px);
        let fills = of(&list, Kind::Fill);
        let colors: Vec<Rgba> = fills.iter().skip(1).map(|f| f.color).collect();
        assert_eq!(colors, [t.ansi[1], t.ansi[2], t.accent, t.ansi[3]], "{dpr}");
        let on = |v: f32| (v * dpr - (v * dpr).round()).abs() < 1e-3;
        assert!(fills.iter().all(|f| f.rect.iter().all(|v| on(*v))), "{dpr}");
        // Neighbors tile with no seam: each edge from its own unit.
        let ([a, b, c, d], k) = ([1, 2, 3, 4].map(|i| fills[i].rect), 97.0 / 30.0);
        let meet = |p: f32, q: f32| (p - q).abs() < 1e-3;
        assert!(meet(a[0] + a[2], b[0]) && b[0] == c[0], "{a:?} {b:?} {c:?}");
        assert!(meet(a[1] + a[3], c[1]) && meet(c[1] + c[3], d[1]), "{a:?} {c:?} {d:?}");
        let unit = |u: f32| (at.x + u * k, at.y + u * k);
        assert!((a[0] - unit(2.0).0).abs() <= 0.5 && (d[1] + d[3] - unit(18.0).1).abs() <= 0.5);
        assert!((d[2] - 20.0 * k).abs() <= 1.0, "a row of one color is one fill: {d:?}");
        // The AI reads its place and size, then its squares, a row a line.
        let said = "40 x 30 units\npixels 2 3, 4 x 3 squares of 5 units\n1122\n..b.\n3333";
        assert_eq!(sem.marks[0].value, said);
    }
    // Past SQUARES in all, a board is its place and size alone.
    let big = shape(Shape::Pixels, 0, [0, 0, 64, 1, 0], &"1".repeat(64 * 16));
    let (_, sem, _) = drawn(t, 1.0, r, (64, 64), &[px[0].clone(), big]);
    assert!(sem.marks[0].value.ends_with("3333\npixels 0 0, 64 x 16 squares of 1 units"));
    // No side, or no cells: nothing drawn.
    for empty in [
        shape(Shape::Pixels, 0, [0, 0, 2, 0, 0], "11"),
        shape(Shape::Pixels, 0, [0, 0, 2, 3, 0], ""),
    ] {
        let (list, ..) = drawn(t, 1.0, RectF::new(O, O, 40.0, 40.0), (8, 8), &[empty]);
        assert_eq!(of(&list, Kind::Fill).len(), 1, "the well alone");
    }
}

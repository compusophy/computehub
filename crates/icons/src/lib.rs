//! The compusophyOS icon set: the compusophy mark and the desktop's glyphs as
//! outlines in a 1000 x 1000 box (x right, y up, origin bottom-left), which
//! `font`'s rasterizer draws crisp at any size (`text::TextSystem::draw_vector`;
//! `ui::icon` draws and tiles them in a theme's colors).
//!
//! - Brutalist geometry on the golden ratio [`PHI`]: discs, rings, arcs and
//!   straight strokes of one weight, [`STROKE`] (about 1000 / φ⁵), with butt
//!   ends and mitered corners, filling about the inner 80% of the box.
//! - Coverage is `min(1, |winding|)`: every solid runs counter-clockwise and
//!   every hole clockwise, so holes punch through and overlapping solids merge.
//! - Arcs are quadratic segments of at most π / 8, within 0.02% of round.

#![forbid(unsafe_code)]

use core::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

pub use font::Point;

/// The golden ratio, φ.
pub const PHI: f32 = 1.618_034;
/// The stroke weight of the outline glyphs, in box units.
pub const STROKE: f32 = 90.0;
/// How far a stroke reaches either side of its center line.
const H: f32 = STROKE / 2.0;
/// The center of the box.
const C: (f32, f32) = (500.0, 500.0);

type Outline = Vec<Vec<Point>>;

/// The icons. `Glyph as u16` is a stable id for caching a drawn size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// compusophy's logo: a disc with 365 dots punched out in Fibonacci rings.
    Mark,
    /// The launcher: a ring around a dot.
    Apps,
    /// Settings: an eight-tooth gear.
    Cog,
    /// A golden rectangle and its spiral.
    Studio,
    /// A four-point sparkle and a small one.
    Assistant,
    /// A prompt chevron and a cursor bar.
    Terminal,
    Folder,
    Home,
    /// A page with a folded corner.
    File,
    /// An app or `.app` file: a window with a title bar.
    Window,
    /// A speech bubble.
    Feedback,
    /// An i in a circle.
    About,
    /// A small right-pointing chevron, for list rows.
    Chevron,
    /// An X.
    Close,
}

impl Glyph {
    /// Every glyph, in order.
    #[rustfmt::skip]
    pub const ALL: [Glyph; 14] = {
        use Glyph::*;
        [Mark, Apps, Cog, Studio, Assistant, Terminal, Folder, Home, File, Window, Feedback,
            About, Chevron, Close]
    };

    /// The function that appends this glyph's contours to its argument.
    pub fn shape(self) -> fn(&mut Vec<Vec<Point>>) {
        use Glyph::*;
        match self {
            Mark => mark,
            Apps => apps,
            Cog => cog,
            Studio => studio,
            Assistant => assistant,
            Terminal => terminal,
            Folder => folder,
            Home => home,
            File => file,
            Window => window,
            Feedback => feedback,
            About => about,
            Chevron => chevron,
            Close => close,
        }
    }
}

/// `g`'s contours, replacing `out`'s.
pub fn outline(g: Glyph, out: &mut Vec<Vec<Point>>) {
    out.clear();
    g.shape()(out);
}

fn pt(x: f32, y: f32) -> Point {
    Point { x, y, on: true }
}

/// Twice the signed area of a contour's polygon (controls too): positive counter-clockwise.
fn area(c: &[Point]) -> f32 {
    c.iter().zip(c.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum()
}

/// Adds `c`, turned counter-clockwise for a solid or clockwise for a hole.
fn add(o: &mut Outline, mut c: Vec<Point>, solid: bool) {
    if (area(&c) > 0.0) != solid {
        c.reverse();
    }
    o.push(c);
}

/// Appends the arc of radius `r` around `c` from angle `a0` to `a1` (radians, counter-clockwise
/// positive), its start included, each control where its segment's end tangents meet.
fn arc(v: &mut Vec<Point>, c: (f32, f32), r: f32, a0: f32, a1: f32) {
    let n = ((a1 - a0).abs() / (PI / 8.0)).ceil().max(1.0);
    let step = (a1 - a0) / n;
    let at = |a: f32, k: f32, on| Point { x: c.0 + k * a.cos(), y: c.1 + k * a.sin(), on };
    v.push(at(a0, r, true));
    for i in 0..n as usize {
        let a = a0 + step * i as f32;
        v.push(at(a + step / 2.0, r / (step / 2.0).cos(), false));
        v.push(at(a + step, r, true));
    }
}

/// A disc of radius `r` around `c`, or a round hole.
fn circle(o: &mut Outline, c: (f32, f32), r: f32, solid: bool) {
    let mut v = Vec::new();
    arc(&mut v, c, r, 0.0, TAU);
    v.pop(); // the contour closes itself
    add(o, v, solid);
}

/// A stroke around a center line of radius `r` from angle `a0` to `a1`, butt-ended.
fn bend(o: &mut Outline, c: (f32, f32), r: f32, a0: f32, a1: f32) {
    let mut v = Vec::new();
    arc(&mut v, c, r + H, a0, a1);
    arc(&mut v, c, r - H, a1, a0);
    add(o, v, true);
}

/// `pts` moved `d` to the left of their path (right if negative), mitered at each corner;
/// `closed` joins the last point to the first.
fn offset(pts: &[(f32, f32)], d: f32, closed: bool) -> Vec<Point> {
    let n = pts.len();
    let normal = |i: usize, j: usize| {
        let ((ax, ay), (bx, by)) = (pts[i % n], pts[j % n]);
        let l = (bx - ax).hypot(by - ay);
        ((ay - by) / l, (bx - ax) / l)
    };
    let corner = |i: usize| {
        let a = if i > 0 || closed { normal(i + n - 1, i) } else { normal(i, i + 1) };
        let b = if i + 1 < n || closed { normal(i, i + 1) } else { a };
        // Along a + b, as far as puts it `d` from both edges.
        let (mx, my) = (a.0 + b.0, a.1 + b.1);
        let k = 2.0 * d / (mx * mx + my * my);
        pt(pts[i].0 + mx * k, pts[i].1 + my * k)
    };
    (0..n).map(corner).collect()
}

/// A stroke along the open path `pts`.
fn stroke(o: &mut Outline, pts: &[(f32, f32)]) {
    let mut v = offset(pts, H, false);
    v.extend(offset(pts, -H, false).into_iter().rev());
    add(o, v, true);
}

/// A stroke around the closed path `pts`: its outer offset less its inner one.
fn frame(o: &mut Outline, pts: &[(f32, f32)]) {
    let (a, b) = (offset(pts, H, true), offset(pts, -H, true));
    let (outer, inner) = if area(&a).abs() > area(&b).abs() { (a, b) } else { (b, a) };
    add(o, outer, true);
    add(o, inner, false);
}

/// A solid rectangle from `(x0, y0)` to `(x1, y1)`.
fn rect(o: &mut Outline, x0: f32, y0: f32, x1: f32, y1: f32) {
    add(o, vec![pt(x0, y0), pt(x1, y0), pt(x1, y1), pt(x0, y1)], true);
}

/// Appends a quarter turn of radius `r` around `c` from angle `a`, as path points.
fn corner(v: &mut Vec<(f32, f32)>, c: (f32, f32), r: f32, a: f32) {
    let at = |i: u8| a + FRAC_PI_2 * f32::from(i) / 8.0;
    v.extend((0..=8).map(|i| (c.0 + r * at(i).cos(), c.1 + r * at(i).sin())));
}

/// The mark's rings, inside out: dot count, ring radius, dot radius. Counts run
/// up the Fibonacci numbers from 8; each ring is 1/φ closer to the last and its
/// dots 1/φ smaller (from a center dot of 80.08 and a first step of 190.74).
fn rings() -> impl Iterator<Item = (usize, f32, f32)> {
    let mut s = (5, 8, 0.0, 190.74, 80.08);
    (0..7).map(move |_| {
        let (prev, n, r, step, dot) = s;
        s = (n, prev + n, r + step, step / PHI, dot / PHI);
        (n, r + step, dot / PHI)
    })
}

/// A disc of radius 500 with the center dot and every ring's dots punched out,
/// each ring's first dot at 12 o'clock and the rest clockwise.
fn mark(o: &mut Outline) {
    circle(o, C, 500.0, true);
    circle(o, C, 80.08, false);
    for (n, r, dot) in rings() {
        for j in 0..n {
            let a = FRAC_PI_2 - TAU * j as f32 / n as f32;
            circle(o, (C.0 + r * a.cos(), C.1 + r * a.sin()), dot, false);
        }
    }
}

/// A ring of radius 1000 / φ² around a dot of 1000 / φ⁴.
fn apps(o: &mut Outline) {
    circle(o, C, 382.0, true);
    circle(o, C, 382.0 - STROKE, false);
    circle(o, C, 146.0, true);
}

/// Eight flat-topped teeth two strokes wide (1/φ of the pitch at the root),
/// their corners on radius 470, on a root circle of 372; a hole of 146.
fn cog(o: &mut Outline) {
    let (root, tip, w) = (372.0f32, 470.0f32, STROKE);
    let (foot, crest, skew) =
        ((root * root - w * w).sqrt(), (tip * tip - w * w).sqrt(), (w / root).asin());
    let mut v = Vec::new();
    for i in 0..8u8 {
        let t = FRAC_PI_2 + FRAC_PI_4 * f32::from(i);
        let (u, n) = ((t.cos(), t.sin()), (-t.sin(), t.cos()));
        let at = |a: f32, s: f32| pt(C.0 + u.0 * a + n.0 * s, C.1 + u.1 * a + n.1 * s);
        v.extend([at(foot, -w), at(crest, -w), at(crest, w), at(foot, w)]);
        arc(&mut v, C, root, t + skew, t + FRAC_PI_4 - skew);
    }
    add(o, v, true);
    circle(o, C, 146.0, false);
}

/// A golden rectangle cut into a square and a smaller golden rectangle, and the
/// spiral of quarter turns through its squares (the cut keeps it a spiral at 21 px).
fn studio(o: &mut Outline) {
    let (x0, x1) = (95.0, 905.0);
    let h = (x1 - x0) / PHI;
    let y0 = C.1 - h / 2.0;
    frame(o, &[(x0, y0), (x1, y0), (x1, y0 + h), (x0, y0 + h)]);
    rect(o, x0 + h - H, y0, x0 + h + H, y0 + h);
    // Each turn spans a square; the next, 1/φ the size, starts where it ends.
    let (mut c, mut r, mut a) = ((x0 + h, y0), h, PI);
    for _ in 0..3 {
        bend(o, c, r, a, a - FRAC_PI_2);
        a -= FRAC_PI_2;
        let k = r - r / PHI;
        (c, r) = ((c.0 + k * a.cos(), c.1 + k * a.sin()), r / PHI);
    }
}

/// A four-point star of radius `r` around `c`, its sides curved in to the center.
fn sparkle(o: &mut Outline, c: (f32, f32), r: f32) {
    let k = r / PHI.powi(5);
    let v = (0..4u8).flat_map(|i| {
        let (x, y) = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)][usize::from(i)];
        let pull = Point { x: c.0 + k * (x - y), y: c.1 + k * (x + y), on: false };
        [pt(c.0 + r * x, c.1 + r * y), pull]
    });
    add(o, v.collect(), true);
}

/// A sparkle of radius 470 and one 1/φ² its size at the upper right.
fn assistant(o: &mut Outline) {
    sparkle(o, C, 470.0);
    sparkle(o, (790.0, 790.0), 470.0 / (PHI * PHI));
}

/// `>_`: a chevron and a bar on its foot.
fn terminal(o: &mut Outline) {
    stroke(o, &[(200.0, 760.0), (460.0, 500.0), (200.0, 240.0)]);
    rect(o, 560.0, 208.0, 830.0, 208.0 + STROKE);
}

/// A body with a tab on its top left.
fn folder(o: &mut Outline) {
    frame(
        o,
        &[
            (120.0, 210.0),
            (880.0, 210.0),
            (880.0, 680.0),
            (480.0, 680.0),
            (410.0, 790.0),
            (120.0, 790.0),
        ],
    );
}

/// Walls, a pitched roof and a door.
fn home(o: &mut Outline) {
    frame(o, &[(170.0, 135.0), (830.0, 135.0), (830.0, 535.0), (500.0, 845.0), (170.0, 535.0)]);
    rect(o, 500.0 - H, 135.0, 500.0 + H, 135.0 + 400.0 / PHI);
}

/// A page in φ proportion, its top right corner folded down by 1/φ² of its width.
fn file(o: &mut Outline) {
    let (x0, x1, y0, y1) = (280.0, 720.0, 145.0, 855.0);
    let f = (x1 - x0) / (PHI * PHI);
    frame(o, &[(x0, y0), (x1, y0), (x1, y1 - f), (x1 - f, y1), (x0, y1)]);
    stroke(o, &[(x1 - f, y1), (x1 - f, y1 - f), (x1, y1 - f)]);
}

/// A golden rectangle with round corners and a title bar 1/φ³ of its height.
fn window(o: &mut Outline) {
    let (x0, x1, r) = (125.0, 875.0, STROKE);
    let h = (x1 - x0) / PHI;
    let (y0, y1) = (C.1 - h / 2.0, C.1 + h / 2.0);
    let mut v = Vec::new();
    corner(&mut v, (x1 - r, y0 + r), r, -FRAC_PI_2);
    corner(&mut v, (x1 - r, y1 - r), r, 0.0);
    corner(&mut v, (x0 + r, y1 - r), r, FRAC_PI_2);
    corner(&mut v, (x0 + r, y0 + r), r, PI);
    frame(o, &v);
    let bar = y1 - H - (h - STROKE) / PHI.powi(3);
    rect(o, x0, bar - STROKE, x1, bar);
}

/// A rounded box in φ proportion, its left side running down into a tail.
fn feedback(o: &mut Outline) {
    let (x0, x1, y0, r) = (120.0, 880.0, 370.0, 110.0);
    let y1 = y0 + (x1 - x0) / PHI;
    let mut v = vec![(x0, 200.0), (330.0, y0)];
    corner(&mut v, (x1 - r, y0 + r), r, -FRAC_PI_2);
    corner(&mut v, (x1 - r, y1 - r), r, 0.0);
    corner(&mut v, (x0 + r, y1 - r), r, FRAC_PI_2);
    frame(o, &v);
}

/// A ring of radius 440 around an i.
fn about(o: &mut Outline) {
    circle(o, C, 440.0, true);
    circle(o, C, 440.0 - STROKE, false);
    circle(o, (C.0, 690.0), 62.0, true);
    rect(o, C.0 - H, 270.0, C.0 + H, 580.0);
}

/// A right angle pointing right.
fn chevron(o: &mut Outline) {
    stroke(o, &[(360.0, 745.0), (605.0, 500.0), (360.0, 255.0)]);
}

/// Two crossed strokes.
fn close(o: &mut Outline) {
    stroke(o, &[(215.0, 215.0), (785.0, 785.0)]);
    stroke(o, &[(215.0, 785.0), (785.0, 215.0)]);
}

#[cfg(test)]
mod tests;

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
//!
//! A `.app` file has no glyph of its own: [`sigil`] draws one from a hash of its name.

#![forbid(unsafe_code)]

use core::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

pub use font::Point;
pub use trig::{cos, sin};

mod trig;

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
    /// compusophy's logo: 365 dots in Fibonacci rings, on nothing (the background shows).
    Mark,
    /// The AI button: a ring around a dot.
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
    /// Feedback in the top bar: a beetle.
    Bug,
}

impl Glyph {
    /// Every glyph, in order.
    #[rustfmt::skip]
    pub const ALL: [Glyph; 15] = {
        use Glyph::*;
        [Mark, Apps, Cog, Studio, Assistant, Terminal, Folder, Home, File, Window, Feedback,
            About, Chevron, Close, Bug]
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
            Bug => bug,
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
    let at = |a: f32, k: f32, on| Point { x: c.0 + k * cos(a), y: c.1 + k * sin(a), on };
    v.push(at(a0, r, true));
    for i in 0..n as usize {
        let a = a0 + step * i as f32;
        v.push(at(a + step / 2.0, r / cos(step / 2.0), false));
        v.push(at(a + step, r, true));
    }
}

/// A disc of radius `r` around `c`, or a round hole.
fn circle(o: &mut Outline, c: (f32, f32), r: f32, solid: bool) {
    oval(o, c, (r, r), solid);
}

/// An ellipse of radii `(rx, ry)` around `c`, solid or a hole: a unit circle stretched (its
/// quadratic segments stay exact under the stretch).
fn oval(o: &mut Outline, c: (f32, f32), (rx, ry): (f32, f32), solid: bool) {
    let mut v = Vec::new();
    arc(&mut v, (0.0, 0.0), 1.0, 0.0, TAU);
    v.pop(); // the contour closes itself
    add(
        o,
        v.into_iter().map(|p| Point { x: c.0 + p.x * rx, y: c.1 + p.y * ry, ..p }).collect(),
        solid,
    );
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
    v.extend((0..=8).map(|i| (c.0 + r * cos(at(i)), c.1 + r * sin(at(i)))));
}

/// The radius of the mark's center dot, in box units (its outer ring reaches 500).
pub const MARK_HOLE: f32 = 80.08;

/// The mark's rings, inside out: dot count, ring radius, dot radius, in box units from the
/// center. Counts run up the Fibonacci numbers from 8; each ring is 1/φ closer to the last and
/// its dots 1/φ smaller (from the center hole, [`MARK_HOLE`], and a first step of 190.74).
pub fn rings() -> impl Iterator<Item = (usize, f32, f32)> {
    let mut s = (5, 8, 0.0, 190.74, MARK_HOLE);
    (0..7).map(move |_| {
        let (prev, n, r, step, dot) = s;
        s = (n, prev + n, r + step, step / PHI, dot / PHI);
        (n, r + step, dot / PHI)
    })
}

/// The center dot and every ring's dots, each ring's first dot at 12 o'clock and the rest
/// clockwise: ink on whatever lies under it, as compusophy.com's mark is white on black.
fn mark(o: &mut Outline) {
    circle(o, C, MARK_HOLE, true);
    for (n, r, dot) in rings() {
        for j in 0..n {
            let a = FRAC_PI_2 - TAU * j as f32 / n as f32;
            circle(o, (C.0 + r * cos(a), C.1 + r * sin(a)), dot, true);
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
        let (u, n) = ((cos(t), sin(t)), (-sin(t), cos(t)));
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
        (c, r) = ((c.0 + k * cos(a), c.1 + k * sin(a)), r / PHI);
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

/// A beetle from above: an oval shell split down the middle, a round head on it, three legs a
/// side (up, out, down) and two antennae.
fn bug(o: &mut Outline) {
    let (c, r) = ((C.0, 390.0), (180.0, 250.0));
    oval(o, c, (r.0 + H, r.1 + H), true);
    oval(o, c, (r.0 - H, r.1 - H), false);
    rect(o, C.0 - H, c.1 - r.1, C.0 + H, c.1 + r.1);
    circle(o, (C.0, 745.0), 80.0, true);
    for s in [-1.0, 1.0] {
        let x = |d: f32| C.0 + s * d;
        for (y0, y1) in [(500.0, 600.0), (390.0, 390.0), (280.0, 180.0)] {
            stroke(o, &[(x(140.0), y0), (x(350.0), y1)]);
        }
        stroke(o, &[(x(30.0), 790.0), (x(110.0), 900.0)]);
    }
}

/// The sigil of `seed` (a hash of a `.app` file's name): sacred geometry in the glyphs' weight
/// on `n` = 5 to 9 points, the first at 12 o'clock, in one of five families, each four ways:
///
/// - a star: the deepest star polygon {n/k}, solid, its heart cut out (1/φ of its inner
///   radius) or not; or in a ring, its points touching it or not;
/// - a rosette: n dots, each 1/φ of what would touch, around a dot 1/φ smaller; or in a ring;
///   turned half a step or not;
/// - a polygon: a triangle to a heptagon of strokes, turned half a step or not; around a dot
///   1/φ² of its inside, or empty;
/// - a sun: n rays around a dot, or shorter ones around a disc; turned half a step or not;
/// - a constellation: n dots joined as the star {n/k}, or as a polygon; around a dot or not.
///
/// A seed is always the same shape.
pub fn sigil(seed: u32, o: &mut Outline) {
    let mut s = seed ^ (seed >> 16);
    s = (s.wrapping_mul(0x7feb_352d) ^ (s >> 15)).wrapping_mul(0x846c_a68b);
    s ^= s >> 16;
    o.clear();
    let (family, a, b) = (s / 5 % 5, s / 25 % 2 == 1, s / 50 % 2 == 1);
    let n = 5 + s % 5 - if family == 2 { 2 } else { 0 };
    let (step, k) = (PI / n as f32, (n - 1) / 2);
    let at = |r: f32, a: f32| (C.0 + r * cos(a), C.1 + r * sin(a));
    // The n points' angles, counter-clockwise from 12 o'clock, `half` a step on if asked.
    let angles = |half: bool| {
        let a0 = FRAC_PI_2 + if half { step } else { 0.0 };
        (0..n).map(move |i| a0 + 2.0 * step * i as f32)
    };
    let ring = |o: &mut Outline| {
        circle(o, C, 440.0, true);
        circle(o, C, 440.0 - STROKE, false);
    };
    if a && family < 2 {
        ring(o);
    }
    match family {
        0 => {
            let r = match (a, b) {
                (true, true) => 440.0 - STROKE,
                (true, false) => 440.0 - 2.0 * STROKE,
                (false, _) => 460.0,
            };
            let inner = r * cos(step * k as f32) / cos(step * (k - 1) as f32);
            let pts = (0..2 * n).map(|i| {
                let (x, y) = at(if i % 2 == 0 { r } else { inner }, FRAC_PI_2 + step * i as f32);
                pt(x, y)
            });
            add(o, pts.collect(), true);
            if b && !a {
                circle(o, C, inner / PHI, false);
            }
        }
        1 => {
            let r = if a { 255.0 } else { 340.0 };
            let dot = r * sin(step) / PHI;
            angles(b).for_each(|x| circle(o, at(r, x), dot, true));
            circle(o, C, dot / PHI, true);
        }
        2 => {
            // Each corner's miter reaches 470, whatever its angle.
            let r = 470.0 - H / sin(FRAC_PI_2 - step);
            frame(o, &angles(a).map(|x| at(r, x)).collect::<Vec<_>>());
            if b {
                circle(o, C, (r * cos(step) - H) / (PHI * PHI), true);
            }
        }
        3 => {
            let (r0, dot) = if a { (270.0, 170.0) } else { (200.0, 200.0 / PHI) };
            angles(b).for_each(|x| stroke(o, &[at(r0, x), at(450.0, x)]));
            circle(o, C, dot, true);
        }
        _ => {
            let pts: Vec<_> = angles(false).map(|x| at(380.0, x)).collect();
            let hop = if a { 1 } else { k as usize };
            for (i, &p) in pts.iter().enumerate() {
                stroke(o, &[p, pts[(i + hop) % pts.len()]]);
            }
            pts.iter().for_each(|&p| circle(o, p, 75.0, true));
            if b {
                circle(o, C, 75.0, true);
            }
        }
    }
}

#[cfg(test)]
mod tests;

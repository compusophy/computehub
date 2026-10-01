use crate::{Bitmap, FontError, Point};

/// How close (in px) flattened lines stay to a curve.
const TOLERANCE: f32 = 0.2;
const MAX_SIDE: f32 = 8192.0;
const MAX_AREA: f32 = 16_777_216.0;

/// `acc` rows are `w + 2` cells wide: right-edge spill lands outside the sum.
struct Raster {
    w: usize,
    h: usize,
    acc: Vec<f32>,
}

fn mid(a: Point, b: Point) -> Point {
    Point { x: (a.x + b.x) / 2.0, y: (a.y + b.y) / 2.0, on: true }
}

impl Raster {
    fn add(&mut self, i: usize, v: f32) {
        self.acc.get_mut(i).into_iter().for_each(|cell| *cell += v);
    }

    /// Adds one line, in bitmap coordinates clamped to `[0, w] x [0, h]`.
    fn line(&mut self, a: Point, b: Point) {
        if (a.y - b.y).abs() <= f32::EPSILON {
            return;
        }
        let (dir, p0, p1) = if a.y < b.y { (1.0, a, b) } else { (-1.0, b, a) };
        let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
        let (wf, mut x, y_end) = (self.w as f32, p0.x, (p1.y.ceil() as usize).min(self.h));
        for y in p0.y as usize..y_end {
            let row = y * (self.w + 2);
            let dy = ((y + 1) as f32).min(p1.y) - (y as f32).max(p0.y);
            let xnext = (x + dxdy * dy).max(0.0).min(wf);
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let (x0floor, x1ceil) = (x0.floor(), x1.ceil());
            let (x0i, x1i) = (x0floor as usize, x1ceil as usize);
            if x1i <= x0i + 1 {
                // The line stays in one cell: split its area by the mean x.
                let xm = 0.5 * (x + xnext) - x0floor;
                self.add(row + x0i, d - d * xm);
                self.add(row + x0i + 1, d * xm);
            } else {
                let (s, x0f, x1f) = ((x1 - x0).recip(), x0 - x0floor, x1 - x1ceil + 1.0);
                let (a0, am) = (0.5 * s * (1.0 - x0f) * (1.0 - x0f), 0.5 * s * x1f * x1f);
                self.add(row + x0i, d * a0);
                if x1i == x0i + 2 {
                    self.add(row + x0i + 1, d * (1.0 - a0 - am));
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.add(row + x0i + 1, d * (a1 - a0));
                    for xi in x0i + 2..x1i - 1 {
                        self.add(row + xi, d * s);
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.add(row + x1i - 1, d * (1.0 - a2 - am));
                }
                self.add(row + x1i, d * am);
            }
            x = xnext;
        }
    }

    /// Flattens a curve: `n` steps stray at most `|a - 2c + b| / (4 n^2)`.
    fn quad(&mut self, a: Point, c: Point, b: Point) {
        let (ddx, ddy) = (a.x - 2.0 * c.x + b.x, a.y - 2.0 * c.y + b.y);
        let dd = (ddx * ddx + ddy * ddy).sqrt();
        let n = ((dd / (4.0 * TOLERANCE)).sqrt().ceil() as usize).clamp(1, 256);
        let mut prev = a;
        for i in 1..=n {
            let (t, u) = (i as f32 / n as f32, 1.0 - i as f32 / n as f32);
            let at = |a: f32, c: f32, b: f32| u * u * a + 2.0 * u * t * c + t * t * b;
            let p = Point { x: at(a.x, c.x, b.x), y: at(a.y, c.y, b.y), on: true };
            self.line(prev, p);
            prev = p;
        }
    }

    /// Walks one closed contour (two off-curve points imply one on it halfway).
    fn contour(&mut self, pts: &[Point]) {
        let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { return };
        let (start, body) = if first.on {
            (first, &pts[1..])
        } else if last.on {
            (last, &pts[..pts.len() - 1])
        } else {
            (mid(first, last), pts)
        };
        let (mut cur, mut ctrl) = (start, None::<Point>);
        for &p in body.iter().chain(core::iter::once(&start)) {
            if p.on {
                match ctrl.take() {
                    Some(c) => self.quad(cur, c, p),
                    None => self.line(cur, p),
                }
                cur = p;
            } else {
                if let Some(c) = ctrl {
                    let m = mid(c, p);
                    self.quad(cur, c, m);
                    cur = m;
                }
                ctrl = Some(p);
            }
        }
    }
}

/// Scales an outline by `scale` px per font unit into the emptied `out`.
pub(crate) fn render(o: &[Vec<Point>], scale: f32, out: &mut Bitmap) -> Result<(), FontError> {
    let inf = f32::INFINITY;
    let (x0, y0, x1, y1) = o.iter().flatten().fold((inf, inf, -inf, -inf), |b, p| {
        (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y))
    });
    if x0 > x1 {
        return Ok(());
    }
    let (left, top) = ((x0 * scale).floor(), (-y1 * scale).floor());
    let (w, h) = ((x1 * scale).ceil() - left, (-y0 * scale).ceil() - top);
    let fits = left.abs() <= 1e8 && top.abs() <= 1e8;
    if !(fits && w <= MAX_SIDE && h <= MAX_SIDE && w * h <= MAX_AREA) {
        return Err(FontError::BadSize);
    }
    if w < 1.0 || h < 1.0 {
        return Ok(());
    }
    let (wu, hu) = (w as usize, h as usize);
    let mut r = Raster { w: wu, h: hu, acc: vec![0.0; (wu + 2) * hu] };
    let px = |v: f32, at: f32, max: f32| (v * scale - at).max(0.0).min(max);
    let mut pts = Vec::new();
    for c in o {
        pts.clear();
        pts.extend(c.iter().map(|p| Point { x: px(p.x, left, w), y: px(-p.y, top, h), on: p.on }));
        r.contour(&pts);
    }
    out.data.reserve(wu * hu);
    for row in r.acc.chunks_exact(wu + 2) {
        let mut sum = 0.0f32;
        for &cell in &row[..wu] {
            sum += cell;
            out.data.push((sum.abs().min(1.0) * 255.0 + 0.5) as u8);
        }
    }
    (out.w, out.h, out.left, out.top) = (wu as u32, hu as u32, left as i32, top as i32);
    Ok(())
}

//! The accumulation-buffer rasterizer: lines add signed area and coverage to
//! the cells they cross, and a running sum along each row gives coverage.

use crate::{Bitmap, FontError, Outline, Point};

/// Curves are flattened until every line is this close to the curve, in px.
const TOLERANCE: f32 = 0.2;
/// Largest bitmap side, in px.
const MAX_SIDE: f32 = 8192.0;
/// Largest bitmap area, in px.
const MAX_AREA: f32 = 16_777_216.0;

struct Raster {
    w: usize,
    h: usize,
    /// Row pitch of `acc`: two cells wider than the bitmap, so a line on the
    /// right edge spills into cells that the row sum ignores.
    stride: usize,
    acc: Vec<f32>,
}

fn mid(a: Point, b: Point) -> Point {
    Point { x: (a.x + b.x) / 2.0, y: (a.y + b.y) / 2.0, on: true }
}

impl Raster {
    fn add(&mut self, i: usize, v: f32) {
        if let Some(cell) = self.acc.get_mut(i) {
            *cell += v;
        }
    }

    /// Adds one line, in bitmap coordinates clamped to `[0, w] x [0, h]`.
    fn line(&mut self, a: Point, b: Point) {
        if (a.y - b.y).abs() <= f32::EPSILON {
            return;
        }
        let (dir, p0, p1) = if a.y < b.y { (1.0, a, b) } else { (-1.0, b, a) };
        let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
        let wf = self.w as f32;
        let mut x = p0.x;
        let y_end = (p1.y.ceil() as usize).min(self.h);
        for y in p0.y as usize..y_end {
            let row = y * self.stride;
            let dy = ((y + 1) as f32).min(p1.y) - (y as f32).max(p0.y);
            let xnext = (x + dxdy * dy).max(0.0).min(wf);
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let x0floor = x0.floor();
            let x0i = x0floor as usize;
            let x1ceil = x1.ceil();
            let x1i = x1ceil as usize;
            if x1i <= x0i + 1 {
                // The line stays in one cell: split its area by the mean x.
                let xm = 0.5 * (x + xnext) - x0floor;
                self.add(row + x0i, d - d * xm);
                self.add(row + x0i + 1, d * xm);
            } else {
                let s = (x1 - x0).recip();
                let x0f = x0 - x0floor;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = x1 - x1ceil + 1.0;
                let am = 0.5 * s * x1f * x1f;
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

    /// Flattens a quadratic curve into lines within [`TOLERANCE`]: `n` equal
    /// steps leave at most `|a - 2c + b| / (4 n^2)` between chord and curve.
    fn quad(&mut self, a: Point, c: Point, b: Point) {
        let (ddx, ddy) = (a.x - 2.0 * c.x + b.x, a.y - 2.0 * c.y + b.y);
        let dd = (ddx * ddx + ddy * ddy).sqrt();
        let n = ((dd / (4.0 * TOLERANCE)).sqrt().ceil() as usize).clamp(1, 256);
        let mut prev = a;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let p = Point {
                x: u * u * a.x + 2.0 * u * t * c.x + t * t * b.x,
                y: u * u * a.y + 2.0 * u * t * c.y + t * t * b.y,
                on: true,
            };
            self.line(prev, p);
            prev = p;
        }
    }

    /// Walks one closed TrueType contour: two off-curve points in a row imply
    /// an on-curve point halfway between them.
    fn contour(&mut self, pts: &[Point]) {
        let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else {
            return;
        };
        let (start, body) = if first.on {
            (first, &pts[1..])
        } else if last.on {
            (last, &pts[..pts.len() - 1])
        } else {
            (mid(first, last), pts)
        };
        let mut cur = start;
        let mut ctrl: Option<Point> = None;
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

/// Scales an outline by `scale` px per font unit and fills `out`, which the
/// caller has already emptied.
pub(crate) fn render(outline: &Outline, scale: f32, out: &mut Bitmap) -> Result<(), FontError> {
    let (mut x0, mut y0) = (f32::INFINITY, f32::INFINITY);
    let (mut x1, mut y1) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in outline.contours.iter().flatten() {
        x0 = x0.min(p.x);
        x1 = x1.max(p.x);
        y0 = y0.min(p.y);
        y1 = y1.max(p.y);
    }
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
    let mut r = Raster { w: wu, h: hu, stride: wu + 2, acc: vec![0.0; (wu + 2) * hu] };
    let mut pts = Vec::new();
    for c in &outline.contours {
        pts.clear();
        pts.extend(c.iter().map(|p| Point {
            x: (p.x * scale - left).max(0.0).min(w),
            y: (-p.y * scale - top).max(0.0).min(h),
            on: p.on,
        }));
        r.contour(&pts);
    }
    out.data.reserve(wu * hu);
    for row in r.acc.chunks_exact(r.stride) {
        let mut sum = 0.0f32;
        for &cell in &row[..wu] {
            sum += cell;
            out.data.push((sum.abs().min(1.0) * 255.0 + 0.5) as u8);
        }
    }
    (out.w, out.h, out.left, out.top) = (wu as u32, hu as u32, left as i32, top as i32);
    Ok(())
}

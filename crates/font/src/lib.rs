//! A dependency-free TrueType reader and glyph rasterizer for compusophyOS.
//!
//! - [`Font::parse`] checks the header and the bounds of every table it needs (`head hhea maxp
//!   cmap hmtx loca glyf`); every later read is bounds-checked again, so malformed input is an
//!   `Err`, never a panic or a garbage picture. One bad `loca` entry fails only its glyph.
//! - `glyf` outlines only (CFF and collections are [`FontError::NotTrueType`]); `cmap` format 12
//!   is preferred over format 4; no hinting, kerning or variations. Composites apply offsets,
//!   scales and 2x2 matrices (point-matched parts sit at offset 0), at most 8 levels deep, 1,024
//!   components and 262,144 points.
//! - [`Font::rasterize`] (and [`render_outline`], for any outline) flattens curves to within 0.2 px
//!   and accumulates signed area per cell (the font-rs technique): coverage is `min(1, |winding|)`,
//!   so holes cancel and overlaps clamp.

#![forbid(unsafe_code)]

use core::fmt;
use core::ops::Range;

mod raster;
#[cfg(test)]
mod tests;

const MAX_DEPTH: u32 = 8;
const MAX_COMPONENTS: u32 = 1024;
const MAX_POINTS: usize = 1 << 18;

/// Why a font or glyph could not be read or drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontError {
    /// A bad header, a CFF font or a collection.
    NotTrueType,
    /// A required table (this tag) is absent.
    MissingTable([u8; 4]),
    /// An offset, length, count or field in this table is out of range.
    Malformed([u8; 4]),
    /// The glyph id is not below the glyph count.
    NoGlyph(u16),
    /// A composite nests or expands past the limits.
    TooComplex,
    /// A size that is not finite and positive, or a bitmap past 8,192 px or 16 M px.
    BadSize,
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use FontError::*;
        match self {
            NotTrueType => write!(f, "not a TrueType font"),
            MissingTable(t) => write!(f, "missing `{}` table", String::from_utf8_lossy(t)),
            Malformed(t) => write!(f, "malformed `{}` table", String::from_utf8_lossy(t)),
            NoGlyph(g) => write!(f, "no glyph {}", u32::from(*g)), // u32: its Display is in the boot
            TooComplex => write!(f, "composite glyph too deep or too large"),
            BadSize => write!(f, "bad pixel size or bitmap too large"),
        }
    }
}

/// An outline point in font units, y up; `on` is false for a quadratic control.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub on: bool,
}

/// An 8-bit coverage bitmap of one glyph: `w * h` bytes in `data`, row-major, its top-left corner
/// `left` px right of the pen and `top` px below the baseline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    pub left: i32,
    pub top: i32,
    pub data: Vec<u8>,
}

/// A parsed TrueType font. It owns its bytes.
#[derive(Clone, Debug)]
pub struct Font {
    data: Vec<u8>,
    metrics: [i32; 4],
    num_glyphs: u16,
    num_hmetrics: u16,
    long_loca: bool,
    hmtx: usize,
    loca: usize,
    glyf: Range<usize>,
    cmap: Cmap,
}

/// The chosen `cmap` subtable: its bytes, its segment or group count, its format.
#[derive(Clone, Debug)]
struct Cmap {
    sub: Range<usize>,
    n: usize,
    f12: bool,
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at.checked_add(2)?)?.try_into().ok().map(u16::from_be_bytes)
}
fn i16_at(d: &[u8], at: usize) -> Option<i16> {
    u16_at(d, at).map(|v| v as i16)
}
fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at.checked_add(4)?)?.try_into().ok().map(u32::from_be_bytes)
}
fn span(d: &[u8], start: usize, len: usize) -> Option<Range<usize>> {
    let end = start.checked_add(len)?;
    (end <= d.len()).then_some(start..end)
}

/// Finds a table of at least `min` bytes in the directory, checked to lie inside the file.
fn table(d: &[u8], tag: &[u8; 4], min: usize) -> Result<Range<usize>, FontError> {
    let count = u16_at(d, 4).ok_or(FontError::NotTrueType)?;
    for i in 0..usize::from(count) {
        let rec = 12 + 16 * i;
        if d.get(rec..rec + 4).ok_or(FontError::NotTrueType)? == tag {
            let at = u32_at(d, rec + 8).zip(u32_at(d, rec + 12));
            let t = at.and_then(|(off, len)| span(d, off as usize, len as usize));
            return t.filter(|t| t.len() >= min).ok_or(FontError::Malformed(*tag));
        }
    }
    Err(FontError::MissingTable(*tag))
}

/// Picks the best Unicode subtable: format 12 over format 4.
fn pick_cmap(d: &[u8], cmap: Range<usize>) -> Result<Cmap, FontError> {
    let bad = FontError::Malformed(*b"cmap");
    let c = d.get(cmap.clone()).ok_or(bad)?;
    let count = u16_at(c, 2).ok_or(bad)?;
    let mut best: Option<Cmap> = None;
    for i in 0..usize::from(count) {
        let rec = 4 + 8 * i;
        let ids = u16_at(c, rec).zip(u16_at(c, rec + 2)).zip(u32_at(c, rec + 4));
        let ((plat, enc), off) = ids.ok_or(bad)?;
        if !(plat == 0 || (plat == 3 && (enc == 1 || enc == 10))) {
            continue;
        }
        let (s, at) = (c.get(off as usize..).ok_or(bad)?, cmap.start + off as usize);
        let found = match u16_at(s, 0).ok_or(bad)? {
            12 if plat == 0 || enc == 10 => {
                let groups = u32_at(s, 12).ok_or(bad)? as usize;
                let len = groups.checked_mul(12).and_then(|n| n.checked_add(16));
                let sub = len.and_then(|n| span(d, at, n)).ok_or(bad)?;
                Cmap { sub, n: groups, f12: true }
            }
            4 => {
                let segs = u16_at(s, 6).map(|n| usize::from(n / 2));
                let n = segs.filter(|&n| n > 0 && 16 + 8 * n <= s.len()).ok_or(bad)?;
                Cmap { sub: at..cmap.end, n, f12: false }
            }
            _ => continue,
        };
        best = best.filter(|b| b.f12 || !found.f12).or(Some(found)); // format 12 wins
    }
    best.ok_or(bad)
}

/// Format 4: sorted segments, each with a delta or glyph id array offset.
fn cmap4(s: &[u8], segs: usize, c: u32) -> Option<u16> {
    let c = u16::try_from(c).ok()?;
    let (starts, deltas, ranges) = (16 + 2 * segs, 16 + 4 * segs, 16 + 6 * segs);
    let (mut lo, mut hi) = (0, segs);
    while lo < hi {
        let mid = (lo + hi) / 2;
        (lo, hi) = if u16_at(s, 14 + 2 * mid)? < c { (mid + 1, hi) } else { (lo, mid) };
    }
    let start = u16_at(s, starts + 2 * lo).filter(|&start| lo < segs && c >= start)?;
    let (delta, range) = (u16_at(s, deltas + 2 * lo)?, usize::from(u16_at(s, ranges + 2 * lo)?));
    if range == 0 {
        return Some(c.wrapping_add(delta));
    }
    let g = u16_at(s, ranges + 2 * lo + range + 2 * usize::from(c - start))?;
    (g != 0).then(|| g.wrapping_add(delta))
}

/// Format 12: sorted groups of consecutive code points and glyph ids.
fn cmap12(s: &[u8], groups: usize, c: u32) -> Option<u16> {
    let (mut lo, mut hi) = (0, groups);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let g = 16 + 12 * mid;
        let (start, end) = (u32_at(s, g)?, u32_at(s, g + 4)?);
        if start <= c && c <= end {
            return u16::try_from(u32_at(s, g + 8)?.checked_add(c - start)?).ok();
        }
        (lo, hi) = if end < c { (mid + 1, hi) } else { (lo, mid) };
    }
    None
}

/// An affine map `[a, b, c, d, e, f]`: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
type Affine = [f32; 6];
const IDENTITY: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The linear part of `m` applied to `(x, y)`.
fn lin(m: &Affine, x: f32, y: f32) -> [f32; 2] {
    [m[0] * x + m[2] * y, m[1] * x + m[3] * y]
}

/// `o` applied after `i`.
fn compose(o: &Affine, i: &Affine) -> Affine {
    let ([a, b], [c, d], [e, f]) = (lin(o, i[0], i[1]), lin(o, i[2], i[3]), lin(o, i[4], i[5]));
    [a, b, c, d, e + o[4], f + o[5]]
}

/// A glyph's outline being built, and what it may still expand to.
struct Walk<'a> {
    out: &'a mut Vec<Vec<Point>>,
    components: u32,
    points: usize,
}

impl Font {
    /// Reads a TrueType font, checking the header and every table it uses.
    pub fn parse(data: Vec<u8>) -> Result<Font, FontError> {
        let d = &data[..];
        if !matches!(u32_at(d, 0), Some(0x0001_0000 | 0x7472_7565)) {
            return Err(FontError::NotTrueType);
        }
        let head = &d[table(d, b"head", 0)?];
        let bad = FontError::Malformed(*b"head");
        let magic = u32_at(head, 12) == Some(0x5F0F_3CF5);
        let upem = u16_at(head, 18).filter(|u| magic && (16..=16384).contains(u)).ok_or(bad)?;
        let long_loca = i16_at(head, 50).filter(|&v| v == 0 || v == 1).ok_or(bad)? == 1;
        let hhea = &d[table(d, b"hhea", 0)?];
        let bad = FontError::Malformed(*b"hhea");
        let [Some(asc), Some(desc), Some(gap)] = [4, 6, 8].map(|at| i16_at(hhea, at)) else {
            return Err(bad);
        };
        let num_hmetrics = u16_at(hhea, 34).filter(|&n| n > 0).ok_or(bad)?;
        let metrics = [upem.into(), asc.into(), desc.into(), gap.into()];
        let maxp = &d[table(d, b"maxp", 0)?];
        let num_glyphs = u16_at(maxp, 4).ok_or(FontError::Malformed(*b"maxp"))?;
        let hmtx = table(d, b"hmtx", 4 * usize::from(num_hmetrics))?.start;
        let entry = if long_loca { 4 } else { 2 };
        let loca = table(d, b"loca", entry * (usize::from(num_glyphs) + 1))?.start;
        let (glyf, cmap) = (table(d, b"glyf", 0)?, pick_cmap(d, table(d, b"cmap", 0)?)?);
        Ok(Font { metrics, num_glyphs, num_hmetrics, long_loca, hmtx, loca, glyf, cmap, data })
    }

    /// Units per em, ascender, descender (usually negative) and line gap.
    pub fn metrics(&self) -> [i32; 4] {
        self.metrics
    }

    /// The glyph for a character; `None` for none, glyph 0 or an id too big.
    pub fn glyph_index(&self, c: char) -> Option<u16> {
        let (s, n) = (self.data.get(self.cmap.sub.clone())?, self.cmap.n);
        let g = if self.cmap.f12 { cmap12(s, n, c.into()) } else { cmap4(s, n, c.into()) }?;
        (g != 0 && g < self.num_glyphs).then_some(g)
    }

    /// Horizontal advance in font units; glyphs past the last metric take it.
    pub fn advance(&self, glyph: u16) -> u16 {
        let i = usize::from(glyph.min(self.num_hmetrics.saturating_sub(1)));
        u16_at(&self.data, self.hmtx + 4 * i).unwrap_or(0)
    }

    /// Where a glyph's `glyf` data lies (empty for a glyph with no outline).
    fn glyph_range(&self, glyph: u16) -> Result<Range<usize>, FontError> {
        if glyph >= self.num_glyphs {
            return Err(FontError::NoGlyph(glyph));
        }
        let (g, i) = (&self.glyf, usize::from(glyph));
        let at = |k: usize| match self.long_loca {
            true => u32_at(&self.data, self.loca + 4 * k).map(|v| v as usize),
            false => u16_at(&self.data, self.loca + 2 * k).map(|v| 2 * usize::from(v)),
        };
        match (at(i), at(i + 1)) {
            (Some(a), Some(b)) if a <= b && b <= g.len() => Ok(g.start + a..g.start + b),
            _ => Err(FontError::Malformed(*b"loca")),
        }
    }

    /// A glyph's contours in font units, replacing `out`'s (empty on error).
    pub fn outline(&self, glyph: u16, out: &mut Vec<Vec<Point>>) -> Result<(), FontError> {
        out.clear();
        let mut w = Walk { out, components: MAX_COMPONENTS, points: MAX_POINTS };
        self.walk(glyph, &IDENTITY, 0, &mut w).inspect_err(|_| w.out.clear())
    }

    fn walk(&self, glyph: u16, m: &Affine, depth: u32, w: &mut Walk) -> Result<(), FontError> {
        let d = &self.data[self.glyph_range(glyph)?];
        if d.is_empty() {
            return Ok(());
        }
        let bad = FontError::Malformed(*b"glyf");
        let contours = i16_at(d, 0).ok_or(bad)?;
        if contours >= 0 {
            return simple(d, contours as usize, m, w);
        }
        let mut p = 10;
        loop {
            let (flags, child) = u16_at(d, p).zip(u16_at(d, p + 2)).ok_or(bad)?;
            p += 4;
            let words = flags & 0x0001 != 0;
            let arg = |k: usize| match words {
                true => i16_at(d, p + 2 * k).map(f32::from),
                false => d.get(p + k).map(|&b| f32::from(b as i8)),
            };
            let (dx, dy) = arg(0).zip(arg(1)).ok_or(bad)?;
            p += if words { 4 } else { 2 };
            // Without ARGS_ARE_XY_VALUES the args name points to match.
            let (dx, dy) = if flags & 0x0002 != 0 { (dx, dy) } else { (0.0, 0.0) };
            // A scale, x and y scales, a 2x2 matrix or none: 1, 2, 4 or 0 F2Dot14 words.
            let f2 = |k: usize| i16_at(d, p + 2 * k).map(|v| f32::from(v) / 16384.0).ok_or(bad);
            let (n, mut t) = if flags & 0x0008 != 0 {
                (1, [f2(0)?, 0.0, 0.0, f2(0)?, 0.0, 0.0])
            } else if flags & 0x0040 != 0 {
                (2, [f2(0)?, 0.0, 0.0, f2(1)?, 0.0, 0.0])
            } else if flags & 0x0080 != 0 {
                (4, [f2(0)?, f2(1)?, f2(2)?, f2(3)?, 0.0, 0.0])
            } else {
                (0, IDENTITY)
            };
            p += 2 * n;
            // SCALED_COMPONENT_OFFSET (without UNSCALED_...) transforms the offset.
            let scaled = flags & 0x1800 == 0x0800;
            [t[4], t[5]] = if scaled { lin(&t, dx, dy) } else { [dx, dy] };
            let left = w.components.checked_sub(1).filter(|_| depth < MAX_DEPTH);
            w.components = left.ok_or(FontError::TooComplex)?;
            self.walk(child, &compose(m, &t), depth + 1, w)?;
            if flags & 0x0020 == 0 {
                return Ok(());
            }
        }
    }

    /// Draws a glyph into `out`; an empty glyph or an error leaves `w = h = 0`.
    pub fn rasterize(&self, glyph: u16, px_per_em: f32, out: &mut Bitmap) -> Result<(), FontError> {
        let mut outline = Vec::new();
        let found = self.outline(glyph, &mut outline);
        render_outline(&outline, px_per_em / self.metrics[0] as f32, out)?;
        found
    }
}

/// Draws any outline (contours as [`Font::outline`] gives them, y up) at `scale` px per unit,
/// as [`Font::rasterize`] draws a glyph: `out`'s corner is relative to the origin.
pub fn render_outline(
    contours: &[Vec<Point>],
    scale: f32,
    out: &mut Bitmap,
) -> Result<(), FontError> {
    (out.w, out.h, out.left, out.top) = (0, 0, 0, 0);
    out.data.clear();
    if !(scale.is_finite() && scale > 0.0) {
        return Err(FontError::BadSize);
    }
    raster::render(contours, scale, out)
}

/// Per-point deltas: `short` is a byte signed by `same`, else `same` repeats.
fn deltas(d: &[u8], p: &mut usize, flags: &[u8], short: u8, same: u8) -> Option<Vec<i32>> {
    let (mut out, mut v) = (Vec::with_capacity(flags.len()), 0i32);
    for &f in flags {
        if f & short != 0 {
            let b = i32::from(*d.get(*p)?);
            *p += 1;
            v = v.wrapping_add(if f & same != 0 { b } else { -b });
        } else if f & same == 0 {
            v = v.wrapping_add(i32::from(i16_at(d, *p)?));
            *p += 2;
        }
        out.push(v);
    }
    Some(out)
}

/// A simple glyph: contour ends, instructions (skipped), flags, x and y deltas.
fn simple(d: &[u8], contours: usize, m: &Affine, w: &mut Walk) -> Result<(), FontError> {
    let bad = FontError::Malformed(*b"glyf");
    if contours == 0 {
        return Ok(());
    }
    let (mut ends, mut n) = (Vec::with_capacity(contours), 0usize);
    for k in 0..contours {
        n = 1 + u16_at(d, 10 + 2 * k).map(usize::from).filter(|&end| end >= n).ok_or(bad)?;
        ends.push(n);
    }
    w.points = w.points.checked_sub(n).ok_or(FontError::TooComplex)?;
    let p = 10 + 2 * contours;
    let mut p = p + 2 + usize::from(u16_at(d, p).ok_or(bad)?);
    let mut flags = Vec::with_capacity(n);
    while flags.len() < n {
        let f = *d.get(p).ok_or(bad)?;
        let reps = if f & 0x08 != 0 { 1 + usize::from(*d.get(p + 1).ok_or(bad)?) } else { 1 };
        p += 1 + usize::from(f & 0x08 != 0);
        flags.extend(core::iter::repeat_n(f, reps.min(n - flags.len())));
    }
    let xs = deltas(d, &mut p, &flags, 0x02, 0x10).ok_or(bad)?;
    let ys = deltas(d, &mut p, &flags, 0x04, 0x20).ok_or(bad)?;
    let point = |(&f, (&x, &y)): (&u8, (&i32, &i32))| {
        let [x, y] = lin(m, x as f32, y as f32);
        Point { x: x + m[4], y: y + m[5], on: f & 0x01 != 0 }
    };
    let points: Vec<Point> = flags.iter().zip(xs.iter().zip(&ys)).map(point).collect();
    let mut start = 0;
    for end in ends {
        w.out.push(points.get(start..end).ok_or(bad)?.to_vec());
        start = end;
    }
    Ok(())
}

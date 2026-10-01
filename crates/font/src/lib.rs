//! A dependency-free TrueType reader and glyph rasterizer for compusophyOS.
//!
//! [`Font::parse`] takes the font's bytes, finds the tables it needs and
//! checks their offsets, lengths and counts; every later read is
//! bounds-checked again, so malformed input is an `Err`, never a panic and
//! never a garbage picture. [`Font::outline`] gives a glyph's contours in font
//! units, and [`Font::rasterize`] turns them into an anti-aliased coverage
//! [`Bitmap`] at any size.
//!
//! # Contract
//!
//! - TrueType (`glyf`) outlines only. Required tables: `head`, `hhea`,
//!   `maxp`, `cmap`, `hmtx`, `loca`, `glyf`. CFF fonts (`OTTO`) and
//!   collections (`ttcf`) are [`FontError::NotTrueType`].
//! - Characters map through `cmap` (3,10) or (0,x) format 12 when present,
//!   else (3,1) or (0,x) format 4. Glyph 0 (`.notdef`) and ids past `maxp`'s
//!   glyph count map to `None`.
//! - Outlines are in font units, y up. Composite glyphs apply their offsets,
//!   scales and 2x2 matrices; point-matching components are placed at offset
//!   0. Nesting deeper than 8 composite levels, more than 1,024 components or
//!   more than 262,144 points is [`FontError::TooComplex`].
//! - `loca` entries are checked per glyph when the glyph is read, so one bad
//!   entry fails that glyph, not the whole font.
//! - Hinting, variations, kerning and vertical metrics are not read.
//!
//! # Rasterizer
//!
//! Outlines are scaled by `px_per_em / units_per_em`, and each quadratic
//! curve is flattened into lines until it is within 0.2 px of the true curve.
//! Each line adds its signed area and coverage to the cells it crosses in an
//! accumulation buffer; a running sum along each row then gives the winding
//! coverage, output as `min(1, |sum|) * 255`. Holes (opposite winding) cancel
//! and overlapping same-direction contours clamp at full coverage.
//!
//! Origin: new in compusophyOS (the accumulation technique follows the
//! well-known font-rs design).

#![forbid(unsafe_code)]

use core::fmt;
use core::ops::Range;

mod raster;
#[cfg(test)]
mod tests;

/// Composite glyphs may nest this many levels deep, and no deeper.
const MAX_DEPTH: u32 = 8;
/// Components one glyph may expand to, counting nested ones.
const MAX_COMPONENTS: u32 = 1024;
/// Points one glyph may expand to, counting every component.
const MAX_POINTS: usize = 1 << 18;

/// Why a font or glyph could not be read or drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontError {
    /// Not a TrueType font this crate reads: a bad header, a CFF font or a
    /// font collection.
    NotTrueType,
    /// A required table is absent; the payload is its tag.
    MissingTable([u8; 4]),
    /// An offset, length, count or field in this table is out of range.
    Malformed([u8; 4]),
    /// The glyph id is not below the font's glyph count.
    NoGlyph(u16),
    /// A composite glyph nests deeper than 8 levels or expands past the
    /// component or point limit.
    TooComplex,
    /// `px_per_em` is not finite and positive, or the bitmap would exceed
    /// 8,192 px on a side or 16 M pixels.
    BadSize,
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = |t: &[u8; 4]| String::from_utf8_lossy(t).into_owned();
        match self {
            FontError::NotTrueType => write!(f, "not a TrueType font"),
            FontError::MissingTable(t) => write!(f, "missing `{}` table", tag(t)),
            FontError::Malformed(t) => write!(f, "malformed `{}` table", tag(t)),
            FontError::NoGlyph(g) => write!(f, "no glyph {g}"),
            FontError::TooComplex => write!(f, "composite glyph too deep or too large"),
            FontError::BadSize => write!(f, "bad pixel size or bitmap too large"),
        }
    }
}

impl std::error::Error for FontError {}

/// One outline point in font units, y up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal position.
    pub x: f32,
    /// Vertical position, up from the baseline.
    pub y: f32,
    /// On the curve; an off-curve point is a quadratic control point, and two
    /// in a row imply an on-curve point halfway between them.
    pub on: bool,
}

/// A glyph's closed contours in font units, y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    /// Each contour in order; the last point connects back to the first.
    pub contours: Vec<Vec<Point>>,
}

/// An 8-bit coverage bitmap of one glyph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bitmap {
    /// Width in px.
    pub w: u32,
    /// Height in px.
    pub h: u32,
    /// The bitmap's left edge relative to the pen x, in px.
    pub left: i32,
    /// The bitmap's top edge relative to the baseline, y down (negative is
    /// above the baseline).
    pub top: i32,
    /// `w * h` coverage bytes, row-major, top row first.
    pub data: Vec<u8>,
}

/// A parsed TrueType font. It owns its bytes.
#[derive(Clone)]
pub struct Font {
    data: Vec<u8>,
    upem: u16,
    ascender: i16,
    descender: i16,
    line_gap: i16,
    num_glyphs: u16,
    num_hmetrics: u16,
    long_loca: bool,
    hmtx: usize,
    loca: usize,
    glyf: Range<usize>,
    cmap: Cmap,
}

/// The chosen `cmap` subtable: its byte range in the font and its count.
#[derive(Clone, Debug)]
enum Cmap {
    Format4 { sub: Range<usize>, segs: usize },
    Format12 { sub: Range<usize>, groups: usize },
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("bytes", &self.data.len())
            .field("units_per_em", &self.upem)
            .field("num_glyphs", &self.num_glyphs)
            .field("cmap", &self.cmap)
            .finish_non_exhaustive()
    }
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

/// `start..start + len` if it lies inside `d`.
fn span(d: &[u8], start: usize, len: usize) -> Option<Range<usize>> {
    let end = start.checked_add(len)?;
    (end <= d.len()).then_some(start..end)
}

/// Finds a table in the directory and checks that it lies inside the file.
fn table(d: &[u8], tag: &[u8; 4]) -> Result<Range<usize>, FontError> {
    let count = u16_at(d, 4).ok_or(FontError::NotTrueType)?;
    for i in 0..usize::from(count) {
        let rec = 12 + 16 * i;
        if d.get(rec..rec + 4).ok_or(FontError::NotTrueType)? == tag {
            let bad = FontError::Malformed(*tag);
            let off = u32_at(d, rec + 8).ok_or(bad)? as usize;
            let len = u32_at(d, rec + 12).ok_or(bad)? as usize;
            return span(d, off, len).ok_or(bad);
        }
    }
    Err(FontError::MissingTable(*tag))
}

/// Picks the best Unicode subtable: format 12 over format 4.
fn pick_cmap(d: &[u8], cmap: Range<usize>) -> Result<Cmap, FontError> {
    let bad = FontError::Malformed(*b"cmap");
    let c = d.get(cmap.clone()).ok_or(bad)?;
    let count = u16_at(c, 2).ok_or(bad)?;
    let mut best: Option<(u8, Cmap)> = None;
    for i in 0..usize::from(count) {
        let rec = 4 + 8 * i;
        let (Some(plat), Some(enc), Some(off)) =
            (u16_at(c, rec), u16_at(c, rec + 2), u32_at(c, rec + 4))
        else {
            return Err(bad);
        };
        if !(plat == 0 || (plat == 3 && (enc == 1 || enc == 10))) {
            continue;
        }
        let off = off as usize;
        let s = c.get(off..).ok_or(bad)?;
        let at = cmap.start + off;
        let found = match u16_at(s, 0).ok_or(bad)? {
            12 if plat == 0 || enc == 10 => {
                let groups = u32_at(s, 12).ok_or(bad)? as usize;
                let len = groups.checked_mul(12).and_then(|n| n.checked_add(16));
                let sub = len.and_then(|n| span(d, at, n)).ok_or(bad)?;
                (2, Cmap::Format12 { sub, groups })
            }
            4 => {
                let segs = usize::from(u16_at(s, 6).ok_or(bad)? / 2);
                if segs == 0 || 16 + 8 * segs > s.len() {
                    return Err(bad);
                }
                let sub = at..cmap.end;
                (1, Cmap::Format4 { sub, segs })
            }
            _ => continue,
        };
        if best.as_ref().is_none_or(|(rank, _)| found.0 > *rank) {
            best = Some(found);
        }
    }
    best.map(|(_, c)| c).ok_or(bad)
}

/// Format 4: segments sorted by end code, each a range with a delta or an
/// offset into the glyph id array.
fn cmap4(s: &[u8], segs: usize, c: u32) -> Option<u16> {
    let c = u16::try_from(c).ok()?;
    let (starts, deltas, ranges) = (16 + 2 * segs, 16 + 4 * segs, 16 + 6 * segs);
    let (mut lo, mut hi) = (0, segs);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if u16_at(s, 14 + 2 * mid)? < c {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == segs {
        return None;
    }
    let start = u16_at(s, starts + 2 * lo)?;
    if c < start {
        return None;
    }
    let delta = u16_at(s, deltas + 2 * lo)?;
    let range = usize::from(u16_at(s, ranges + 2 * lo)?);
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
        if end < c {
            lo = mid + 1;
        } else if start > c {
            hi = mid;
        } else {
            return u16::try_from(u32_at(s, g + 8)?.checked_add(c - start)?).ok();
        }
    }
    None
}

/// An affine map `[a, b, c, d, e, f]`: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
type Affine = [f32; 6];

const IDENTITY: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `outer` applied after `inner`.
fn compose(outer: &Affine, inner: &Affine) -> Affine {
    let [a, b, c, d, e, f] = *outer;
    let [ia, ib, ic, id, ie, if_] = *inner;
    [
        a * ia + c * ib,
        b * ia + d * ib,
        a * ic + c * id,
        b * ic + d * id,
        a * ie + c * if_ + e,
        b * ie + d * if_ + f,
    ]
}

/// What one glyph may still expand to.
struct Budget {
    components: u32,
    points: usize,
}

impl Font {
    /// Reads a TrueType font, checking the header, every required table's
    /// bounds and the fields this crate uses.
    pub fn parse(data: Vec<u8>) -> Result<Font, FontError> {
        let d = &data[..];
        if !matches!(u32_at(d, 0), Some(0x0001_0000 | 0x7472_7565)) {
            return Err(FontError::NotTrueType);
        }
        let head = &d[table(d, b"head")?];
        let bad = FontError::Malformed(*b"head");
        let upem = u16_at(head, 18).ok_or(bad)?;
        if u32_at(head, 12) != Some(0x5F0F_3CF5) || !(16..=16384).contains(&upem) {
            return Err(bad);
        }
        let long_loca = match i16_at(head, 50) {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(bad),
        };
        let hhea = &d[table(d, b"hhea")?];
        let bad = FontError::Malformed(*b"hhea");
        let (Some(ascender), Some(descender), Some(line_gap), Some(num_hmetrics)) =
            (i16_at(hhea, 4), i16_at(hhea, 6), i16_at(hhea, 8), u16_at(hhea, 34))
        else {
            return Err(bad);
        };
        if num_hmetrics == 0 {
            return Err(bad);
        }
        let maxp = &d[table(d, b"maxp")?];
        let num_glyphs = u16_at(maxp, 4).ok_or(FontError::Malformed(*b"maxp"))?;
        let hmtx = table(d, b"hmtx")?;
        if hmtx.len() < 4 * usize::from(num_hmetrics) {
            return Err(FontError::Malformed(*b"hmtx"));
        }
        let loca = table(d, b"loca")?;
        let entry = if long_loca { 4 } else { 2 };
        if loca.len() < entry * (usize::from(num_glyphs) + 1) {
            return Err(FontError::Malformed(*b"loca"));
        }
        let glyf = table(d, b"glyf")?;
        let cmap = pick_cmap(d, table(d, b"cmap")?)?;
        Ok(Font {
            upem,
            ascender,
            descender,
            line_gap,
            num_glyphs,
            num_hmetrics,
            long_loca,
            hmtx: hmtx.start,
            loca: loca.start,
            glyf,
            cmap,
            data,
        })
    }

    /// Font units per em (`head`).
    pub fn units_per_em(&self) -> u16 {
        self.upem
    }

    /// Typographic ascender in font units (`hhea`).
    pub fn ascender(&self) -> i16 {
        self.ascender
    }

    /// Typographic descender in font units, usually negative (`hhea`).
    pub fn descender(&self) -> i16 {
        self.descender
    }

    /// Extra space between lines in font units (`hhea`).
    pub fn line_gap(&self) -> i16 {
        self.line_gap
    }

    /// Number of glyphs (`maxp`); valid glyph ids are below it.
    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    /// The glyph for a character, or `None` if the font maps it to nothing,
    /// to glyph 0, or to an id past the glyph count.
    pub fn glyph_index(&self, c: char) -> Option<u16> {
        let g = match &self.cmap {
            Cmap::Format4 { sub, segs } => cmap4(self.data.get(sub.clone())?, *segs, c.into()),
            Cmap::Format12 { sub, groups } => {
                cmap12(self.data.get(sub.clone())?, *groups, c.into())
            }
        }?;
        (g != 0 && g < self.num_glyphs).then_some(g)
    }

    /// Horizontal advance in font units (`hmtx`); glyphs past
    /// `numberOfHMetrics` take the last advance.
    pub fn advance(&self, glyph: u16) -> u16 {
        let i = usize::from(glyph.min(self.num_hmetrics.saturating_sub(1)));
        u16_at(&self.data, self.hmtx + 4 * i).unwrap_or(0)
    }

    /// Where a glyph's `glyf` data lies in the font's bytes (empty for a
    /// glyph with no outline).
    fn glyph_range(&self, glyph: u16) -> Result<Range<usize>, FontError> {
        if glyph >= self.num_glyphs {
            return Err(FontError::NoGlyph(glyph));
        }
        let i = usize::from(glyph);
        let at = |k: usize| {
            if self.long_loca {
                u32_at(&self.data, self.loca + 4 * k).map(|v| v as usize)
            } else {
                u16_at(&self.data, self.loca + 2 * k).map(|v| 2 * usize::from(v))
            }
        };
        match (at(i), at(i + 1)) {
            (Some(a), Some(b)) if a <= b && b <= self.glyf.len() => {
                Ok(self.glyf.start + a..self.glyf.start + b)
            }
            _ => Err(FontError::Malformed(*b"loca")),
        }
    }

    /// A glyph's contours in font units, y up, replacing `out`'s contents.
    /// On error `out` is left empty.
    pub fn outline(&self, glyph: u16, out: &mut Outline) -> Result<(), FontError> {
        out.contours.clear();
        let mut budget = Budget { components: MAX_COMPONENTS, points: MAX_POINTS };
        let r = self.outline_into(glyph, &IDENTITY, 0, &mut budget, out);
        if r.is_err() {
            out.contours.clear();
        }
        r
    }

    fn outline_into(
        &self,
        glyph: u16,
        m: &Affine,
        depth: u32,
        budget: &mut Budget,
        out: &mut Outline,
    ) -> Result<(), FontError> {
        let range = self.glyph_range(glyph)?;
        let d = &self.data[range];
        if d.is_empty() {
            return Ok(());
        }
        let bad = FontError::Malformed(*b"glyf");
        let contours = i16_at(d, 0).ok_or(bad)?;
        if contours >= 0 {
            return simple(d, contours as usize, m, budget, out);
        }
        let mut p = 10;
        loop {
            let (Some(flags), Some(child)) = (u16_at(d, p), u16_at(d, p + 2)) else {
                return Err(bad);
            };
            p += 4;
            let words = flags & 0x0001 != 0;
            let arg = |k: usize| match words {
                true => i16_at(d, p + 2 * k).map(f32::from),
                false => d.get(p + k).map(|&b| f32::from(b as i8)),
            };
            let (Some(dx), Some(dy)) = (arg(0), arg(1)) else {
                return Err(bad);
            };
            p += if words { 4 } else { 2 };
            // Without ARGS_ARE_XY_VALUES the args name points to match;
            // those components are placed at offset 0.
            let xy = flags & 0x0002 != 0;
            let (dx, dy) = if xy { (dx, dy) } else { (0.0, 0.0) };
            let f2 = |at: usize| i16_at(d, at).map(|v| f32::from(v) / 16384.0).ok_or(bad);
            let (a, b, c, e) = if flags & 0x0008 != 0 {
                let s = f2(p)?;
                p += 2;
                (s, 0.0, 0.0, s)
            } else if flags & 0x0040 != 0 {
                p += 4;
                (f2(p - 4)?, 0.0, 0.0, f2(p - 2)?)
            } else if flags & 0x0080 != 0 {
                p += 8;
                (f2(p - 8)?, f2(p - 6)?, f2(p - 4)?, f2(p - 2)?)
            } else {
                (1.0, 0.0, 0.0, 1.0)
            };
            // SCALED_COMPONENT_OFFSET (and not UNSCALED_...): the offset is
            // transformed too.
            let (dx, dy) = if flags & 0x1800 == 0x0800 {
                (a * dx + c * dy, b * dx + e * dy)
            } else {
                (dx, dy)
            };
            if depth >= MAX_DEPTH || budget.components == 0 {
                return Err(FontError::TooComplex);
            }
            budget.components -= 1;
            let child_m = compose(m, &[a, b, c, e, dx, dy]);
            self.outline_into(child, &child_m, depth + 1, budget, out)?;
            if flags & 0x0020 == 0 {
                return Ok(());
            }
        }
    }

    /// Draws a glyph at `px_per_em` pixels per em into `out`, replacing its
    /// contents. Empty glyphs give `w = h = 0`; on error `out` is empty too.
    pub fn rasterize(&self, glyph: u16, px_per_em: f32, out: &mut Bitmap) -> Result<(), FontError> {
        (out.w, out.h, out.left, out.top) = (0, 0, 0, 0);
        out.data.clear();
        if !(px_per_em.is_finite() && px_per_em > 0.0) {
            return Err(FontError::BadSize);
        }
        let mut outline = Outline::default();
        self.outline(glyph, &mut outline)?;
        raster::render(&outline, px_per_em / f32::from(self.upem), out)
    }
}

/// Reads per-point coordinate deltas: `short` marks a one-byte delta whose
/// sign is `same` (set = positive); otherwise `same` repeats the previous
/// value and its absence means a two-byte signed delta.
fn deltas(d: &[u8], p: &mut usize, flags: &[u8], short: u8, same: u8) -> Option<Vec<i32>> {
    let mut out = Vec::with_capacity(flags.len());
    let mut v = 0i32;
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

/// A simple glyph: contour end points, instructions (skipped), flags, then
/// x and y deltas.
fn simple(
    d: &[u8],
    contours: usize,
    m: &Affine,
    budget: &mut Budget,
    out: &mut Outline,
) -> Result<(), FontError> {
    let bad = FontError::Malformed(*b"glyf");
    if contours == 0 {
        return Ok(());
    }
    let mut ends = Vec::with_capacity(contours);
    let mut n = 0usize;
    let mut p = 10;
    for _ in 0..contours {
        let end = usize::from(u16_at(d, p).ok_or(bad)?);
        if end < n {
            return Err(bad);
        }
        n = end + 1;
        ends.push(n);
        p += 2;
    }
    if n > budget.points {
        return Err(FontError::TooComplex);
    }
    budget.points -= n;
    p += 2 + usize::from(u16_at(d, p).ok_or(bad)?);
    let mut flags = Vec::with_capacity(n);
    while flags.len() < n {
        let f = *d.get(p).ok_or(bad)?;
        p += 1;
        let mut reps = 1;
        if f & 0x08 != 0 {
            reps += usize::from(*d.get(p).ok_or(bad)?);
            p += 1;
        }
        let reps = reps.min(n - flags.len());
        flags.extend(core::iter::repeat_n(f, reps));
    }
    let xs = deltas(d, &mut p, &flags, 0x02, 0x10).ok_or(bad)?;
    let ys = deltas(d, &mut p, &flags, 0x04, 0x20).ok_or(bad)?;
    let points: Vec<Point> = flags
        .iter()
        .zip(xs.iter().zip(&ys))
        .map(|(&f, (&x, &y))| {
            let (x, y) = (x as f32, y as f32);
            Point {
                x: m[0] * x + m[2] * y + m[4],
                y: m[1] * x + m[3] * y + m[5],
                on: f & 0x01 != 0,
            }
        })
        .collect();
    let mut start = 0;
    for end in ends {
        out.contours.push(points.get(start..end).ok_or(bad)?.to_vec());
        start = end;
    }
    Ok(())
}

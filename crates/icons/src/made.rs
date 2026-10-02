//! Made icons: the icon a `.app` file draws for itself, which Studio's AI writes for every app it
//! makes. One of the file's leading comments reads `// icon:` and then shapes on a 24 x 24 grid
//! (x right, y down, whole numbers), each a word and its numbers:
//!
//! - `line x y x y ..`: a stroke through 2 to 12 points; `loop x y ..`: a closed one, 3 to 12
//!   (as is a line that ends where it began);
//! - `fill x y ..`: a solid polygon of 3 to 12 points;
//! - `ring x y r`: a circle's stroke; `dot x y r`: a disc;
//! - `arc x y r from to`: a circle's stroke clockwise from clock angle `from` to `to` (degrees,
//!   0 at 12 o'clock; equal angles go all the way round).
//!
//! ```text
//! // Tic-tac-toe: tap a square; X goes first; three in a row wins.
//! // icon: line 2 7 10 15 line 10 7 2 15 ring 17 11 4
//! ```
//!
//! The line chooses only geometry; the OS draws it in the house style: every stroke the glyphs'
//! one weight ([`crate::STROKE`]), each side of a line or loop butt-ended with a disc of that
//! weight on its corners and where two lines' ends meet (so any angle joins round and none can
//! spike), in the theme's ink on the plate of the name's hue. Grid `(x, y)` lands at
//! `(20 + 40x, 980 - 40y)` in the glyphs' 1000-unit box and a radius `r` at `40r`, so the usual
//! 2..22 spans 100..900, as the system glyphs do.
//!
//! Spaces, commas, semicolons and tabs separate. A line reads whole or not at all: anything
//! else is an [`IconError`] (coded E0931 to E0937, at the byte it is about), and the tile shows
//! the sigil. Every limit is a bound a hostile line cannot pass: [`MAX_TEXT`] bytes, numbers of
//! one to three ASCII digits, [`MAX_SHAPES`] shapes, [`MAX_NUMBERS`] numbers, points on the grid
//! and circles wholly in it, so its ink stays within 25 box units of the box. Bytes throughout:
//! no UTF-8 decoding, no Unicode tables, nothing that formats, no heap but the outline's.

use core::f32::consts::PI;

use super::{H, Outline, add, bend, circle, pt, stroke};

/// The grid's side, in its units: points lie in 0..=24 (a model is asked for 2..22).
pub const GRID: u16 = 24;
/// The most bytes an icon's text takes (after `icon:`).
pub const MAX_TEXT: usize = 320;
/// The most shapes an icon holds.
pub const MAX_SHAPES: usize = 16;
/// The most numbers an icon holds, all its shapes together.
pub const MAX_NUMBERS: usize = 64;
/// The most points a line, loop or fill takes.
pub const MAX_POINTS: usize = 12;
/// How far into a file its icon is looked for: a line this cuts is never read.
pub const HEAD: usize = 4096;
/// The shapes' words, in their code's order.
const WORDS: [&[u8]; 6] = [b"line", b"loop", b"fill", b"ring", b"dot", b"arc"];
const LINE: u16 = 0;
const LOOP: u16 = 1;
const FILL: u16 = 2;
const RING: u16 = 3;
const DOT: u16 = 4;
const ARC: u16 = 5;

/// What is wrong with an icon's text: [`Why`], at byte `at` of it (for a shape whose count or
/// place is wrong, its word's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconError {
    pub at: u16,
    pub why: Why,
}

impl IconError {
    /// Its code: E0931 for [`Why::Text`] to E0937 for [`Why::Empty`].
    pub fn code(self) -> u16 {
        931 + self.why as u16
    }
}

/// Why an icon's text draws nothing of itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Past [`MAX_TEXT`] bytes.
    Text,
    /// A token neither a shape's word nor a number (`L`, `rect`, `-1`, `1.5`, non-ASCII).
    Word,
    /// A number before any shape, or of more than 3 digits.
    Number,
    /// Too few or too many numbers for its shape (an odd count of a line's).
    Count,
    /// A point off the grid, a circle not inside it, a radius too small (a ring's or arc's under
    /// 2, a dot's under 1), an angle past 360.
    Range,
    /// Past [`MAX_SHAPES`] shapes or [`MAX_NUMBERS`] numbers.
    Many,
    /// No shape at all.
    Empty,
}

/// A checked icon: each shape as its word's index and number count (`kind | n << 8`), then its
/// numbers. `Copy`, no heap: it rides in a home screen entry and a dock tile as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Made {
    hash: u32,
    len: u8,
    code: [u16; MAX_SHAPES + MAX_NUMBERS],
}

/// What follows `// icon:` on the first line that begins so (once trimmed of ASCII spaces) among
/// `file`'s leading lines, blank ones and `//` comments; none if code or a `/*` comes first, or
/// if that line does not end within the first [`HEAD`] bytes. `//icon:` and `// Icon:` are
/// ordinary comments.
pub fn header(file: &[u8]) -> Option<&[u8]> {
    let whole = file.len() <= HEAD;
    let mut rest = file.get(..HEAD).unwrap_or(file);
    loop {
        let (line, next) = match rest.iter().position(|&b| b == b'\n') {
            Some(i) => (rest.get(..i)?, rest.get(i + 1..)),
            None if whole => (rest, None),
            None => return None,
        };
        let line = line.trim_ascii();
        if let Some(icon) = line.strip_prefix(b"// icon:") {
            return Some(icon);
        }
        if !line.is_empty() && !line.starts_with(b"//") {
            return None;
        }
        rest = next?;
    }
}

/// The icon `file` draws, if its header has one that reads whole.
pub fn read(file: &[u8]) -> Option<Made> {
    Made::parse(header(file)?).ok()
}

/// Whether the line or loop `kind` through `v` is closed, and its points: one that ends where it
/// began is the loop through the rest.
fn shut(kind: u16, v: &[u16]) -> (bool, &[u16]) {
    let n = v.len().saturating_sub(2);
    match v.get(..2) == v.get(n..) {
        true => (true, v.get(..n).unwrap_or(v)),
        false => (kind == LOOP, v),
    }
}

fn sep(b: u8) -> bool {
    matches!(b, b' ' | b',' | b';' | b'\t')
}

impl Made {
    /// The icon `text` draws (what follows `// icon:`), checked whole.
    pub fn parse(text: &[u8]) -> Result<Made, IconError> {
        let mut m = Made { hash: 2_166_136_261, len: 0, code: [0; MAX_SHAPES + MAX_NUMBERS] };
        if text.len() > MAX_TEXT {
            return Err(IconError { at: 0, why: Why::Text });
        }
        // The open shape (where its word is in the code and in the text), shapes and numbers.
        let (mut i, mut open, mut shapes, mut numbers) = (0, None::<(usize, u16)>, 0, 0);
        loop {
            while text.get(i).copied().is_some_and(sep) {
                i += 1;
            }
            let at = i;
            while text.get(i).is_some_and(|&b| !sep(b)) {
                i += 1;
            }
            // `at` is at most MAX_TEXT, so it fits.
            let err = |why| IconError { at: at as u16, why };
            let Some(tok) = text.get(at..i).filter(|t| !t.is_empty()) else { break };
            let v = match WORDS.iter().position(|w| *w == tok) {
                Some(k) => {
                    if let Some(o) = open {
                        m.close(o)?;
                    }
                    shapes += 1;
                    if shapes > MAX_SHAPES {
                        return Err(err(Why::Many));
                    }
                    open = Some((usize::from(m.len), at as u16));
                    k as u16
                }
                None => {
                    if !tok.iter().all(u8::is_ascii_digit) {
                        return Err(err(Why::Word));
                    }
                    if open.is_none() || tok.len() > 3 {
                        return Err(err(Why::Number));
                    }
                    numbers += 1;
                    if numbers > MAX_NUMBERS {
                        return Err(err(Why::Many));
                    }
                    tok.iter().fold(0, |v, &b| v * 10 + u16::from(b - b'0'))
                }
            };
            // Shapes and numbers are counted, so the code has room.
            *m.code.get_mut(usize::from(m.len)).ok_or(err(Why::Many))? = v;
            m.len += 1;
        }
        m.close(open.ok_or(IconError { at: 0, why: Why::Empty })?)?;
        for &v in &m.code[..usize::from(m.len)] {
            for b in v.to_le_bytes() {
                m.hash = (m.hash ^ u32::from(b)).wrapping_mul(16_777_619);
            }
        }
        Ok(m)
    }

    /// Checks the last shape so far, whose word is at `h` in the code (`at` in the text): its
    /// count, and that it lies on the grid. Writes its count beside its word.
    fn close(&mut self, (h, at): (usize, u16)) -> Result<(), IconError> {
        let end = usize::from(self.len);
        let (kind, v) = (self.code[h], &self.code[h + 1..end]);
        let n = v.len();
        let bad = |why| Err(IconError { at, why });
        if kind < RING {
            let least = if kind == LOOP || kind == FILL { 6 } else { 4 };
            if n % 2 == 1 || n < least || n > 2 * MAX_POINTS {
                return bad(Why::Count);
            }
            if v.iter().any(|&c| c > GRID) {
                return bad(Why::Range);
            }
        } else {
            if n != if kind == ARC { 5 } else { 3 } {
                return bad(Why::Count);
            }
            // Wholly on the grid; a stroke's hole stays open (r - 1.125 > 0).
            let (x, y, r) = (v[0], v[1], v[2]);
            let on = |c: u16| c >= r && c + r <= GRID;
            let least = if kind == DOT { 1 } else { 2 };
            if r < least || !on(x) || !on(y) || v[3..].iter().any(|&a| a > 360) {
                return bad(Why::Range);
            }
        }
        self.code[h] = kind | ((n as u16) << 8);
        Ok(())
    }

    /// What names this icon in a glyph cache: FNV-1a of its code, so the same shapes written
    /// another way (commas for spaces) are one icon.
    pub fn hash(&self) -> u32 {
        self.hash
    }

    /// Each shape: its word's index and its numbers.
    fn shapes(&self) -> impl Iterator<Item = (u16, &[u16])> {
        let mut rest = &self.code[..usize::from(self.len)];
        core::iter::from_fn(move || {
            let (&head, more) = rest.split_first()?;
            let (v, next) = more.split_at(usize::from(head >> 8).min(more.len()));
            rest = next;
            Some((head & 0xff, v))
        })
    }

    /// Its contours in the glyphs' 1000 x 1000 box (y up), replacing `o`'s.
    pub fn outline(&self, o: &mut Outline) {
        o.clear();
        let u = 40.0f32;
        let at = |q: &[u16]| (20.0 + f32::from(q[0]) * u, 980.0 - f32::from(q[1]) * u);
        // The open lines' ends, in order: where two meet they join round, as corners do.
        let mut ends = Vec::new();
        for (_, v) in self.shapes().filter(|s| s.0 == LINE && !shut(s.0, s.1).0) {
            ends.extend([v.get(..2), v.get(v.len().saturating_sub(2)..)]);
        }
        let mut k = 0;
        for (kind, v) in self.shapes() {
            let pts = v.chunks_exact(2).map(at);
            match kind {
                LINE | LOOP => {
                    // Each side butt-ended (a repeated point none), a disc on each corner.
                    let (closed, v) = shut(kind, v);
                    let last = v.len() / 2;
                    let mut prev = if closed { v.rchunks_exact(2).next().map(at) } else { None };
                    for (j, q) in v.chunks_exact(2).enumerate() {
                        let b = at(q);
                        if let Some(a) = prev.filter(|&a| a != b) {
                            stroke(o, &[a, b]);
                        }
                        // An end met by another line's: its disc, once.
                        let end = !closed && (j == 0 || j + 1 == last);
                        let met = end && {
                            let (before, after) = ends.split_at(k.min(ends.len()));
                            k += 1;
                            let e = Some(q);
                            !before.contains(&e) && after.iter().skip(1).any(|x| *x == e)
                        };
                        if !end || met {
                            circle(o, b, H, true);
                        }
                        prev = Some(b);
                    }
                }
                FILL => add(o, pts.map(|(x, y)| pt(x, y)).collect(), true),
                _ => {
                    let (c, r) = (at(v), f32::from(v[2]) * u);
                    match kind {
                        RING => {
                            circle(o, c, r + H, true);
                            circle(o, c, r - H, false);
                        }
                        DOT => circle(o, c, r, true),
                        _ => {
                            let sweep = match (v[4] + 360 - v[3]) % 360 {
                                0 => 360,
                                s => s,
                            };
                            let a0 = (90.0 - f32::from(v[3])) * (PI / 180.0);
                            bend(o, c, r, a0, a0 - f32::from(sweep) * (PI / 180.0));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

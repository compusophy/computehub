//! The glyph atlas: one single-channel (A8) texture, packed on the CPU and
//! uploaded by the platform one dirty row band at a time.

/// A CPU-side single-channel (A8) texture with shelf packing, which the
/// platform mirrors as the `R8` texture [`crate::Kind::Glyph`] samples.
///
/// - [`Atlas::alloc`] places rects on horizontal shelves. Every rect keeps a
///   1 px empty gutter from its neighbors and from the atlas edges (column 0
///   and row 0 are always gutter), so no sampler ever reads a neighbor.
/// - [`Atlas::write`] copies coverage bytes in and widens the dirty row band;
///   [`Atlas::take_dirty`] hands the band to the uploader and resets it. A new
///   atlas starts all dirty, so the first upload initializes the texture.
/// - [`Atlas::clear`] drops every rect, zeroes the pixels and bumps
///   [`Atlas::generation`]: a cache of uv rects keyed by generation knows its
///   entries are stale.
///
/// ```
/// use gfx::Atlas;
///
/// let mut atlas = Atlas::new(64, 32);
/// assert_eq!(atlas.take_dirty(), Some((0, 32)));
/// let (x, y) = atlas.alloc(3, 2).unwrap();
/// assert_eq!((x, y), (1, 1));
/// assert!(atlas.write(x, y, 3, 2, &[9, 9, 9, 9, 9, 9]));
/// assert_eq!(atlas.take_dirty(), Some((1, 3)));
/// assert_eq!(atlas.pixels()[64 + 1], 9);
/// ```
#[derive(Clone, Debug)]
pub struct Atlas {
    w: u32,
    h: u32,
    pixels: Vec<u8>,
    shelves: Vec<Shelf>,
    bottom: u32, // the first row below the last shelf
    generation: u64,
    dirty: Option<(u32, u32)>,
}

/// A horizontal strip of slots. Heights and widths include the 1 px gutter
/// below and to the right of each rect.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: u32,
    h: u32,
    x: u32, // the next free column
}

impl Atlas {
    /// An empty, all-zero `w` x `h` atlas, marked all dirty.
    ///
    /// # Panics
    ///
    /// If `w * h` bytes do not fit in `usize`.
    pub fn new(w: u32, h: u32) -> Atlas {
        let n = (w as usize).checked_mul(h as usize).expect("atlas size overflows usize");
        let mut atlas = Atlas {
            w,
            h,
            pixels: vec![0; n],
            shelves: Vec::new(),
            bottom: 1,
            generation: 0,
            dirty: None,
        };
        atlas.mark(0, h);
        atlas
    }

    /// Reserves a `w` x `h` rect and returns its top-left corner, or `None`
    /// when it does not fit. It takes the lowest shelf with room, unless that
    /// shelf is at least twice as tall as the rect needs (gutter included)
    /// and a new shelf still fits below the last one. A zero-sized request
    /// returns `(0, 0)` (always gutter) and reserves nothing.
    pub fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w == 0 || h == 0 {
            return Some((0, 0));
        }
        let (sw, sh) = (w.checked_add(1)?, h.checked_add(1)?);
        let width = self.w;
        let best = self
            .shelves
            .iter()
            .enumerate()
            .filter(|(_, s)| s.h >= sh && width - s.x >= sw)
            .min_by_key(|(_, s)| s.h)
            .map(|(i, _)| i);
        let room = self.h.saturating_sub(self.bottom) >= sh && width.saturating_sub(1) >= sw;
        let i = match best {
            Some(i) if !room || self.shelves[i].h / 2 < sh => i,
            _ if room => {
                let y = self.bottom;
                self.bottom += sh;
                self.shelves.push(Shelf { y, h: sh, x: 1 });
                self.shelves.len() - 1
            }
            _ => return None,
        };
        let s = &mut self.shelves[i];
        let at = (s.x, s.y);
        s.x += sw;
        Some(at)
    }

    /// Copies `data`, `w` x `h` bytes row-major, to the rect at `(x, y)` and
    /// widens the dirty band to its rows. Writes nothing and returns `false`
    /// if the rect leaves the atlas or `data.len() != w * h`.
    pub fn write(&mut self, x: u32, y: u32, w: u32, h: u32, data: &[u8]) -> bool {
        let fits = |at: u32, len: u32, max: u32| at.checked_add(len).is_some_and(|end| end <= max);
        let len = (w as usize).checked_mul(h as usize);
        if !fits(x, w, self.w) || !fits(y, h, self.h) || len != Some(data.len()) {
            return false;
        }
        if w == 0 || h == 0 {
            return true;
        }
        let stride = self.w as usize;
        for (row, src) in data.chunks_exact(w as usize).enumerate() {
            let at = (y as usize + row) * stride + x as usize;
            self.pixels[at..at + src.len()].copy_from_slice(src);
        }
        self.mark(y, y + h);
        true
    }

    /// Drops every rect, zeroes the pixels, bumps [`Atlas::generation`] and
    /// marks the whole atlas dirty.
    pub fn clear(&mut self) {
        self.pixels.fill(0);
        self.shelves.clear();
        self.bottom = 1;
        self.generation = self.generation.wrapping_add(1);
        self.mark(0, self.h);
    }

    /// How many times [`Atlas::clear`] has run: uv rects from an older
    /// generation are stale.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Width and height in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// The coverage bytes, row-major, `w` bytes per row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The rows changed since the last call, as `[y0, y1)` across the full
    /// width (upload `pixels()[y0 * w..y1 * w]`), then clears it; `None` when
    /// nothing changed.
    pub fn take_dirty(&mut self) -> Option<(u32, u32)> {
        self.dirty.take()
    }

    fn mark(&mut self, y0: u32, y1: u32) {
        if self.w == 0 || y0 >= y1 {
            return;
        }
        self.dirty = Some(match self.dirty {
            Some((a, b)) => (a.min(y0), b.max(y1)),
            None => (y0, y1),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::Atlas;

    #[test]
    fn new_is_zero_and_all_dirty() {
        let mut a = Atlas::new(8, 4);
        assert_eq!(a.size(), (8, 4));
        assert_eq!(a.pixels(), &[0; 32]);
        assert_eq!(a.generation(), 0);
        assert_eq!(a.take_dirty(), Some((0, 4)));
        assert_eq!(a.take_dirty(), None);
        assert_eq!(Atlas::new(0, 4).take_dirty(), None);
        assert_eq!(Atlas::new(4, 0).take_dirty(), None);
    }

    #[test]
    fn shelves_keep_a_one_pixel_gutter() {
        let mut a = Atlas::new(16, 16);
        assert_eq!(a.alloc(4, 3), Some((1, 1)));
        assert_eq!(a.alloc(4, 3), Some((6, 1)));
        assert_eq!(a.alloc(4, 2), Some((11, 1))); // shorter: same shelf
        assert_eq!(a.alloc(4, 3), Some((1, 5))); // shelf 1 is full: x 16 used
        assert_eq!(a.alloc(2, 1), Some((1, 9))); // 2x shorter: its own shelf
        assert_eq!(a.alloc(3, 1), Some((4, 9)));
        assert_eq!(a.alloc(14, 5), None); // no gutter row below it
        assert_eq!(a.alloc(14, 4), Some((1, 11))); // gutter to both edges
        assert_eq!(a.alloc(1, 1), Some((8, 9)));
        // No row left for a new shelf, so a tall shelf takes a short rect.
        assert_eq!(a.alloc(6, 1), Some((6, 5)));
        assert_eq!(a.alloc(15, 1), None);
        assert_eq!(a.alloc(20, 20), None);
        assert_eq!(a.alloc(u32::MAX, 1), None);
        assert_eq!(a.alloc(1, u32::MAX), None);
        assert_eq!(a.alloc(0, 7), Some((0, 0)));
        assert_eq!(a.alloc(7, 0), Some((0, 0)));
        assert_eq!(Atlas::new(0, 0).alloc(1, 1), None);
        assert_eq!(Atlas::new(3, 3).alloc(1, 1), Some((1, 1)));
        assert_eq!(Atlas::new(3, 3).alloc(2, 1), None);
    }

    #[test]
    fn packing_never_overlaps() {
        let mut a = Atlas::new(64, 64);
        let mut taken = vec![0u8; 64 * 64];
        let mut n = 0;
        for i in 0..200u32 {
            let (w, h) = (1 + i * 7 % 9, 1 + i * 5 % 11);
            let Some((x, y)) = a.alloc(w, h) else {
                continue;
            };
            n += 1;
            assert!(x >= 1 && y >= 1 && x + w < 64 && y + h < 64);
            // No earlier rect touches this one or its gutter ring.
            for yy in y - 1..=y + h {
                for xx in x - 1..=x + w {
                    assert_eq!(taken[(yy * 64 + xx) as usize], 0, "overlap at {xx},{yy}");
                }
            }
            for yy in y..y + h {
                taken[(yy * 64 + x) as usize..(yy * 64 + x + w) as usize].fill(1);
            }
        }
        assert!(n > 40, "only {n} rects fit");
    }

    #[test]
    fn write_copies_rows_and_widens_the_dirty_band() {
        let mut a = Atlas::new(4, 6);
        a.take_dirty();
        assert!(a.write(1, 2, 2, 2, &[1, 2, 3, 4]));
        assert!(a.write(0, 4, 1, 1, &[5]));
        #[rustfmt::skip]
        let want = [
            0, 0, 0, 0,
            0, 0, 0, 0,
            0, 1, 2, 0,
            0, 3, 4, 0,
            5, 0, 0, 0,
            0, 0, 0, 0,
        ];
        assert_eq!(a.pixels(), &want);
        assert_eq!(a.take_dirty(), Some((2, 5)));
        assert!(a.write(3, 5, 1, 1, &[6]));
        assert!(a.write(0, 0, 0, 3, &[]));
        assert_eq!(a.take_dirty(), Some((5, 6)));
    }

    #[test]
    fn bad_writes_do_nothing() {
        let mut a = Atlas::new(4, 4);
        a.take_dirty();
        assert!(!a.write(3, 0, 2, 1, &[1, 1]));
        assert!(!a.write(0, 4, 1, 1, &[1]));
        assert!(!a.write(0, 0, 2, 2, &[1, 1, 1]));
        assert!(!a.write(0, 0, 2, 2, &[1, 1, 1, 1, 1]));
        assert!(!a.write(u32::MAX, 0, 2, 1, &[1, 1]));
        assert!(!a.write(0, 1, 1, u32::MAX, &[1]));
        assert!(!a.write(5, 0, 0, 0, &[]));
        assert_eq!(a.pixels(), &[0; 16]);
        assert_eq!(a.take_dirty(), None);
    }

    #[test]
    fn clear_drops_rects_and_bumps_the_generation() {
        let mut a = Atlas::new(8, 8);
        let (x, y) = a.alloc(2, 2).unwrap();
        a.write(x, y, 2, 2, &[7; 4]);
        assert_eq!(a.alloc(5, 5), None);
        a.take_dirty();
        a.clear();
        assert_eq!(a.generation(), 1);
        assert_eq!(a.pixels(), &[0; 64]);
        assert_eq!(a.take_dirty(), Some((0, 8)));
        assert_eq!(a.alloc(5, 5), Some((1, 1)));
        a.clear();
        assert_eq!(a.generation(), 2);
    }
}

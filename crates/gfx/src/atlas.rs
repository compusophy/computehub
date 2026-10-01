/// A CPU-side one-byte-per-pixel texture with shelf packing, which the
/// platform mirrors as the `R8` texture glyphs sample.
///
/// Every rect keeps a 1 px empty gutter from its neighbors and the edges
/// (column 0 and row 0 are always gutter), so no sampler reads a neighbor.
/// Writes widen a dirty row band that [`Atlas::take_dirty`] hands to the
/// uploader; a new or cleared atlas is all dirty.
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

/// A horizontal strip of slots; `h` and widths include the gutter.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: u32,
    h: u32,
    x: u32, // the next free column
}

impl Atlas {
    /// An all-zero `w` x `h` atlas. Panics if `w * h` overflows `usize`.
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

    /// Reserves a `w` x `h` rect and returns its corner, or `None` if it does
    /// not fit. It takes the lowest shelf with room, unless that shelf is at
    /// least twice as tall as needed and a new shelf still fits below. A
    /// zero-sized request returns `(0, 0)` and reserves nothing.
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

    /// Copies `w` x `h` row-major bytes to the rect at `(x, y)`; writes
    /// nothing and returns `false` if it leaves the atlas or the length is off.
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

    /// Drops every rect, zeroes the pixels and bumps [`Atlas::generation`].
    pub fn clear(&mut self) {
        self.pixels.fill(0);
        self.shelves.clear();
        self.bottom = 1;
        self.generation = self.generation.wrapping_add(1);
        self.mark(0, self.h);
    }

    /// How many times the atlas was cleared: older uv rects are stale.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// The pixels, row-major, `w` bytes per row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The rows `[y0, y1)` changed since the last call, if any.
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
    fn new_and_clear_are_zero_and_all_dirty() {
        let mut a = Atlas::new(8, 4);
        assert_eq!((a.size(), a.pixels(), a.generation()), ((8, 4), &[0; 32][..], 0));
        assert_eq!((a.take_dirty(), a.take_dirty()), (Some((0, 4)), None));
        assert_eq!((Atlas::new(0, 4).take_dirty(), Atlas::new(4, 0).take_dirty()), (None, None));
        let mut a = Atlas::new(8, 8);
        let (x, y) = a.alloc(2, 2).unwrap();
        a.write(x, y, 2, 2, &[7; 4]);
        assert_eq!(a.alloc(5, 5), None);
        a.take_dirty();
        a.clear();
        assert_eq!((a.take_dirty(), a.generation(), a.pixels()), (Some((0, 8)), 1, &[0; 64][..]));
        assert_eq!(a.alloc(5, 5), Some((1, 1)));
        a.clear();
        assert_eq!(a.generation(), 2);
    }

    #[test]
    fn shelves_keep_a_one_pixel_gutter() {
        let mut a = Atlas::new(16, 16);
        let reqs = [(4, 3), (4, 3), (4, 2), (4, 3), (2, 1), (3, 1), (14, 5), (14, 4), (1, 1)];
        // A shorter rect shares a shelf; a full shelf or one 2x too tall does
        // not; no gutter row below (14, 5); with no row left for a new
        // shelf, a tall shelf takes a short rect.
        let got: Vec<_> = reqs
            .into_iter()
            .chain([(6, 1), (15, 1), (20, 20)])
            .map(|(w, h)| a.alloc(w, h))
            .collect();
        let s = Some;
        let want =
            [s((1, 1)), s((6, 1)), s((11, 1)), s((1, 5)), s((1, 9)), s((4, 9)), None, s((1, 11))];
        assert_eq!(got, [&want[..], &[s((8, 9)), s((6, 5)), None, None]].concat());
        let edge = [(u32::MAX, 1), (1, u32::MAX), (0, 7), (7, 0)].map(|(w, h)| a.alloc(w, h));
        assert_eq!(edge, [None, None, s((0, 0)), s((0, 0))]);
        assert_eq!(Atlas::new(0, 0).alloc(1, 1), None);
        assert_eq!((Atlas::new(3, 3).alloc(1, 1), Atlas::new(3, 3).alloc(2, 1)), (s((1, 1)), None));
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
    fn writes_copy_rows_and_widen_the_dirty_band() {
        let mut a = Atlas::new(4, 6);
        a.take_dirty();
        assert!(a.write(1, 2, 2, 2, &[1, 2, 3, 4]) && a.write(0, 4, 1, 1, &[5]));
        let want = [[0; 4], [0; 4], [0, 1, 2, 0], [0, 3, 4, 0], [5, 0, 0, 0], [0; 4]];
        assert_eq!((a.take_dirty(), a.pixels()), (Some((2, 5)), &want.concat()[..]));
        assert!(a.write(3, 5, 1, 1, &[6]) && a.write(0, 0, 0, 3, &[]));
        assert_eq!(a.take_dirty(), Some((5, 6)));
        // Bad writes do nothing.
        let mut a = Atlas::new(4, 4);
        a.take_dirty();
        let bad = [(3, 0, 2, 1, 2), (0, 4, 1, 1, 1), (0, 0, 2, 2, 3), (0, 0, 2, 2, 5)];
        let more = [(u32::MAX, 0, 2, 1, 2), (0, 1, 1, u32::MAX, 1), (5, 0, 0, 0, 0)];
        for (x, y, w, h, n) in bad.into_iter().chain(more) {
            assert!(!a.write(x, y, w, h, &vec![1; n]), "{x} {y} {w} {h} {n}");
        }
        assert_eq!((a.take_dirty(), a.pixels()), (None, &[0; 16][..]));
    }
}

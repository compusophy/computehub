//! Text on the glyph atlas for compusophyOS (re-exported by `ui`): three UI
//! font slots plus fallbacks, measured in logical pixels.
//!
//! - Sans is the boot font. Until [`TextSystem::set_font`] fills them,
//!   SansBold is Sans and Mono keeps JetBrains Mono's grid but draws nothing.
//! - Lookup: the style's face, the other built-in family, the fallbacks,
//!   else `.notdef`. No kerning; a tab is four spaces, controls are empty.
//! - Glyphs are rasterized at `size * dpr`, cached, and placed on device
//!   pixels (the atlas samples 1:1). A full atlas is cleared and
//!   [`TextSystem::take_atlas_reset`] asks for the frame again.
//! - Vector shapes ([`TextSystem::draw_vector`]: icons) share the cache and atlas.
//! - On a recording list ([`DrawList::recording`]) text is noted, not drawn, and shapes are
//!   skipped: recording never touches the atlas.
//! - [`Editor`] is the text buffer behind `ui`'s code editor.

#![forbid(unsafe_code)]

mod edit;
pub use edit::Editor;
pub use font::Point;

use font::{Bitmap, Font};
use gfx::{Atlas, DrawList, RectF, Rgba};

/// Side of the glyph atlas, in pixels.
pub const ATLAS_SIZE: u32 = 1024;
/// Most fallback fonts [`TextSystem::add_fallback`] takes.
pub const MAX_FALLBACKS: usize = 8;
/// Glyphs above this many device pixels per em (and vector shapes wider) are not drawn.
const MAX_PX: f32 = 1000.0;
const BUILTIN: usize = 3;
/// The cache's face index for vector shapes (seeded ones one less): past any font slot.
const VECTOR: u64 = 0xffff;
/// JetBrains Mono's metrics and advance, for an empty Mono.
const MONO_METRICS: [i32; 4] = [1000, 1020, -300, 0];
const MONO_ADVANCE: i32 = 600;

/// The built-in font slots: Inter Regular, Inter SemiBold, JetBrains Mono.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontId {
    Sans,
    SansBold,
    Mono,
}

/// How a run of text looks: face, em size in logical pixels, straight color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub font: FontId,
    pub size: f32,
    pub color: Rgba,
}

impl TextStyle {
    pub const fn new(font: FontId, size: f32, color: Rgba) -> TextStyle {
        TextStyle { font, size, color }
    }

    pub const fn with_color(self, color: Rgba) -> TextStyle {
        TextStyle::new(self.font, self.size, color)
    }
}

/// A cached glyph: atlas rect, offset from pen and baseline (device px).
#[derive(Clone, Copy, Debug)]
struct Slot {
    uv: RectF,
    left: i32,
    top: i32,
}

/// The faces (slots in [`FontId`] order, then fallbacks), a glyph cache
/// sorted by face, glyph and px per em in 64ths (vectors: face `0xffff`, id
/// and side in px; seeded ones face `0xfffe`, seed and side), and its atlas.
pub struct TextSystem {
    faces: Vec<Option<Font>>,
    cache: Vec<(u64, Option<Slot>)>,
    atlas: Atlas,
    bitmap: Bitmap,
    dpr: f32,
    reset: bool,
}

fn usable(size: f32) -> bool {
    size.is_finite() && size > 0.0
}

/// Whether a line may break after `c`: a `+`, `/` or `-` between letters or
/// digits, but not between two digits (`Alt+Shift`, `a/b`; not `1-4`, `--x`).
fn breaks_after(prev: Option<char>, c: char, next: Option<char>) -> bool {
    let (Some(p), Some(n)) = (prev, next) else { return false };
    let digits = p.is_ascii_digit() && n.is_ascii_digit();
    matches!(c, '+' | '/' | '-') && p.is_alphanumeric() && n.is_alphanumeric() && !digits
}

fn scaled(units: i32, size: f32, upem: i32) -> f32 {
    units as f32 * size / upem as f32
}

fn parse(name: &str, bytes: Vec<u8>) -> Result<Font, String> {
    Font::parse(bytes).map_err(|e| format!("{name}: {e}"))
}

impl TextSystem {
    /// Parses the boot face, Sans; an error starts with `sans`.
    pub fn new(sans: Vec<u8>) -> Result<TextSystem, String> {
        let faces = vec![Some(parse("sans", sans)?), None, None];
        let (cache, atlas) = (Vec::new(), Atlas::new(ATLAS_SIZE, ATLAS_SIZE));
        Ok(TextSystem { faces, cache, atlas, bitmap: Bitmap::default(), dpr: 1.0, reset: false })
    }

    /// Fills slot `id`, dropping its cached glyphs; an error names the slot.
    pub fn set_font(&mut self, id: FontId, bytes: Vec<u8>) -> Result<(), String> {
        let i = id as usize;
        self.faces[i] = Some(parse(["sans", "sans bold", "mono"][i], bytes)?);
        self.cache.retain(|e| (e.0 >> 48) as usize != i);
        Ok(())
    }

    pub fn has_font(&self, id: FontId) -> bool {
        self.faces[id as usize].is_some()
    }

    /// Adds a font to try after the others, up to [`MAX_FALLBACKS`].
    pub fn add_fallback(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        if self.fallback_count() >= MAX_FALLBACKS {
            return Err(format!("at most {MAX_FALLBACKS} fallback fonts"));
        }
        self.faces.push(Some(parse("fallback font", bytes)?));
        Ok(())
    }

    pub fn fallback_count(&self) -> usize {
        self.faces.len() - BUILTIN
    }

    pub fn atlas_mut(&mut self) -> &mut Atlas {
        &mut self.atlas
    }

    /// Sets the device pixel ratio, clamped to `0.25..=8`; others are ignored.
    pub fn set_dpr(&mut self, dpr: f32) {
        if usable(dpr) {
            self.dpr = dpr.clamp(0.25, 8.0);
        }
    }

    pub fn dpr(&self) -> f32 {
        self.dpr
    }

    /// `v` rounded to the nearest device pixel.
    pub fn snap(&self, v: f32) -> f32 {
        (v * self.dpr).round() / self.dpr
    }

    /// Whether the atlas was cleared since the last call: redraw the frame then.
    pub fn take_atlas_reset(&mut self) -> bool {
        std::mem::take(&mut self.reset)
    }

    /// The width of `text` in logical pixels, as [`TextSystem::draw_text`] advances.
    pub fn measure(&mut self, text: &str, style: TextStyle) -> f32 {
        if !usable(style.size) {
            return 0.0;
        }
        text.chars().map(|c| self.lookup(style.font, c, style.size).1).sum()
    }

    /// Ascender - descender + line gap, on device pixels (at least one).
    pub fn line_height(&self, style: TextStyle) -> f32 {
        let [_, asc, desc, gap] = self.vmetrics(style.font);
        self.vertical(style, asc - desc + gap)
    }

    /// From the top of a line to its baseline, on device pixels.
    pub fn ascent(&self, style: TextStyle) -> f32 {
        self.vertical(style, self.vmetrics(style.font)[1])
    }

    /// The depth below the baseline, on device pixels.
    pub fn descent(&self, style: TextStyle) -> f32 {
        self.vertical(style, (-self.vmetrics(style.font)[2]).max(0))
    }

    fn vertical(&self, style: TextStyle, units: i32) -> f32 {
        if !usable(style.size) {
            return 0.0;
        }
        let upem = self.vmetrics(style.font)[0];
        let px = (scaled(units, style.size, upem) * self.dpr).round();
        px.max(1.0) / self.dpr
    }

    /// The slot and face `font` draws with: Sans for an empty SansBold.
    fn face(&self, font: FontId) -> Option<(usize, &Font)> {
        let bold = font == FontId::SansBold && self.faces[1].is_none();
        let i = if bold { 0 } else { font as usize };
        Some((i, self.faces[i].as_ref()?))
    }

    fn vmetrics(&self, font: FontId) -> [i32; 4] {
        self.face(font).map_or(MONO_METRICS, |(_, f)| f.metrics())
    }

    /// Draws `text` on one line from pen `x` on `baseline` (a recording list notes it); returns
    /// its advance.
    pub fn draw_text(
        &mut self,
        list: &mut DrawList,
        x: f32,
        baseline: f32,
        text: &str,
        style: TextStyle,
    ) -> f32 {
        if !usable(style.size) {
            return 0.0;
        }
        if list.sem().is_some() {
            let adv = self.measure(text, style);
            list.note_text(x, baseline, style.size, adv, text);
            return adv;
        }
        let px = (style.size * self.dpr * 4.0).round() / 4.0; // cached to a quarter px
        let mut adv = 0.0;
        for c in text.chars() {
            let (g, a) = self.lookup(style.font, c, style.size);
            if let (Some((face, gid)), false) = (g, c.is_whitespace() || c.is_control()) {
                self.put(list, (face, gid, px), (x + adv, baseline), style.color);
            }
            adv += a;
        }
        adv
    }

    /// Lines no wider than `width`: split at `\n` (and `\r\n`), else at the last
    /// space run or joiner that fits, else between chars; spaces at a break
    /// are dropped and every paragraph gives at least one line.
    pub fn wrap<'t>(&mut self, text: &'t str, style: TextStyle, width: f32) -> Vec<&'t str> {
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let para = para.strip_suffix('\r').unwrap_or(para);
            let (mut start, mut w, mut brk, mut prev) = (0, 0.0, None, None);
            let cut = |a: usize, b: usize| para.get(a..b).unwrap_or_default();
            for (i, c) in para.char_indices() {
                let adv = self.lookup(style.font, c, style.size).1;
                if c == ' ' {
                    // A run of spaces breaks before its first space.
                    let end = brk.filter(|_| prev == Some(' ')).map_or(i, |b: (usize, _)| b.0);
                    brk = Some((end, i + 1));
                    (prev, w) = (Some(c), w + adv);
                    continue;
                }
                if w + adv > width && i > start {
                    if let Some((end, next)) = brk.take() {
                        // Leading spaces leave nothing before their break.
                        if end > start {
                            lines.push(cut(start, end));
                        }
                        start = next;
                        w = self.measure(cut(start, i), style);
                    }
                    if w + adv > width && i > start {
                        lines.push(cut(start, i));
                        (start, w) = (i, 0.0);
                    }
                }
                w += adv;
                let after = i + c.len_utf8();
                if breaks_after(prev, c, cut(after, para.len()).chars().next()) {
                    brk = Some((after, after));
                }
                prev = Some(c);
            }
            lines.push(cut(start, para.len()).trim_end_matches(' '));
        }
        lines
    }

    /// `s`, or its longest prefix that fits in `room` with an ellipsis.
    pub fn ellipsize(&mut self, s: &str, style: TextStyle, room: f32) -> String {
        if self.measure(s, style) <= room {
            return s.to_string();
        }
        let budget = room - self.measure("\u{2026}", style);
        if budget < 0.0 {
            return String::new();
        }
        let (mut used, mut end) = (0.0, 0);
        for (i, c) in s.char_indices() {
            used += self.lookup(style.font, c, style.size).1;
            if used > budget {
                break;
            }
            end = i + c.len_utf8();
        }
        s.get(..end).unwrap_or_default().trim_end().to_string() + "\u{2026}"
    }

    /// A terminal cell's width: Mono's advance of `0`, on device pixels.
    pub fn cell_width(&mut self, size: f32) -> f32 {
        if !usable(size) {
            return 0.0;
        }
        let adv = self.lookup(FontId::Mono, '0', size).1;
        (adv * self.dpr).round().max(1.0) / self.dpr
    }

    /// Draws `c` centered in a cell; a wider fallback shrinks, and box drawing
    /// and blocks (U+2500-259F) grow to cover cell and Mono row, so they join.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_cell_char(
        &mut self,
        list: &mut DrawList,
        x: f32,
        baseline: f32,
        cell_w: f32,
        c: char,
        size: f32,
        color: Rgba,
    ) {
        let mono = FontId::Mono as usize;
        let skip = c == ' ' || c.is_control() || !usable(size) || !usable(cell_w);
        if !skip && list.sem().is_some() {
            return list.note_text(x, baseline, size, cell_w, c.encode_utf8(&mut [0; 4]));
        }
        if skip || self.faces[mono].is_none() {
            return;
        }
        let style = TextStyle::new(FontId::Mono, size, color);
        let Some((face, gid)) = self.find(FontId::Mono, c) else {
            let h = (size * 0.7).max(2.0);
            let r = RectF::new(x + cell_w * 0.15, baseline - h, cell_w * 0.7, h);
            list.border(r, 0.0, 1.0, color);
            return;
        };
        let (adv, d) = (self.advance(face, gid, size), self.dpr);
        let px = size * d;
        let (scale, lift, exact) = if face == mono && ('\u{2500}'..='\u{259f}').contains(&c) {
            // Cover the cell (width) and the row (ascent and descent).
            let [upem, ascender, descender, _] = self.vmetrics(FontId::Mono);
            let em = |u: i32| u.abs() as f32 * px / upem as f32;
            let asc = (self.ascent(style) * d).round();
            let below = (self.line_height(style) * d).round() - asc;
            let wide = (cell_w * d).round() / (adv * d);
            let fits = [wide, asc / em(ascender), below / em(descender)];
            (fits.into_iter().filter(|v| v.is_finite()).fold(1.0, f32::max), 0.0, true)
        } else if face != mono && adv > cell_w {
            // Shrink about the center line, halfway between ascender and descender.
            let s = cell_w / adv;
            let center = (self.ascent(style) - self.descent(style)) / 2.0;
            (s, center * (1.0 - s), false)
        } else {
            (1.0, 0.0, false)
        };
        let px = if exact { px * scale } else { (px * scale * 4.0).round() / 4.0 };
        let pen = x + (cell_w - adv * scale) / 2.0;
        self.put(list, (face, gid, px), (pen, baseline - lift), color);
    }

    fn find(&self, font: FontId, c: char) -> Option<(usize, u16)> {
        let (own, _) = self.face(font)?;
        let other = if font == FontId::Mono { 0 } else { 2 };
        let mut order = [own, other].into_iter().chain(BUILTIN..self.faces.len());
        order.find_map(|i| self.faces[i].as_ref()?.glyph_index(c).map(|g| (i, g)))
    }

    fn advance(&self, face: usize, gid: u16, size: f32) -> f32 {
        let adv = |f: &Font| scaled(f.advance(gid).into(), size, f.metrics()[0]);
        self.faces[face].as_ref().map_or(0.0, adv)
    }

    /// Face and glyph (none for an empty Mono) and advance of `c` at `size`.
    fn lookup(&self, font: FontId, c: char, size: f32) -> (Option<(usize, u16)>, f32) {
        let key = if c == '\t' { ' ' } else { c };
        let g = self.find(font, key).or(self.face(font).map(|(own, _)| (own, 0)));
        let empty = scaled(MONO_ADVANCE, size, MONO_METRICS[0]);
        let adv = g.map_or(empty, |(f, gid)| self.advance(f, gid, size));
        let adv = match c {
            '\t' => 4.0 * adv,
            c if c.is_control() => 0.0,
            _ => adv,
        };
        (g, adv)
    }

    /// Draws a vector shape in `color`: `outline` fills a 1000 x 1000 box (x right, y up, origin
    /// bottom-left), fitted to the square of side `min(r.w, r.h)` centered in `r`. It is
    /// rasterized at that side in whole device pixels (at most 1000) and cached under `id`, so
    /// one id must always name one shape.
    pub fn draw_vector(
        &mut self,
        list: &mut DrawList,
        r: RectF,
        id: u16,
        outline: fn(&mut Vec<Vec<Point>>),
        color: Rgba,
    ) {
        let key = (VECTOR << 48) | (u64::from(id) << 32);
        self.shape(list, r, key, &|o| outline(o), color);
    }

    /// Draws the shape `outline` makes from `seed` (as [`TextSystem::draw_vector`] draws one),
    /// cached under the seed: one function must make every seeded shape.
    pub fn draw_seeded(
        &mut self,
        list: &mut DrawList,
        r: RectF,
        seed: u32,
        outline: fn(u32, &mut Vec<Vec<Point>>),
        color: Rgba,
    ) {
        let key = ((VECTOR - 1) << 48) | (u64::from(seed) << 16);
        self.shape(list, r, key, &|o| outline(seed, o), color);
    }

    /// A vector shape cached under `key` and its side in device pixels.
    fn shape(
        &mut self,
        list: &mut DrawList,
        r: RectF,
        key: u64,
        outline: &dyn Fn(&mut Vec<Vec<Point>>),
        color: Rgba,
    ) {
        let (d, side) = (self.dpr, (r.w.min(r.h) * self.dpr).round());
        if !(1.0..=MAX_PX).contains(&side) || list.sem().is_some() {
            return;
        }
        let slot = self.cached(key | side as u64, |t| {
            let mut o = Vec::new();
            outline(&mut o);
            font::render_outline(&o, side / 1000.0, &mut t.bitmap).ok()?;
            t.pack()
        });
        let Some(s) = slot else { return };
        // The box's bottom-left corner, on a device pixel, is the outline's origin.
        let x = ((r.x + r.w / 2.0) * d - side / 2.0).round() + s.left as f32;
        let y = ((r.y + r.h / 2.0) * d + side / 2.0).round() + s.top as f32;
        list.glyph(RectF::new(x / d, y / d, s.uv.w / d, s.uv.h / d), s.uv, color);
    }

    /// Pushes glyph `g` (face, glyph, px per em) with pen and baseline at `at`.
    fn put(&mut self, list: &mut DrawList, g: (usize, u16, f32), at: (f32, f32), color: Rgba) {
        let Some(s) = self.glyph(g.0, g.1, g.2) else { return };
        let d = self.dpr;
        let gx = (at.0 * d).round() + s.left as f32;
        let gy = (at.1 * d).round() + s.top as f32;
        let dst = RectF::new(gx / d, gy / d, s.uv.w / d, s.uv.h / d);
        list.glyph(dst, s.uv, color);
    }

    /// The cached glyph, rasterized into the atlas on a miss.
    fn glyph(&mut self, face: usize, gid: u16, px: f32) -> Option<Slot> {
        if !(px > 0.0 && px <= MAX_PX) {
            return None;
        }
        let px64 = u64::from((px * 64.0).round() as u32);
        let key = ((face as u64) << 48) | (u64::from(gid) << 32) | px64;
        self.cached(key, |t| {
            t.faces[face].as_ref()?.rasterize(gid, px, &mut t.bitmap).ok()?;
            t.pack()
        })
    }

    /// The slot cached under `key`, made on a miss.
    fn cached(&mut self, key: u64, make: impl FnOnce(&mut Self) -> Option<Slot>) -> Option<Slot> {
        let at = |cache: &[(u64, Option<Slot>)]| cache.partition_point(|e| e.0 < key);
        if let Some(e) = self.cache.get(at(&self.cache)).filter(|e| e.0 == key) {
            return e.1;
        }
        // Making may clear the cache, so find the spot afterwards.
        let slot = make(self);
        self.cache.insert(at(&self.cache), (key, slot));
        slot
    }

    /// Copies the bitmap into the atlas, clearing a full one.
    fn pack(&mut self) -> Option<Slot> {
        let Bitmap { w, h, left, top, .. } = self.bitmap;
        if w == 0 || h == 0 {
            return None;
        }
        let (x, y) = match self.atlas.alloc(w, h) {
            Some(at) => at,
            // It would fit an empty atlas: start over.
            None if w + 2 <= ATLAS_SIZE && h + 2 <= ATLAS_SIZE => {
                self.atlas.clear();
                self.cache.clear();
                self.reset = true;
                self.atlas.alloc(w, h)?
            }
            None => return None,
        };
        self.atlas.write(x, y, w, h, &self.bitmap.data);
        let uv = RectF::new(x as f32, y as f32, w as f32, h as f32);
        Some(Slot { uv, left, top })
    }
}

#[cfg(test)]
mod tests;

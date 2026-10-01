//! Text on the glyph atlas: three UI font slots plus fallbacks, measured in
//! logical pixels and drawn as device-pixel-exact glyphs. Split from `ui`,
//! which re-exports it; pure Rust, no browser.

#![forbid(unsafe_code)]

use font::{Bitmap, Font};
use gfx::{Atlas, DrawList, RectF, Rgba};

/// Side of the glyph atlas, in pixels.
pub const ATLAS_SIZE: u32 = 1024;
/// Most fallback fonts [`TextSystem::add_fallback`] takes.
pub const MAX_FALLBACKS: usize = 8;
/// Glyphs above this many device pixels per em are not drawn.
const MAX_PX: f32 = 1000.0;
/// The built-in slots, in [`FontId`] order; fallbacks follow.
const BUILTIN: usize = 3;
/// JetBrains Mono's units per em, ascender, descender, line gap and advance:
/// an empty Mono measures with these, so the grid holds when it arrives.
const MONO_METRICS: [i32; 5] = [1000, 1020, -300, 0, 600];

/// One of the three built-in font slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontId {
    /// Inter Regular: body text.
    Sans,
    /// Inter SemiBold: headings and emphasis.
    SansBold,
    /// JetBrains Mono: code and the terminal.
    Mono,
}

/// How a run of text looks: face, size in logical pixels, color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// The face.
    pub font: FontId,
    /// The em size in logical pixels.
    pub size: f32,
    /// Straight sRGB color.
    pub color: Rgba,
}

impl TextStyle {
    /// A style from its parts.
    pub const fn new(font: FontId, size: f32, color: Rgba) -> TextStyle {
        TextStyle { font, size, color }
    }

    /// The same style in `color`.
    pub const fn with_color(self, color: Rgba) -> TextStyle {
        TextStyle::new(self.font, self.size, color)
    }
}

/// A cached glyph: its atlas rect and its offset from the pen and baseline,
/// all in device pixels.
#[derive(Clone, Copy, Debug)]
struct Slot {
    uv: RectF,
    left: i32,
    top: i32,
}

/// Fonts, a glyph cache and the atlas the cache lives in.
///
/// - **Slots:** [`TextSystem::new`] takes only Sans (the boot font);
///   [`TextSystem::set_font`] fills SansBold and Mono when they arrive.
///   Until then SansBold draws and measures as Sans, and Mono measures as
///   JetBrains Mono (0.6 em a char, the same cell grid) but draws nothing.
/// - **Lookup:** from the style's face, else the other built-in family (Mono
///   for Sans, Sans for Mono), else each fallback in the order added. A char
///   found nowhere draws the face's `.notdef` box, or in a cell a hollow box.
/// - **Metrics:** the fonts' own advances, scaled, with no kerning. A tab is
///   four spaces wide; other control chars have no width and no ink.
/// - **Pixels:** glyphs are rasterized at `size * dpr` device pixels per em
///   rounded to a quarter pixel, cached by (face, glyph, that size), and
///   placed with pen and baseline rounded to device pixels
///   (`round(v * dpr) / dpr`), so the atlas is sampled 1:1.
/// - **Atlas full:** it is cleared, the cache dropped, and
///   [`TextSystem::take_atlas_reset`] turns true once: glyphs already pushed
///   this frame show stale pixels, so the caller redraws the frame.
pub struct TextSystem {
    /// The slots in [`FontId`] order (Sans always filled), then fallbacks.
    faces: Vec<Option<Font>>,
    /// Glyphs by [`cache_key`], sorted.
    cache: Vec<(u64, Option<Slot>)>,
    atlas: Atlas,
    bitmap: Bitmap,
    dpr: f32,
    reset: bool,
}

impl std::fmt::Debug for TextSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextSystem({} fallbacks)", self.fallback_count())
    }
}

fn usable(size: f32) -> bool {
    size.is_finite() && size > 0.0
}

/// Whether a line may break after `c` (a joiner, kept on the line) between
/// `prev` and `next`: a `+`, `/` or `-` between letters or digits, but not
/// between two digits (`Alt+Shift`, `a/b`, `well-known`; not `1-4`, `--x`).
fn breaks_after(prev: Option<char>, c: char, next: Option<char>) -> bool {
    let (Some(p), Some(n)) = (prev, next) else {
        return false;
    };
    let digits = p.is_ascii_digit() && n.is_ascii_digit();
    matches!(c, '+' | '/' | '-') && p.is_alphanumeric() && n.is_alphanumeric() && !digits
}

/// `px` rounded to a quarter pixel: the sizes text glyphs are cached at.
fn quarter(px: f32) -> f32 {
    (px * 4.0).round() / 4.0
}

/// `units` of a font with `upem` units per em, in pixels at `size`.
fn scaled(units: i32, size: f32, upem: i32) -> f32 {
    units as f32 * size / upem as f32
}

/// The cache key of a glyph: face, glyph, and device px per em in 64ths.
fn cache_key(face: usize, gid: u16, px: f32) -> u64 {
    ((face as u64) << 48) | (u64::from(gid) << 32) | u64::from((px * 64.0).round() as u32)
}

fn parse(name: &str, bytes: Vec<u8>) -> Result<Font, String> {
    Font::parse(bytes).map_err(|e| format!("{name}: {e}"))
}

impl TextSystem {
    /// Parses the boot face, Sans; SansBold and Mono start empty (see
    /// [`TextSystem::set_font`]). The error starts with `sans`.
    pub fn new(sans: Vec<u8>) -> Result<TextSystem, String> {
        Ok(TextSystem {
            faces: vec![Some(parse("sans", sans)?), None, None],
            cache: Vec::new(),
            atlas: Atlas::new(ATLAS_SIZE, ATLAS_SIZE),
            bitmap: Bitmap::default(),
            dpr: 1.0,
            reset: false,
        })
    }

    /// Fills (or replaces) the slot `id` and drops its cached glyphs; redraw
    /// afterwards. On a bad font the slot is unchanged and the error starts
    /// with the slot's name (`sans`, `sans bold` or `mono`).
    pub fn set_font(&mut self, id: FontId, bytes: Vec<u8>) -> Result<(), String> {
        let i = id as usize;
        self.faces[i] = Some(parse(["sans", "sans bold", "mono"][i], bytes)?);
        self.cache.retain(|e| (e.0 >> 48) as usize != i);
        Ok(())
    }

    /// Whether the slot `id` holds a font.
    pub fn has_font(&self, id: FontId) -> bool {
        self.faces[id as usize].is_some()
    }

    /// Adds a font to try, after the built-in faces and earlier fallbacks,
    /// for chars they lack. Fails on a bad font or past [`MAX_FALLBACKS`].
    pub fn add_fallback(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        if self.fallback_count() >= MAX_FALLBACKS {
            return Err(format!("at most {MAX_FALLBACKS} fallback fonts"));
        }
        self.faces.push(Some(parse("fallback font", bytes)?));
        Ok(())
    }

    /// How many fallback fonts were added.
    pub fn fallback_count(&self) -> usize {
        self.faces.len() - BUILTIN
    }

    /// The atlas, for the platform to upload its dirty rows
    /// ([`Atlas::take_dirty`]).
    pub fn atlas_mut(&mut self) -> &mut Atlas {
        &mut self.atlas
    }

    /// Sets the device pixel ratio. Values that are not finite and positive
    /// are ignored; others are clamped to `0.25..=8`.
    pub fn set_dpr(&mut self, dpr: f32) {
        if usable(dpr) {
            self.dpr = dpr.clamp(0.25, 8.0);
        }
    }

    /// The device pixel ratio (1 until [`TextSystem::set_dpr`]).
    pub fn dpr(&self) -> f32 {
        self.dpr
    }

    /// `v` rounded to the nearest device pixel.
    pub fn snap(&self, v: f32) -> f32 {
        (v * self.dpr).round() / self.dpr
    }

    /// Whether the atlas was cleared since the last call. When true, redraw
    /// the frame: glyphs pushed before the clear show stale pixels.
    pub fn take_atlas_reset(&mut self) -> bool {
        std::mem::take(&mut self.reset)
    }

    /// The width of `text` in logical pixels: the sum of its advances.
    /// Equal to what [`TextSystem::draw_text`] returns for the same text.
    pub fn measure(&mut self, text: &str, style: TextStyle) -> f32 {
        if !usable(style.size) {
            return 0.0;
        }
        text.chars().map(|c| self.lookup(style.font, c, style.size).1).sum()
    }

    /// Distance between baselines: the face's ascender minus descender plus
    /// line gap, scaled, rounded to device pixels (at least one). 0 for an
    /// unusable size.
    pub fn line_height(&self, style: TextStyle) -> f32 {
        let [_, asc, desc, gap] = self.vmetrics(style.font);
        self.vertical(style, asc - desc + gap)
    }

    /// The face's ascender: logical pixels from the top of a line to its
    /// baseline, rounded to device pixels.
    pub fn ascent(&self, style: TextStyle) -> f32 {
        self.vertical(style, self.vmetrics(style.font)[1])
    }

    /// The face's descender as a positive depth below the baseline, rounded
    /// to device pixels.
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

    /// The slot `font` draws with and its face: its own, Sans for an empty
    /// SansBold, `None` for an empty Mono.
    fn face(&self, font: FontId) -> Option<(usize, &Font)> {
        let i = match (font, &self.faces[1]) {
            (FontId::SansBold, None) => 0,
            _ => font as usize,
        };
        Some((i, self.faces[i].as_ref()?))
    }

    /// Units per em, ascender, descender and line gap of `font`'s face, or
    /// JetBrains Mono's for an empty Mono.
    fn vmetrics(&self, font: FontId) -> [i32; 4] {
        let [upem, asc, desc, gap, _] = MONO_METRICS;
        self.face(font).map_or([upem, asc, desc, gap], |(_, f)| {
            let (asc, desc, gap) = (f.ascender(), f.descender(), f.line_gap());
            [f.units_per_em().into(), asc.into(), desc.into(), gap.into()]
        })
    }

    /// Draws `text` on one line with its pen starting at `x` and its baseline
    /// at `baseline`, and returns its advance (as [`TextSystem::measure`]).
    /// Nothing is drawn for an unusable size.
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
        let px = quarter(style.size * self.dpr);
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

    /// Breaks `text` into lines no wider than `width`: at each `\n` (a
    /// trailing `\r` is dropped), else at the last break that fits (before
    /// a run of spaces, or after a `+`, `/` or `-` between letters or
    /// digits, as in `Alt+Shift+` / `Enter`), else (a word longer than the
    /// line) between chars. Every line holds at least one char, spaces at a
    /// break are dropped, and an empty paragraph is an empty line, so the
    /// result is never empty.
    pub fn wrap<'t>(&mut self, text: &'t str, style: TextStyle, width: f32) -> Vec<&'t str> {
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let para = para.strip_suffix('\r').unwrap_or(para);
            let (mut start, mut w, mut brk, mut prev) = (0, 0.0, None, None);
            for (i, c) in para.char_indices() {
                let adv = self.lookup(style.font, c, style.size).1;
                if c == ' ' {
                    // A run of spaces breaks before its first space.
                    brk = Some(match brk {
                        Some((end, _)) if prev == Some(' ') => (end, i + 1),
                        _ => (i, i + 1),
                    });
                    (prev, w) = (Some(c), w + adv);
                    continue;
                }
                if w + adv > width && i > start {
                    if let Some((end, next)) = brk.take() {
                        // Leading spaces leave nothing before their break.
                        if end > start {
                            lines.push(&para[start..end]);
                        }
                        start = next;
                        w = self.measure(&para[start..i], style);
                    }
                    if w + adv > width && i > start {
                        lines.push(&para[start..i]);
                        (start, w) = (i, 0.0);
                    }
                }
                w += adv;
                let after = i + c.len_utf8();
                if breaks_after(prev, c, para[after..].chars().next()) {
                    brk = Some((after, after));
                }
                prev = Some(c);
            }
            lines.push(para[start..].trim_end_matches(' '));
        }
        lines
    }

    /// The width of the widest piece of `text` that [`TextSystem::wrap`]
    /// breaks only between chars: the narrowest `width` at which it wraps
    /// without splitting a word. 0 for an unusable size.
    pub fn min_width(&mut self, text: &str, style: TextStyle) -> f32 {
        if !usable(style.size) {
            return 0.0;
        }
        let (mut widest, mut w, mut prev) = (0.0f32, 0.0, None);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != ' ' && c != '\n' {
                w += self.lookup(style.font, c, style.size).1;
            }
            if c == ' ' || c == '\n' || breaks_after(prev, c, chars.peek().copied()) {
                (widest, w) = (widest.max(w), 0.0);
            }
            prev = Some(c);
        }
        widest.max(w)
    }

    /// `s` if it fits in `room`, else its longest prefix that fits with an
    /// ellipsis after it (spaces before the ellipsis dropped); empty if not
    /// even the ellipsis fits.
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
        s[..end].trim_end().to_string() + "\u{2026}"
    }

    /// The width of a terminal cell at `size`: the Mono advance of `0`
    /// (0.6 * `size` while Mono is empty, as JetBrains Mono's), rounded to
    /// device pixels (at least one) so a grid of cells stays on the pixel
    /// grid. 0 for an unusable size.
    pub fn cell_width(&mut self, size: f32) -> f32 {
        if !usable(size) {
            return 0.0;
        }
        let adv = self.lookup(FontId::Mono, '0', size).1;
        (adv * self.dpr).round().max(1.0) / self.dpr
    }

    /// Draws `c` centered (by advance) in the terminal cell at `x`, `cell_w`
    /// wide, with its baseline at `baseline`. Rows are expected to be Mono's
    /// [`TextSystem::line_height`] tall, the baseline [`TextSystem::ascent`]
    /// below the row's top. A fallback glyph wider than the cell shrinks to
    /// fit about the cell's center line; box drawing and blocks (U+2500-259F)
    /// grow just enough to cover the cell and row, so they join without
    /// seams. A char in no font draws a hollow box; spaces, control chars
    /// and anything while Mono is empty draw nothing.
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
        let empty = self.faces[mono].is_none();
        if empty || !usable(size) || !usable(cell_w) || c == ' ' || c.is_control() {
            return;
        }
        let style = TextStyle::new(FontId::Mono, size, color);
        let Some((face, gid)) = self.find(FontId::Mono, c) else {
            let h = (size * 0.7).max(2.0);
            let r = RectF::new(x + cell_w * 0.15, baseline - h, cell_w * 0.7, h);
            list.border(r, 0.0, 1.0, color);
            return;
        };
        let adv = self.advance(face, gid, size);
        let d = self.dpr;
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
            // Shrink about the cell's center line, halfway between the Mono
            // ascender and descender.
            let s = cell_w / adv;
            let center = (self.ascent(style) - self.descent(style)) / 2.0;
            (s, center * (1.0 - s), false)
        } else {
            (1.0, 0.0, false)
        };
        let px = px * scale;
        let px = if exact { px } else { quarter(px) };
        let pen = x + (cell_w - adv * scale) / 2.0;
        self.put(list, (face, gid, px), (pen, baseline - lift), color);
    }

    /// The face and glyph for `c`, or `None` when no font has it (or `font`
    /// is an empty Mono).
    fn find(&self, font: FontId, c: char) -> Option<(usize, u16)> {
        let (own, _) = self.face(font)?;
        let other = if font == FontId::Mono { 0 } else { 2 };
        [own, other]
            .into_iter()
            .chain(BUILTIN..self.faces.len())
            .find_map(|i| self.faces[i].as_ref()?.glyph_index(c).map(|g| (i, g)))
    }

    fn advance(&self, face: usize, gid: u16, size: f32) -> f32 {
        let adv = |f: &Font| scaled(f.advance(gid).into(), size, f.units_per_em().into());
        self.faces[face].as_ref().map_or(0.0, adv)
    }

    /// Face and glyph (none for an empty Mono) and advance for `c` at
    /// `size`: `.notdef` of the style's face when no font has it, four
    /// spaces for a tab, zero width for other control chars.
    fn lookup(&self, font: FontId, c: char, size: f32) -> (Option<(usize, u16)>, f32) {
        let key = if c == '\t' { ' ' } else { c };
        let notdef = self.face(font).map(|(own, _)| (own, 0));
        let g = self.find(font, key).or(notdef);
        let [upem, .., mono] = MONO_METRICS;
        let empty = scaled(mono, size, upem);
        let adv = g.map_or(empty, |(f, gid)| self.advance(f, gid, size));
        let adv = match c {
            '\t' => 4.0 * adv,
            c if c.is_control() => 0.0,
            _ => adv,
        };
        (g, adv)
    }

    /// Pushes glyph `g` = (face, glyph, device px per em) with its pen and
    /// baseline at `at`, snapped to device pixels.
    fn put(&mut self, list: &mut DrawList, g: (usize, u16, f32), at: (f32, f32), color: Rgba) {
        let Some(s) = self.glyph(g.0, g.1, g.2) else {
            return;
        };
        let d = self.dpr;
        let gx = (at.0 * d).round() + s.left as f32;
        let gy = (at.1 * d).round() + s.top as f32;
        let dst = RectF::new(gx / d, gy / d, s.uv.w / d, s.uv.h / d);
        list.glyph(dst, s.uv, color);
    }

    /// The cached glyph at `px` device pixels per em, rasterizing it into the
    /// atlas on a miss. `None` for an empty or undrawable glyph.
    fn glyph(&mut self, face: usize, gid: u16, px: f32) -> Option<Slot> {
        if !(px > 0.0 && px <= MAX_PX) {
            return None;
        }
        let key = cache_key(face, gid, px);
        let at = |cache: &[(u64, Option<Slot>)]| cache.partition_point(|e| e.0 < key);
        if let Some(e) = self.cache.get(at(&self.cache)).filter(|e| e.0 == key) {
            return e.1;
        }
        // Rasterizing may clear the cache, so find the spot afterwards.
        let slot = self.rasterize(face, gid, px);
        self.cache.insert(at(&self.cache), (key, slot));
        slot
    }

    fn rasterize(&mut self, face: usize, gid: u16, px: f32) -> Option<Slot> {
        let f = self.faces[face].as_ref()?;
        f.rasterize(gid, px, &mut self.bitmap).ok()?;
        let b = &self.bitmap;
        let (w, h, left, top) = (b.w, b.h, b.left, b.top);
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

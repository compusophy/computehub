//! Draw lists for compusophyOS: every fill, border, shadow, icon, glyph, gradient, glow and grain
//! pass is one [`Instance`] of a single quad, and a frame is one flat byte buffer that the
//! `platform` crate draws with one instanced WebGL2 call through [`VERTEX_SHADER`] and
//! [`FRAGMENT_SHADER`], which live here beside the byte contract they read.
//!
//! - Logical (CSS) pixels as `f32`, origin top-left, y down. Colors are straight sRGB; the shader
//!   writes premultiplied (`ONE, ONE_MINUS_SRC_ALPHA`).
//! - Draw order is push order. A push is skipped when its rect is empty, a value is not finite,
//!   every alpha is 0, or its reach misses the clip (the rect grown by 1 px of antialiasing, by
//!   blur + 1 for a shadow, by nothing for a glyph or glow). Radii clamp to `[0, min(w, h) / 2]`.
//! - Each instance keeps the clip current at its push ([`NO_CLIP`] if none).
//! - Glyphs sample the [`Atlas`] 1:1: a glyph's rect is its uv size over the device pixel ratio,
//!   on a device pixel.
//! - A [`DrawList::recording`] list draws nothing to see: it keeps what the screen says instead
//!   ([`Sem`]), the text drawn where it shows and the marks widgets leave, for the AI that reads
//!   the screen.

#![forbid(unsafe_code)]

mod atlas;
mod shader;

pub use atlas::Atlas;
pub use shader::{FRAGMENT_SHADER, VERTEX_SHADER};

/// Bytes per encoded [`Instance`] (the attribute stride), little-endian: `rect` 4 x f32 at 0,
/// `radius kind p0 p1` 4 x f32 at 16, `color` 4 x u8 at 32, `clip` 4 x f32 at 36, `uv` 4 x f32 at
/// 52, `color2` 4 x u8 at 68.
pub const INSTANCE_BYTES: usize = 72;

/// The clip (x, y, w, h) of an unclipped instance: beyond any canvas.
pub const NO_CLIP: [f32; 4] = [-1e9, -1e9, 2e9, 2e9];

/// A straight (not premultiplied) sRGB color with alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

const CLEAR: Rgba = Rgba(0, 0, 0, 0);

impl Rgba {
    /// An opaque color from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Rgba {
        Rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
    }

    /// The same color with alpha `a`.
    pub const fn with_alpha(self, a: u8) -> Rgba {
        Rgba(self.0, self.1, self.2, a)
    }
}

/// A rectangle in logical pixels: top-left corner, width and height.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    /// A rect from its top-left corner and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF { x, y, w, h }
    }

    /// A rect from integer coordinates (such as a `wm` layout rect).
    pub fn from_i32(x: i32, y: i32, w: i32, h: i32) -> RectF {
        RectF::new(x as f32, y as f32, w as f32, h as f32)
    }

    /// Every side moved in by `d` (out when negative); the size stays `>= 0`.
    pub fn inset(self, d: f32) -> RectF {
        RectF::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }

    /// Whether the point lies inside, half-open: `x <= px < x + w`, same in y.
    pub fn contains(self, px: f32, py: f32) -> bool {
        (self.x..self.x + self.w).contains(&px) && (self.y..self.y + self.h).contains(&py)
    }

    /// The overlap; zero-sized (at where it would start) when there is none.
    pub fn intersect(self, o: RectF) -> RectF {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (right, bottom) = ((self.x + self.w).min(o.x + o.w), (self.y + self.h).min(o.y + o.h));
        RectF::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    }
}

/// What an [`Instance`] draws; encoded as its discriminant in `f32`. Unused parameters are 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A filled rounded rect.
    Fill = 0,
    /// A ring `p0` wide just inside the rounded rect's edge.
    Border = 1,
    /// A soft shadow reaching `p0` px (the blur) outside the rounded rect.
    Shadow = 2,
    /// [`Icon`] `p0` stroked `p1` wide in the rect's centered square.
    Icon = 3,
    /// The [`Atlas`] pixels at `uv` over the rect; coverage scales the alpha.
    Glyph = 4,
    /// A linear gradient from `color` to `color2` along angle `p0` (0 runs
    /// top to bottom, `PI / 2` left to right), spanning the rect as in CSS.
    Gradient = 5,
    /// An elliptical light: alpha times `(1 - d^2)^2`, `d` 1 on the inscribed ellipse.
    Glow = 6,
    /// Film grain fixed to the screen: per pixel, white or black noise from
    /// seed `p0`, up to the color's alpha.
    Grain = 7,
    /// A segment `p0` wide with round ends across the rect, its bounding box grown by `p0 / 2`:
    /// from the top-left to the bottom-right, or with `p1` 1 from the bottom-left to the top-right.
    Line = 8,
}

/// Vector icons in the rect's centered square, of side `s = min(w, h)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Two strokes through the center, `0.7 s` long.
    Plus = 0,
    /// Plus turned 45 degrees.
    Cross = 1,
    /// Plus's horizontal stroke.
    Minus = 2,
    /// A filled circle `0.4 s` across.
    Dot = 3,
    /// A square outline `0.56 s` on a side.
    Square = 4,
    /// Four dots `0.24 s` across at plus or minus `0.22 s`.
    Grid = 5,
}

/// One quad of the instanced draw; [`INSTANCE_BYTES`] once encoded. Rects are x, y, w, h: `rect`
/// in logical pixels, `clip` (the instance is visible only inside it) and, for a glyph, `uv` in
/// [`Atlas`] pixels. `radius` is within `[0, min(w, h) / 2]`, `kind` the [`Kind`] as `f32` with
/// its parameters `p0` and `p1`, `color` straight and `color2` a gradient's end color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub rect: [f32; 4],
    pub radius: f32,
    pub kind: f32,
    pub p0: f32,
    pub p1: f32,
    pub color: Rgba,
    pub clip: [f32; 4],
    pub uv: [f32; 4],
    pub color2: Rgba,
}

/// What a recording [`DrawList`] noted: each line of text where it shows, and the marks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sem {
    pub runs: Vec<Run>,
    pub marks: Vec<Mark>,
}

/// A line of text and the part of its em box (from an em above its baseline, 1.25 em tall) the
/// clip shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub rect: RectF,
    pub text: String,
}

/// What widget `id` says of itself, past its text: its role, state flags and value (`ui::sem`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub id: u32,
    pub role: u8,
    pub flags: u8,
    pub value: String,
}

/// An ordered list of instances and a stack of clip rects; recording, what the screen says.
#[derive(Default, Clone, Debug)]
pub struct DrawList {
    items: Vec<Instance>,
    clips: Vec<RectF>,
    sem: Option<Box<Sem>>,
}

impl DrawList {
    pub fn new() -> DrawList {
        DrawList::default()
    }

    /// A list that records what is drawn as [`Sem`] (text and marks) and draws no text.
    pub fn recording() -> DrawList {
        DrawList { sem: Some(Box::default()), ..DrawList::default() }
    }

    /// What it recorded, if it records.
    pub fn sem(&self) -> Option<&Sem> {
        self.sem.as_deref()
    }

    /// What it recorded, leaving it a list that does not record.
    pub fn take_sem(&mut self) -> Option<Sem> {
        self.sem.take().map(|s| *s)
    }

    /// Notes `text`, `adv` wide, drawn at pen `x` on `baseline` at `size` px per em, if it
    /// records and the text shows in the clip. On the line of the last run noted (its baseline
    /// and size), less than an em after it, it joins that run: with a space unless under 0.15 em
    /// apart, so text drawn a char or a cell at a time reads as lines (but not a label as one
    /// with the next widget's).
    pub fn note_text(&mut self, x: f32, baseline: f32, size: f32, adv: f32, text: &str) {
        let clip = self.clip();
        let Some(sem) = self.sem.as_mut().filter(|_| !text.trim().is_empty()) else { return };
        let rect = RectF::new(x, baseline - size, adv, 1.25 * size).intersect(clip);
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        if let Some(last) = sem.runs.last_mut().filter(|l| l.rect.y == rect.y && l.rect.h == rect.h)
        {
            let gap = x - (last.rect.x + last.rect.w);
            if (-1.0..size).contains(&gap) {
                if gap >= 0.15 * size && !last.text.ends_with(' ') {
                    last.text.push(' ');
                }
                last.text.push_str(text);
                last.rect.w = rect.x + rect.w - last.rect.x;
                return;
            }
        }
        sem.runs.push(Run { rect, text: text.to_string() });
    }

    /// Notes widget `id`'s role, `flags` and `value`, if it records.
    pub fn mark(&mut self, id: u32, role: u8, flags: u8, value: &str) {
        if let Some(sem) = &mut self.sem {
            sem.marks.push(Mark { id, role, flags, value: value.to_string() });
        }
    }

    /// Removes every instance and clip (and what it recorded), keeping the allocations.
    pub fn clear(&mut self) {
        self.items.clear();
        self.clips.clear();
        if let Some(sem) = &mut self.sem {
            **sem = Sem::default();
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// A filled rounded rect.
    pub fn fill(&mut self, r: RectF, radius: f32, color: Rgba) {
        self.push(r, radius, Kind::Fill, [0.0; 2], [color, CLEAR], [0.0; 4]);
    }

    /// A ring `width` px wide just inside the rounded rect's edge; none if `width <= 0`.
    pub fn border(&mut self, r: RectF, radius: f32, width: f32, color: Rgba) {
        if width > 0.0 {
            self.push(r, radius, Kind::Border, [width, 0.0], [color, CLEAR], [0.0; 4]);
        }
    }

    /// A soft shadow of the rounded rect, fading out `blur` px beyond it.
    pub fn shadow(&mut self, r: RectF, radius: f32, blur: f32, color: Rgba) {
        self.push(r, radius, Kind::Shadow, [blur, 0.0], [color, CLEAR], [0.0; 4]);
    }

    /// [`DrawList::shadow`] of the rect moved down by a finite `dy`.
    pub fn shadow_offset(&mut self, r: RectF, radius: f32, blur: f32, dy: f32, color: Rgba) {
        if dy.is_finite() {
            self.shadow(RectF::new(r.x, r.y + dy, r.w, r.h), radius, blur, color);
        }
    }

    /// An icon in the rect's centered square, stroked `stroke` px wide.
    pub fn icon(&mut self, r: RectF, icon: Icon, stroke: f32, color: Rgba) {
        self.push(r, 0.0, Kind::Icon, [icon as u8 as f32, stroke], [color, CLEAR], [0.0; 4]);
    }

    /// A rounded rect in a linear gradient along `angle` (see [`Kind::Gradient`]).
    pub fn gradient(&mut self, r: RectF, radius: f32, from: Rgba, to: Rgba, angle: f32) {
        self.push(r, radius, Kind::Gradient, [angle, 0.0], [from, to], [0.0; 4]);
    }

    /// A soft elliptical light filling the rect (see [`Kind::Glow`]).
    pub fn glow(&mut self, r: RectF, color: Rgba) {
        self.push(r, 0.0, Kind::Glow, [0.0; 2], [color, CLEAR], [0.0; 4]);
    }

    /// Film grain of at most `strength / 255` alpha; each seed is a pattern.
    pub fn grain(&mut self, r: RectF, strength: u8, seed: f32) {
        let color = Rgba(255, 255, 255, strength);
        self.push(r, 0.0, Kind::Grain, [seed, 0.0], [color, CLEAR], [0.0; 4]);
    }

    /// A segment from `a` to `b`, `width` px wide with round ends (none if `width <= 0`; a dot
    /// if `a` is `b`): [`Kind::Line`] over its bounding box grown by half the width.
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: Rgba) {
        let (x, y, h) = (a.0.min(b.0), a.1.min(b.1), width / 2.0);
        let r = RectF::new(x - h, y - h, (a.0 - b.0).abs() + width, (a.1 - b.1).abs() + width);
        self.segment(r, width, (b.0 - a.0) * (b.1 - a.1) < 0.0, color);
    }

    /// [`Kind::Line`] in `r`, `width` px wide, rising (`up`) or falling left to right.
    pub fn segment(&mut self, r: RectF, width: f32, up: bool, color: Rgba) {
        if width > 0.0 {
            let p = [width, f32::from(u8::from(up))];
            self.push(r, 0.0, Kind::Line, p, [color, CLEAR], [0.0; 4]);
        }
    }

    /// The [`Atlas`] pixel rect `uv` (skipped if empty) drawn into `dst`.
    pub fn glyph(&mut self, dst: RectF, uv: RectF, color: Rgba) {
        if uv.w > 0.0 && uv.h > 0.0 {
            self.push(dst, 0.0, Kind::Glyph, [0.0; 2], [color, CLEAR], [uv.x, uv.y, uv.w, uv.h]);
        }
    }

    /// Pushes `r` intersected with the current clip; a non-finite `r` hides all.
    pub fn push_clip(&mut self, r: RectF) {
        let cur = self.clip();
        let finite = [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite());
        let next = if finite { cur.intersect(r) } else { RectF::new(cur.x, cur.y, 0.0, 0.0) };
        self.clips.push(next);
    }

    /// Restores the clip before the last push; a no-op when none is pushed.
    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    /// The current clip: the last pushed one, or [`NO_CLIP`].
    pub fn clip(&self) -> RectF {
        let [x, y, w, h] = NO_CLIP;
        self.clips.last().copied().unwrap_or(RectF::new(x, y, w, h))
    }

    /// The instances in draw order.
    pub fn instances(&self) -> &[Instance] {
        &self.items
    }

    /// Clears `out`, then appends [`INSTANCE_BYTES`] per instance, in order.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.items.len() * INSTANCE_BYTES);
        let le = |o: &mut Vec<u8>, v: &[f32]| o.extend(v.iter().flat_map(|v| v.to_le_bytes()));
        for it in &self.items {
            le(out, [it.rect, [it.radius, it.kind, it.p0, it.p1]].as_flattened());
            out.extend_from_slice(&[it.color.0, it.color.1, it.color.2, it.color.3]);
            le(out, [it.clip, it.uv].as_flattened());
            out.extend_from_slice(&[it.color2.0, it.color2.1, it.color2.2, it.color2.3]);
        }
    }

    fn push(&mut self, r: RectF, radius: f32, kind: Kind, p: [f32; 2], c: [Rgba; 2], uv: [f32; 4]) {
        let ([p0, p1], [color, color2], clip) = (p, c, self.clip());
        let finite = [r.x, r.y, r.w, r.h, radius, p0, p1].iter().chain(&uv).all(|v| v.is_finite());
        // Widths, blurs and strokes are lengths; an angle or a seed is not.
        let length = !matches!(kind, Kind::Gradient | Kind::Grain);
        let (p0, p1) = if length { (p0.max(0.0), p1.max(0.0)) } else { (p0, p1) };
        // How far past the rect it draws, by kind: a shadow its blur + 1, a glyph or glow 0.
        let reach = [1.0, 1.0, p0 + 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0][kind as usize];
        let seen = r.inset(-reach).intersect(clip);
        let empty = r.w <= 0.0 || r.h <= 0.0 || seen.w <= 0.0 || seen.h <= 0.0;
        if !finite || empty || (color.3 == 0 && color2.3 == 0) {
            return;
        }
        let (rect, radius) = ([r.x, r.y, r.w, r.h], radius.max(0.0).min(r.w.min(r.h) / 2.0));
        let (kind, clip) = (kind as u8 as f32, [clip.x, clip.y, clip.w, clip.h]);
        self.items.push(Instance { rect, radius, kind, p0, p1, color, clip, uv, color2 });
    }
}

#[cfg(test)]
mod tests;

//! Draw lists for compusophyOS: every fill, border, shadow, icon, glyph,
//! gradient, glow and grain pass is one [`Instance`] of a single quad, and a
//! frame is one flat byte buffer that the `platform` crate draws with one
//! instanced WebGL2 call through [`VERTEX_SHADER`] and [`FRAGMENT_SHADER`],
//! which live here beside the byte contract they read.
//!
//! - Logical (CSS) pixels as `f32`, origin top-left, y down. Colors are
//!   straight sRGB; the shader writes premultiplied (`ONE, ONE_MINUS_SRC_ALPHA`).
//! - Draw order is push order. A push is skipped when its rect is empty, a
//!   value is not finite, every alpha is 0, or its reach misses the clip
//!   (the rect grown by 1 px of antialiasing, by blur + 1 for a shadow, by
//!   nothing for a glyph or glow). The radius is clamped to `[0, min(w, h) / 2]`.
//! - Each instance keeps the clip current at its push ([`NO_CLIP`] if none).
//! - Glyphs sample the [`Atlas`] 1:1: a glyph's rect is its uv size over the
//!   device pixel ratio, on a device pixel.

#![forbid(unsafe_code)]

mod atlas;
mod shader;

pub use atlas::Atlas;
pub use shader::{FRAGMENT_SHADER, VERTEX_SHADER};

/// Bytes per encoded [`Instance`] (the attribute stride), little-endian:
/// `rect` 4 x f32 at 0, `radius kind p0 p1` 4 x f32 at 16, `color` 4 x u8 at
/// 32, `clip` 4 x f32 at 36, `uv` 4 x f32 at 52, `color2` 4 x u8 at 68.
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
        let right = (self.x + self.w).min(o.x + o.w);
        let bottom = (self.y + self.h).min(o.y + o.h);
        RectF::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    }
}

/// What an [`Instance`] draws; encoded as its discriminant in `f32`. Unused
/// parameters are 0.
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

/// One quad of the instanced draw; [`INSTANCE_BYTES`] once encoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    /// x, y, w, h in logical pixels.
    pub rect: [f32; 4],
    /// Corner radius, within `[0, min(w, h) / 2]`.
    pub radius: f32,
    /// The [`Kind`] as `f32`.
    pub kind: f32,
    /// The kind's parameters.
    pub p0: f32,
    pub p1: f32,
    /// Straight color.
    pub color: Rgba,
    /// x, y, w, h: the instance is visible only inside it.
    pub clip: [f32; 4],
    /// x, y, w, h in [`Atlas`] pixels, for a glyph.
    pub uv: [f32; 4],
    /// The end color, for a gradient.
    pub color2: Rgba,
}

/// An ordered list of instances and a stack of clip rects.
#[derive(Default, Clone, Debug)]
pub struct DrawList {
    items: Vec<Instance>,
    clips: Vec<RectF>,
}

impl DrawList {
    pub fn new() -> DrawList {
        DrawList::default()
    }

    /// Removes every instance and clip, keeping the allocations.
    pub fn clear(&mut self) {
        self.items.clear();
        self.clips.clear();
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
        let id = icon as u8 as f32;
        self.push(r, 0.0, Kind::Icon, [id, stroke], [color, CLEAR], [0.0; 4]);
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

    /// The [`Atlas`] pixel rect `uv` (skipped if empty) drawn into `dst`.
    pub fn glyph(&mut self, dst: RectF, uv: RectF, color: Rgba) {
        if uv.w > 0.0 && uv.h > 0.0 {
            let uv = [uv.x, uv.y, uv.w, uv.h];
            self.push(dst, 0.0, Kind::Glyph, [0.0; 2], [color, CLEAR], uv);
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
        let le = |out: &mut Vec<u8>, vs: [f32; 4]| {
            for v in vs {
                out.extend_from_slice(&v.to_le_bytes());
            }
        };
        for it in &self.items {
            le(out, it.rect);
            le(out, [it.radius, it.kind, it.p0, it.p1]);
            let Rgba(r, g, b, a) = it.color;
            out.extend_from_slice(&[r, g, b, a]);
            le(out, it.clip);
            le(out, it.uv);
            let Rgba(r, g, b, a) = it.color2;
            out.extend_from_slice(&[r, g, b, a]);
        }
    }

    fn push(&mut self, r: RectF, radius: f32, kind: Kind, p: [f32; 2], c: [Rgba; 2], uv: [f32; 4]) {
        let [p0, p1] = p;
        let values = [r.x, r.y, r.w, r.h, radius, p0, p1];
        let finite = values.iter().chain(&uv).all(|v| v.is_finite());
        let [color, color2] = c;
        if !finite || r.w <= 0.0 || r.h <= 0.0 || (color.3 == 0 && color2.3 == 0) {
            return;
        }
        // Widths, blurs and strokes are lengths; an angle or a seed is not.
        let (p0, p1) = match kind {
            Kind::Gradient | Kind::Grain => (p0, p1),
            _ => (p0.max(0.0), p1.max(0.0)),
        };
        let reach = match kind {
            Kind::Glyph | Kind::Glow => 0.0,
            Kind::Shadow => p0 + 1.0,
            _ => 1.0,
        };
        let clip = self.clip();
        let seen = r.inset(-reach).intersect(clip);
        if seen.w <= 0.0 || seen.h <= 0.0 {
            return;
        }
        self.items.push(Instance {
            rect: [r.x, r.y, r.w, r.h],
            radius: radius.max(0.0).min(r.w.min(r.h) / 2.0),
            kind: kind as u8 as f32,
            p0,
            p1,
            color,
            clip: [clip.x, clip.y, clip.w, clip.h],
            uv,
            color2,
        });
    }
}

#[cfg(test)]
mod tests;

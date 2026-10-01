//! Draw lists for compusophyOS: every rounded rect, border, soft shadow,
//! gradient, glow, grain pass, icon and text glyph on screen is one
//! [`Instance`] of a single quad, and a whole frame is one flat byte buffer
//! drawn with one instanced WebGL2 call.
//!
//! This crate is pure Rust: it builds the list, packs the glyph [`Atlas`] and
//! encodes the bytes; the `platform` crate uploads them and runs
//! [`VERTEX_SHADER`] and [`FRAGMENT_SHADER`], which live here so the byte
//! contract and its reader change together.
//!
//! # Contract
//!
//! - Coordinates are logical (CSS) pixels as `f32`, origin top-left, y down.
//! - Colors are straight (not premultiplied) sRGB plus alpha. The fragment
//!   shader writes premultiplied color; blend with `ONE, ONE_MINUS_SRC_ALPHA`.
//! - Draw order is push order: later instances draw on top.
//! - A push is skipped when the rect has `w <= 0` or `h <= 0`, any value
//!   (angle and seed included) is not finite, every color's alpha is 0 (both
//!   of a gradient's), a border's width is `<= 0`, or a glyph's uv rect has
//!   `w <= 0` or `h <= 0`. The radius is clamped to `[0, min(w, h) / 2]`; a
//!   negative blur or stroke becomes 0.
//! - Clipping: [`DrawList::push_clip`] intersects a rect with the current
//!   clip and [`DrawList::pop_clip`] restores the previous one; with no clip
//!   pushed the clip is [`NO_CLIP`]. Every instance records the clip current
//!   at its push and is visible only inside it (a hard edge, tested on the
//!   fragment's logical position). A push whose reach misses the clip is
//!   skipped: the reach is the rect grown by 1 px of antialiasing, by the blur
//!   plus 1 px for a shadow, and not at all for a glyph or a glow.
//! - Each instance is [`INSTANCE_BYTES`] little-endian bytes:
//!
//! | offset | field | type |
//! |---|---|---|
//! | 0 | `rect` x, y, w, h | 4 x `f32` |
//! | 16 | `radius`, `kind`, `p0`, `p1` | 4 x `f32` |
//! | 32 | `color` r, g, b, a | 4 x `u8` |
//! | 36 | `clip` x, y, w, h | 4 x `f32` |
//! | 52 | `uv` x, y, w, h (atlas pixels) | 4 x `f32` |
//! | 68 | `color2` r, g, b, a | 4 x `u8` |
//!
//! [`Kind`] sets what `p0`, `p1`, `uv` and `color2` mean: Fill and Glow use
//! none; Border's `p0` is the stroke width (the ring sits just inside the
//! edge); Shadow's `p0` is the blur (the shadow reaches `p0` px outside the
//! rect); Icon's `p0` is the [`Icon`] id and `p1` the stroke width; Glyph's
//! `uv` is the [`Atlas`] pixel rect stretched over the rect; Gradient's `p0`
//! is the angle and `color2` the end color; Grain's `p0` is the seed. Unused
//! fields are 0.
//!
//! # Text
//!
//! Glyphs are coverage bitmaps in one [`Atlas`], rasterized at the exact
//! device pixel size and placed on device pixels, so the atlas is sampled
//! `NEAREST` and a glyph's rect is its uv size divided by the device pixel
//! ratio. Its coverage (the texel's red channel) scales the color's alpha.
//!
//! # Shaders
//!
//! GLSL ES 3.00. Instanced attributes (divisor 1, stride [`INSTANCE_BYTES`]):
//! location 0 `a_rect` (offset 0), location 1 `a_params` = (radius, kind,
//! p0, p1) (offset 16), location 2 `a_color` as normalized `UNSIGNED_BYTE`
//! x4 (offset 32), location 3 `a_clip` (offset 36), location 4 `a_uv`
//! (offset 52), location 5 `a_color2` as normalized `UNSIGNED_BYTE` x4
//! (offset 68); every other attribute is 4 x `FLOAT`. Uniforms: `u_viewport`
//! (logical canvas size), `u_dpr` (device pixel ratio), `u_atlas` (a
//! `sampler2D` on texture unit 0: the [`Atlas`] as an `R8` texture with
//! `NEAREST` filtering, uploaded with `UNPACK_ALIGNMENT` 1) and
//! `u_atlas_size` (its size in pixels). Draw with
//! `drawArraysInstanced(TRIANGLE_STRIP, 0, 4, n)` and no vertex buffer.
//! Edges are antialiased over one physical pixel. Smooth ramps (shadows,
//! gradients, glows) are dithered by under half an 8-bit step, which breaks
//! up banding and leaves a fully transparent pixel's target unchanged.
//!
//! # Example
//!
//! ```
//! use gfx::{Atlas, DrawList, INSTANCE_BYTES, Icon, RectF, Rgba};
//! use std::f32::consts::FRAC_PI_4;
//!
//! let mut atlas = Atlas::new(256, 256);
//! let (u, v) = atlas.alloc(7, 9).unwrap();
//! atlas.write(u, v, 7, 9, &[255; 63]);
//!
//! let mut list = DrawList::new();
//! let screen = RectF::new(0.0, 0.0, 1280.0, 800.0);
//! list.gradient(screen, 0.0, Rgba::hex(0x0b0d12), Rgba::hex(0x1a1f2c), FRAC_PI_4);
//! list.glow(RectF::new(-200.0, -300.0, 900.0, 700.0), Rgba(90, 120, 255, 40));
//! let win = RectF::new(80.0, 60.0, 640.0, 480.0);
//! list.shadow_offset(win, 12.0, 32.0, 12.0, Rgba(0, 0, 0, 110));
//! list.fill(win, 12.0, Rgba::hex(0x1b1b1b));
//! list.border(win, 12.0, 1.0, Rgba::hex(0x3a3a3a));
//! list.icon(RectF::new(692.0, 64.0, 16.0, 16.0), Icon::Cross, 1.5, Rgba::hex(0xe0e0e0));
//! list.push_clip(win.inset(1.0));
//! let uv = RectF::new(u as f32, v as f32, 7.0, 9.0);
//! list.glyph(RectF::new(92.0, 92.0, 7.0, 9.0), uv, Rgba::hex(0xe0e0e0));
//! list.fill(RectF::new(0.0, 600.0, 10.0, 10.0), 0.0, Rgba::hex(0xffffff)); // outside: skipped
//! list.pop_clip();
//! list.fill(RectF::new(0.0, 0.0, 0.0, 10.0), 0.0, Rgba::hex(0xffffff)); // zero width: skipped
//! list.grain(screen, 7, 1.0);
//!
//! let mut bytes = Vec::new();
//! list.encode_into(&mut bytes);
//! assert_eq!(bytes.len(), 8 * INSTANCE_BYTES);
//! ```

#![forbid(unsafe_code)]

mod atlas;
mod shader;

pub use atlas::Atlas;
pub use shader::{FRAGMENT_SHADER, VERTEX_SHADER};

/// Bytes per encoded [`Instance`]; also the attribute stride.
pub const INSTANCE_BYTES: usize = 72;

/// The clip of an unclipped instance, as x, y, w, h: a square a billion
/// logical pixels out from the origin, beyond any canvas.
pub const NO_CLIP: [f32; 4] = [-1e9, -1e9, 2e9, 2e9];

/// A straight (not premultiplied) sRGB color with alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

/// Transparent black: the `color2` of every kind but [`Kind::Gradient`].
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
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
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

    /// Shrinks every side by `d` (grows for negative `d`); the size never
    /// goes below zero.
    pub fn inset(self, d: f32) -> RectF {
        RectF::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }

    /// Whether the point lies inside, half-open: `x <= px < x + w` and
    /// `y <= py < y + h`.
    pub fn contains(self, px: f32, py: f32) -> bool {
        (self.x..self.x + self.w).contains(&px) && (self.y..self.y + self.h).contains(&py)
    }

    /// The overlap of two rects; when they do not overlap, the size is zero
    /// (and the corner is where the overlap would start).
    pub fn intersect(self, o: RectF) -> RectF {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let right = (self.x + self.w).min(o.x + o.w);
        let bottom = (self.y + self.h).min(o.y + o.h);
        RectF::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    }
}

/// What an [`Instance`] draws; encoded as its discriminant in `f32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A filled rounded rect.
    Fill = 0,
    /// A ring of width `p0` just inside the rounded rect's edge.
    Border = 1,
    /// A soft shadow reaching `p0` px outside the rounded rect.
    Shadow = 2,
    /// Icon `p0` drawn with stroke width `p1` in the rect's centered square.
    Icon = 3,
    /// The [`Atlas`] pixels at `uv` stretched over the rect; each texel's
    /// coverage scales the color's alpha.
    Glyph = 4,
    /// A filled rounded rect shaded with a linear gradient from `color` to
    /// `color2`, interpolated premultiplied. `p0` is the angle in radians:
    /// the gradient runs along `(sin p0, cos p0)` with y down, so 0 runs top
    /// to bottom and `PI / 2` left to right. As in CSS, it spans the rect's
    /// extent along that direction: `color` at the farthest corner behind,
    /// `color2` at the farthest ahead.
    Gradient = 5,
    /// A soft elliptical light filling the rect: the color's alpha times
    /// `(1 - d^2)^2`, where `d` is the elliptical distance from the center
    /// (0 there, 1 on the ellipse inscribed in the rect); zero beyond it.
    Glow = 6,
    /// Film grain over the rect: per device pixel, a hash of the pixel and
    /// the seed `p0` gives noise `n` in `[-1, 1]` (triangular, the sum of two
    /// uniform halves), drawn as white at alpha `n * a` where `n > 0` and
    /// black at alpha `-n * a` where `n < 0`, `a` being the color's alpha.
    /// The pattern is fixed to the screen and changes with the seed.
    Grain = 7,
}

/// Built-in vector icons, drawn in the centered square inscribed in the rect
/// (side `s = min(w, h)`); encoded as their discriminant in `f32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Two perpendicular strokes through the center, each 70% of `s` long.
    Plus = 0,
    /// [`Icon::Plus`] rotated 45 degrees.
    Cross = 1,
    /// The horizontal stroke of [`Icon::Plus`].
    Minus = 2,
    /// A filled circle, 40% of `s` across.
    Dot = 3,
    /// A square outline, 56% of `s` on a side.
    Square = 4,
    /// Four filled dots in a 2 x 2 grid, 24% of `s` across, centered at
    /// plus or minus 22% of `s`.
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
    /// First kind-specific parameter (see [`Kind`]).
    pub p0: f32,
    /// Second kind-specific parameter (see [`Kind`]).
    pub p1: f32,
    /// Straight color.
    pub color: Rgba,
    /// x, y, w, h in logical pixels: the instance is visible only inside it.
    /// [`NO_CLIP`] when no clip was pushed.
    pub clip: [f32; 4],
    /// x, y, w, h in [`Atlas`] pixels, for [`Kind::Glyph`]; zero otherwise.
    pub uv: [f32; 4],
    /// Straight end color, for [`Kind::Gradient`]; transparent black
    /// otherwise.
    pub color2: Rgba,
}

/// An ordered list of instances and a stack of clip rects: one frame, or one
/// layer of one.
#[derive(Default, Clone, Debug)]
pub struct DrawList {
    items: Vec<Instance>,
    clips: Vec<RectF>,
}

impl DrawList {
    /// An empty list.
    pub fn new() -> DrawList {
        DrawList::default()
    }

    /// Removes every instance and every pushed clip, keeping the
    /// allocations.
    pub fn clear(&mut self) {
        self.items.clear();
        self.clips.clear();
    }

    /// Number of instances.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no instances.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// A filled rounded rect.
    pub fn fill(&mut self, r: RectF, radius: f32, color: Rgba) {
        self.push(r, radius, Kind::Fill, [0.0; 2], [color, CLEAR], [0.0; 4]);
    }

    /// A ring `width` px wide just inside the rounded rect's edge. Skipped
    /// when `width <= 0`.
    pub fn border(&mut self, r: RectF, radius: f32, width: f32, color: Rgba) {
        if width > 0.0 {
            self.push(r, radius, Kind::Border, [width, 0.0], [color, CLEAR], [0.0; 4]);
        }
    }

    /// A soft shadow of the rounded rect that fades out `blur` px beyond it.
    pub fn shadow(&mut self, r: RectF, radius: f32, blur: f32, color: Rgba) {
        self.push(r, radius, Kind::Shadow, [blur, 0.0], [color, CLEAR], [0.0; 4]);
    }

    /// [`DrawList::shadow`] of the rect moved down by `dy` px (up when
    /// negative), as a light above the screen casts it. Skipped when `dy`
    /// is not finite.
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

    /// A rounded rect filled with a linear gradient from `from` to `to`
    /// along `angle` radians (0 top to bottom, `PI / 2` left to right; see
    /// [`Kind::Gradient`]). Skipped when both alphas are 0.
    pub fn gradient(&mut self, r: RectF, radius: f32, from: Rgba, to: Rgba, angle: f32) {
        self.push(r, radius, Kind::Gradient, [angle, 0.0], [from, to], [0.0; 4]);
    }

    /// A soft elliptical light filling the rect, full `color` at the center
    /// and fading smoothly to nothing at the inscribed ellipse (see
    /// [`Kind::Glow`]).
    pub fn glow(&mut self, r: RectF, color: Rgba) {
        self.push(r, 0.0, Kind::Glow, [0.0; 2], [color, CLEAR], [0.0; 4]);
    }

    /// Film grain over the rect, at most `strength / 255` alpha per pixel;
    /// each `seed` gives another fixed pattern (see [`Kind::Grain`]).
    /// Skipped when `strength` is 0.
    pub fn grain(&mut self, r: RectF, strength: u8, seed: f32) {
        let color = Rgba(255, 255, 255, strength);
        self.push(r, 0.0, Kind::Grain, [seed, 0.0], [color, CLEAR], [0.0; 4]);
    }

    /// The [`Atlas`] pixel rect `uv` drawn into `dst`, its coverage tinting
    /// `color`. For crisp text, `dst` is `uv`'s size divided by the device
    /// pixel ratio, on a device pixel. Skipped when `uv.w <= 0` or
    /// `uv.h <= 0`.
    pub fn glyph(&mut self, dst: RectF, uv: RectF, color: Rgba) {
        if uv.w > 0.0 && uv.h > 0.0 {
            let uv = [uv.x, uv.y, uv.w, uv.h];
            self.push(dst, 0.0, Kind::Glyph, [0.0; 2], [color, CLEAR], uv);
        }
    }

    /// Pushes the intersection of `r` and the current clip as the new clip.
    /// A rect with a non-finite value clips everything away.
    pub fn push_clip(&mut self, r: RectF) {
        let cur = self.clip();
        let finite = [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite());
        let next = if finite { cur.intersect(r) } else { RectF::new(cur.x, cur.y, 0.0, 0.0) };
        self.clips.push(next);
    }

    /// Restores the clip before the last [`DrawList::push_clip`]; a no-op
    /// when none is pushed.
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
mod tests {
    use super::*;

    const WHITE: Rgba = Rgba::hex(0xffffff);

    #[test]
    fn colors() {
        assert_eq!(Rgba::hex(0x12ab34), Rgba(0x12, 0xab, 0x34, 255));
        assert_eq!(Rgba::hex(0xff00_0000), Rgba(0, 0, 0, 255));
        assert_eq!(Rgba::hex(0x010203).with_alpha(7), Rgba(1, 2, 3, 7));
    }

    #[test]
    fn rects() {
        let r = RectF::from_i32(10, 20, 30, 40);
        assert_eq!(r, RectF::new(10.0, 20.0, 30.0, 40.0));
        assert_eq!(r.inset(5.0), RectF::new(15.0, 25.0, 20.0, 30.0));
        assert_eq!(r.inset(-1.0), RectF::new(9.0, 19.0, 32.0, 42.0));
        assert_eq!(r.inset(18.0), RectF::new(28.0, 38.0, 0.0, 4.0));
        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(39.9, 59.9));
        assert!(!r.contains(40.0, 30.0));
        assert!(!r.contains(20.0, 60.0));
        assert!(!r.contains(9.9, 30.0));
        assert!(!RectF::default().contains(0.0, 0.0));
        let a = RectF::new(0.0, 0.0, 10.0, 10.0);
        let b = RectF::new(5.0, -5.0, 10.0, 10.0);
        assert_eq!(a.intersect(b), RectF::new(5.0, 0.0, 5.0, 5.0));
        assert_eq!(b.intersect(a), RectF::new(5.0, 0.0, 5.0, 5.0));
        let far = RectF::new(20.0, 3.0, 5.0, 5.0);
        assert_eq!(a.intersect(far), RectF::new(20.0, 3.0, 0.0, 5.0));
    }

    #[test]
    fn encode_exact_bytes() {
        let mut list = DrawList::new();
        list.fill(RectF::new(1.0, 2.0, 3.0, 4.0), 0.5, Rgba(10, 20, 30, 40));
        list.push_clip(RectF::new(0.0, 0.0, 8.0, 8.0));
        let uv = RectF::new(1.0, 2.0, 3.0, 4.0);
        list.glyph(RectF::new(-2.0, 0.0, 16.0, 8.0), uv, WHITE);
        let mut out = vec![9; 200];
        list.encode_into(&mut out);
        #[rustfmt::skip]
        let want: [u8; 2 * INSTANCE_BYTES] = [
            0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x40, // x 1.0, y 2.0
            0x00, 0x00, 0x40, 0x40, 0x00, 0x00, 0x80, 0x40, // w 3.0, h 4.0
            0x00, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x00, 0x00, // radius 0.5, kind Fill
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p0 0, p1 0
            10, 20, 30, 40,
            0x28, 0x6b, 0x6e, 0xce, 0x28, 0x6b, 0x6e, 0xce, // clip x, y -1e9
            0x28, 0x6b, 0xee, 0x4e, 0x28, 0x6b, 0xee, 0x4e, // clip w, h 2e9
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // uv x, y 0
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // uv w, h 0
            0, 0, 0, 0, // color2 unused
            0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, // x -2.0, y 0.0
            0x00, 0x00, 0x80, 0x41, 0x00, 0x00, 0x00, 0x41, // w 16.0, h 8.0
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x40, // radius 0, kind Glyph
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p0 0, p1 0
            255, 255, 255, 255,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // clip x, y 0
            0x00, 0x00, 0x00, 0x41, 0x00, 0x00, 0x00, 0x41, // clip w, h 8.0
            0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x40, // uv x 1.0, y 2.0
            0x00, 0x00, 0x40, 0x40, 0x00, 0x00, 0x80, 0x40, // uv w 3.0, h 4.0
            0, 0, 0, 0, // color2 unused
        ];
        assert_eq!(out, want);
        list.clear();
        list.encode_into(&mut out);
        assert!(out.is_empty() && list.is_empty());

        // A gradient: angle -0.5 in p0, its end color in the last 4 bytes.
        list.gradient(
            RectF::new(0.0, 0.0, 4.0, 4.0),
            1.0,
            Rgba(1, 2, 3, 4),
            Rgba(5, 6, 7, 0),
            -0.5,
        );
        list.encode_into(&mut out);
        assert_eq!(out.len(), INSTANCE_BYTES);
        let f = |at: usize| f32::from_le_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]);
        assert_eq!([f(16), f(20), f(24), f(28)], [1.0, 5.0, -0.5, 0.0]);
        assert_eq!((&out[32..36], &out[68..72]), (&[1, 2, 3, 4][..], &[5, 6, 7, 0][..]));
    }

    #[test]
    fn layout_offsets() {
        // Four 16-byte vec4s and two 4-byte colors, in the documented order.
        assert_eq!(INSTANCE_BYTES, 4 * 16 + 2 * 4);
        let mut list = DrawList::new();
        let mut out = Vec::new();
        list.gradient(RectF::new(1.0, 1.0, 9.0, 9.0), 0.0, Rgba(9, 9, 9, 9), Rgba(1, 2, 3, 4), 0.0);
        list.glyph(RectF::new(1.0, 1.0, 9.0, 9.0), RectF::new(7.0, 7.0, 7.0, 7.0), WHITE);
        list.encode_into(&mut out);
        let at =
            |i: usize, off: usize| &out[i * INSTANCE_BYTES + off..i * INSTANCE_BYTES + off + 4];
        assert_eq!(at(0, 68), [1, 2, 3, 4]);
        assert_eq!(at(1, 32), [255, 255, 255, 255]);
        assert_eq!(at(1, 52), 7.0f32.to_le_bytes());
        assert_eq!(at(1, 68), [0, 0, 0, 0]);
    }

    #[test]
    fn kinds_and_params() {
        let r = RectF::new(0.0, 0.0, 10.0, 10.0);
        let mut list = DrawList::new();
        list.fill(r, 2.0, WHITE);
        list.border(r, 2.0, 1.5, WHITE);
        list.shadow(r, 2.0, 12.0, WHITE);
        list.shadow(r, 2.0, -3.0, WHITE);
        list.icon(r, Icon::Square, -1.0, WHITE);
        list.glyph(r, r, WHITE);
        list.gradient(r, 2.0, WHITE, Rgba(1, 2, 3, 4), -1.25);
        list.glow(r, WHITE);
        list.grain(r, 9, -3.5);
        list.shadow_offset(r, 2.0, 8.0, 6.0, WHITE);
        list.shadow_offset(r, 2.0, 8.0, -2.0, WHITE);
        let got: Vec<[f32; 3]> = list.instances().iter().map(|i| [i.kind, i.p0, i.p1]).collect();
        let want = [
            [0.0, 0.0, 0.0],
            [1.0, 1.5, 0.0],
            [2.0, 12.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 4.0, 0.0],
            [4.0, 0.0, 0.0],
            [5.0, -1.25, 0.0], // an angle may be negative
            [6.0, 0.0, 0.0],
            [7.0, -3.5, 0.0], // so may a seed
            [2.0, 8.0, 0.0],
            [2.0, 8.0, 0.0],
        ];
        assert_eq!(got, want);
        assert_eq!(list.len(), 11);
        let items = list.instances();
        // Only a gradient has a second color; grain is white at the strength.
        let seconds: Vec<Rgba> = items.iter().map(|i| i.color2).collect();
        let mut want2 = [CLEAR; 11];
        want2[6] = Rgba(1, 2, 3, 4);
        assert_eq!(seconds, want2);
        assert_eq!(items[8].color, Rgba(255, 255, 255, 9));
        // Offset shadows move down (or up), keeping their size and radius.
        assert_eq!(items[9].rect, [0.0, 6.0, 10.0, 10.0]);
        assert_eq!(items[10].rect, [0.0, -2.0, 10.0, 10.0]);
        assert_eq!((items[9].radius, items[6].radius, items[7].radius), (2.0, 2.0, 0.0));
        let kinds = [Kind::Fill, Kind::Border, Kind::Shadow, Kind::Icon, Kind::Glyph];
        let more = [Kind::Gradient, Kind::Glow, Kind::Grain];
        let ids: Vec<u8> = kinds.iter().chain(&more).map(|&k| k as u8).collect();
        assert_eq!(ids, [0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn degenerate_pushes_are_skipped() {
        let ok = RectF::new(0.0, 0.0, 10.0, 10.0);
        let mut list = DrawList::new();
        list.fill(RectF::new(0.0, 0.0, 0.0, 10.0), 0.0, WHITE);
        list.fill(RectF::new(0.0, 0.0, 10.0, -1.0), 0.0, WHITE);
        list.fill(RectF::new(f32::NAN, 0.0, 10.0, 10.0), 0.0, WHITE);
        list.fill(RectF::new(0.0, f32::INFINITY, 10.0, 10.0), 0.0, WHITE);
        list.fill(RectF::new(0.0, 0.0, f32::NAN, 10.0), 0.0, WHITE);
        list.fill(ok, f32::INFINITY, WHITE);
        list.fill(ok, 0.0, WHITE.with_alpha(0));
        list.border(ok, 0.0, 0.0, WHITE);
        list.border(ok, 0.0, f32::NAN, WHITE);
        list.shadow(ok, 0.0, f32::INFINITY, WHITE);
        list.icon(ok, Icon::Dot, f32::NAN, WHITE);
        list.icon(RectF::new(0.0, 0.0, 10.0, 0.0), Icon::Dot, 1.0, WHITE);
        let none = WHITE.with_alpha(0);
        list.gradient(ok, 0.0, none, Rgba(9, 9, 9, 0), 0.0);
        list.gradient(ok, 0.0, WHITE, WHITE, f32::NAN);
        list.gradient(ok, 0.0, WHITE, WHITE, f32::INFINITY);
        list.gradient(ok, f32::NAN, WHITE, WHITE, 0.0);
        list.gradient(RectF::new(0.0, 0.0, -1.0, 10.0), 0.0, WHITE, WHITE, 0.0);
        list.glow(ok, none);
        list.glow(RectF::new(0.0, 0.0, 10.0, 0.0), WHITE);
        list.glow(RectF::new(0.0, f32::NAN, 10.0, 10.0), WHITE);
        list.grain(ok, 0, 1.0);
        list.grain(ok, 8, f32::NAN);
        list.grain(ok, 8, f32::NEG_INFINITY);
        list.grain(RectF::new(0.0, 0.0, 0.0, 0.0), 8, 1.0);
        list.shadow_offset(ok, 0.0, 4.0, f32::NAN, WHITE);
        list.shadow_offset(ok, 0.0, 4.0, f32::INFINITY, WHITE);
        list.shadow_offset(ok, 0.0, f32::NAN, 2.0, WHITE);
        list.shadow_offset(ok, 0.0, 4.0, 2.0, none);
        list.shadow_offset(RectF::new(0.0, 0.0, 10.0, -3.0), 0.0, 4.0, 2.0, WHITE);
        list.shadow_offset(RectF::new(0.0, f32::MAX, 10.0, 10.0), 0.0, 4.0, f32::MAX, WHITE);
        assert!(list.is_empty());
        list.fill(ok, 0.0, WHITE.with_alpha(1));
        // One opaque end is enough for a gradient.
        list.gradient(ok, 0.0, none, Rgba(0, 0, 0, 1), 0.0);
        list.gradient(ok, 0.0, Rgba(0, 0, 0, 1), none, 0.0);
        list.grain(ok, 1, 0.0);
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn glyphs_carry_their_uv() {
        let mut list = DrawList::new();
        let dst = RectF::new(3.0, 4.0, 7.0, 9.0);
        let uv = RectF::new(10.0, 20.0, 14.0, 18.0);
        list.glyph(dst, uv, WHITE);
        list.glyph(dst, RectF::new(10.0, 20.0, 0.0, 18.0), WHITE);
        list.glyph(dst, RectF::new(10.0, 20.0, 14.0, -1.0), WHITE);
        list.glyph(dst, RectF::new(f32::NAN, 20.0, 14.0, 18.0), WHITE);
        list.glyph(dst, RectF::new(10.0, f32::INFINITY, 14.0, 18.0), WHITE);
        list.glyph(RectF::new(3.0, 4.0, 0.0, 9.0), uv, WHITE);
        list.glyph(dst, uv, WHITE.with_alpha(0));
        assert_eq!(list.len(), 1);
        let g = list.instances()[0];
        assert_eq!(g.kind, 4.0);
        assert_eq!((g.rect, g.uv), ([3.0, 4.0, 7.0, 9.0], [10.0, 20.0, 14.0, 18.0]));
        assert_eq!((g.radius, g.p0, g.p1, g.clip), (0.0, 0.0, 0.0, NO_CLIP));
        list.fill(dst, 0.0, WHITE);
        assert_eq!(list.instances()[1].uv, [0.0; 4]);
    }

    #[test]
    fn clips_nest_and_pop() {
        let [x, y, w, h] = NO_CLIP;
        let none = RectF::new(x, y, w, h);
        let big = RectF::new(0.0, 0.0, 200.0, 200.0);
        let mut list = DrawList::new();
        assert_eq!(list.clip(), none);
        list.pop_clip(); // empty: a no-op
        assert_eq!(list.clip(), none);
        list.push_clip(RectF::new(10.0, 10.0, 100.0, 50.0));
        list.push_clip(RectF::new(50.0, 0.0, 100.0, 30.0));
        assert_eq!(list.clip(), RectF::new(50.0, 10.0, 60.0, 20.0));
        list.fill(big, 0.0, WHITE);
        list.pop_clip();
        list.fill(big, 0.0, WHITE);
        list.pop_clip();
        list.pop_clip();
        assert_eq!(list.clip(), none);
        list.fill(big, 0.0, WHITE);
        let clips: Vec<[f32; 4]> = list.instances().iter().map(|i| i.clip).collect();
        let want = [[50.0, 10.0, 60.0, 20.0], [10.0, 10.0, 100.0, 50.0], NO_CLIP];
        assert_eq!(clips, want);

        // Disjoint, inverted and non-finite clips hide everything until popped.
        list.push_clip(RectF::new(0.0, 0.0, 10.0, 10.0));
        list.push_clip(RectF::new(20.0, 0.0, 10.0, 10.0));
        list.fill(big, 0.0, WHITE);
        list.shadow(big, 0.0, 1000.0, WHITE);
        list.pop_clip();
        list.push_clip(RectF::new(0.0, 0.0, -5.0, 10.0));
        list.fill(big, 0.0, WHITE);
        list.pop_clip();
        list.push_clip(RectF::new(f32::NAN, 0.0, 10.0, 10.0));
        list.fill(big, 0.0, WHITE);
        list.pop_clip();
        list.push_clip(RectF::new(0.0, f32::INFINITY, 10.0, 10.0));
        list.fill(big, 0.0, WHITE);
        list.pop_clip();
        assert_eq!(list.clip(), RectF::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(list.len(), 3);
        list.clear();
        assert_eq!(list.clip(), none);
    }

    #[test]
    fn pushes_outside_the_clip_are_skipped() {
        let uv = RectF::new(0.0, 0.0, 5.0, 5.0);
        let red = |r| Rgba(r, 0, 0, 255);
        let mut list = DrawList::new();
        list.push_clip(RectF::new(100.0, 100.0, 50.0, 50.0));
        // Antialiased kinds reach 1 px past their rect.
        list.fill(RectF::new(90.0, 100.0, 9.0, 10.0), 0.0, WHITE);
        list.fill(RectF::new(90.0, 100.0, 9.5, 10.0), 0.0, red(1));
        list.border(RectF::new(151.0, 100.0, 10.0, 10.0), 0.0, 1.0, WHITE);
        list.icon(RectF::new(100.0, 150.5, 10.0, 10.0), Icon::Dot, 1.0, red(2));
        // A shadow reaches its blur plus 1 px.
        list.shadow(RectF::new(100.0, 40.0, 10.0, 10.0), 0.0, 49.0, WHITE);
        list.shadow(RectF::new(100.0, 40.0, 10.0, 10.0), 0.0, 50.0, red(3));
        // A glyph reaches exactly its rect.
        list.glyph(RectF::new(150.0, 100.0, 5.0, 5.0), uv, WHITE);
        list.glyph(RectF::new(149.0, 145.0, 5.0, 5.0), uv, red(4));
        // So does a glow; gradients and grain antialias their edges.
        list.glow(RectF::new(90.0, 100.0, 10.0, 10.0), WHITE);
        list.glow(RectF::new(90.0, 100.0, 10.5, 10.0), red(5));
        list.gradient(RectF::new(100.0, 151.0, 9.0, 9.0), 0.0, WHITE, WHITE, 0.0);
        list.gradient(RectF::new(100.0, 150.5, 9.0, 9.0), 0.0, red(6), WHITE, 0.0);
        list.grain(RectF::new(80.0, 80.0, 19.0, 19.0), 255, 0.0);
        list.grain(RectF::new(80.0, 80.0, 19.5, 19.5), 255, 0.0);
        // An offset shadow reaches from where it lands.
        list.shadow_offset(RectF::new(100.0, 151.0, 9.0, 9.0), 0.0, 0.0, 10.0, WHITE);
        list.shadow_offset(RectF::new(100.0, 89.0, 9.0, 9.0), 0.0, 0.0, 10.0, red(7));
        let reds: Vec<u8> = list.instances().iter().map(|i| i.color.0).collect();
        assert_eq!(reds, [1, 2, 3, 4, 5, 6, 255, 7]);
    }

    #[test]
    fn radius_is_clamped() {
        let mut list = DrawList::new();
        list.fill(RectF::new(0.0, 0.0, 20.0, 10.0), 100.0, WHITE);
        list.fill(RectF::new(0.0, 0.0, 20.0, 10.0), -3.0, WHITE);
        list.fill(RectF::new(0.0, 0.0, 20.0, 10.0), 3.0, WHITE);
        list.border(RectF::new(0.0, 0.0, 4.0, 30.0), 9.0, 1.0, WHITE);
        let radii: Vec<f32> = list.instances().iter().map(|i| i.radius).collect();
        assert_eq!(radii, [5.0, 0.0, 3.0, 2.0]);
    }

    #[test]
    fn draw_order_is_push_order() {
        let mut list = DrawList::new();
        for i in 0..4u8 {
            list.fill(RectF::new(f32::from(i), 0.0, 1.0, 1.0), 0.0, Rgba(i, 0, 0, 255));
        }
        let reds: Vec<u8> = list.instances().iter().map(|i| i.color.0).collect();
        assert_eq!(reds, [0, 1, 2, 3]);
        let copy = list.clone();
        list.clear();
        assert_eq!((list.len(), copy.len()), (0, 4));
    }

    #[test]
    fn shaders_name_the_contract() {
        for src in [VERTEX_SHADER, FRAGMENT_SHADER] {
            assert!(src.starts_with("#version 300 es\n"));
            assert!(src.contains("precision highp float;"));
            assert!(src.contains("uniform float u_dpr;"));
        }
        for name in [
            "layout(location = 0) in vec4 a_rect;",
            "layout(location = 1) in vec4 a_params;",
            "layout(location = 2) in vec4 a_color;",
            "layout(location = 3) in vec4 a_clip;",
            "layout(location = 4) in vec4 a_uv;",
            "layout(location = 5) in vec4 a_color2;",
            "v_color2 = a_color2;",
            "uniform vec2 u_viewport;",
            "gl_VertexID",
            // Glyphs and glows draw no antialiasing margin.
            "kind == 4 || kind == 6 ? 0.0 : px",
        ] {
            assert!(VERTEX_SHADER.contains(name), "vertex shader lacks {name}");
        }
        for name in [
            "precision highp int;",
            "out vec4 fragColor;",
            "uniform sampler2D u_atlas;",
            "uniform vec2 u_atlas_size;",
            "kind == 1",
            "kind == 2",
            "kind == 3",
            "kind == 4",
            "kind == 5",
            "kind == 6",
            "kind == 7",
            "v_color2",
            "floatBitsToUint(v_params.z)",
            "gl_FragCoord",
        ] {
            assert!(FRAGMENT_SHADER.contains(name), "fragment shader lacks {name}");
        }
        // Every varying the vertex shader writes, the fragment shader reads.
        for line in VERTEX_SHADER.lines().filter(|l| l.contains("out ")) {
            let decl = line.replace("out ", "in ");
            assert!(FRAGMENT_SHADER.contains(&decl), "fragment shader lacks {decl}");
        }
    }
}

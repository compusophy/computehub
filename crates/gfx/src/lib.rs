//! Draw lists for compusophyOS: every rounded rect, border, soft shadow and
//! icon on screen is one [`Instance`] of a single quad, and a whole frame is
//! one flat byte buffer drawn with one instanced WebGL2 call.
//!
//! This crate is pure Rust: it builds the list and encodes the bytes; the
//! `platform` crate uploads them and runs [`VERTEX_SHADER`] and
//! [`FRAGMENT_SHADER`], which live here so the byte contract and its reader
//! change together.
//!
//! # Contract
//!
//! - Coordinates are logical (CSS) pixels as `f32`, origin top-left, y down.
//! - Colors are straight (not premultiplied) sRGB plus alpha. The fragment
//!   shader writes premultiplied color; blend with `ONE, ONE_MINUS_SRC_ALPHA`.
//! - Draw order is push order: later instances draw on top.
//! - A push is skipped when the rect has `w <= 0` or `h <= 0`, any value is
//!   not finite, the alpha is 0, or a border's width is `<= 0`. The radius is
//!   clamped to `[0, min(w, h) / 2]`; a negative blur or stroke becomes 0.
//! - Each instance is [`INSTANCE_BYTES`] little-endian bytes:
//!
//! | offset | field | type |
//! |---|---|---|
//! | 0 | `rect` x, y, w, h | 4 x `f32` |
//! | 16 | `radius`, `kind`, `p0`, `p1` | 4 x `f32` |
//! | 32 | `color` r, g, b, a | 4 x `u8` |
//!
//! [`Kind`] sets what `p0` and `p1` mean: Fill uses neither; Border's `p0` is
//! the stroke width (the ring sits just inside the edge); Shadow's `p0` is
//! the blur (the shadow reaches `p0` px outside the rect); Icon's `p0` is the
//! [`Icon`] id and `p1` the stroke width.
//!
//! # Shaders
//!
//! GLSL ES 3.00. Instanced attributes (divisor 1, stride [`INSTANCE_BYTES`]):
//! location 0 `a_rect` (offset 0), location 1 `a_params` = (radius, kind,
//! p0, p1) (offset 16), location 2 `a_color` as normalized `UNSIGNED_BYTE`
//! x4 (offset 32). Uniforms: `u_viewport` (logical canvas size) and `u_dpr`
//! (device pixel ratio). Draw with `drawArraysInstanced(TRIANGLE_STRIP, 0, 4,
//! n)` and no vertex buffer. Edges are antialiased over one physical pixel.
//!
//! # Example
//!
//! ```
//! use gfx::{DrawList, INSTANCE_BYTES, Icon, RectF, Rgba};
//!
//! let mut list = DrawList::new();
//! let win = RectF::new(8.0, 8.0, 640.0, 480.0);
//! list.shadow(win, 12.0, 24.0, Rgba(0, 0, 0, 96));
//! list.fill(win, 12.0, Rgba::hex(0x1b1b1b));
//! list.border(win, 12.0, 1.0, Rgba::hex(0x3a3a3a));
//! list.icon(RectF::new(620.0, 12.0, 16.0, 16.0), Icon::Cross, 1.5, Rgba::hex(0xe0e0e0));
//! list.fill(RectF::new(0.0, 0.0, 0.0, 10.0), 0.0, Rgba::hex(0xffffff)); // zero width: skipped
//!
//! let mut bytes = Vec::new();
//! list.encode_into(&mut bytes);
//! assert_eq!(bytes.len(), 4 * INSTANCE_BYTES);
//! ```

#![forbid(unsafe_code)]

mod shader;

pub use shader::{FRAGMENT_SHADER, VERTEX_SHADER};

/// Bytes per encoded [`Instance`]; also the attribute stride.
pub const INSTANCE_BYTES: usize = 36;

/// A straight (not premultiplied) sRGB color with alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

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
        RectF::new(
            self.x + d,
            self.y + d,
            (self.w - 2.0 * d).max(0.0),
            (self.h - 2.0 * d).max(0.0),
        )
    }

    /// Whether the point lies inside, half-open: `x <= px < x + w` and
    /// `y <= py < y + h`.
    pub fn contains(self, px: f32, py: f32) -> bool {
        (self.x..self.x + self.w).contains(&px) && (self.y..self.y + self.h).contains(&py)
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
}

/// An ordered list of instances: one frame, or one layer of one.
#[derive(Default, Clone, Debug)]
pub struct DrawList {
    items: Vec<Instance>,
}

impl DrawList {
    /// An empty list.
    pub fn new() -> DrawList {
        DrawList::default()
    }

    /// Removes every instance, keeping the allocation.
    pub fn clear(&mut self) {
        self.items.clear();
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
        self.push(r, radius, Kind::Fill, 0.0, 0.0, color);
    }

    /// A ring `width` px wide just inside the rounded rect's edge. Skipped
    /// when `width <= 0`.
    pub fn border(&mut self, r: RectF, radius: f32, width: f32, color: Rgba) {
        if width > 0.0 {
            self.push(r, radius, Kind::Border, width, 0.0, color);
        }
    }

    /// A soft shadow of the rounded rect that fades out `blur` px beyond it.
    pub fn shadow(&mut self, r: RectF, radius: f32, blur: f32, color: Rgba) {
        self.push(r, radius, Kind::Shadow, blur, 0.0, color);
    }

    /// An icon in the rect's centered square, stroked `stroke` px wide.
    pub fn icon(&mut self, r: RectF, icon: Icon, stroke: f32, color: Rgba) {
        let id = icon as u8 as f32;
        self.push(r, 0.0, Kind::Icon, id, stroke, color);
    }

    /// The instances in draw order.
    pub fn instances(&self) -> &[Instance] {
        &self.items
    }

    /// Clears `out`, then appends [`INSTANCE_BYTES`] per instance, in order.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.items.len() * INSTANCE_BYTES);
        for it in &self.items {
            let [x, y, w, h] = it.rect;
            for v in [x, y, w, h, it.radius, it.kind, it.p0, it.p1] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            let Rgba(r, g, b, a) = it.color;
            out.extend_from_slice(&[r, g, b, a]);
        }
    }

    fn push(&mut self, r: RectF, radius: f32, kind: Kind, p0: f32, p1: f32, color: Rgba) {
        let values = [r.x, r.y, r.w, r.h, radius, p0, p1];
        if values.iter().any(|v| !v.is_finite()) || r.w <= 0.0 || r.h <= 0.0 || color.3 == 0 {
            return;
        }
        self.items.push(Instance {
            rect: [r.x, r.y, r.w, r.h],
            radius: radius.clamp(0.0, r.w.min(r.h) / 2.0),
            kind: kind as u8 as f32,
            p0: p0.max(0.0),
            p1: p1.max(0.0),
            color,
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
    }

    #[test]
    fn encode_exact_bytes() {
        let mut list = DrawList::new();
        list.fill(RectF::new(1.0, 2.0, 3.0, 4.0), 0.5, Rgba(10, 20, 30, 40));
        list.icon(RectF::new(-2.0, 0.0, 16.0, 8.0), Icon::Grid, 1.5, WHITE);
        let mut out = vec![9; 100];
        list.encode_into(&mut out);
        #[rustfmt::skip]
        let want: [u8; 2 * INSTANCE_BYTES] = [
            0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x40, // x 1.0, y 2.0
            0x00, 0x00, 0x40, 0x40, 0x00, 0x00, 0x80, 0x40, // w 3.0, h 4.0
            0x00, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x00, 0x00, // radius 0.5, kind Fill
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p0 0, p1 0
            10, 20, 30, 40,
            0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, // x -2.0, y 0.0
            0x00, 0x00, 0x80, 0x41, 0x00, 0x00, 0x00, 0x41, // w 16.0, h 8.0
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x40, // radius 0, kind Icon
            0x00, 0x00, 0xa0, 0x40, 0x00, 0x00, 0xc0, 0x3f, // p0 Grid, p1 1.5
            255, 255, 255, 255,
        ];
        assert_eq!(out, want);
        list.clear();
        list.encode_into(&mut out);
        assert!(out.is_empty() && list.is_empty());
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
        let got: Vec<[f32; 3]> = list
            .instances()
            .iter()
            .map(|i| [i.kind, i.p0, i.p1])
            .collect();
        let want = [
            [0.0, 0.0, 0.0],
            [1.0, 1.5, 0.0],
            [2.0, 12.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 4.0, 0.0],
        ];
        assert_eq!(got, want);
        assert_eq!(list.len(), 5);
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
        assert!(list.is_empty());
        list.fill(ok, 0.0, WHITE.with_alpha(1));
        assert_eq!(list.len(), 1);
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
            list.fill(
                RectF::new(f32::from(i), 0.0, 1.0, 1.0),
                0.0,
                Rgba(i, 0, 0, 255),
            );
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
            "uniform vec2 u_viewport;",
            "gl_VertexID",
        ] {
            assert!(VERTEX_SHADER.contains(name), "vertex shader lacks {name}");
        }
        assert!(FRAGMENT_SHADER.contains("out vec4 fragColor;"));
    }
}

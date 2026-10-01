//! Window frame geometry: where a window's controls sit, which edge the
//! pointer is on and how a drag of it resizes, and where a dropped window
//! snaps. Pure functions of rects and points.

use gfx::RectF;
use wm::{Rect, Snap};

use crate::{Cursor, TITLEBAR_H};

/// Diameter of a window control, the space between controls, and their
/// distance from the window's right edge.
pub const CTL: f32 = 12.0;
pub const CTL_GAP: f32 = 8.0;
pub const CTL_MARGIN: f32 = 12.0;
/// How far outside and inside a window's edge it can be grabbed to resize,
/// and how far along an edge from a corner the corner reaches.
pub const EDGE_OUT: f32 = 3.0;
pub const EDGE_IN: f32 = 3.0;
pub const CORNER: f32 = 14.0;
/// How near a screen edge a drop snaps to its half (or, at the top,
/// maximizes), and how near two edges to their quarter.
pub const SNAP_EDGE: f32 = 6.0;
pub const SNAP_CORNER: f32 = 24.0;

/// Where a dropped window goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// It maximizes.
    Max,
    /// It snaps to a half or quarter.
    Snap(Snap),
}

impl Zone {
    /// The rect a window dropped here takes in `area`.
    pub fn rect(self, area: Rect) -> Rect {
        match self {
            Zone::Max => area,
            Zone::Snap(s) => s.rect(area),
        }
    }
}

/// The minimize, maximize and close circles of a window at `r`, left to
/// right, vertically centered in the titlebar; `None` when the window is
/// too small for them.
pub fn controls(r: RectF) -> Option<[RectF; 3]> {
    if r.w < 4.0 * (CTL + CTL_GAP) + CTL_MARGIN || r.h < TITLEBAR_H {
        return None;
    }
    let y = r.y + ((TITLEBAR_H - CTL) / 2.0).round();
    let at = |i: f32| RectF::new(r.x + r.w - CTL_MARGIN - CTL - i * (CTL + CTL_GAP), y, CTL, CTL);
    Some([at(2.0), at(1.0), at(0.0)])
}

/// Which edge or corner of a window at `r` the point is on: -1, 0 or 1 in
/// x and in y (`(-1, 0)` the left edge, `(1, 1)` the bottom-right corner).
/// The band reaches [`EDGE_OUT`] outside and [`EDGE_IN`] inside; along it,
/// [`CORNER`] from a corner is the corner.
pub fn edge(r: RectF, x: f32, y: f32) -> Option<(i8, i8)> {
    if !r.inset(-EDGE_OUT).contains(x, y) || r.inset(EDGE_IN).contains(x, y) {
        return None;
    }
    let side = |v: f32, lo: f32, len: f32, reach: f32| match v {
        _ if v < lo + reach => -1,
        _ if v >= lo + len - reach => 1,
        _ => 0,
    };
    let (ex, ey) = (side(x, r.x, r.w, EDGE_IN), side(y, r.y, r.h, EDGE_IN));
    let (cx, cy) = (side(x, r.x, r.w, CORNER), side(y, r.y, r.h, CORNER));
    Some(if ex != 0 { (ex, cy) } else { (cx, ey) })
}

/// The cursor that resizes along `edge`.
pub fn edge_cursor((dx, dy): (i8, i8)) -> Cursor {
    match dx * dy {
        _ if dy == 0 => Cursor::EwResize,
        0 => Cursor::NsResize,
        1 => Cursor::NwseResize,
        _ => Cursor::NeswResize,
    }
}

/// `r` with the edges `edge` names dragged by `(dx, dy)`: the opposite
/// edges stay, the rect stays at least [`wm::MIN_W`] x [`wm::MIN_H`], and a
/// dragged top edge stops at `top`.
pub fn resized(r: Rect, (ex, ey): (i8, i8), (dx, dy): (f32, f32), top: i32) -> Rect {
    let (x, w) = grow(r.x, r.w, ex, dx, wm::MIN_W, -wm::MAX_COORD);
    let (y, h) = grow(r.y, r.h, ey, dy, wm::MIN_H, top);
    Rect::new(x, y, w, h)
}

/// One axis of [`resized`]: the start (`e == -1`) moves by `d` but no
/// nearer the end than `min` nor before `lo`; the end (`e == 1`) moves by
/// `d` but no nearer the start than `min`.
fn grow(pos: i32, len: i32, e: i8, d: f32, min: i32, lo: i32) -> (i32, i32) {
    let d = if d.is_finite() { d.round().clamp(-2e6, 2e6) as i32 } else { 0 };
    let end = pos.saturating_add(len);
    match e {
        -1 => {
            let start = pos.saturating_add(d).min(end - min).max(lo.min(pos));
            (start, end - start)
        }
        1 => (pos, len.saturating_add(d).max(min)),
        _ => (pos, len),
    }
}

/// Where a window dropped with the pointer at `(x, y)` on a `w` x `h`
/// screen goes: a quarter within [`SNAP_CORNER`] of two edges, else a half
/// within [`SNAP_EDGE`] of the left or right edge, else maximized within
/// [`SNAP_EDGE`] of the top; nowhere otherwise.
pub fn zone((w, h): (f32, f32), x: f32, y: f32) -> Option<Zone> {
    let near = |v: f32, len: f32, d: f32| match v {
        _ if v <= d => -1,
        _ if v >= len - d => 1,
        _ => 0,
    };
    let corner = (near(x, w, SNAP_CORNER), near(y, h, SNAP_CORNER));
    Some(match (corner, near(x, w, SNAP_EDGE), near(y, h, SNAP_EDGE)) {
        ((-1, -1), ..) => Zone::Snap(Snap::TopLeft),
        ((1, -1), ..) => Zone::Snap(Snap::TopRight),
        ((-1, 1), ..) => Zone::Snap(Snap::BottomLeft),
        ((1, 1), ..) => Zone::Snap(Snap::BottomRight),
        (_, -1, _) => Zone::Snap(Snap::Left),
        (_, 1, _) => Zone::Snap(Snap::Right),
        (_, _, -1) => Zone::Max,
        _ => return None,
    })
}

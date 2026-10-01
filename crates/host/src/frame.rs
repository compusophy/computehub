//! Window frame geometry: controls, resize edges and drags, and where a dropped window snaps.

use gfx::RectF;
use wm::{Rect, Snap};

use crate::{Cursor, TITLEBAR_H};

/// A window control's diameter, the space between them, and their margin.
pub const CTL: f32 = 12.0;
pub const CTL_GAP: f32 = 8.0;
const CTL_MARGIN: f32 = 12.0;
/// How far either side of its edge a window resizes; a corner's reach.
const EDGE: f32 = 3.0;
const CORNER: f32 = 14.0;
/// How near an edge, or two, a drop snaps.
const SNAP_EDGE: f32 = 6.0;
const SNAP_CORNER: f32 = 24.0;

/// Where a dropped window goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Max,
    Snap(Snap),
}

impl Zone {
    pub fn rect(self, area: Rect) -> Rect {
        match self {
            Zone::Max => area,
            Zone::Snap(s) => s.rect(area),
        }
    }
}

/// The minimize, maximize and close circles of a large enough window.
pub fn controls(r: RectF) -> Option<[RectF; 3]> {
    if r.w < 4.0 * (CTL + CTL_GAP) + CTL_MARGIN || r.h < TITLEBAR_H {
        return None;
    }
    let y = r.y + ((TITLEBAR_H - CTL) / 2.0).round();
    let at = |i: f32| RectF::new(r.x + r.w - CTL_MARGIN - CTL - i * (CTL + CTL_GAP), y, CTL, CTL);
    Some([at(2.0), at(1.0), at(0.0)])
}

/// The edge or corner of `r` the point is on: -1, 0 or 1 in x and y.
pub fn edge(r: RectF, x: f32, y: f32) -> Option<(i8, i8)> {
    if !r.inset(-EDGE).contains(x, y) || r.inset(EDGE).contains(x, y) {
        return None;
    }
    let side = |v: f32, lo: f32, len: f32, reach: f32| match v {
        _ if v < lo + reach => -1,
        _ if v >= lo + len - reach => 1,
        _ => 0,
    };
    let (ex, ey) = (side(x, r.x, r.w, EDGE), side(y, r.y, r.h, EDGE));
    let (cx, cy) = (side(x, r.x, r.w, CORNER), side(y, r.y, r.h, CORNER));
    Some(if ex != 0 { (ex, cy) } else { (cx, ey) })
}

pub fn edge_cursor((dx, dy): (i8, i8)) -> Cursor {
    match dx * dy {
        _ if dy == 0 => Cursor::EwResize,
        0 => Cursor::NsResize,
        1 => Cursor::NwseResize,
        _ => Cursor::NeswResize,
    }
}

/// `r` with its `edge` dragged by `(dx, dy)`, at least the wm's minimum; the top stops at `top`.
pub fn resized(r: Rect, (ex, ey): (i8, i8), (dx, dy): (f32, f32), top: i32) -> Rect {
    let (x, w) = grow(r.x, r.w, ex, dx, wm::MIN_W, -wm::MAX_COORD);
    let (y, h) = grow(r.y, r.h, ey, dy, wm::MIN_H, top);
    Rect::new(x, y, w, h)
}

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

/// Where a window dropped at `(x, y)` on a `w` x `h` screen goes.
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

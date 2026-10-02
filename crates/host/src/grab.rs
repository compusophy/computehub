//! Windows held by the pointer ([`Grab`]): a titlebar's press holds its window to move it (past
//! 4 px of travel, then exactly under the pointer; a maximized or snapped one comes back to its
//! normal size, as far across under it), and a second press within 350 ms toggles maximize
//! instead; an edge's or corner's resizes it, its top stopping at the work area's. Dropped near
//! the screen's edges, a moved window snaps there ([`frame::zone`]).

use wm::{Cmd, Placement, Rect, State, WinId};

use crate::frame::{self, Zone};
use crate::{Host, rectf};

/// Travel before a titlebar press moves its window; most time between a double click's presses.
const DRAG_PX: f32 = 4.0;
const DOUBLE_MS: f64 = 350.0;

/// A window held by the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    /// Moving: the press, the pointer's offset in the window, whether it moved yet, where it
    /// would drop.
    Move { win: WinId, at: (f32, f32), off: (f32, f32), moving: bool, zone: Option<Zone> },
    /// Resizing by `edge`: the press and the rect then.
    Size { win: WinId, edge: (i8, i8), at: (f32, f32), from: Rect },
}

impl Grab {
    /// Where the window moved would go, dropped now.
    pub fn zone(&self) -> Option<Zone> {
        if let Grab::Move { zone, .. } = *self { zone } else { None }
    }
}

impl Host {
    /// A press at `at` on window `win`'s titlebar (or its `edge`) at `now`, `last` the last
    /// titlebar press: the window held, or none for a double click's second, which toggles
    /// maximize.
    pub fn grab(
        &mut self,
        win: WinId,
        at: (f32, f32),
        edge: Option<(i8, i8)>,
        (now, last): (f64, &mut Option<(WinId, f64)>),
    ) -> Option<Grab> {
        let from = self.wm.layout().into_iter().find(|p| p.win == win)?.rect;
        if let Some(edge) = edge {
            return Some(Grab::Size { win, edge, at, from });
        }
        if last.take().is_some_and(|(w, t)| w == win && now - t <= DOUBLE_MS) {
            self.apply(Cmd::ToggleMaximize(win));
            return None;
        }
        *last = Some((win, now));
        let r = rectf(from);
        Some(Grab::Move { win, at, off: (at.0 - r.x, at.1 - r.y), moving: false, zone: None })
    }

    /// The pointer holding `grab` moved to `(x, y)` on a `screen`: the window follows (see the
    /// module docs). Whether it follows exactly, unanimated.
    pub fn drag(&mut self, grab: &mut Grab, (x, y): (f32, f32), screen: (f32, f32)) -> bool {
        match *grab {
            Grab::Move { win, at, mut off, moving, .. } => {
                if !moving && (x - at.0).abs().max((y - at.1).abs()) < DRAG_PX {
                    return false;
                }
                let back = |p: &Placement| p.state == State::Maximized || p.snap.is_some();
                let placed = self.wm.layout().into_iter().find(|p| p.win == win);
                let exact = match (placed.filter(|p| !moving && back(p)), self.wm.normal_rect(win))
                {
                    (Some(p), Some(n)) => {
                        off.0 = (off.0 / p.rect.w.max(1) as f32 * n.w as f32).round();
                        false
                    }
                    _ => true,
                };
                let (nx, ny) = ((x - off.0).round() as i32, (y - off.1).round() as i32);
                self.apply(Cmd::Move { win, x: nx, y: ny });
                let zone = frame::zone(screen, x, y);
                *grab = Grab::Move { win, at, off, moving: true, zone };
                exact
            }
            Grab::Size { win, edge, at, from } => {
                let rect = frame::resized(from, edge, (x - at.0, y - at.1), self.wm.area().y);
                self.apply(Cmd::Resize { win, rect });
                true
            }
        }
    }

    /// Lets go of `grab`: a window dropped in a zone snaps there, or maximizes.
    pub fn drop_grab(&mut self, grab: Grab) {
        let (Grab::Move { win, .. }, Some(zone)) = (grab, grab.zone()) else { return };
        self.apply(match zone {
            Zone::Max => Cmd::Maximize(win),
            Zone::Snap(snap) => Cmd::SnapTo { win, snap },
        });
    }
}

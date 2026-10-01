//! Deterministic floating window manager for compusophyOS.
//!
//! [`Wm`] owns window geometry, stacking and focus, and nothing else: no
//! rendering, no browser, no clocks, no floating point, no hash-ordered
//! collections. The shell turns pointer drags, titlebar buttons and keys into
//! [`Cmd`]s, [`Wm::apply`] (the only mutator) applies them, and the
//! compositor draws [`Wm::layout`] bottom to top.
//!
//! # Model
//!
//! - Windows float and overlap, as on Windows or COSMIC. Each has a *normal
//!   rect* (its free geometry, and the one [`Cmd::Restore`] returns to) and a
//!   mode: free (drawn at the normal rect), snapped to a half or quarter of
//!   the area ([`Snap`]), or maximized to the whole area. Snapped and
//!   maximized rects are derived from the area, so they follow it.
//! - The stack holds the visible windows bottom to top. The focused window is
//!   always its top, so focusing and raising are one act. Minimizing takes a
//!   window off the stack and keeps its mode; a command that shows it again
//!   puts it back on top, focused.
//! - Window ids start at 1, grow by one per open and are never reused.
//! - Normal rects are at least [`MIN_W`] x [`MIN_H`] and, where the area is
//!   larger, at most the area's size. They keep [`VISIBLE_W`] px of their
//!   width inside the area, and their top edge (the titlebar) between the
//!   area's top and [`TITLE_H`] px above its bottom.
//!
//! # Determinism
//!
//! All math is integer, on inputs clamped to [`MAX_COORD`] first, so no
//! command sequence panics or overflows. `apply` answers [`Outcome::Noop`]
//! when nothing changed, and an error leaves the state untouched. Replaying a
//! command log on a fresh `Wm::new` with the same area reproduces
//! [`Wm::state_hash`], an FNV-1a hash of a canonical encoding of the whole
//! logical state.
//!
//! # Example
//!
//! ```
//! use wm::{Cmd, Outcome, Rect, Snap, State, WinId, Wm};
//!
//! let mut wm = Wm::new(Rect::new(0, 0, 1920, 1080));
//! assert_eq!(wm.apply(Cmd::Open { size: None }), Ok(Outcome::Opened(WinId(1))));
//! wm.apply(Cmd::Open { size: None }).unwrap();
//! let rects: Vec<Rect> = wm.layout().iter().map(|p| p.rect).collect();
//! assert_eq!(rects, [Rect::new(320, 180, 1280, 720), Rect::new(348, 208, 1280, 720)]);
//! assert_eq!(wm.focused(), Some(WinId(2)));
//!
//! wm.apply(Cmd::SnapTo { win: WinId(1), snap: Snap::Left }).unwrap();
//! assert_eq!(wm.layout()[0].rect, Rect::new(0, 0, 960, 1080));
//! wm.apply(Cmd::Minimize(WinId(2))).unwrap();
//! assert_eq!(wm.focused(), Some(WinId(1)));
//! assert_eq!(wm.windows(), [(WinId(1), State::Normal), (WinId(2), State::Minimized)]);
//! ```

#![forbid(unsafe_code)]

use std::{fmt, mem};

/// Smallest width of a normal rect.
pub const MIN_W: i32 = 320;
/// Smallest height of a normal rect.
pub const MIN_H: i32 = 200;
/// Height of the titlebar the shell draws. A window's top edge stays at
/// least this far above the area's bottom, so its titlebar can be grabbed.
pub const TITLE_H: i32 = 40;
/// Offset, right and down, between new windows that would open on top of
/// each other.
pub const CASCADE: i32 = 28;
/// How much of a window's width always stays inside the area.
pub const VISIBLE_W: i32 = 64;
/// Input bound: positions are clamped to `±MAX_COORD`, sizes to `[0, MAX_COORD]`.
pub const MAX_COORD: i32 = 1 << 20;

/// A window id, handed out 1, 2, 3, ... by [`Cmd::Open`] and never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WinId(pub u32);

/// An integer rectangle: top-left corner, width and height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

impl Rect {
    /// Builds a rectangle.
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    fn normalized(self) -> Rect {
        let len = |v: i32| v.clamp(0, MAX_COORD);
        Rect::new(coord(self.x), coord(self.y), len(self.w), len(self.h))
    }
}

/// A position clamped to `±MAX_COORD`.
fn coord(v: i32) -> i32 {
    v.clamp(-MAX_COORD, MAX_COORD)
}

/// A window's state, as the dock shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Visible at its normal rect, or snapped.
    Normal,
    /// Visible, filling the area.
    Maximized,
    /// Hidden from the layout until shown again.
    Minimized,
}

/// A half or quarter of the area a window can snap to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// The left half.
    Left,
    /// The right half.
    Right,
    /// The top-left quarter.
    TopLeft,
    /// The top-right quarter.
    TopRight,
    /// The bottom-left quarter.
    BottomLeft,
    /// The bottom-right quarter.
    BottomRight,
}

impl Snap {
    /// The part of `area` (normalized as [`Cmd::SetArea`] would) this snap
    /// fills: the left column is `area.w / 2` wide and the right one takes the
    /// rest; the top row is `area.h / 2` high and the bottom one the rest.
    /// The shell can draw it as the drop preview while a drag hovers an edge.
    pub fn rect(self, area: Rect) -> Rect {
        let a = area.normalized();
        let (lw, th) = (a.w / 2, a.h / 2);
        let (x, w) = match self {
            Snap::Left | Snap::TopLeft | Snap::BottomLeft => (a.x, lw),
            Snap::Right | Snap::TopRight | Snap::BottomRight => (a.x + lw, a.w - lw),
        };
        let (y, h) = match self {
            Snap::Left | Snap::Right => (a.y, a.h),
            Snap::TopLeft | Snap::TopRight => (a.y, th),
            Snap::BottomLeft | Snap::BottomRight => (a.y + th, a.h - th),
        };
        Rect::new(x, y, w, h)
    }
}

/// A command: the only way to change a [`Wm`] (see [`Wm::apply`]). A command
/// naming a window that is not open fails with [`WmError::UnknownWindow`].
///
/// "Shows" below means: a minimized window goes back on top of the stack,
/// focused. Commands never restack a visible window unless they say so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Open a window, focused on top. Each side of `size` (by default two
    /// thirds of the area's) is clamped to at least the minimum and at most
    /// the area. The window opens centered, stepping [`CASCADE`] px right and
    /// down while a visible window already has that top-left corner, and back
    /// to the area's corner plus `CASCADE` when the next step would leave
    /// the area.
    Open {
        /// Width and height, or `None` for the default.
        size: Option<(i32, i32)>,
    },
    /// Close a window. If it was focused, the new top of the stack is.
    Close(WinId),
    /// Focus a window: raise it to the top, showing it if minimized (in the
    /// mode it was minimized in).
    Focus(WinId),
    /// Move a window's top-left corner to `(x, y)`, then keep it visible. A
    /// maximized or snapped window first gets its normal size back and
    /// becomes free. A minimized window ignores it.
    Move {
        /// The window.
        win: WinId,
        /// New left edge.
        x: i32,
        /// New top edge.
        y: i32,
    },
    /// Set a free or snapped window's rect, clamped to the minimum and the
    /// area's size and kept visible. It becomes the normal rect, and the
    /// window becomes free. Maximized and minimized windows ignore it.
    Resize {
        /// The window.
        win: WinId,
        /// Its new rect.
        rect: Rect,
    },
    /// Fill the area, remembering the normal rect. Shows the window.
    Maximize(WinId),
    /// Back to the normal rect, neither maximized nor snapped. Shows the window.
    Restore(WinId),
    /// Restore a maximized window; maximize any other. Shows the window.
    ToggleMaximize(WinId),
    /// Hide a window from the layout, keeping its mode. If it was focused,
    /// the new top of the stack is.
    Minimize(WinId),
    /// Snap to a half or quarter of the area, remembering the normal rect.
    /// Shows the window.
    SnapTo {
        /// The window.
        win: WinId,
        /// Where it snaps.
        snap: Snap,
    },
    /// Send the top window to the bottom, focusing the new top.
    FocusNext,
    /// Raise the bottom window to the top, focusing it.
    FocusPrev,
    /// Set the screen area. Normal rects are clamped to it and kept visible
    /// in it again; maximized and snapped rects follow it.
    SetArea(Rect),
}

/// What a successful [`Wm::apply`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A window was opened with this id.
    Opened(WinId),
    /// The state changed.
    Changed,
    /// The command was valid but changed nothing.
    Noop,
}

/// Why [`Wm::apply`] refused a command. The state is left untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmError {
    /// No open window has this id.
    UnknownWindow(WinId),
}

impl fmt::Display for WmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            WmError::UnknownWindow(w) => write!(f, "unknown window {}", w.0),
        }
    }
}

impl std::error::Error for WmError {}

/// Where one visible window goes on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The window.
    pub win: WinId,
    /// Its rectangle: the area when maximized, the snap's part of the area
    /// when snapped, its normal rect otherwise.
    pub rect: Rect,
    /// [`State::Normal`] or [`State::Maximized`]; minimized windows have no
    /// placement.
    pub state: State,
    /// Where it is snapped, if it is.
    pub snap: Option<Snap>,
    /// Whether it has the focus (it is then the top window).
    pub focused: bool,
}

/// How a window is drawn when visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Free,
    Snapped(Snap),
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Win {
    id: WinId,
    normal: Rect,
    mode: Mode,
}

/// A size clamped to at least the minimum and at most the area (the minimum
/// wins in an area smaller than it).
fn clamp_size(w: i32, h: i32, a: Rect) -> (i32, i32) {
    (w.clamp(MIN_W, MIN_W.max(a.w)), h.clamp(MIN_H, MIN_H.max(a.h)))
}

/// A normal rect for `r` in area `a`: sized by [`clamp_size`], then moved the
/// least that keeps it visible. Bounds can only conflict vertically, in an
/// area lower than `TITLE_H`; the top edge then sits on the area's top.
/// Inputs are normalized, so no sum here leaves `±2^22`.
fn fit(r: Rect, a: Rect) -> Rect {
    let (w, h) = clamp_size(r.w, r.h, a);
    let x = r.x.min(a.x + a.w - VISIBLE_W).max(a.x + VISIBLE_W - w);
    let y = r.y.min(a.y + a.h - TITLE_H).max(a.y);
    Rect::new(x, y, w, h)
}

/// FNV-1a, 64 bit.
struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    fn len(&mut self, v: usize) {
        self.bytes(&(v as u64).to_le_bytes());
    }

    fn rect(&mut self, r: Rect) {
        for v in [r.x, r.y, r.w, r.h] {
            self.bytes(&v.to_le_bytes());
        }
    }
}

/// The window manager: windows, their geometry, stacking and focus.
#[derive(Clone, Debug)]
pub struct Wm {
    area: Rect,
    next: u32,
    /// Every open window, by id (which is creation order).
    wins: Vec<Win>,
    /// The visible windows, bottom to top; the top one is focused.
    stack: Vec<WinId>,
}

impl Wm {
    /// An empty window manager over `area`, normalized as [`Cmd::SetArea`] would.
    pub fn new(area: Rect) -> Wm {
        Wm { area: area.normalized(), next: 1, wins: Vec::new(), stack: Vec::new() }
    }

    /// Applies one command; the only way to change a `Wm`.
    ///
    /// Returns [`Outcome::Opened`] from [`Cmd::Open`], [`Outcome::Changed`]
    /// when the state changed and [`Outcome::Noop`] when a valid command
    /// changed nothing. On error nothing changes. (After id `u32::MAX - 1`,
    /// ids have run out and `Open` is a `Noop` rather than reuse one.)
    pub fn apply(&mut self, cmd: Cmd) -> Result<Outcome, WmError> {
        let changed = match cmd {
            Cmd::Open { size } => return Ok(self.open(size)),
            Cmd::Close(win) => {
                let i = self.index(win)?;
                self.wins.remove(i);
                self.stack.retain(|&s| s != win);
                true
            }
            Cmd::Focus(win) => {
                self.index(win)?;
                self.raise(win)
            }
            Cmd::Move { win, x, y } => {
                let i = self.index(win)?;
                let n = self.wins[i].normal;
                let r = Rect::new(coord(x), coord(y), n.w, n.h);
                self.visible(win) && self.reshape(i, r)
            }
            Cmd::Resize { win, rect } => {
                let i = self.index(win)?;
                let free = self.visible(win) && self.wins[i].mode != Mode::Max;
                free && self.reshape(i, rect.normalized())
            }
            Cmd::Maximize(win) => self.set_mode(win, |_| Mode::Max)?,
            Cmd::Restore(win) => self.set_mode(win, |_| Mode::Free)?,
            Cmd::ToggleMaximize(win) => {
                self.set_mode(win, |m| if m == Mode::Max { Mode::Free } else { Mode::Max })?
            }
            Cmd::Minimize(win) => {
                self.index(win)?;
                let before = self.stack.len();
                self.stack.retain(|&s| s != win);
                self.stack.len() != before
            }
            Cmd::SnapTo { win, snap } => self.set_mode(win, |_| Mode::Snapped(snap))?,
            Cmd::FocusNext | Cmd::FocusPrev if self.stack.len() < 2 => false,
            Cmd::FocusNext => {
                self.stack.rotate_right(1);
                true
            }
            Cmd::FocusPrev => {
                self.stack.rotate_left(1);
                true
            }
            Cmd::SetArea(rect) => {
                let area = rect.normalized();
                let changed = mem::replace(&mut self.area, area) != area;
                for w in &mut self.wins {
                    w.normal = fit(w.normal, area);
                }
                changed
            }
        };
        Ok(if changed { Outcome::Changed } else { Outcome::Noop })
    }

    /// The visible windows bottom to top, as the compositor draws them.
    /// Minimized windows are left out; the last placement is the focused one.
    pub fn layout(&self) -> Vec<Placement> {
        let top = self.focused();
        let place = |&id: &WinId| {
            let w = *self.win(id)?;
            let (state, snap) = match w.mode {
                Mode::Free => (State::Normal, None),
                Mode::Snapped(s) => (State::Normal, Some(s)),
                Mode::Max => (State::Maximized, None),
            };
            let rect = self.rect_of(w);
            Some(Placement { win: id, rect, state, snap, focused: Some(id) == top })
        };
        self.stack.iter().filter_map(place).collect()
    }

    /// Every open window in creation order with its state, as a dock lists them.
    pub fn windows(&self) -> Vec<(WinId, State)> {
        let state = |w: &Win| match w.mode {
            _ if !self.visible(w.id) => State::Minimized,
            Mode::Max => State::Maximized,
            Mode::Free | Mode::Snapped(_) => State::Normal,
        };
        self.wins.iter().map(|w| (w.id, state(w))).collect()
    }

    /// The focused window: the top of the stack, if any window is visible.
    pub fn focused(&self) -> Option<WinId> {
        self.stack.last().copied()
    }

    /// The screen area.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// The normal rect of `win`: where it is when free, and the geometry
    /// [`Cmd::Restore`] returns to. `None` if it is not open.
    pub fn normal_rect(&self, win: WinId) -> Option<Rect> {
        self.win(win).map(|w| w.normal)
    }

    /// FNV-1a 64 over a canonical little-endian encoding of the whole logical
    /// state: the area, the next id, every window in creation order (id,
    /// normal rect, mode), then the stack bottom to top; lists are
    /// length-prefixed. Equal states hash equally however they were reached.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        h.rect(self.area);
        h.u32(self.next);
        h.len(self.wins.len());
        for w in &self.wins {
            h.u32(w.id.0);
            h.rect(w.normal);
            match w.mode {
                Mode::Free => h.bytes(&[0]),
                Mode::Snapped(s) => h.bytes(&[1, s as u8]),
                Mode::Max => h.bytes(&[2]),
            }
        }
        h.len(self.stack.len());
        for id in &self.stack {
            h.u32(id.0);
        }
        h.0
    }

    fn index(&self, win: WinId) -> Result<usize, WmError> {
        self.wins.binary_search_by_key(&win, |w| w.id).map_err(|_| WmError::UnknownWindow(win))
    }

    fn win(&self, win: WinId) -> Option<&Win> {
        self.index(win).ok().map(|i| &self.wins[i])
    }

    fn visible(&self, win: WinId) -> bool {
        self.stack.contains(&win)
    }

    /// Where a window is drawn when visible.
    fn rect_of(&self, w: Win) -> Rect {
        match w.mode {
            Mode::Free => w.normal,
            Mode::Snapped(s) => s.rect(self.area),
            Mode::Max => self.area,
        }
    }

    /// Puts `win` on top of the stack; whether that changed anything.
    fn raise(&mut self, win: WinId) -> bool {
        if self.focused() == Some(win) {
            return false;
        }
        self.stack.retain(|&s| s != win);
        self.stack.push(win);
        true
    }

    /// Frees window `i` at the normal rect `fit` makes of `r`.
    fn reshape(&mut self, i: usize, r: Rect) -> bool {
        let w = self.wins[i];
        let new = Win { normal: fit(r, self.area), mode: Mode::Free, ..w };
        mem::replace(&mut self.wins[i], new) != new
    }

    /// Sets the mode of `win` to `f` of its mode, showing it if minimized.
    fn set_mode(&mut self, win: WinId, f: impl FnOnce(Mode) -> Mode) -> Result<bool, WmError> {
        let i = self.index(win)?;
        let mode = f(self.wins[i].mode);
        let changed = mem::replace(&mut self.wins[i].mode, mode) != mode;
        let shown = !self.visible(win);
        if shown {
            self.stack.push(win);
        }
        Ok(changed || shown)
    }

    fn open(&mut self, size: Option<(i32, i32)>) -> Outcome {
        let Some(next) = self.next.checked_add(1) else {
            return Outcome::Noop;
        };
        let id = WinId(mem::replace(&mut self.next, next));
        let a = self.area;
        // a.w and a.h are at most MAX_COORD, so doubling them cannot overflow.
        let (dw, dh) = size.unwrap_or((a.w * 2 / 3, a.h * 2 / 3));
        let (w, h) = clamp_size(dw, dh, a);
        let (x, y) = self.cascade(w, h);
        self.wins.push(Win { id, normal: fit(Rect::new(x, y, w, h), a), mode: Mode::Free });
        self.stack.push(id);
        Outcome::Opened(id)
    }

    /// The top-left corner for a new `w` x `h` window (see [`Cmd::Open`]).
    /// Each run of steps visits distinct corners, each taken by a different
    /// window, and there are at most two runs, so this ends.
    fn cascade(&self, w: i32, h: i32) -> (i32, i32) {
        let a = self.area;
        let shown = self.stack.iter().filter_map(|&id| self.win(id));
        let corners: Vec<(i32, i32)> =
            shown.map(|&w| self.rect_of(w)).map(|r| (r.x, r.y)).collect();
        let taken = |p: (i32, i32)| corners.contains(&p);
        let fits = |(x, y): (i32, i32)| x + w <= a.x + a.w && y + h <= a.y + a.h;
        let mut p = (a.x + (a.w - w) / 2, a.y + (a.h - h) / 2);
        let mut wrapped = false;
        while taken(p) {
            let mut q = (p.0 + CASCADE, p.1 + CASCADE);
            if !fits(q) {
                q = (a.x + CASCADE, a.y + CASCADE);
                if wrapped || !fits(q) {
                    break;
                }
                wrapped = true;
            }
            p = q;
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_run_out_instead_of_wrapping() {
        let mut w = Wm::new(Rect::new(0, 0, 1920, 1080));
        w.next = u32::MAX - 1;
        assert_eq!(w.apply(Cmd::Open { size: None }), Ok(Outcome::Opened(WinId(u32::MAX - 1))));
        let h = w.state_hash();
        assert_eq!(w.apply(Cmd::Open { size: None }), Ok(Outcome::Noop));
        assert_eq!((w.state_hash(), w.wins.len()), (h, 1));
    }

    #[test]
    fn fnv_matches_the_reference_vector() {
        let mut f = Fnv(0xcbf2_9ce4_8422_2325);
        f.bytes(b"a");
        assert_eq!(f.0, 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn hash_ignores_storage_artifacts() {
        let mut w = Wm::new(Rect::new(0, 0, 800, 600));
        w.apply(Cmd::Open { size: None }).unwrap();
        let h = w.state_hash();
        w.wins.reserve(64);
        w.stack.reserve(64);
        assert_eq!(w.state_hash(), h);
        assert_eq!(w.clone().state_hash(), h);
    }

    #[test]
    fn crowded_cascades_end() {
        // Area-sized windows can never step: every one opens on the center.
        let mut w = Wm::new(Rect::new(0, 0, 640, 480));
        for _ in 0..300 {
            w.apply(Cmd::Open { size: Some((640, 480)) }).unwrap();
        }
        assert!(w.layout().iter().all(|p| p.rect == Rect::new(0, 0, 640, 480)));
        // Small windows fill both runs, then pile up on the last corner.
        let mut w = Wm::new(Rect::new(0, 0, 400, 260));
        for _ in 0..12 {
            w.apply(Cmd::Open { size: None }).unwrap();
        }
        let corners: Vec<(i32, i32)> = w.layout().iter().map(|p| (p.rect.x, p.rect.y)).collect();
        let mut want = vec![(40, 30), (68, 58), (28, 28)];
        want.resize(12, (56, 56));
        assert_eq!(corners, want);
    }
}

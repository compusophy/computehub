//! Deterministic floating window manager for compusophyOS.
//!
//! [`Wm`] owns window geometry, stacking and focus, nothing else: no rendering, clocks, floats or
//! hash-ordered collections. [`Wm::apply`] is the only mutator; the compositor draws
//! [`Wm::layout`] bottom to top.
//!
//! Each window has a *normal rect* (its free geometry, which [`Cmd::Restore`] returns to) and a
//! mode: free, snapped ([`Snap`]) or maximized; snapped and maximized rects derive from the area.
//! The stack holds the visible windows bottom to top, and its top is focused. Ids start at 1 and
//! are never reused. Normal rects are at least [`MIN_W`] x [`MIN_H`], at most the area's size
//! where it is larger, keep [`VISIBLE_W`] px of width and the top edge inside the area. All math
//! is integer on inputs clamped to [`MAX_COORD`], so nothing overflows; an error changes nothing;
//! replaying the same commands on a `Wm::new` of the same area reproduces [`Wm::state_hash`].

#![forbid(unsafe_code)]

use std::mem;

/// Smallest width of a normal rect.
pub const MIN_W: i32 = 320;
/// Smallest height of a normal rect.
pub const MIN_H: i32 = 200;
/// Titlebar height: a window's top edge stays this far above the area's bottom.
pub const TITLE_H: i32 = 40;
/// Offset, right and down, of a new window from the focused one.
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
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Positions clamped to `±MAX_COORD`, sizes to `[0, MAX_COORD]`.
    fn normalized(self) -> Rect {
        let c = |v: i32, lo: i32| v.max(lo).min(MAX_COORD);
        Rect::new(c(self.x, -MAX_COORD), c(self.y, -MAX_COORD), c(self.w, 0), c(self.h, 0))
    }
}

/// A window's state, as the dock shows it: `Normal` (free or snapped),
/// `Maximized`, or `Minimized` (hidden from the layout until shown again).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Normal,
    Maximized,
    Minimized,
}

/// A half or quarter of the area a window can snap to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Snap {
    /// The part of `area` (normalized) this snap fills: the left column is
    /// `w / 2` wide, the top row `h / 2` high; the right and bottom take the rest.
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

/// A command, the only way to change a [`Wm`]; naming a window that is not open
/// fails. "Shows" means a minimized window goes back on top, focused; no
/// command restacks a visible window unless it says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Open a window, focused on top, [`CASCADE`] px right of and below the focused window's
    /// top-left; where that crosses the area's right or bottom edge, at the area's top-left plus
    /// `CASCADE`; centered if no window is visible or neither fits. Each side of `size` (by
    /// default two thirds of the area's) is clamped to the minimum and the area.
    Open { size: Option<(i32, i32)> },
    /// Close a window. If it was focused, the new top of the stack is.
    Close(WinId),
    /// Raise a window to the top, focused, showing it if minimized.
    Focus(WinId),
    /// Move a window's top-left to `(x, y)`, kept visible; a maximized or snapped
    /// window becomes free at its normal size. A minimized window ignores it.
    Move { win: WinId, x: i32, y: i32 },
    /// Set a free or snapped window's normal rect (clamped, kept visible) and
    /// free it. Maximized and minimized windows ignore it.
    Resize { win: WinId, rect: Rect },
    /// Fill the area, remembering the normal rect. Shows the window.
    Maximize(WinId),
    /// Back to the normal rect, neither maximized nor snapped. Shows the window.
    Restore(WinId),
    /// Restore a maximized window; maximize any other. Shows the window.
    ToggleMaximize(WinId),
    /// Hide a window from the layout, keeping its mode.
    Minimize(WinId),
    /// Snap to a half or quarter of the area, remembering the normal rect. Shows the window.
    SnapTo { win: WinId, snap: Snap },
    /// Send the top window to the bottom, focusing the new top.
    FocusNext,
    /// Raise the bottom window to the top, focusing it.
    FocusPrev,
    /// Set the screen area. If it changed, every normal rect shrinks to it (not
    /// below the minimum) and moves the least to lie inside it, as far as it
    /// fits; maximized and snapped rects follow the area.
    SetArea(Rect),
}

/// What a successful [`Wm::apply`] did: opened a window with this id, changed
/// the state, or nothing (`Noop`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Opened(WinId),
    Changed,
    Noop,
}

/// Why [`Wm::apply`] refused a command (no open window has this id); the state is untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmError {
    UnknownWindow(WinId),
}

/// Where one visible window goes: `rect` is the area when maximized, the snap's part of it when
/// snapped, else the normal rect; `state` is never `Minimized`; `focused` holds for the top only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub win: WinId,
    pub rect: Rect,
    pub state: State,
    pub snap: Option<Snap>,
    pub focused: bool,
}

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

/// A normal rect for normalized `r` in `a`: each side clamped to at least the minimum and at most
/// the area (the minimum wins), then moved the least that keeps `VISIBLE_W` x `TITLE_H` of it
/// inside, or, if `whole`, all of it that fits. The top edge never leaves the area's top.
fn fit(r: Rect, a: Rect, whole: bool) -> Rect {
    let (w, h) = (r.w.max(MIN_W).min(MIN_W.max(a.w)), r.h.max(MIN_H).min(MIN_H.max(a.h)));
    let (vw, vh) = if whole { (w.min(a.w), h.min(a.h)) } else { (0, 0) };
    let (vw, vh) = (vw.max(VISIBLE_W), vh.max(TITLE_H));
    let x = r.x.min(a.x + a.w - vw).max(a.x + vw - w);
    Rect::new(x, r.y.min(a.y + a.h - vh).max(a.y), w, h)
}

/// FNV-1a 64 of `bytes`, continuing from `h`.
fn fnv(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The window manager: windows, their geometry, stacking and focus.
#[derive(Clone, Debug)]
pub struct Wm {
    area: Rect,
    next: u32,
    /// Every open window, in id (creation) order.
    wins: Vec<Win>,
    /// The visible windows, bottom to top; the top one is focused.
    stack: Vec<WinId>,
}

impl Wm {
    /// An empty window manager over `area`, normalized.
    pub fn new(area: Rect) -> Wm {
        Wm { area: area.normalized(), next: 1, wins: Vec::new(), stack: Vec::new() }
    }

    /// Applies one command: `Noop` if it changed nothing. Once ids run out
    /// (after `u32::MAX - 1`), `Open` is a `Noop` rather than reuse one.
    pub fn apply(&mut self, cmd: Cmd) -> Result<Outcome, WmError> {
        let changed = match cmd {
            Cmd::Open { size } => return Ok(self.open(size)),
            Cmd::Close(win) => {
                self.wins.remove(self.index(win)?);
                self.hide(win);
                true
            }
            Cmd::Focus(win) => {
                let raised = self.index(win).map(|_| self.focused() != Some(win))?;
                self.hide(win);
                self.stack.push(win);
                raised
            }
            Cmd::Move { win, x, y } => {
                let i = self.index(win)?;
                self.stack.contains(&win) && self.reshape(i, Rect { x, y, ..self.wins[i].normal })
            }
            Cmd::Resize { win, rect } => {
                let i = self.index(win)?;
                self.stack.contains(&win) && self.wins[i].mode != Mode::Max && self.reshape(i, rect)
            }
            Cmd::Maximize(win) => self.set_mode(win, Mode::Max)?,
            Cmd::Restore(win) => self.set_mode(win, Mode::Free)?,
            Cmd::ToggleMaximize(win) => {
                let max = self.wins[self.index(win)?].mode == Mode::Max;
                self.set_mode(win, if max { Mode::Free } else { Mode::Max })?
            }
            Cmd::Minimize(win) => self.index(win).map(|_| self.hide(win))?,
            Cmd::SnapTo { win, snap } => self.set_mode(win, Mode::Snapped(snap))?,
            Cmd::FocusNext | Cmd::FocusPrev => {
                let n = self.stack.len();
                // Next sends the top to the bottom; Prev raises the bottom.
                let k = if cmd == Cmd::FocusNext { n.saturating_sub(1) } else { n.min(1) };
                self.stack.rotate_left(k);
                n > 1
            }
            Cmd::SetArea(rect) if rect.normalized() == self.area => false,
            Cmd::SetArea(rect) => {
                let a = rect.normalized();
                self.area = a;
                self.wins.iter_mut().for_each(|w| w.normal = fit(w.normal, a, true));
                true
            }
        };
        Ok(if changed { Outcome::Changed } else { Outcome::Noop })
    }

    /// The visible windows bottom to top, as the compositor draws them; the last one is focused.
    pub fn layout(&self) -> Vec<Placement> {
        let top = self.focused();
        let place = |w: &Win| {
            let (state, snap, rect) = match w.mode {
                Mode::Free => (State::Normal, None, w.normal),
                Mode::Snapped(s) => (State::Normal, Some(s), s.rect(self.area)),
                Mode::Max => (State::Maximized, None, self.area),
            };
            Placement { win: w.id, rect, state, snap, focused: Some(w.id) == top }
        };
        self.stack.iter().filter_map(|&id| self.win(id)).map(place).collect()
    }

    /// Every open window in creation order with its state, as a dock lists them.
    pub fn windows(&self) -> Vec<(WinId, State)> {
        let state = |w: &Win| match w.mode {
            _ if !self.stack.contains(&w.id) => State::Minimized,
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

    /// The normal rect of `win`, or `None` if it is not open.
    pub fn normal_rect(&self, win: WinId) -> Option<Rect> {
        self.win(win).map(|w| w.normal)
    }

    /// FNV-1a 64 over a canonical little-endian encoding of the logical state:
    /// area, next id, every window in creation order (id, normal rect, mode),
    /// then the stack bottom to top; lists are length-prefixed.
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325;
        let mut put = |bytes: &[u8]| h = fnv(h, bytes);
        let rect = |r: Rect| [r.x, r.y, r.w, r.h].map(i32::to_le_bytes);
        put(rect(self.area).as_flattened());
        put(&self.next.to_le_bytes());
        put(&(self.wins.len() as u64).to_le_bytes());
        for w in &self.wins {
            put(&w.id.0.to_le_bytes());
            put(rect(w.normal).as_flattened());
            match w.mode {
                Mode::Free => put(&[0]),
                Mode::Snapped(s) => put(&[1, s as u8]),
                Mode::Max => put(&[2]),
            }
        }
        put(&(self.stack.len() as u64).to_le_bytes());
        self.stack.iter().for_each(|id| put(&id.0.to_le_bytes()));
        h
    }

    fn index(&self, win: WinId) -> Result<usize, WmError> {
        self.wins.binary_search_by_key(&win, |w| w.id).map_err(|_| WmError::UnknownWindow(win))
    }

    fn win(&self, win: WinId) -> Option<&Win> {
        self.index(win).ok().map(|i| &self.wins[i])
    }

    /// Takes `win` off the stack (it is there at most once); whether it was on it.
    fn hide(&mut self, win: WinId) -> bool {
        let at = self.stack.iter().position(|&s| s == win);
        at.map(|i| self.stack.remove(i)).is_some()
    }

    /// Frees window `i` at the normal rect `fit` makes of `r`.
    fn reshape(&mut self, i: usize, r: Rect) -> bool {
        let normal = fit(r.normalized(), self.area, false);
        let new = Win { normal, mode: Mode::Free, ..self.wins[i] };
        mem::replace(&mut self.wins[i], new) != new
    }

    /// Sets the mode of `win`, showing it if minimized.
    fn set_mode(&mut self, win: WinId, mode: Mode) -> Result<bool, WmError> {
        let i = self.index(win)?;
        let changed = mem::replace(&mut self.wins[i].mode, mode) != mode;
        let shown = !self.stack.contains(&win);
        self.stack.extend(shown.then_some(win));
        Ok(changed || shown)
    }

    fn open(&mut self, size: Option<(i32, i32)>) -> Outcome {
        let Some(next) = self.next.checked_add(1) else { return Outcome::Noop };
        let id = WinId(mem::replace(&mut self.next, next));
        let a = self.area;
        // a.w and a.h are at most MAX_COORD, so doubling them cannot overflow.
        let (w, h) = size.unwrap_or((a.w * 2 / 3, a.h * 2 / 3));
        let Rect { w, h, .. } = fit(Rect { w, h, ..a }, a, false); // the size it will get
        let top = self.layout().last().map(|p| p.rect);
        let steps = top.map(|t| [(t.x + CASCADE, t.y + CASCADE), (a.x + CASCADE, a.y + CASCADE)]);
        let fits = |&(x, y): &(i32, i32)| x + w <= a.x + a.w && y + h <= a.y + a.h;
        let center = (a.x + (a.w - w) / 2, a.y + (a.h - h) / 2);
        let (x, y) = steps.into_iter().flatten().find(fits).unwrap_or(center);
        self.wins.push(Win { id, normal: fit(Rect::new(x, y, w, h), a, false), mode: Mode::Free });
        self.stack.push(id);
        Outcome::Opened(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internals() {
        // FNV-1a's reference vector.
        assert_eq!(fnv(0xcbf2_9ce4_8422_2325, b"a"), 0xaf63_dc4c_8601_ec8c);
        // Spare capacity and clones leave the hash alone.
        let mut w = Wm::new(Rect::new(0, 0, 800, 600));
        w.apply(Cmd::Open { size: None }).unwrap();
        let h = w.state_hash();
        w.wins.reserve(64);
        w.stack.reserve(64);
        assert_eq!((w.state_hash(), w.clone().state_hash()), (h, h));
        // Ids run out instead of wrapping.
        w.next = u32::MAX - 1;
        assert_eq!(w.apply(Cmd::Open { size: None }), Ok(Outcome::Opened(WinId(u32::MAX - 1))));
        let h = w.state_hash();
        assert_eq!(w.apply(Cmd::Open { size: None }), Ok(Outcome::Noop));
        assert_eq!((w.state_hash(), w.wins.len()), (h, 2));
    }
}

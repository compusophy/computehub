//! Deterministic tiling window manager for compusophyOS.
//!
//! [`Wm`] owns window placement and nothing else: no rendering, no browser,
//! no clocks, no floating point, no hash-ordered collections. Input handling
//! turns keys into [`Cmd`]s, [`Wm::apply`] (the only mutator) applies them,
//! and the compositor draws whatever [`Wm::layout`] returns.
//!
//! # Model
//!
//! - A fixed set of workspaces, one of them active. Each holds a binary
//!   tiling tree (leaves are windows; splits carry an [`Axis`] and the first
//!   child's share as a fixed-point ratio out of [`RATIO_ONE`]), a floating
//!   stack ordered bottom to top, and a focus history ordered by last focus.
//!   A workspace's focused window is always the last entry of its history.
//! - Window ids are global, start at 1, grow by one per open and are never
//!   reused.
//! - A new tiled window splits the most recently focused tiled window along
//!   its longer side (ties go side by side) and takes the second half.
//!   Closing a tiled window gives its space to its sibling subtree.
//! - Tiling fills the area inset by the outer gap; every split leaves the
//!   inner gap between its children. Floating windows are at least
//!   [`FLOAT_MIN`] px in each dimension and keep that much on screen.
//!
//! # Determinism
//!
//! All math is integer and ratio products are taken in `i64`. Inputs are
//! clamped to [`MAX_COORD`] first, so no command sequence panics or
//! overflows. `apply` answers [`Outcome::Noop`] when nothing changed, and an
//! error leaves the state untouched. Replaying a command log on a fresh
//! `Wm::new` with the same arguments reproduces [`Wm::state_hash`], an
//! FNV-1a hash of a canonical encoding of the whole logical state.
//!
//! # Example
//!
//! ```
//! use wm::{Cmd, Gaps, Outcome, Rect, WinId, Wm};
//!
//! let mut wm = Wm::new(Rect::new(0, 0, 1920, 1080), Gaps { outer: 8, inner: 8 }, 4);
//! assert_eq!(wm.apply(Cmd::Open { floating: false }), Ok(Outcome::Opened(WinId(1))));
//! wm.apply(Cmd::Open { floating: false }).unwrap();
//! let rects: Vec<Rect> = wm.layout().iter().map(|p| p.rect).collect();
//! assert_eq!(rects, [Rect::new(8, 8, 948, 1064), Rect::new(964, 8, 948, 1064)]);
//! assert_eq!(wm.focused(), Some(WinId(2)));
//! ```

#![forbid(unsafe_code)]

use std::{fmt, mem};

/// Fixed-point scale of split ratios: a ratio is the first child's share of this.
pub const RATIO_ONE: u32 = 65_536;
/// Smallest split ratio; ratios stay within `[RATIO_MIN, RATIO_ONE - RATIO_MIN]`.
pub const RATIO_MIN: u32 = 3_277;
/// Resizing keeps both children of a split at least this long when possible.
pub const MIN_TILE: i32 = 48;
/// Floating windows are at least this wide and tall and keep this much on screen.
pub const FLOAT_MIN: i32 = 48;
/// Input bound: positions are clamped to `±MAX_COORD`, sizes and gaps to `[0, MAX_COORD]`.
pub const MAX_COORD: i32 = 1 << 20;
/// Most workspaces a [`Wm`] can have.
pub const MAX_WORKSPACES: usize = 64;

/// Parent index of a tree root.
const NONE: usize = usize::MAX;

/// A window id: global across workspaces, handed out 1, 2, 3, ... by
/// [`Cmd::Open`] and never reused.
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
        let pos = |v: i32| v.clamp(-MAX_COORD, MAX_COORD);
        let len = |v: i32| v.clamp(0, MAX_COORD);
        Rect::new(pos(self.x), pos(self.y), len(self.w), len(self.h))
    }

    /// Start and length along `axis`, widened for arithmetic.
    fn span(self, axis: Axis) -> (i64, i64) {
        match axis {
            Axis::Horizontal => (i64::from(self.x), i64::from(self.w)),
            Axis::Vertical => (i64::from(self.y), i64::from(self.h)),
        }
    }

    /// This rect with its start and length along `axis` replaced.
    fn with_span(self, axis: Axis, start: i64, len: i64) -> Rect {
        let (s, l) = (start as i32, len as i32);
        match axis {
            Axis::Horizontal => Rect { x: s, w: l, ..self },
            Axis::Vertical => Rect { y: s, h: l, ..self },
        }
    }
}

/// A direction for focus, move and resize commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// Toward smaller x.
    Left,
    /// Toward larger x.
    Right,
    /// Toward smaller y.
    Up,
    /// Toward larger y.
    Down,
}

impl Dir {
    fn axis(self) -> Axis {
        match self {
            Dir::Left | Dir::Right => Axis::Horizontal,
            Dir::Up | Dir::Down => Axis::Vertical,
        }
    }

    /// Right and Down point toward larger coordinates.
    fn forward(self) -> bool {
        matches!(self, Dir::Right | Dir::Down)
    }
}

/// How a split arranges its two children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Side by side: the first child left, the second right.
    Horizontal,
    /// Stacked: the first child on top, the second below.
    Vertical,
}

impl Axis {
    fn flip(self) -> Axis {
        match self {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        }
    }
}

/// Gaps in pixels: `outer` around the tiled area, `inner` between siblings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Gaps {
    /// Inset of the tiled area from the screen area, on all four sides.
    pub outer: i32,
    /// Space between the two children of every split.
    pub inner: i32,
}

impl Gaps {
    fn normalized(self) -> Gaps {
        let (outer, inner) = (
            self.outer.clamp(0, MAX_COORD),
            self.inner.clamp(0, MAX_COORD),
        );
        Gaps { outer, inner }
    }
}

/// A command: the only way to change a [`Wm`] (see [`Wm::apply`]).
///
/// Workspace indices are `usize`, whose width differs by platform. Valid ones
/// are below [`MAX_WORKSPACES`], so a wire format should carry them at a fixed
/// width and decode a value too wide for `usize` as `usize::MAX` (never
/// truncate it), so that every platform answers [`WmError::NoSuchWorkspace`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Open a new window on the active workspace and focus it.
    Open {
        /// Float it instead of tiling it.
        floating: bool,
    },
    /// Close a window on any workspace.
    Close(WinId),
    /// Focus a window, activating its workspace and raising it if floating.
    Focus(WinId),
    /// Focus the nearest window in a direction on the active workspace.
    FocusDir(Dir),
    /// Swap the focused tiled window with its nearest tiled neighbor.
    MoveDir(Dir),
    /// Move the divider on the `dir` side of the focused tiled window.
    Resize {
        /// The edge of the focused window that moves.
        dir: Dir,
        /// Pixels; positive grows the window toward `dir`.
        px: i32,
    },
    /// Float the focused tiled window, or tile the focused floating one.
    ToggleFloat,
    /// Flip the axis of the split holding the focused tiled window.
    ToggleOrientation,
    /// Move or resize a floating window; it is kept on screen.
    SetFloatRect {
        /// The floating window.
        win: WinId,
        /// Its new rectangle.
        rect: Rect,
    },
    /// Make a workspace active.
    SwitchWorkspace(usize),
    /// Move a window to another workspace, where it becomes the focus. An
    /// unknown window is reported before a bad workspace.
    MoveToWorkspace {
        /// The window.
        win: WinId,
        /// The destination workspace.
        ws: usize,
    },
    /// Set the screen area; floating windows are pulled back on screen.
    SetArea(Rect),
    /// Set the gaps.
    SetGaps(Gaps),
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

fn outcome(changed: bool) -> Outcome {
    if changed {
        Outcome::Changed
    } else {
        Outcome::Noop
    }
}

/// Why [`Wm::apply`] refused a command. The state is left untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmError {
    /// No live window has this id.
    UnknownWindow(WinId),
    /// The workspace index is out of range.
    NoSuchWorkspace(usize),
    /// The window is tiled, but the command needs a floating one.
    NotFloating(WinId),
}

impl fmt::Display for WmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            WmError::UnknownWindow(w) => write!(f, "unknown window {}", w.0),
            WmError::NoSuchWorkspace(i) => write!(f, "no such workspace {i}"),
            WmError::NotFloating(w) => write!(f, "window {} is not floating", w.0),
        }
    }
}

impl std::error::Error for WmError {}

/// Where one window goes on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The window.
    pub win: WinId,
    /// Its rectangle.
    pub rect: Rect,
    /// Whether it floats above the tiling.
    pub floating: bool,
    /// Whether it is its workspace's focused window.
    pub focused: bool,
}

/// A node of a tiling tree, stored in pre-order: a split is followed by its
/// first subtree, then its second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Leaf(WinId),
    Split(Axis, u32),
}

/// One workspace: tiling tree, floating stack (bottom to top) and focus
/// history (most recent last).
#[derive(Clone, Debug, Default)]
struct Space {
    tree: Vec<Node>,
    floats: Vec<(WinId, Rect)>,
    history: Vec<WinId>,
}

impl Space {
    fn leaf(&self, win: WinId) -> Option<usize> {
        self.tree.iter().position(|n| *n == Node::Leaf(win))
    }

    fn float(&self, win: WinId) -> Option<usize> {
        self.floats.iter().position(|f| f.0 == win)
    }

    fn focused(&self) -> Option<WinId> {
        self.history.last().copied()
    }

    /// Parent index of node `i`, `NONE` for the root.
    fn parent(&self, i: usize) -> usize {
        shape(&self.tree, Rect::default(), 0)
            .get(i)
            .map_or(NONE, |x| x.1)
    }
}

/// First child's length of a split with `avail` pixels to share.
fn a_len(avail: i64, ratio: u32) -> i64 {
    avail * i64::from(ratio) / i64::from(RATIO_ONE)
}

/// Rect and parent index (`NONE` for the root) of every node of a pre-order
/// tree laid out in `root`. Iterative, so deep trees cannot overflow the stack.
fn shape(tree: &[Node], root: Rect, inner: i32) -> Vec<(Rect, usize)> {
    let inner = i64::from(inner);
    let mut out = Vec::with_capacity(tree.len());
    let mut pending = vec![(root, NONE)];
    for (i, node) in tree.iter().enumerate() {
        let (r, parent) = pending.pop().unwrap_or((Rect::default(), NONE));
        out.push((r, parent));
        if let Node::Split(axis, ratio) = *node {
            let (start, e) = r.span(axis);
            let avail = (e - inner).max(0);
            let a = a_len(avail, ratio);
            let b_start = start + e.min(a + inner);
            pending.push((r.with_span(axis, b_start, avail - a), i));
            pending.push((r.with_span(axis, start, a), i));
        }
    }
    out
}

/// Clamps a floating rect to at least `FLOAT_MIN` square and at most the
/// area's size, with at least `FLOAT_MIN` px of it inside the area.
fn keep_visible(r: Rect, area: Rect) -> Rect {
    let w = r.w.clamp(FLOAT_MIN, FLOAT_MIN.max(area.w));
    let h = r.h.clamp(FLOAT_MIN, FLOAT_MIN.max(area.h));
    let fit = |pos: i32, len: i32, start: i32, extent: i32| {
        let (lo, hi) = (start + FLOAT_MIN - len, start + extent - FLOAT_MIN);
        if lo > hi { start } else { pos.clamp(lo, hi) }
    };
    let x = fit(r.x, w, area.x, area.w);
    let y = fit(r.y, h, area.y, area.h);
    Rect::new(x, y, w, h)
}

/// FocusDir rank of candidate `c` seen from `f`, smaller is better; `None`
/// when `c` is not entirely beyond `f` in `dir` or shares no perpendicular pixel.
fn rank(f: Rect, c: Rect, dir: Dir) -> Option<(i64, i64, i64)> {
    let (along, perp) = (dir.axis(), dir.axis().flip());
    let ((fa, fl), (ca, cl)) = (f.span(along), c.span(along));
    let gap = if dir.forward() {
        ca - (fa + fl)
    } else {
        fa - (ca + cl)
    };
    let ((fp, fq), (cp, cq)) = (f.span(perp), c.span(perp));
    let overlap = (fp + fq).min(cp + cq) - fp.max(cp);
    if gap < 0 || overlap < 1 {
        return None;
    }
    Some((gap, -overlap, ((2 * cp + cq) - (2 * fp + fq)).abs()))
}

/// The best FocusDir candidate from the focused placement, if any.
fn neighbor(places: &[Placement], dir: Dir, tiled_only: bool) -> Option<WinId> {
    let f = places.iter().find(|p| p.focused)?;
    let others = places
        .iter()
        .filter(|c| c.win != f.win && !(tiled_only && c.floating));
    let ranked = others.filter_map(|c| Some((rank(f.rect, c.rect, dir)?, c.win)));
    ranked.min().map(|(_, win)| win)
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

    fn size(&mut self, v: usize) {
        self.bytes(&(v as u64).to_le_bytes());
    }

    fn rect(&mut self, r: Rect) {
        for v in [r.x, r.y, r.w, r.h] {
            self.bytes(&v.to_le_bytes());
        }
    }
}

/// The window manager: workspaces, their windows, and focus.
#[derive(Clone, Debug)]
pub struct Wm {
    area: Rect,
    gaps: Gaps,
    active: usize,
    next: u32,
    spaces: Vec<Space>,
}

impl Wm {
    /// An empty window manager with `workspaces` workspaces (clamped to
    /// `[1, MAX_WORKSPACES]`); workspace 0 is active. `area` and `gaps` are
    /// normalized as [`Cmd::SetArea`] and [`Cmd::SetGaps`] would.
    pub fn new(area: Rect, gaps: Gaps, workspaces: usize) -> Wm {
        Wm {
            area: area.normalized(),
            gaps: gaps.normalized(),
            active: 0,
            next: 1,
            spaces: vec![Space::default(); workspaces.clamp(1, MAX_WORKSPACES)],
        }
    }

    /// Applies one command; the only way to change a `Wm`.
    ///
    /// Returns [`Outcome::Opened`] from [`Cmd::Open`], [`Outcome::Changed`]
    /// when the state changed and [`Outcome::Noop`] when a valid command
    /// changed nothing. On error nothing changes. (After id `u32::MAX - 1`,
    /// ids have run out and `Open` is a `Noop` rather than reuse one.)
    pub fn apply(&mut self, cmd: Cmd) -> Result<Outcome, WmError> {
        Ok(match cmd {
            Cmd::Open { floating } => self.open(floating),
            Cmd::Close(win) => {
                let ws = self.find(win)?;
                self.detach(ws, win);
                Outcome::Changed
            }
            Cmd::Focus(win) => {
                let ws = self.find(win)?;
                self.focus(ws, win)
            }
            Cmd::FocusDir(dir) => match neighbor(&self.layout(), dir, false) {
                Some(win) => self.focus(self.active, win),
                None => Outcome::Noop,
            },
            Cmd::MoveDir(dir) => self.move_dir(dir),
            Cmd::Resize { dir, px } => self.resize(dir, px),
            Cmd::ToggleFloat => self.toggle_float(),
            Cmd::ToggleOrientation => self.toggle_orientation(),
            Cmd::SetFloatRect { win, rect } => self.set_float_rect(win, rect)?,
            Cmd::SwitchWorkspace(ws) => {
                self.spaces.get(ws).ok_or(WmError::NoSuchWorkspace(ws))?;
                outcome(mem::replace(&mut self.active, ws) != ws)
            }
            Cmd::MoveToWorkspace { win, ws } => {
                let from = self.find(win)?;
                self.spaces.get(ws).ok_or(WmError::NoSuchWorkspace(ws))?;
                if from != ws {
                    let float = self.detach(from, win);
                    self.place(ws, win, float);
                    self.spaces[ws].history.push(win);
                }
                outcome(from != ws)
            }
            Cmd::SetArea(rect) => {
                let area = rect.normalized();
                let changed = mem::replace(&mut self.area, area) != area;
                if changed {
                    for f in self.spaces.iter_mut().flat_map(|s| s.floats.iter_mut()) {
                        f.1 = keep_visible(f.1, area);
                    }
                }
                outcome(changed)
            }
            Cmd::SetGaps(gaps) => {
                let gaps = gaps.normalized();
                outcome(mem::replace(&mut self.gaps, gaps) != gaps)
            }
        })
    }

    /// Placements on the active workspace: tiled windows in tree order, then
    /// floating windows bottom to top.
    pub fn layout(&self) -> Vec<Placement> {
        self.layout_of(self.active)
    }

    /// Placements on workspace `ws`, as [`Wm::layout`]; empty if out of range.
    pub fn layout_of(&self, ws: usize) -> Vec<Placement> {
        let Some(s) = self.spaces.get(ws) else {
            return Vec::new();
        };
        let nodes = s
            .tree
            .iter()
            .zip(shape(&s.tree, self.root(), self.gaps.inner));
        let tiled = nodes.filter_map(|(node, (rect, _))| match *node {
            Node::Leaf(win) => Some((win, rect, false)),
            Node::Split(..) => None,
        });
        let floats = s.floats.iter().map(|&(win, rect)| (win, rect, true));
        let place = |(win, rect, floating)| Placement {
            win,
            rect,
            floating,
            focused: Some(win) == s.focused(),
        };
        tiled.chain(floats).map(place).collect()
    }

    /// The focused window of the active workspace.
    pub fn focused(&self) -> Option<WinId> {
        self.spaces.get(self.active).and_then(Space::focused)
    }

    /// Index of the active workspace.
    pub fn active_workspace(&self) -> usize {
        self.active
    }

    /// Number of workspaces.
    pub fn workspace_count(&self) -> usize {
        self.spaces.len()
    }

    /// The workspace holding `win`, if it is open.
    pub fn workspace_of(&self, win: WinId) -> Option<usize> {
        let holds = |s: &Space| s.leaf(win).is_some() || s.float(win).is_some();
        self.spaces.iter().position(holds)
    }

    /// The screen area.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// The gaps.
    pub fn gaps(&self) -> Gaps {
        self.gaps
    }

    /// Whether `win` floats; `None` if it is not open.
    pub fn is_floating(&self, win: WinId) -> Option<bool> {
        let s = self.spaces.get(self.workspace_of(win)?)?;
        Some(s.float(win).is_some())
    }

    /// FNV-1a 64 over a canonical little-endian encoding of the whole logical
    /// state: area, gaps, active workspace, next id, then per workspace its
    /// tree in pre-order, floating stack and focus history, each
    /// length-prefixed. Equal states hash equally however they were reached.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        h.rect(self.area);
        h.bytes(&self.gaps.outer.to_le_bytes());
        h.bytes(&self.gaps.inner.to_le_bytes());
        h.size(self.active);
        h.u32(self.next);
        h.size(self.spaces.len());
        for s in &self.spaces {
            h.size(s.tree.len());
            for node in &s.tree {
                match *node {
                    Node::Leaf(win) => {
                        h.bytes(&[0]);
                        h.u32(win.0);
                    }
                    Node::Split(axis, ratio) => {
                        h.bytes(&[1, u8::from(axis == Axis::Vertical)]);
                        h.u32(ratio);
                    }
                }
            }
            h.size(s.floats.len());
            for &(win, rect) in &s.floats {
                h.u32(win.0);
                h.rect(rect);
            }
            h.size(s.history.len());
            for win in &s.history {
                h.u32(win.0);
            }
        }
        h.0
    }

    /// The area inset by the outer gap: where tiling happens.
    fn root(&self) -> Rect {
        let (a, o) = (self.area, self.gaps.outer);
        Rect::new(a.x + o, a.y + o, (a.w - 2 * o).max(0), (a.h - 2 * o).max(0))
    }

    fn find(&self, win: WinId) -> Result<usize, WmError> {
        self.workspace_of(win).ok_or(WmError::UnknownWindow(win))
    }

    fn open(&mut self, floating: bool) -> Outcome {
        let Some(next) = self.next.checked_add(1) else {
            return Outcome::Noop;
        };
        let win = WinId(mem::replace(&mut self.next, next));
        let a = self.area;
        let (w, h) = (FLOAT_MIN.max(a.w * 2 / 3), FLOAT_MIN.max(a.h * 2 / 3));
        let rect = Rect::new(a.x + (a.w - w) / 2, a.y + (a.h - h) / 2, w, h);
        self.place(self.active, win, floating.then(|| keep_visible(rect, a)));
        self.spaces[self.active].history.push(win);
        Outcome::Opened(win)
    }

    /// Puts `win` on workspace `ws` without touching its history: on top of
    /// the floating stack at `float`, or else into the tree by splitting the
    /// most recently focused tiled window across its longer side.
    fn place(&mut self, ws: usize, win: WinId, float: Option<Rect>) {
        let (root, inner) = (self.root(), self.gaps.inner);
        let s = &mut self.spaces[ws];
        if let Some(rect) = float {
            s.floats.push((win, rect));
            return;
        }
        let target = s.history.iter().rev().find_map(|&h| Some((s.leaf(h)?, h)));
        let Some((i, old)) = target else {
            s.tree = vec![Node::Leaf(win)];
            return;
        };
        let r = shape(&s.tree, root, inner)[i].0;
        let axis = [Axis::Vertical, Axis::Horizontal][usize::from(r.w >= r.h)];
        let split = [
            Node::Split(axis, RATIO_ONE / 2),
            Node::Leaf(old),
            Node::Leaf(win),
        ];
        s.tree.splice(i..=i, split);
    }

    /// Takes `win` off workspace `ws`'s tree or floating stack, leaving its
    /// history alone; returns its rect if it was floating. A removed leaf's
    /// parent split is replaced by the sibling subtree.
    fn unplace(&mut self, ws: usize, win: WinId) -> Option<Rect> {
        let s = &mut self.spaces[ws];
        if let Some(i) = s.float(win) {
            return Some(s.floats.remove(i).1);
        }
        let i = s.leaf(win)?;
        let p = s.parent(i);
        if p == NONE {
            s.tree.clear();
        } else if i == p + 1 {
            // A first-child leaf sits right after its parent.
            s.tree.drain(p..=i);
        } else {
            // A second-child leaf ends its parent's range.
            s.tree.remove(i);
            s.tree.remove(p);
        }
        None
    }

    /// Removes `win` from workspace `ws` entirely, as Close does.
    fn detach(&mut self, ws: usize, win: WinId) -> Option<Rect> {
        let float = self.unplace(ws, win);
        self.spaces[ws].history.retain(|&h| h != win);
        float
    }

    fn focus(&mut self, ws: usize, win: WinId) -> Outcome {
        let mut changed = mem::replace(&mut self.active, ws) != ws;
        let s = &mut self.spaces[ws];
        if s.focused() != Some(win) {
            s.history.retain(|&h| h != win);
            s.history.push(win);
            changed = true;
        }
        if let Some(i) = s.float(win).filter(|&i| i + 1 != s.floats.len()) {
            let f = s.floats.remove(i);
            s.floats.push(f);
            changed = true;
        }
        outcome(changed)
    }

    fn move_dir(&mut self, dir: Dir) -> Outcome {
        let places = self.layout();
        let me = match places.iter().find(|p| p.focused) {
            Some(p) if !p.floating => p.win,
            _ => return Outcome::Noop,
        };
        let Some(other) = neighbor(&places, dir, true) else {
            return Outcome::Noop;
        };
        for node in &mut self.spaces[self.active].tree {
            if *node == Node::Leaf(me) {
                *node = Node::Leaf(other);
            } else if *node == Node::Leaf(other) {
                *node = Node::Leaf(me);
            }
        }
        Outcome::Changed
    }

    fn resize(&mut self, dir: Dir, px: i32) -> Outcome {
        let px = i64::from(px.clamp(-MAX_COORD, MAX_COORD));
        let (root, inner) = (self.root(), self.gaps.inner);
        let s = &mut self.spaces[self.active];
        let Some(mut c) = s.focused().and_then(|w| s.leaf(w)).filter(|_| px != 0) else {
            return Outcome::Noop;
        };
        let shape = shape(&s.tree, root, inner);
        // Walk up to the nearest split on dir's axis whose divider is on the
        // dir side: for Right/Down the window lies in its first child.
        loop {
            let p = shape.get(c).map_or(NONE, |x| x.1);
            let (Some(&(rect, _)), Some(&Node::Split(axis, ratio))) = (shape.get(p), s.tree.get(p))
            else {
                return Outcome::Noop;
            };
            if axis == dir.axis() && (c == p + 1) == dir.forward() {
                let (min, one) = (i64::from(MIN_TILE), i64::from(RATIO_ONE));
                let avail = (rect.span(axis).1 - i64::from(inner)).max(0);
                if avail < 2 * min {
                    return Outcome::Noop;
                }
                let delta = if dir.forward() { px } else { -px };
                let new_a = (a_len(avail, ratio) + delta).clamp(min, avail - min);
                let r = (new_a * one + avail - 1) / avail;
                let r = r.clamp(i64::from(RATIO_MIN), one - i64::from(RATIO_MIN)) as u32;
                s.tree[p] = Node::Split(axis, r);
                return outcome(r != ratio);
            }
            c = p;
        }
    }

    fn toggle_float(&mut self) -> Outcome {
        let Some(win) = self.focused() else {
            return Outcome::Noop;
        };
        // A tiled window floats at its current rect; a floating one retiles.
        let places = self.layout();
        let tiled = places.iter().find(|p| p.win == win && !p.floating);
        let float = tiled.map(|p| keep_visible(p.rect, self.area));
        self.unplace(self.active, win);
        self.place(self.active, win, float);
        Outcome::Changed
    }

    fn toggle_orientation(&mut self) -> Outcome {
        let s = &mut self.spaces[self.active];
        let Some(i) = s.focused().and_then(|w| s.leaf(w)) else {
            return Outcome::Noop;
        };
        let p = s.parent(i);
        let Some(Node::Split(axis, _)) = s.tree.get_mut(p) else {
            return Outcome::Noop;
        };
        *axis = axis.flip();
        Outcome::Changed
    }

    fn set_float_rect(&mut self, win: WinId, rect: Rect) -> Result<Outcome, WmError> {
        let ws = self.find(win)?;
        let r = keep_visible(rect.normalized(), self.area);
        let s = &mut self.spaces[ws];
        let i = s.float(win).ok_or(WmError::NotFloating(win))?;
        Ok(outcome(mem::replace(&mut s.floats[i].1, r) != r))
    }
}

#[cfg(test)]
mod tests {
    use super::Dir::{Down, Left, Right, Up};
    use super::*;
    use std::collections::BTreeSet;

    const TILE: Cmd = Cmd::Open { floating: false };
    const FLOAT: Cmd = Cmd::Open { floating: true };
    const CHANGED: Result<Outcome, WmError> = Ok(Outcome::Changed);
    const NOOP: Result<Outcome, WmError> = Ok(Outcome::Noop);

    fn g(outer: i32, inner: i32) -> Gaps {
        Gaps { outer, inner }
    }

    fn wm() -> Wm {
        Wm::new(Rect::new(0, 0, 1920, 1080), g(8, 8), 4)
    }

    fn rs(dir: Dir, px: i32) -> Cmd {
        Cmd::Resize { dir, px }
    }

    fn run(wm: &mut Wm, cmds: &[Cmd]) {
        for c in cmds {
            wm.apply(c.clone()).unwrap();
        }
    }

    fn rect_of(wm: &Wm, id: u32) -> Rect {
        let lay = wm.layout_of(wm.workspace_of(WinId(id)).unwrap());
        lay.iter().find(|p| p.win == WinId(id)).unwrap().rect
    }

    fn order(wm: &Wm) -> Vec<u32> {
        wm.layout().iter().map(|p| p.win.0).collect()
    }

    #[test]
    fn worked_example_from_the_spec() {
        let mut w = wm();
        assert_eq!(w.apply(TILE), Ok(Outcome::Opened(WinId(1))));
        assert_eq!(rect_of(&w, 1), Rect::new(8, 8, 1904, 1064));
        assert_eq!(w.apply(TILE), Ok(Outcome::Opened(WinId(2))));
        assert_eq!(rect_of(&w, 1), Rect::new(8, 8, 948, 1064));
        assert_eq!(rect_of(&w, 2), Rect::new(964, 8, 948, 1064));
        assert_eq!(w.apply(TILE), Ok(Outcome::Opened(WinId(3))));
        assert_eq!(rect_of(&w, 2), Rect::new(964, 8, 948, 528));
        assert_eq!(rect_of(&w, 3), Rect::new(964, 544, 948, 528));
        assert_eq!(order(&w), [1, 2, 3]);
        assert_eq!(w.focused(), Some(WinId(3)));
        let focused: Vec<bool> = w.layout().iter().map(|p| p.focused).collect();
        assert_eq!(focused, [false, false, true]);
    }

    #[test]
    fn close_hands_space_to_sibling_and_ids_are_never_reused() {
        let mut w = wm();
        run(&mut w, &[TILE, TILE, TILE, Cmd::Close(WinId(1))]);
        // The sibling subtree keeps its vertical split and fills the root.
        assert_eq!(rect_of(&w, 2), Rect::new(8, 8, 1904, 528));
        assert_eq!(rect_of(&w, 3), Rect::new(8, 544, 1904, 528));
        assert_eq!(w.focused(), Some(WinId(3)));
        assert_eq!(w.apply(TILE), Ok(Outcome::Opened(WinId(4))));
        assert_eq!(rect_of(&w, 4), Rect::new(964, 544, 948, 528));
        run(&mut w, &[Cmd::Close(WinId(4))]);
        assert_eq!(w.focused(), Some(WinId(3)));
        assert_eq!(rect_of(&w, 3), Rect::new(8, 544, 1904, 528));
        run(&mut w, &[Cmd::Close(WinId(3)), Cmd::Close(WinId(2))]);
        assert_eq!(w.focused(), None);
        assert!(w.layout().is_empty());
        let h = w.state_hash();
        let unknown = Err(WmError::UnknownWindow(WinId(2)));
        assert_eq!(w.apply(Cmd::Close(WinId(2))), unknown);
        assert_eq!(w.state_hash(), h);
        assert_eq!(w.apply(TILE), Ok(Outcome::Opened(WinId(5))));
        assert_eq!(rect_of(&w, 5), Rect::new(8, 8, 1904, 1064));
        // Ids run out instead of wrapping: the last one is u32::MAX - 1.
        w.next = u32::MAX - 1;
        assert_eq!(w.apply(FLOAT), Ok(Outcome::Opened(WinId(u32::MAX - 1))));
        let h = w.state_hash();
        assert_eq!(w.apply(TILE), NOOP);
        assert_eq!(w.state_hash(), h);
    }

    #[test]
    fn floating_windows_open_centered_and_stay_visible() {
        let mut w = wm();
        run(&mut w, &[TILE, FLOAT]);
        assert_eq!(rect_of(&w, 2), Rect::new(320, 180, 1280, 720));
        assert_eq!(w.is_floating(WinId(2)), Some(true));
        assert_eq!(w.is_floating(WinId(1)), Some(false));
        assert_eq!(w.is_floating(WinId(9)), None);
        assert_eq!(order(&w), [1, 2]);
        let set = |win, rect| Cmd::SetFloatRect { win, rect };
        let far = Rect::new(5000, -5000, 10, 99_999);
        assert_eq!(w.apply(set(WinId(2), far)), CHANGED);
        assert_eq!(rect_of(&w, 2), Rect::new(1872, -1032, 48, 1080));
        assert_eq!(w.apply(set(WinId(2), far)), NOOP);
        let not_floating = Err(WmError::NotFloating(WinId(1)));
        assert_eq!(w.apply(set(WinId(1), far)), not_floating);
        let small = Cmd::SetArea(Rect::new(0, 0, 100, 100));
        assert_eq!(w.apply(small.clone()), CHANGED);
        assert_eq!(rect_of(&w, 2), Rect::new(52, -52, 48, 100));
        assert_eq!(w.apply(small), NOOP);
        // An area smaller than FLOAT_MIN pins the window to its corner.
        let pinned = keep_visible(Rect::new(500, 500, 5, 5), Rect::new(10, 10, 20, 20));
        assert_eq!(pinned, Rect::new(10, 10, 48, 48));
    }

    #[test]
    fn focus_raises_floats_and_switches_workspace() {
        let mut w = wm();
        run(&mut w, &[FLOAT, FLOAT]);
        assert_eq!(order(&w), [1, 2]);
        assert_eq!(w.apply(Cmd::Focus(WinId(1))), CHANGED);
        assert_eq!(order(&w), [2, 1]);
        assert_eq!(w.apply(Cmd::Focus(WinId(1))), NOOP);
        run(&mut w, &[Cmd::SwitchWorkspace(2), TILE]);
        assert_eq!(w.workspace_of(WinId(3)), Some(2));
        assert_eq!(w.apply(Cmd::Focus(WinId(2))), CHANGED);
        assert_eq!((w.active_workspace(), w.focused()), (0, Some(WinId(2))));
        assert!(w.layout_of(2)[0].focused);
        // Closing a floating window hands focus back down the history.
        run(&mut w, &[Cmd::Close(WinId(2))]);
        assert_eq!(w.focused(), Some(WinId(1)));
    }

    #[test]
    fn focus_dir_ranks_gap_then_overlap_then_center_then_id() {
        let p = |id, rect| Placement {
            win: WinId(id),
            rect,
            floating: false,
            focused: id == 1,
        };
        let f = p(1, Rect::new(0, 0, 100, 100));
        let far = p(2, Rect::new(150, 0, 10, 10));
        let thin = p(3, Rect::new(120, 90, 10, 100));
        let wide = p(4, Rect::new(120, 0, 10, 50));
        let centered = p(5, Rect::new(120, 25, 10, 50));
        let below = p(6, Rect::new(120, 200, 10, 10));
        let inside = p(7, Rect::new(50, 0, 100, 100));
        let twin = p(9, wide.rect);
        let pick = |ps: &[Placement], dir| neighbor(ps, dir, false).map(|w| w.0);
        assert_eq!(pick(&[f, far, below, inside], Right), Some(2));
        assert_eq!(pick(&[f, far, thin], Right), Some(3));
        assert_eq!(pick(&[f, far, thin, wide], Right), Some(4));
        assert_eq!(pick(&[f, wide, centered], Right), Some(5));
        assert_eq!(pick(&[f, twin, wide], Right), Some(4));
        assert_eq!(pick(&[f, far, thin], Left), None);
        assert_eq!(pick(&[f, below], Down), None);

        let mut w = wm();
        run(&mut w, &[TILE, TILE, TILE]);
        for (dir, want) in [(Left, 1), (Right, 2), (Down, 3), (Up, 2)] {
            assert_eq!(w.apply(Cmd::FocusDir(dir)), CHANGED);
            assert_eq!(w.focused(), Some(WinId(want)));
        }
        assert_eq!(w.apply(Cmd::FocusDir(Up)), NOOP);
        assert_eq!(w.apply(Cmd::FocusDir(Right)), NOOP);
    }

    #[test]
    fn move_dir_swaps_tiled_leaves() {
        let mut w = wm();
        run(&mut w, &[TILE, TILE, TILE, Cmd::Focus(WinId(1))]);
        assert_eq!(w.apply(Cmd::MoveDir(Right)), CHANGED);
        assert_eq!(rect_of(&w, 2), Rect::new(8, 8, 948, 1064));
        assert_eq!(rect_of(&w, 1), Rect::new(964, 8, 948, 528));
        assert_eq!(w.focused(), Some(WinId(1)));
        assert_eq!(w.apply(Cmd::MoveDir(Up)), NOOP);
        run(&mut w, &[FLOAT]);
        assert_eq!(w.apply(Cmd::MoveDir(Left)), NOOP);
    }

    #[test]
    fn resize_moves_the_divider_on_the_named_side() {
        let mut w = wm();
        run(&mut w, &[TILE, TILE]);
        assert_eq!(w.apply(rs(Right, 100)), NOOP);
        assert_eq!(w.apply(rs(Up, 100)), NOOP);
        assert_eq!(w.apply(rs(Left, 0)), NOOP);
        assert_eq!(w.apply(rs(Left, 100)), CHANGED);
        assert_eq!(rect_of(&w, 1), Rect::new(8, 8, 848, 1064));
        assert_eq!(rect_of(&w, 2), Rect::new(864, 8, 1048, 1064));
        // Here RATIO_MIN binds before MIN_TILE: 1896 * 3277 / 65536 = 94.
        assert_eq!(w.apply(rs(Left, i32::MAX)), CHANGED);
        assert_eq!(rect_of(&w, 1).w, 94);
        assert_eq!(w.apply(rs(Left, 5)), NOOP);
        assert_eq!(w.apply(rs(Left, i32::MIN)), CHANGED);
        assert_eq!(rect_of(&w, 2).w, 95);
        // A nested window walks up past a split on the other axis.
        let mut w = wm();
        run(&mut w, &[TILE, TILE, TILE, rs(Left, 100), rs(Up, 100)]);
        assert_eq!(rect_of(&w, 1).w, 848);
        assert_eq!(rect_of(&w, 2), Rect::new(864, 8, 1048, 428));
        assert_eq!(rect_of(&w, 3), Rect::new(864, 444, 1048, 628));
        // Splits too small to keep both sides at MIN_TILE refuse.
        run(&mut w, &[Cmd::SetArea(Rect::new(0, 0, 100, 100))]);
        assert_eq!(w.apply(rs(Up, 5)), NOOP);
        assert_eq!(w.apply(rs(Left, 5)), NOOP);
        // The ceiling makes MIN_TILE exact when RATIO_MIN does not bind.
        let mut w = Wm::new(Rect::new(0, 0, 800, 600), g(0, 0), 1);
        run(&mut w, &[TILE, TILE, rs(Left, i32::MAX)]);
        assert_eq!(rect_of(&w, 1).w, MIN_TILE);
        // Ratio products stay exact on a huge area.
        let mut w = Wm::new(Rect::new(0, 0, MAX_COORD, 100), g(0, 0), 1);
        run(&mut w, &[TILE, TILE, rs(Left, MAX_COORD)]);
        let root = Node::Split(Axis::Horizontal, RATIO_MIN);
        assert_eq!(w.spaces[0].tree[0], root);
    }

    #[test]
    fn toggles_float_and_orientation() {
        let mut w = wm();
        assert_eq!(w.apply(Cmd::ToggleFloat), NOOP);
        assert_eq!(w.apply(Cmd::ToggleOrientation), NOOP);
        run(&mut w, &[TILE]);
        assert_eq!(w.apply(Cmd::ToggleOrientation), NOOP);
        run(&mut w, &[TILE]);
        let before = w.state_hash();
        assert_eq!(w.apply(Cmd::ToggleFloat), CHANGED);
        assert_eq!(w.is_floating(WinId(2)), Some(true));
        assert_eq!(rect_of(&w, 2), Rect::new(964, 8, 948, 1064));
        assert_eq!(rect_of(&w, 1), Rect::new(8, 8, 1904, 1064));
        assert_eq!(w.focused(), Some(WinId(2)));
        assert_eq!(w.apply(Cmd::ToggleOrientation), NOOP);
        assert_eq!(w.apply(Cmd::ToggleFloat), CHANGED);
        assert_eq!(w.state_hash(), before);
        assert_eq!(w.apply(Cmd::ToggleOrientation), CHANGED);
        assert_eq!(rect_of(&w, 1), Rect::new(8, 8, 1904, 528));
        assert_eq!(rect_of(&w, 2), Rect::new(8, 544, 1904, 528));
    }

    #[test]
    fn workspaces_switch_and_move() {
        assert_eq!(Wm::new(Rect::default(), g(0, 0), 0).workspace_count(), 1);
        assert_eq!(Wm::new(Rect::default(), g(0, 0), 999).workspace_count(), 64);
        let mut w = wm();
        run(&mut w, &[TILE, TILE]);
        let mv = |win, ws| Cmd::MoveToWorkspace { win, ws };
        let unknown = Err(WmError::UnknownWindow(WinId(99)));
        assert_eq!(w.apply(mv(WinId(99), 99)), unknown);
        let bad = Err(WmError::NoSuchWorkspace(4));
        assert_eq!(w.apply(mv(WinId(1), 4)), bad);
        assert_eq!(w.apply(mv(WinId(1), 0)), NOOP);
        assert_eq!(w.apply(mv(WinId(2), 1)), CHANGED);
        assert_eq!((w.active_workspace(), w.focused()), (0, Some(WinId(1))));
        assert_eq!(rect_of(&w, 1), Rect::new(8, 8, 1904, 1064));
        let there = w.layout_of(1);
        assert_eq!(there.len(), 1);
        assert!(there[0].focused && !there[0].floating);
        assert!(w.layout_of(4).is_empty());
        assert_eq!(w.apply(Cmd::SwitchWorkspace(4)), bad);
        assert_eq!(w.apply(Cmd::SwitchWorkspace(0)), NOOP);
        assert_eq!(w.apply(Cmd::SwitchWorkspace(1)), CHANGED);
        assert_eq!(w.focused(), Some(WinId(2)));
        // A floating window keeps its rect across workspaces.
        run(&mut w, &[FLOAT, mv(WinId(3), 0)]);
        assert_eq!(w.focused(), Some(WinId(2)));
        let focus: Vec<bool> = w.layout_of(0).iter().map(|p| p.focused).collect();
        assert_eq!(focus, [false, true]);
        assert_eq!(rect_of(&w, 3), Rect::new(320, 180, 1280, 720));
    }

    #[test]
    fn normalization_and_gaps() {
        let big = Rect::new(i32::MIN, i32::MAX, i32::MIN, i32::MAX);
        let mut w = Wm::new(big, g(-5, i32::MAX), 2);
        assert_eq!(w.area(), Rect::new(-MAX_COORD, MAX_COORD, 0, MAX_COORD));
        assert_eq!(w.gaps(), g(0, MAX_COORD));
        let area = Cmd::SetArea(Rect::new(0, 0, 100, 60));
        run(&mut w, &[area, TILE, TILE]);
        assert_eq!(rect_of(&w, 1), Rect::new(0, 0, 0, 60));
        assert_eq!(rect_of(&w, 2), Rect::new(100, 0, 0, 60));
        assert_eq!(w.apply(Cmd::SetGaps(g(0, 1 << 21))), NOOP);
        assert_eq!(w.apply(Cmd::SetGaps(g(10, 0))), CHANGED);
        assert_eq!(rect_of(&w, 1), Rect::new(10, 10, 40, 40));
        assert_eq!(rect_of(&w, 2), Rect::new(50, 10, 40, 40));
    }

    #[test]
    fn state_hash_is_canonical() {
        let mut f = Fnv(0xcbf2_9ce4_8422_2325);
        f.bytes(b"a");
        assert_eq!(f.0, 0xaf63_dc4c_8601_ec8c);
        let (mut a, mut b) = (wm(), wm());
        run(&mut a, &[TILE, TILE, TILE, Cmd::Close(WinId(2))]);
        let flip = Cmd::ToggleOrientation;
        run(&mut b, &[TILE, TILE, TILE, flip.clone(), flip]);
        run(&mut b, &[Cmd::Focus(WinId(1)), Cmd::Focus(WinId(3))]);
        run(&mut b, &[Cmd::Close(WinId(2))]);
        assert_eq!(a.state_hash(), b.state_hash());
        b.spaces[0].tree.reserve(100);
        assert_eq!(a.state_hash(), b.state_hash());
        run(&mut b, &[rs(Left, 1)]);
        assert_ne!(a.state_hash(), b.state_hash());
        let mut c = a.clone();
        run(&mut c, &[Cmd::SetGaps(g(8, 9))]);
        assert_ne!(a.state_hash(), c.state_hash());
    }

    #[test]
    fn state_hash_golden_values() {
        // Pinned: a change to the encoding would orphan every recorded hash.
        let mut w = wm();
        assert_eq!(w.state_hash(), 0x31ed_45f4_5e88_7269);
        let (up, flip, win, ws) = (rs(Up, 100), Cmd::ToggleOrientation, WinId(5), 2);
        run(&mut w, &[TILE, TILE, TILE, up, flip, TILE, FLOAT]);
        let mv = Cmd::MoveToWorkspace { win, ws };
        run(&mut w, &[mv, Cmd::SwitchWorkspace(3)]);
        assert_eq!(w.state_hash(), 0x18de_31f8_3174_0179);
    }

    #[test]
    fn deep_trees_do_not_recurse() {
        let mut w = wm();
        for _ in 0..2000 {
            w.apply(TILE).unwrap();
        }
        assert_eq!(w.layout().len(), 2000);
        assert_eq!(w.clone().state_hash(), w.state_hash());
        for id in 1..=2000 {
            w.apply(Cmd::Close(WinId(id))).unwrap();
        }
        assert!(w.spaces[0].tree.is_empty());
    }

    /// SplitMix64: a seeded generator for randomized command runs.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            (z ^ (z >> 31)) % n.max(1)
        }

        /// Mostly ordinary values, sometimes extremes.
        fn val(&mut self) -> i32 {
            match self.below(8) {
                0 => i32::MIN,
                1 => i32::MAX,
                2 => 0,
                3 => -(self.below(3000) as i32),
                _ => self.below(3000) as i32,
            }
        }

        fn small(&mut self, n: u64) -> i32 {
            if self.below(5) == 0 {
                self.val()
            } else {
                self.below(n) as i32
            }
        }

        fn cmd(&mut self, next: u32) -> Cmd {
            let win = WinId(self.below(u64::from(next) + 1) as u32);
            let dir = [Left, Right, Up, Down][self.below(4) as usize];
            let ws = self.below(5) as usize;
            let rect = Rect::new(self.val(), self.val(), self.val(), self.val());
            match self.below(18) {
                0..=2 => TILE,
                3 => FLOAT,
                4 => Cmd::Close(win),
                5 => Cmd::Focus(win),
                6 | 7 => Cmd::FocusDir(dir),
                8 => Cmd::MoveDir(dir),
                9 | 10 => rs(dir, self.small(400).saturating_sub(200)),
                11 => Cmd::ToggleFloat,
                12 => Cmd::ToggleOrientation,
                13 => Cmd::SetFloatRect { win, rect },
                14 => Cmd::SwitchWorkspace(ws),
                15 => Cmd::MoveToWorkspace { win, ws },
                16 => Cmd::SetGaps(g(self.small(20), self.small(20))),
                _ if self.below(3) == 0 => Cmd::SetArea(rect),
                _ => Cmd::SetArea(Rect::new(0, 0, self.small(3000), self.small(2000))),
            }
        }
    }

    /// Invariants I1 to I5.
    fn check(w: &Wm) {
        let root = w.root();
        let mut all = Vec::new();
        assert!(w.active < w.spaces.len());
        assert_eq!(w.focused(), w.spaces[w.active].history.last().copied());
        for (i, s) in w.spaces.iter().enumerate() {
            let mut need = 1i64;
            for node in &s.tree {
                assert!(need > 0, "malformed tree");
                need += match *node {
                    Node::Leaf(_) => -1,
                    Node::Split(_, r) => {
                        assert!((RATIO_MIN..=RATIO_ONE - RATIO_MIN).contains(&r));
                        1
                    }
                };
            }
            assert!(s.tree.is_empty() || need == 0);
            let lay = w.layout_of(i);
            let mut mine: Vec<WinId> = lay.iter().map(|p| p.win).collect();
            let mut hist = s.history.clone();
            mine.sort();
            hist.sort();
            assert_eq!(mine, hist);
            all.extend(mine);
            for (j, p) in lay.iter().enumerate() {
                assert_eq!(p.focused, Some(p.win) == s.focused());
                if p.floating {
                    assert!(p.rect.w >= FLOAT_MIN && p.rect.h >= FLOAT_MIN);
                    continue;
                }
                let r = p.rect;
                assert!(r.w >= 0 && r.h >= 0 && r.x >= root.x && r.y >= root.y);
                assert!(r.x + r.w <= root.x + root.w && r.y + r.h <= root.y + root.h);
                for q in lay[j + 1..].iter().filter(|q| !q.floating) {
                    let ix = (r.x + r.w).min(q.rect.x + q.rect.w) - r.x.max(q.rect.x);
                    let iy = (r.y + r.h).min(q.rect.y + q.rect.h) - r.y.max(q.rect.y);
                    assert!(ix <= 0 || iy <= 0, "{p:?} overlaps {q:?}");
                }
            }
        }
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "a window lives in two places");
        assert!(all.iter().all(|id| id.0 >= 1 && id.0 < w.next));
    }

    #[test]
    fn random_runs_keep_invariants_and_replay() {
        let mut changers = BTreeSet::new();
        for seed in 0..40 {
            let mut rng = Rng(seed);
            let mut w = wm();
            let mut log = Vec::new();
            for _ in 0..300 {
                let cmd = rng.cmd(w.next);
                let before = w.state_hash();
                let out = w.apply(cmd.clone());
                let after = w.state_hash();
                match out {
                    Ok(Outcome::Noop) | Err(_) => assert_eq!(before, after, "{cmd:?}"),
                    Ok(_) => assert_ne!(before, after, "{cmd:?}"),
                }
                if matches!(out, Ok(Outcome::Changed | Outcome::Opened(_))) {
                    let name = format!("{cmd:?}");
                    let variant: String =
                        name.chars().take_while(char::is_ascii_alphabetic).collect();
                    changers.insert(variant);
                }
                check(&w);
                log.push((cmd, out, after));
            }
            let mut replay = wm();
            for (cmd, out, hash) in log {
                assert_eq!(replay.apply(cmd), out);
                assert_eq!(replay.state_hash(), hash);
            }
        }
        assert_eq!(
            changers.len(),
            13,
            "some command never changed state: {changers:?}"
        );
    }
}

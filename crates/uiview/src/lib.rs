//! The desktop's half of [`uiwire`]: a GUI program's window as `ui` draws it. [`draw`] lays out a
//! frame's nodes top to bottom, [`PAD`] inside the content rect, and draws them in the frame's
//! theme, scrolled as the window's [`View`] says: a following view at the bottom stays there as
//! the content grows, an [`Area`] being typed in keeps its caret in view, and while the content
//! (or a Scroll's) overflows, a thumb at the right shows how far down it is. Only what is in view
//! is drawn, so a long list costs little. While the user edits an Input, a Code or an Area its
//! text is the host's ([`Texts`]). A Glyph or an Entry's tile whose glyph the desktop does not
//! know is empty space; a revealed mark comes in ring by ring by the window's own clock (the page
//! clock at its first draw), frames asked for only meanwhile ([`View::animating`]). In a window
//! narrower than [`NARROW`] (a phone's) chips and quiet buttons are touch targets, [`TOUCH`] tall.

#![forbid(unsafe_code)]

mod area;
#[cfg(test)]
mod tests;
mod texts;

use std::mem;

pub use area::Area;
use gfx::RectF;
pub use texts::Texts;
use ui::icon::{Glyph, MARK_HOLE, cos, rings, sin};
use ui::{AppIcon, BUTTON_H, CARD_PAD, FIELD_H, FontId, PAD, RADIUS_SM, Rgba, SPACING, Sense};
use ui::{TextStyle, TextSystem, Theme, Ui, WidgetId};
use uiwire::{Event, Node, REVEAL, Request, SIGIL, Style, Variant};

/// An Item's height, a chip's, a touch target's (a chip's or a quiet button's on a narrow
/// window), a chip's padding either side of its label, an Entry's (a touch target, Fibonacci as
/// its tile's side is), a Toggle's.
pub const ITEM_H: f32 = 36.0;
pub const CHIP_H: f32 = 28.0;
pub const TOUCH: f32 = 44.0;
const CHIP_PAD: f32 = 12.0;
pub const ENTRY_H: f32 = 55.0;
const TILE: f32 = 34.0;
pub const TOGGLE_H: f32 = 44.0;
/// Inter's cap height in ems: one-line controls center it.
const CAP: f32 = 0.727;
/// The widest window that is narrow.
pub const NARROW: f32 = 560.0;
/// The mark's reveal: band `k` (the center, then each of its seven rings of dots, the last
/// with the rim) fades in from `STEP * k` ms for `FADE` ms, Fibonacci numbers both: 618 ms.
const STEP: f64 = 55.0;
const FADE: f64 = 233.0;
pub const REVEAL_MS: f64 = 7.0 * STEP + FADE;

/// How a window is scrolled: pixels down, and the content's and the view's height as last drawn;
/// whether a view at the bottom stays there as the content grows (the Assistant's transcript);
/// each Scroll as last drawn; the page clock when a revealed glyph first drew; each Grid with an
/// id as last drawn.
#[derive(Debug, Default)]
pub struct View {
    pub scroll: f32,
    pub heights: (f32, f32),
    pub follow: bool,
    pub scrolls: Vec<Scrolled>,
    pub reveal: Option<f64>,
    pub grids: Vec<Board>,
}

/// A Grid with an id as last drawn: its id, its place among the frame's Grids (the same grid in
/// a later frame, whatever its id there), its squares' rect, its columns and squares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Board {
    pub id: u32,
    pub nth: u32,
    pub rect: RectF,
    pub cols: u16,
    pub n: u32,
}

impl Board {
    /// The square at `(x, y)`, as its column and row, if one is there.
    pub fn at(&self, x: f32, y: f32) -> Option<(u32, u32)> {
        let (r, cols) = (self.rect, u32::from(self.cols.max(1)));
        let side = r.w / cols as f32;
        let (col, row) = ((((x - r.x) / side) as u32).min(cols - 1), ((y - r.y) / side) as u32);
        (r.contains(x, y) && row * cols + col < self.n).then_some((col, row))
    }
}

/// What a program asked of its window for play: a timer every `timer` ms (0: none), and plain
/// keys; with when the last Tick went and whether it is unanswered, the Grid pressed (its place
/// among the frame's Grids) and the square of it last tapped (the Grid's id then, the square's
/// column and row) while the pointer is down, the frames until the last event the person (or
/// the AI) sent is answered, and whether an answer came since the window drew.
#[derive(Debug, Default)]
pub struct Play {
    pub timer: u32,
    pub keys: bool,
    at: Option<f64>,
    waiting: bool,
    grid: Option<u32>,
    last: Option<(u32, u32, u32)>,
    owed: u8,
    moved: bool,
}

impl Play {
    /// Takes a frame's [`Request::Timer`] or [`Request::Keys`]; whether it was one.
    pub fn ask(&mut self, r: &Request) -> bool {
        match *r {
            Request::Timer { ms } => (self.timer, self.at) = (ms, None),
            Request::Keys { on } => self.keys = on,
            _ => return false,
        }
        true
    }

    /// A click, submit, tap or plain key went, after a Change still unanswered if `changing`: the
    /// program reads events in order and answers each of these (and each Tick and Change) with a
    /// frame, so its answer is the frame after those for the Tick and Change out.
    pub fn sent(&mut self, changing: bool) {
        self.owed = 1 + u8::from(changing) + u8::from(self.waiting);
    }

    /// A frame came: it answers the last Tick, or one of the frames owed.
    pub fn answered(&mut self) {
        self.moved |= self.owed > 0;
        (self.waiting, self.owed) = (false, self.owed.saturating_sub(1));
    }

    /// Whether what was sent is still unanswered: the window is busy.
    pub fn busy(&self) -> bool {
        self.owed > 0
    }

    /// The window drew what came.
    pub fn drew(&mut self) {
        self.moved = false;
    }

    /// How long a Tick waits after the last: the timer's ms, a second at least while the last
    /// is unanswered.
    fn wait(&self) -> f64 {
        f64::from(if self.waiting { self.timer.max(1000) } else { self.timer })
    }

    /// The ms from `now` until the next Tick is due (0: due, or its count starts), when the
    /// window wants its next frame; `None` with no timer.
    pub fn due_in(&self, now: f64) -> Option<u32> {
        let at = self.at.unwrap_or(f64::MIN);
        (self.timer > 0).then(|| (at + self.wait() - now).max(0.0).ceil() as u32)
    }

    /// The Tick due at `now` (page ms), the window last drawn at `drawn`: one only while it
    /// shows (drew within half a second of the wait), the timer's ms after the last, once that
    /// was answered or a second went by; `ms` the time since the last.
    pub fn tick(&mut self, now: f64, drawn: f64) -> Option<Event> {
        if self.timer == 0 || now - drawn > self.wait() + 500.0 {
            self.at = None;
            return None;
        }
        let at = *self.at.get_or_insert(now);
        (now - at >= self.wait()).then(|| {
            (self.at, self.waiting) = (Some(now), true);
            Event::Tick { ms: (now - at) as u32 }
        })
    }

    /// The pointer went down on hit `id` (`None`: dragged while down) at `(x, y)`: the Grid to
    /// tap (its id now) and its squares newly under it. A drag stays on the Grid pressed (found
    /// by its place in the frame, as its id may change) and taps each square on the line from
    /// the last one, as the pointer moves past squares between samples; off the squares and
    /// back, it goes on from where it came back. It waits while what was sent is unanswered,
    /// and until the answer draws (which may give the Grids other ids), so it never taps by a
    /// stale id.
    pub fn tap(&mut self, view: &View, id: Option<u32>, x: f32, y: f32) -> (u32, Vec<u32>) {
        let mut taps = (0, Vec::new());
        if id.is_some() {
            (self.grid, self.last) = (None, None);
        } else if self.owed > 0 || self.moved {
            return taps;
        }
        let grid = self.grid;
        let b = view.grids.iter().find(|b| match (id, grid) {
            (Some(id), _) => b.id == id && b.rect.contains(x, y),
            (_, Some(nth)) => b.nth == nth,
            _ => false,
        });
        let Some(b) = b else { return taps };
        // From the last square tapped, of the grid as it was then.
        let from = self.last.filter(|l| l.0 == b.id).map(|l| (l.1, l.2));
        let to = b.at(x, y);
        (self.grid, self.last, taps.0) = (Some(b.nth), to.map(|(c, r)| (b.id, c, r)), b.id);
        if let Some((c, r)) = to {
            let (c0, r0) = from.unwrap_or((c, r));
            // The square alone for a press or a return; none for the same square.
            let n = c.abs_diff(c0).max(r.abs_diff(r0)).max(u32::from(from.is_none()));
            // `i` of `n` steps from `p` to `q`, rounded.
            let step = |p: u32, q: u32, i: u32| (p * (n - i) + q * i + n / 2) / n;
            for i in 1..=n {
                let cell = step(r0, r, i) * u32::from(b.cols) + step(c0, c, i);
                if cell < b.n {
                    taps.1.push(cell);
                }
            }
        }
        taps
    }

    /// Whether the pointer's last press was on a Grid: its tap went, so its Click is none.
    pub fn pressed(&self) -> bool {
        self.grid.is_some()
    }
}

/// The largest square of a Grid, in logical px: a finger's target several times over.
pub const SQUARE: f32 = 96.0;

/// A grid's square for `cols` across `w`: whole device px, as wide as `w` holds up to [`SQUARE`]
/// (a device px at least), times `fit` (the share of their room the boards get), 6 px at least.
fn side(ts: &TextSystem, (w, cols): (f32, u16), fit: f32) -> f32 {
    let d = ts.dpr();
    // Not `clamp`: its panic message would link float formatting into the boot.
    let most = (w / f32::from(cols) * d).floor().min((SQUARE * d).floor()).max(1.0);
    (most * fit).floor().max((6.0 * d).ceil()).min(most) / d
}

/// A Scroll as last drawn: its id, how far down it is, its content's height and its rect.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scrolled {
    pub id: u32,
    pub y: f32,
    pub content: f32,
    pub rect: RectF,
}

impl View {
    /// Whether a reveal runs at `now_ms` (the page clock): the window wants frames meanwhile.
    pub fn animating(&self, now_ms: f64) -> bool {
        self.reveal.is_some_and(|start| now_ms - start < REVEAL_MS)
    }
}

/// The wheel at `(x, y)` in the window's content, `dy` px down: it scrolls the Code under it,
/// else the innermost Scroll under it that overflows, else the window. Whether anything moved.
pub fn wheel(texts: &mut Texts, view: &mut View, x: f32, y: f32, dy: f32) -> bool {
    if !dy.is_finite() {
        return false;
    }
    if let Some(c) = texts.codes.iter_mut().find(|c| c.1.contains(x, y)) {
        return c.1.wheel(dy);
    }
    let over = view.scrolls.iter_mut().rev().find(|s| s.rect.contains(x, y));
    let (at, max) = match over.filter(|s| s.content > s.rect.h) {
        Some(s) => (&mut s.y, s.content - s.rect.h),
        None => (&mut view.scroll, view.heights.0 - view.heights.1),
    };
    let to = (*at + dy).min(max).max(0.0);
    mem::replace(at, to) != to
}

/// Lays out `nodes` in `ui`'s rect and draws them, scrolled as `view` says (see the crate docs).
pub fn draw(ui: &mut Ui<'_>, nodes: &[Node], texts: &mut Texts, view: &mut View) {
    let r = ui.rect();
    let (w, inner) = ((r.w - 2.0 * PAD).max(0.0), r.h - 2.0 * PAD);
    let (t, now, touch) = (ui.theme(), ui.state().now_ms, r.w < NARROW);
    let old = mem::take(&mut view.scrolls);
    #[rustfmt::skip]
    let mut lay = Lay { t, texts, sizes: Vec::new(), extents: Vec::new(), extra: 0.0, fills: 0,
        slack: Vec::new(), again: false, i: 0, e: 0, y: 0.0, touch, right: r.x + r.w, old,
        scrolls: Vec::new(), now, reveal: view.reveal, grids: Vec::new(), fit: 0.0,
        room: (0.0, 0.0), nth: 0 };
    let ts = ui.text_system();
    let mut h = lay.stack(ts, nodes, w, SPACING, None);
    // Again with the boards (measured at their least: Grids of 6 px squares) sharing what the
    // rest leaves, a third of the view's height at least, each as large as it can be up to its
    // full width: a game takes the room its labels and buttons leave.
    let (least, most) = lay.room;
    if most > 0.0 {
        lay.fit = ((inner - h + least).max(inner / 3.0) / most).min(1.0);
        lay.fills = 0;
        lay.slack.clear();
        h = lay.restack(ts, nodes, w);
    }
    // Again with the room left shared by the Fills and Scrolls, each also taking what it is
    // short of a taller sibling in a Row.
    let fills = mem::take(&mut lay.fills);
    if fills > 0 && (h < inner || lay.slack.iter().any(|s| *s > 0.0)) {
        (lay.extra, lay.again) = ((inner - h).max(0.0) / fills as f32, true);
        h = lay.restack(ts, nodes, w);
    }
    let end = view.follow && view.scroll >= view.heights.0 - view.heights.1;
    view.heights = (h + 2.0 * PAD, r.h);
    let mut y = if end { f32::MAX } else { view.scroll };
    // A caret just moved or typed at, in view.
    if let Some((_, a)) = lay.texts.areas.iter_mut().find(|a| a.1.follow) {
        let (top, bottom) = a.caret_band();
        (y, a.follow) = (y.max(PAD + bottom - r.h).min(PAD + top), false);
    }
    view.scroll = y.min(view.heights.0 - r.h).max(0.0);
    lay.draw_stack(ui, nodes, (r.x + PAD, r.y + PAD - view.scroll), SPACING);
    ui.thumb(r, view.scroll, view.heights.0);
    (view.scrolls, view.reveal, view.grids) = (lay.scrolls, lay.reveal, lay.grids);
}

/// How a Text of `style` is set.
fn style_of(style: Style, t: &Theme) -> TextStyle {
    match style {
        Style::Body => t.body(),
        Style::Title => t.title(),
        Style::Heading => t.heading(),
        Style::Subheading => t.subheading(),
        Style::Small => t.small(),
        Style::Mono => t.mono(),
        Style::Dim => t.body().with_color(t.text_dim),
        Style::Error => t.body().with_color(t.danger),
        Style::Success => t.body().with_color(t.ansi[2]),
        Style::Accent => t.body().with_color(t.accent),
        Style::Display => TextStyle::new(FontId::SansBold, 34.0, t.text),
    }
}

/// One frame's layout in theme `t` over the host's text: each node's size in pre-order, each
/// Scroll's content height and each Strip's content width in pre-order, the height each Fill or
/// Scroll adds, the Fills and Scrolls so far, each one's shortfall beside a taller sibling,
/// whether this is the second pass, the next size and extent to draw, the top of the node being
/// measured in the content, whether chips and quiet buttons are touch targets, the window's
/// right edge, the Scrolls as last drawn and as drawn now, the page clock, when the reveal
/// began, the boards drawn, the share of its largest size each board takes (0: its least), the
/// boards' heights as measured and at their largest, and the boards drawn or passed.
struct Lay<'t> {
    t: &'t Theme,
    texts: &'t mut Texts,
    sizes: Vec<(f32, f32)>,
    extents: Vec<f32>,
    extra: f32,
    fills: usize,
    slack: Vec<f32>,
    again: bool,
    i: usize,
    e: usize,
    y: f32,
    touch: bool,
    right: f32,
    old: Vec<Scrolled>,
    scrolls: Vec<Scrolled>,
    now: f64,
    reveal: Option<f64>,
    grids: Vec<Board>,
    fit: f32,
    room: (f32, f32),
    nth: u32,
}

impl Lay<'_> {
    /// Measures `ns` stacked `gap` apart in width `w`; their height. Each Code
    /// takes `g` more height (in a Fill).
    fn stack(&mut self, ts: &mut TextSystem, ns: &[Node], w: f32, gap: f32, g: Option<f32>) -> f32 {
        let (top, mut h) = (self.y, 0.0);
        for n in ns {
            self.y = top + h;
            h += self.measure(ts, n, w, g).1 + gap;
        }
        self.y = top;
        h - if ns.is_empty() { 0.0 } else { gap }
    }

    /// Measures `ns`, the frame's nodes `w` wide, again; their height.
    fn restack(&mut self, ts: &mut TextSystem, ns: &[Node], w: f32) -> f32 {
        self.sizes.clear();
        self.extents.clear();
        self.stack(ts, ns, w, SPACING, None)
    }

    /// The height a Fill or Scroll adds to its own: its share of the room left, and what it is
    /// short of a taller sibling.
    fn grow(&mut self) -> f32 {
        if self.slack.len() <= self.fills {
            self.slack.push(0.0);
        }
        self.fills += 1;
        self.extra + self.slack[self.fills - 1]
    }

    /// A board (a Grid or a Canvas) `w` wide, `h` tall as measured and `most` at its largest:
    /// its size, its room noted.
    fn board(&mut self, w: f32, h: f32, most: f32) -> (f32, f32) {
        self.room = (self.room.0 + h, self.room.1 + most);
        (w, h)
    }

    /// Measures `n` given width `w` (Buttons, Spacers and Glyphs take their own).
    fn measure(&mut self, ts: &mut TextSystem, n: &Node, w: f32, grow: Option<f32>) -> (f32, f32) {
        let (at, t) = (self.sizes.len(), self.t);
        self.sizes.push((0.0, 0.0));
        let size = match n {
            Node::Col { gap, children: c, .. } | Node::Center { gap, children: c, .. } => {
                (w, self.stack(ts, c, w, f32::from(*gap), None))
            }
            Node::Card { children, .. } => {
                self.y += CARD_PAD;
                let h = self.stack(ts, children, w - 2.0 * CARD_PAD, SPACING, None);
                self.y -= CARD_PAD;
                (w, h + 2.0 * CARD_PAD)
            }
            // Its Codes and Areas share the room it adds (else it takes it under them).
            Node::Fill { children, .. } => {
                let extra = self.grow();
                let edits = |c: &&Node| matches!(c, Node::Code { .. } | Node::Area { .. });
                let codes = children.iter().filter(edits).count();
                let grow = Some(extra / codes.max(1) as f32);
                (w, self.stack(ts, children, w, SPACING, grow) + [extra, 0.0][codes.min(1)])
            }
            // A row tall at least, and what the others leave; its children as tall as they are.
            Node::Scroll { children, .. } => {
                let (extra, e) = (self.grow(), self.extents.len());
                self.extents.push(0.0);
                self.extents[e] = self.stack(ts, children, w, SPACING, None);
                (w, ENTRY_H + extra)
            }
            Node::Pane { w: pw, children, .. } => {
                let w = w.min(f32::from(*pw));
                (w, self.stack(ts, children, w, SPACING, None))
            }
            // Buttons, Spacers and Glyphs take their width, each Text its own up to an even
            // share of the rest (in a Strip, all of it), the others share what is left. A Strip
            // takes the width it is given and keeps what its children take.
            Node::Row { gap, children, .. } | Node::Strip { gap, children, .. } => {
                let strip = matches!(n, Node::Strip { .. });
                let e = self.extents.len();
                if strip {
                    self.extents.push(0.0);
                }
                let (gap, mut x, mut h) = (f32::from(*gap), 0.0, 0.0f32);
                let mut own: Vec<_> = children.iter().map(|c| own_width(ts, t, c, w)).collect();
                let gaps = gap * children.len().saturating_sub(1) as f32;
                let mut room = (w - own.iter().flatten().sum::<f32>() - gaps).max(0.0);
                let mut flex = own.iter().filter(|o| o.is_none()).count();
                let share = if strip { f32::MAX } else { room / flex.max(1) as f32 };
                for (c, o) in children.iter().zip(&mut own) {
                    if let Node::Text { style, text, .. } = c {
                        let style = style_of(*style, t);
                        let lines = ts.wrap(text, style, f32::MAX);
                        let one = lines.iter().map(|l| ts.measure(l, style)).fold(0.0, f32::max);
                        // A pixel more: snapping must not wrap it.
                        let one = (one.ceil() + 1.0).min(share);
                        (*o, room, flex) = (Some(one), (room - one).max(0.0), flex - 1);
                    }
                }
                let share = room / flex.max(1) as f32;
                let mut fills = Vec::new();
                for (c, o) in children.iter().zip(own) {
                    let first = self.fills;
                    let s = self.measure(ts, c, o.unwrap_or(share), None);
                    (x, h) = (x + s.0 + gap, h.max(s.1));
                    fills.extend((self.fills > first).then_some((first, s.1)));
                }
                // A child's first Fill grows by what the child is short of the Row's height.
                for (fill, ch) in fills.into_iter().filter(|_| !self.again) {
                    self.slack[fill] += h - ch;
                }
                let x = x - if children.is_empty() { 0.0 } else { gap };
                if strip {
                    self.extents[e] = x;
                }
                (if strip { w } else { x }, h)
            }
            // 3 to 12 rows as the text has lines; 3 and what is left in a Fill.
            Node::Code { id, text, .. } => {
                let n = self.texts.codes.iter().find(|c| c.0 == *id).map(|c| c.1.ed.line_count());
                let rows = grow.map_or(n.unwrap_or(text.lines().count()).clamp(3, 12), |_| 3);
                (w, rows as f32 * ts.line_height(t.mono()) + 12.0 + grow.unwrap_or(0.0))
            }
            Node::Text { style, text, .. } => {
                let style = style_of(*style, t);
                (w, ts.wrap(text, style, w).len() as f32 * ts.line_height(style))
            }
            Node::Button { variant: Variant::Chip | Variant::On, label, .. } => {
                (chip_width(ts, t, label), if self.touch { TOUCH } else { CHIP_H })
            }
            Node::Button { variant: Variant::Quiet, label, .. } => {
                (quiet_width(ts, t, label), if self.touch { TOUCH } else { BUTTON_H })
            }
            Node::Button { label, .. } => (ui::button_width(ts, t, label), BUTTON_H),
            Node::Spacer { px: side } | Node::Glyph { size: side, .. } => {
                (f32::from(*side), f32::from(*side))
            }
            Node::Input { .. } => (w, FIELD_H),
            Node::Item { .. } => (w, ITEM_H),
            Node::Entry { .. } => (w, ENTRY_H),
            Node::Toggle { .. } => (w, TOGGLE_H),
            // As tall as its rows, and what it takes of a Fill's room.
            Node::Area { id, .. } => {
                let (y, a) = (self.y, self.texts.areas.iter_mut().find(|a| a.0 == *id));
                let h = a.map_or(area::MIN_H, |a| a.1.layout(ts, t.body(), w, y));
                (w, h + grow.unwrap_or(0.0))
            }
            Node::Separator => (w, 1.0),
            Node::Grid { cols, cells, .. } => {
                let (rows, cols) =
                    (cells.len().div_ceil(usize::from(*cols).max(1)), (*cols).max(1));
                let rows = rows as f32;
                self.board(w, rows * side(ts, (w, cols), self.fit), rows * side(ts, (w, cols), 1.0))
            }
        };
        self.sizes[at] = size;
        size
    }

    /// The next size to draw; `take` moves past it.
    fn next(&mut self, take: bool) -> (f32, f32) {
        self.i += usize::from(take);
        self.sizes.get(self.i - usize::from(take)).copied().unwrap_or_default()
    }

    /// The next Scroll's content height or Strip's content width, moving past it.
    fn extent(&mut self) -> f32 {
        self.e += 1;
        self.extents.get(self.e - 1).copied().unwrap_or_default()
    }

    /// Draws `nodes` down from `(x, y)`, `gap` apart.
    fn draw_stack(&mut self, ui: &mut Ui<'_>, nodes: &[Node], (x, mut y): (f32, f32), gap: f32) {
        for n in nodes {
            let h = self.next(false).1;
            self.draw(ui, n, x, y);
            y += h + gap;
        }
    }

    /// Draws `n` at `(x, y)` in its measured size: a leaf out of the clip (scrolled away) not at
    /// all, but for an editor, which keeps where it was drawn.
    fn draw(&mut self, ui: &mut Ui<'_>, n: &Node, x: f32, y: f32) {
        let ((w, h), t) = (self.next(true), self.t);
        // Grids count in the frame's order, drawn or not.
        self.nth += u32::from(matches!(n, Node::Grid { .. }));
        let r = ui.snapped(RectF::new(x, y, w, h));
        let clip = ui.list().clip();
        let editor = matches!(n, Node::Code { .. } | Node::Area { .. });
        if n.children().is_empty() && !editor && (r.y >= clip.y + clip.h || r.y + r.h <= clip.y) {
            return;
        }
        let at = |ui: &mut Ui<'_>, f: &mut dyn FnMut(&mut Ui<'_>)| ui.within(r, |ui| f(ui));
        match n {
            Node::Col { gap, children: c, .. } => self.draw_stack(ui, c, (x, y), f32::from(*gap)),
            Node::Center { gap, children, .. } => {
                let mut y = y;
                for c in children {
                    let (cw, ch) = self.next(false);
                    match c {
                        Node::Text { style, text, .. } => {
                            self.next(true);
                            lines(ui, text, style_of(*style, t), (x, w), y);
                        }
                        c => self.draw(ui, c, x + (w - cw) / 2.0, y),
                    }
                    y += ch + f32::from(*gap);
                }
            }
            Node::Fill { children, .. } | Node::Pane { children, .. } => {
                self.draw_stack(ui, children, (x, y), SPACING)
            }
            Node::Card { children, .. } => {
                ui.raised(r);
                self.draw_stack(ui, children, (x + CARD_PAD, y + CARD_PAD), SPACING);
            }
            // Where it was scrolled to, if it is the same Scroll; its thumb at the window's edge.
            Node::Scroll { id, children } => {
                let content = self.extent();
                let was = self.old.iter().find(|s| s.id == *id).map_or(0.0, |s| s.y);
                let down = was.min(content - h).max(0.0);
                ui.push_clip(r);
                self.draw_stack(ui, children, (x, y - down), SPACING);
                ui.pop_clip();
                ui.thumb(RectF { w: (r.w + PAD).min(self.right - r.x), ..r }, down, content);
                self.scrolls.push(Scrolled { id: *id, y: down, content, rect: r });
            }
            // Leaves and Strips center on the row's height; the editors and the other containers
            // keep to its top. A Strip that overflows slides left under a clip, its last child at
            // its right edge.
            Node::Row { gap, children, .. } | Node::Strip { gap, children, .. } => {
                let strip = matches!(n, Node::Strip { .. });
                let over = if strip { (self.extent() - w).max(0.0) } else { 0.0 };
                let mut cx = x - over;
                if strip {
                    ui.push_clip(r);
                }
                for c in children {
                    let (cw, ch) = self.next(false);
                    let edits = matches!(c, Node::Code { .. } | Node::Area { .. });
                    let line = c.children().is_empty() || matches!(c, Node::Strip { .. });
                    let cy = if line && !edits { y + (h - ch) / 2.0 } else { y };
                    self.draw(ui, c, cx, cy);
                    cx += cw + f32::from(*gap);
                }
                if strip {
                    ui.pop_clip();
                }
            }
            Node::Text { style, text, .. } => {
                at(ui, &mut |ui| _ = ui.wrapped(text, style_of(*style, t)))
            }
            Node::Button { id, variant, label } => at(ui, &mut |ui| {
                let id = WidgetId(*id);
                _ = match variant {
                    Variant::Normal => ui.button(id, label),
                    Variant::Primary => ui.button_primary(id, label),
                    Variant::Danger => ui.button_danger(id, label),
                    Variant::Chip | Variant::On => chip(ui, id, r, label, *variant == Variant::On),
                    Variant::Quiet => quiet(ui, id, r, label),
                }
            }),
            Node::Input { id, placeholder, .. } => {
                let value = self.texts.inputs.iter().find(|i| i.0 == *id).map_or("", |i| &i.1);
                let focus = self.texts.focus == *id && *id != 0;
                at(ui, &mut |ui| _ = ui.text_field(WidgetId(*id), value, focus, placeholder));
            }
            Node::Code { id, line_numbers, text, .. } => {
                let focus = self.texts.focus == *id;
                match self.texts.codes.iter_mut().find(|c| c.0 == *id) {
                    Some(c) => c.1.draw(ui, WidgetId(*id), r, *line_numbers, focus),
                    None => at(ui, &mut |ui| _ = ui.wrapped(text, t.mono())),
                }
            }
            Node::Area { id, value, placeholder } => {
                let focus = self.texts.focus == *id && *id != 0;
                match self.texts.areas.iter_mut().find(|a| a.0 == *id) {
                    Some((_, a)) => a.draw(ui, WidgetId(*id), r, focus, placeholder),
                    None => at(ui, &mut |ui| _ = ui.wrapped(value, t.body())),
                }
            }
            Node::Item { id, text, detail, selected } => {
                item(ui, WidgetId(*id), r, [text, detail], *selected)
            }
            // A revealed mark's clock starts at its first draw; only the mark has rings.
            Node::Glyph { glyph, .. } => match Glyph::ALL.get(usize::from(glyph & !REVEAL)) {
                Some(Glyph::Mark) if glyph & REVEAL != 0 => {
                    let since = self.now - *self.reveal.get_or_insert(self.now);
                    mark(ui, r, since);
                }
                Some(&g) => ui.glyph(r, g, t.text),
                None => {}
            },
            Node::Entry { id, glyph, hue, text, detail, more } => {
                entry(ui, WidgetId(*id), r, (*glyph, *hue), [text, detail], *more)
            }
            Node::Toggle { id, on, label } => toggle(ui, WidgetId(*id), r, label, *on),
            Node::Separator => ui.fill(RectF { h: ui.px(1.0), ..r }, 0.0, t.border),
            Node::Spacer { .. } => {}
            Node::Grid { id, cols, cells, texts } => {
                // The square measured: the rows share the height.
                let cols = (*cols).max(1);
                let side = h / cells.len().div_ceil(usize::from(cols)).max(1) as f32;
                let rect = grid(ui, (r, side), *id, cols, (cells, texts));
                let (id, nth, n) = (*id, self.nth - 1, cells.len() as u32);
                self.grids.extend((id != 0).then_some(Board { id, nth, rect, cols, n }));
            }
        }
    }
}

/// A Grid's squares centered across `r` in their colors ([`color`]), a text in each that has one
/// and room (made to fit); the empty ones outlined, as text fields are, and the whole where no
/// gap parts them, so a board shows on any surface. With an id, a click hit over the squares
/// and, for the AI, a mark of its size, a row of colors a line, then each text by its square.
/// Where the squares are.
fn grid(
    ui: &mut Ui<'_>,
    (r, side): (RectF, f32),
    id: u32,
    cols: u16,
    (cells, texts): (&[u8], &[String]),
) -> RectF {
    let (t, ts) = (ui.theme(), ui.text_system());
    let n = usize::from(cols);
    let x = ts.snap(r.x + (r.w - side * f32::from(cols)) / 2.0);
    let at = RectF::new(x, r.y, side * f32::from(cols), r.h);
    let (edge, round) = (ui.px(1.0), (side / 8.0).floor());
    let gap = if side < 10.0 { 0.0 } else { edge };
    for (i, &c) in cells.iter().enumerate() {
        let (cx, cy) = (x + (i % n) as f32 * side, r.y + (i / n) as f32 * side);
        let square = RectF::new(cx, cy, side - gap, side - gap);
        let (fill, ink) = color(t, c);
        ui.fill(square, round, fill);
        if c == 0 && gap > 0.0 {
            ui.border(square, round, edge, t.border);
        }
        if let Some(text) = texts.get(i).filter(|s| !s.is_empty() && side >= 14.0) {
            fit(ui, square, text, TextStyle::new(FontId::Sans, (side * 0.5).round(), ink));
        }
    }
    if gap == 0.0 {
        ui.border(at, 0.0, edge, t.border);
    }
    if id != 0 {
        ui.hit(WidgetId(id), at, Sense::Click);
        if ui.list().sem().is_some() {
            let mut v = String::new();
            for (num, k) in [(n, " columns, "), (cells.len().div_ceil(n), " rows")] {
                ui::push_num(&mut v, num);
                v += k;
            }
            for (i, &c) in cells.iter().enumerate() {
                if i % n == 0 {
                    v.push('\n');
                }
                v.push(char::from(b'0' + c));
            }
            for (i, s) in texts.iter().enumerate() {
                if !s.is_empty() {
                    v.push('\n');
                    ui::push_num(&mut v, i);
                    v += ": ";
                    v += s;
                }
            }
            ui.mark(WidgetId(id), ui::sem::GRID, 0, &v);
        }
    }
    at
}

/// The fill of a Grid's square of color `c` in theme `t`, and the ink of its text: 0 the
/// sunken well, 1 to 8 the palette (ANSI 1 to 8); but 7, silver, is the faint ink on a light
/// theme, whose ANSI white is dark and next to 8, gray.
fn color(t: &Theme, c: u8) -> (Rgba, Rgba) {
    match c {
        0 => (t.surface_lo, t.text),
        7 if !t.dark => (t.text_faint, t.text),
        c => (t.ansi[usize::from(c.min(8))], t.base),
    }
}

/// `text` centered in the square `r` in `style`, smaller where that is too wide (2 px spare
/// each side; 8 px at least), cut short where even that is.
fn fit(ui: &mut Ui<'_>, r: RectF, text: &str, mut style: TextStyle) {
    let (ts, room) = (ui.text_system(), r.w - 4.0);
    let w = ts.measure(text, style);
    if w > room {
        style.size = (style.size * room / w).floor().max(8.0);
    }
    let shown = ts.ellipsize(text, style, room);
    label_in(ui, r, &shown, style);
}

/// The width a Button, Spacer, Glyph or Pane takes in a Row `w` wide; `None` for the others.
fn own_width(ts: &mut TextSystem, t: &Theme, n: &Node, w: f32) -> Option<f32> {
    match n {
        Node::Button { variant: Variant::Chip | Variant::On, label, .. } => {
            Some(chip_width(ts, t, label))
        }
        Node::Button { variant: Variant::Quiet, label, .. } => Some(quiet_width(ts, t, label)),
        Node::Button { label, .. } => Some(ui::button_width(ts, t, label)),
        Node::Spacer { px: side } | Node::Glyph { size: side, .. } => Some(f32::from(*side)),
        Node::Pane { w: pw, .. } => Some(f32::from(*pw).min(w)),
        _ => None,
    }
}

/// `w` logical px on whole device pixels, rounded up.
fn device(ts: &TextSystem, w: f32) -> f32 {
    (w * ts.dpr()).ceil() / ts.dpr()
}

/// The width of a chip labeled `label`, and of a quiet button.
fn chip_width(ts: &mut TextSystem, t: &Theme, label: &str) -> f32 {
    let w = ts.measure(label, t.small()) + 2.0 * CHIP_PAD;
    device(ts, w)
}

fn quiet_width(ts: &mut TextSystem, t: &Theme, label: &str) -> f32 {
    let w = (ts.measure(label, t.body()) + 16.0).max(BUTTON_H);
    device(ts, w)
}

/// compusophy's mark in the text color, in the square centered in `r`, `ms` into its reveal:
/// from the center out, the center dot and then each ring of dots fades in, then the glyph
/// stands (as it does at once with no clock: NaN).
fn mark(ui: &mut Ui<'_>, r: RectF, ms: f64) {
    let t = ui.theme();
    if ms.is_nan() || ms >= REVEAL_MS {
        return ui.glyph(r, Glyph::Mark, t.text);
    }
    // The box the glyph fills: a whole number of device pixels, its edges on them.
    let d = ui.text_system().dpr();
    let side = (r.w.min(r.h) * d).round();
    let left = ((r.x + r.w / 2.0) * d - side / 2.0).round();
    let top = ((r.y + r.h / 2.0) * d + side / 2.0).round() - side;
    let (cx, cy, k) = ((left + side / 2.0) / d, (top + side / 2.0) / d, side / d / 1000.0);
    // Ring `i` (the center dot first) fades in, smoothstepped, `STEP` after the one inside it.
    let ink = |i: usize| {
        let x = ((ms - i as f64 * STEP) / FADE).clamp(0.0, 1.0) as f32;
        t.text.with_alpha((f32::from(t.text.3) * x * x * (3.0 - 2.0 * x)).round() as u8)
    };
    let dot = |ui: &mut Ui<'_>, (x, y): (f32, f32), radius: f32, color: Rgba| {
        let s = radius * k;
        ui.fill(RectF::new(x - s, y - s, 2.0 * s, 2.0 * s), s, color);
    };
    dot(ui, (cx, cy), MARK_HOLE, ink(0));
    for (i, (n, at, size)) in rings().enumerate() {
        for j in 0..n {
            let a = core::f32::consts::FRAC_PI_2 - core::f32::consts::TAU * j as f32 / n as f32;
            dot(ui, (cx + at * k * cos(a), cy - at * k * sin(a)), size, ink(i + 1));
        }
    }
}

/// The baseline that centers the capitals of `style` in a band `h` tall from `top`.
fn cap_base(ui: &mut Ui<'_>, top: f32, h: f32, style: TextStyle) -> f32 {
    ui.text_system().snap(top + (h + CAP * style.size) / 2.0)
}

/// `label` centered in `r`.
fn label_in(ui: &mut Ui<'_>, r: RectF, label: &str, style: TextStyle) {
    let base = cap_base(ui, r.y, r.h, style);
    let ts = ui.text_system();
    let lw = ts.measure(label, style);
    let x = ts.snap(r.x + (r.w - lw) / 2.0);
    ui.text(x, base, label, style);
}

/// `text` wrapped to `w`, each line centered on `x + w / 2`, from `top` down.
fn lines(ui: &mut Ui<'_>, text: &str, style: TextStyle, (x, w): (f32, f32), top: f32) {
    let ts = ui.text_system();
    let (lh, a, d) = (ts.line_height(style), ts.ascent(style), ts.descent(style));
    let base = ts.snap((lh - a - d) / 2.0 + a);
    for (i, line) in ts.wrap(text, style, w).into_iter().enumerate() {
        let ts = ui.text_system();
        let lw = ts.measure(line, style);
        let (lx, ly) = (ts.snap(x + (w - lw) / 2.0), ts.snap(top + i as f32 * lh));
        ui.text(lx, ly + base, line, style);
    }
}

/// A chip in `r`: `label` small on a pill, brightening under the pointer, or on the accent when
/// `on`; a [`Sense::Click`] hit.
fn chip(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str, on: bool) -> RectF {
    let (t, s) = (ui.theme(), ui.state());
    let hover = s.hover == Some(id);
    let ink = if on {
        ui.fill(r, r.h / 2.0, t.accent);
        t.accent_text
    } else {
        ui.fill(r, r.h / 2.0, t.surface_lo);
        if hover {
            ui.fill(r, r.h / 2.0, t.wash(s.pressed == Some(id)));
        }
        let edge = ui.px(1.0);
        ui.border(r, r.h / 2.0, edge, t.border);
        if hover { t.text } else { t.text_dim }
    };
    label_in(ui, r, label, t.small().with_color(ink));
    ui.hit(id, r, Sense::Click);
    r
}

/// A quiet button in `r`: `label` dim, bright on a wash under the pointer; a click hit.
fn quiet(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str) -> RectF {
    let (t, s) = (ui.theme(), ui.state());
    let hover = s.hover == Some(id);
    if hover {
        ui.fill(r, RADIUS_SM, t.wash(s.pressed == Some(id)));
    }
    label_in(ui, r, label, t.body().with_color(if hover { t.text } else { t.text_dim }));
    ui.hit(id, r, Sense::Click);
    r
}

/// A list row in `r`: `text`, then `detail` dim at the right; washed under
/// the pointer, tinted when `selected`; a [`Sense::Click`] hit.
fn item(ui: &mut Ui<'_>, id: WidgetId, r: RectF, [text, detail]: [&String; 2], selected: bool) {
    let (t, s) = (ui.theme(), ui.state());
    if selected || s.hover == Some(id) {
        ui.fill(r, RADIUS_SM, if selected { t.selection } else { t.wash(s.pressed == Some(id)) });
    }
    let (body, small) = (t.body(), t.small());
    let base = cap_base(ui, r.y, r.h, body);
    let ts = ui.text_system();
    let dw = ts.measure(detail, small);
    let right = ts.snap(r.x + r.w - 12.0 - dw);
    ui.push_clip(RectF { w: (right - r.x - 12.0).max(0.0), ..r });
    ui.text(r.x + 12.0, base, text, body);
    ui.pop_clip();
    ui.push_clip(r);
    ui.text(right, base, detail, small);
    ui.pop_clip();
    ui.hit(id, r, Sense::Click);
}

/// An Entry in `r`: its tile (a `.app` file's sigil for [`SIGIL`]), `text` (cut to fit; a second
/// line small under the first), `detail` small at the right and a chevron when `more`; washed
/// under the pointer, a hairline under it from the text on; a click hit.
fn entry(
    ui: &mut Ui<'_>,
    id: WidgetId,
    r: RectF,
    (glyph, hue): (u8, u32),
    [text, detail]: [&String; 2],
    more: bool,
) {
    let (t, s) = (ui.theme(), ui.state());
    if s.hover == Some(id) {
        ui.fill(r, RADIUS_SM, t.wash(s.pressed == Some(id)));
    }
    let at = ui.snapped(RectF::new(r.x + 8.0, r.y + (r.h - TILE) / 2.0, TILE, TILE));
    match Glyph::ALL.get(usize::from(glyph)) {
        _ if glyph == SIGIL => ui.sigil(at, hue),
        Some(&glyph) => ui.app_icon(at, AppIcon { glyph, hue: Rgba::hex(hue) }),
        None => {}
    }
    let (x, mut right) = (at.x + TILE + 13.0, r.x + r.w - 8.0);
    if more {
        let at = ui.snapped(RectF::new(right - 13.0, r.y + (r.h - 13.0) / 2.0, 13.0, 13.0));
        ui.glyph(at, Glyph::Chevron, t.text_dim);
        right = at.x - 8.0;
    }
    let (body, small) = (t.body(), t.small());
    if !detail.is_empty() {
        let base = cap_base(ui, r.y, r.h, small);
        let ts = ui.text_system();
        let dw = ts.measure(detail, small);
        let dx = ts.snap(right - dw);
        ui.text(dx, base, detail, small);
        right = dx - 13.0;
    }
    // Its lines, as wrap splits them (no search of its own: the boot download is small).
    let (room, ts) = ((right - x).max(0.0), ui.text_system());
    let lines = ts.wrap(text, body, f32::MAX);
    let (name, line) = (lines.first().copied().unwrap_or(""), lines.get(1).copied().unwrap_or(""));
    let (name, line) = (ts.ellipsize(name, body, room), ts.ellipsize(line, small, room));
    // One line centered; two each centered in its own line's band, the pair in the row.
    let (nh, sh) = (ts.line_height(body), ts.line_height(small) * f32::from(!line.is_empty()));
    let top = r.y + (r.h - nh - sh) / 2.0;
    let (b1, b2) = (cap_base(ui, top, nh, body), cap_base(ui, top + nh, sh, small));
    ui.text(x, b1, &name, body);
    ui.text(x, b2, &line, small);
    let line = ui.px(1.0);
    let rule = ui.snapped(RectF::new(x, r.y + r.h - line, r.x + r.w - x, line));
    ui.fill(RectF { h: line, ..rule }, 0.0, t.border);
    ui.hit(id, r, Sense::Click);
}

/// A Toggle in `r`: `label` (cut to fit) and a switch 34 x 21 at the right, its track the accent
/// and its knob right when `on`, else sunken with its knob left; washed under the pointer; a
/// click hit.
fn toggle(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str, on: bool) {
    let (t, s, line) = (ui.theme(), ui.state(), ui.px(1.0));
    if s.hover == Some(id) {
        ui.fill(r, RADIUS_SM, t.wash(s.pressed == Some(id)));
    }
    let track = RectF::new(r.x + r.w - 8.0 - 34.0, r.y + (r.h - 21.0) / 2.0, 34.0, 21.0);
    let track = ui.snapped(track);
    let (fill, knob) = if on { (t.accent, t.accent_text) } else { (t.surface_lo, t.text_dim) };
    ui.fill(track, 10.5, fill);
    if !on {
        ui.border(track, 10.5, line, t.border);
    }
    let x = if on { track.x + track.w - 18.0 } else { track.x + 3.0 };
    let dot = ui.snapped(RectF::new(x, track.y + 3.0, 15.0, 15.0));
    ui.fill(dot, dot.w / 2.0, knob);
    let body = t.body();
    let shown = ui.text_system().ellipsize(label, body, (track.x - r.x - 21.0).max(0.0));
    let base = cap_base(ui, r.y, r.h, body);
    ui.text(r.x + 8.0, base, &shown, body);
    ui.hit(id, r, Sense::Click);
}

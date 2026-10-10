//! The programs a window's frame holds ([`Node::Embed`]): each a [`Remote`] of its own, run in a
//! process of its own and drawn in the rect its holder's frame gives it, so an app holds apps
//! and they hold theirs at any depth (the fractal). A held program cannot tell: its window is
//! the rect (told as its size: [`Event::Resize`]), the pointer is in its rect's own place, its
//! keys come while it has the focus (a press in it gives it, a press outside takes it back), and
//! its ids are its own: they are shifted apart from its holder's on the way in ([`SHIFT`] times a
//! slot the top window hands out) and back on the way out, so a button `id + i` (a Pages' page)
//! stays one. A held program's Close and Size are not honored (only the top window's program
//! shapes the window), and its end leaves a note where it was, never closing the window. A frame
//! without an Embed ends what it held there ([`wire::KILLED`]); another program or args under
//! the same id starts it anew. Held programs go [`MAX_DEPTH`] deep at most and a window holds
//! [`MAX_HELD`] at a level, each one process of the kernel's ([`wire::MAX_PROCS`]); one whose
//! rect is under [`LEAST`] px either way waits to start until it grows.

use std::cell::Cell;
use std::rc::Rc;

use gfx::RectF;
use ui::kernel::wire;
use ui::{App, AppEvent, Cx, Ui, WidgetId};
use uiwire::{Event, Frame, Node, Request};

use super::Remote;

/// How far a held program's ids are shifted per slot: slot `n`'s ids are its own plus `n << 24`.
pub const SHIFT: u32 = 24;
/// The most held programs a window holds at a level, and how deep they go (the top window's
/// program is 0, the programs it holds 1).
pub const MAX_HELD: usize = 8;
pub const MAX_DEPTH: u8 = 5;
/// The least side, in logical px, of a held program's window it starts in.
pub const LEAST: f32 = 24.0;

/// What a [`Remote`] holds and, when it is held, how: the programs its frame holds, which has
/// the keys (none: its own), which the pointer last pressed, and whether one's size waits to be
/// told; when held, its ids' shift and its depth; the slots the top window handed out so far.
#[derive(Default)]
pub(super) struct Nest {
    kids: Vec<Kid>,
    focus: Option<usize>,
    press: Option<usize>,
    due: bool,
    shift: u32,
    pub(super) depth: u8,
    slots: Rc<Cell<u32>>,
}

/// A held program: its Embed's id (as the holder's frame has it, shifted), what it runs, its
/// window, and its rect as last drawn.
struct Kid {
    id: u32,
    run: (String, String),
    win: Box<Remote>,
    rect: Option<RectF>,
}

impl Nest {
    /// The next slot's shift, from those the top window handed out (255 slots, then again).
    fn next_shift(&self) -> u32 {
        let n = self.slots.get() % 255 + 1;
        self.slots.set(n);
        n << SHIFT
    }
}

/// The Embeds in `nodes` (in a container or not), as id, program and args, in order.
fn embeds(nodes: &[Node], out: &mut Vec<(u32, String, String)>) {
    for n in nodes {
        match n {
            Node::Embed { id, program, args, .. } => {
                out.push((*id, program.clone(), args.clone()));
            }
            _ => embeds(n.children(), out),
        }
    }
}

/// Whether node id `id` is in `nodes`.
fn has_id(nodes: &[Node], id: u32) -> bool {
    let mut found = false;
    for n in nodes {
        let mut n = n.clone();
        n.ids_mut(&mut |i| found |= *i == id);
        if found {
            return true;
        }
    }
    false
}

impl Remote {
    /// The window of `program` (a `/bin` name) held at `depth` with `args` (`\t` apart), its ids
    /// shifted by a slot the top window's `slots` hand out.
    fn nested(
        program: &str,
        args: &str,
        ai: &crate::ai::Ai,
        depth: u8,
        slots: &Rc<Cell<u32>>,
    ) -> Remote {
        let words = args.split('\t').filter(|a| !a.is_empty());
        let argv = std::iter::once(program).chain(words).map(String::from).collect();
        let mut r = Remote::new(&["/bin/", program].concat(), argv, ai);
        r.title = program.into();
        r.nest.slots = slots.clone();
        (r.nest.shift, r.nest.depth) = (r.nest.next_shift(), depth);
        r
    }

    /// Whether this window is one a frame holds.
    pub(super) fn is_held(&self) -> bool {
        self.nest.depth > 0
    }

    /// A frame from this one's program, its ids shifted apart from its holder's (a Focus's too).
    pub(super) fn shift_in(&self, frame: &mut Frame) {
        let shift = self.nest.shift;
        if shift == 0 {
            return;
        }
        let on = |id: &mut u32| *id = if *id == 0 { 0 } else { id.wrapping_add(shift) };
        frame.nodes.iter_mut().for_each(|n| n.ids_mut(&mut |id| on(id)));
        for r in &mut frame.requests {
            if let Request::Focus { id } = r {
                on(id);
            }
        }
    }

    /// An event for this one's program, the node it names shifted back to the program's own.
    pub(super) fn shift_out(&self, ev: &mut Event) {
        let shift = self.nest.shift;
        if let Some(id) = ev.id_mut().filter(|id| shift != 0 && **id != 0) {
            *id = id.wrapping_sub(shift);
        }
    }

    /// The programs `nodes` hold, after a frame: those gone (or changed) end, those new are
    /// made, to start at their first size.
    pub(super) fn hold_kids(&mut self, nodes: &[Node], cx: &mut Cx<'_>) {
        let mut want = Vec::new();
        embeds(nodes, &mut want);
        want.truncate(MAX_HELD);
        let deep = self.nest.depth >= MAX_DEPTH;
        let mut i = 0;
        while i < self.nest.kids.len() {
            let k = &self.nest.kids[i];
            if deep || !want.iter().any(|w| w.0 == k.id && (&w.1, &w.2) == (&k.run.0, &k.run.1)) {
                let mut k = self.nest.kids.remove(i);
                k.win.end(cx);
                (self.nest.focus, self.nest.press) = (None, None);
            } else {
                i += 1;
            }
        }
        if deep {
            return;
        }
        for (id, program, args) in want {
            if !self.nest.kids.iter().any(|k| k.id == id) {
                let (depth, slots) = (self.nest.depth + 1, &self.nest.slots);
                let win = Box::new(Remote::nested(&program, &args, &self.ai, depth, slots));
                self.nest.kids.push(Kid { id, run: (program, args), win, rect: None });
            }
        }
    }

    /// Ends this held program and those it holds: Close, then killed and reaped.
    fn end(&mut self, cx: &mut Cx<'_>) {
        self.nest.kids.iter_mut().for_each(|k| k.win.end(cx));
        self.closing(cx);
        if let Some(pid) = self.pid.take() {
            cx.kernel.kill(pid, wire::KILLED);
            _ = cx.kernel.reap(pid);
            self.ai.ask(pid, Request::Close);
        }
    }

    /// The held program whose rect (as last drawn) holds `(x, y)` in the page, the last drawn on
    /// top.
    fn kid_at(&self, x: f32, y: f32) -> Option<usize> {
        self.nest.kids.iter().rposition(|k| k.rect.is_some_and(|r| r.contains(x, y)))
    }

    /// `(x, y)` in this window, as in held program `k`'s.
    fn kid_xy(&self, k: usize, x: f32, y: f32) -> (f32, f32) {
        let o = self.nest.kids[k].win.origin;
        (x + self.origin.0 - o.0, y + self.origin.1 - o.1)
    }

    /// The keys go to held program `k` (none: this one's): each told it gained or lost them.
    fn focus_kid(&mut self, k: Option<usize>, cx: &mut Cx<'_>) -> bool {
        let was = std::mem::replace(&mut self.nest.focus, k);
        if was == k {
            return false;
        }
        if k.is_some() {
            self.texts.focus = 0;
        }
        let tell = |s: &mut Self, k: Option<usize>, on: bool, cx: &mut Cx<'_>| {
            if let Some(kid) = k.and_then(|k| s.nest.kids.get_mut(k)) {
                kid.win.event(AppEvent::Focus(on), cx);
            }
        };
        tell(self, was, false, cx);
        tell(self, k, true, cx);
        true
    }

    /// `ev` for a held program, if it is one's (the pointer in its rect or pressed there, a
    /// click on its ids, a key while it has the focus): handled there, whether to redraw.
    pub(super) fn route(&mut self, ev: &AppEvent, cx: &mut Cx<'_>) -> Option<bool> {
        if self.nest.kids.is_empty() {
            return None;
        }
        let (k, ev) = match *ev {
            AppEvent::PointerDown { x, y, id } => {
                let (px, py) = (x + self.origin.0, y + self.origin.1);
                let k = self.kid_at(px, py);
                self.nest.press = k;
                let refocus = self.focus_kid(k, cx);
                // Outside them: this window's own press (redrawn as it handles it).
                let (Some(k), _) = (k, refocus) else { return None };
                let (x, y) = self.kid_xy(k, x, y);
                (k, AppEvent::PointerDown { x, y, id })
            }
            AppEvent::Drag { x, y } => {
                let k = self.nest.press?;
                let (x, y) = self.kid_xy(k, x, y);
                (k, AppEvent::Drag { x, y })
            }
            AppEvent::Wheel { x, y, dy } => {
                let k = self.kid_at(x + self.origin.0, y + self.origin.1)?;
                let (x, y) = self.kid_xy(k, x, y);
                (k, AppEvent::Wheel { x, y, dy })
            }
            AppEvent::Click(WidgetId(id)) => {
                let mine = self.frame.as_ref().is_some_and(|f| has_id(&f.nodes, id));
                let theirs = self.nest.kids.iter().position(|k| k.win.shows(id));
                let k = theirs.or(if mine { None } else { self.nest.press.or(self.nest.focus) })?;
                (k, AppEvent::Click(WidgetId(id)))
            }
            AppEvent::Key { key, mods } => (self.nest.focus?, AppEvent::Key { key, mods }),
            AppEvent::Text(ref s) => (self.nest.focus?, AppEvent::Text(s.clone())),
            _ => return None,
        };
        let kid = self.nest.kids.get_mut(k)?;
        Some(kid.win.event(ev, cx))
    }

    /// Whether node id `id` is one this window or a program it holds shows.
    fn shows(&self, id: u32) -> bool {
        self.frame.as_ref().is_some_and(|f| has_id(&f.nodes, id))
            || self.nest.kids.iter().any(|k| k.win.shows(id))
    }

    /// A Tick, the kernel's io or the window's focus for the programs it holds, and the sizes
    /// due (a rect changed as drawn): whether to redraw.
    pub(super) fn kids(&mut self, ev: &AppEvent, cx: &mut Cx<'_>) -> bool {
        let mut redraw = false;
        if std::mem::take(&mut self.nest.due) {
            for k in &mut self.nest.kids {
                let Some(r) = k.rect.filter(|r| r.w >= LEAST && r.h >= LEAST) else { continue };
                let size = (r.w as u16, r.h as u16);
                if k.win.told != Some(size) {
                    redraw |= k.win.event(AppEvent::Resized { w: r.w, h: r.h }, cx);
                }
            }
        }
        let focus = self.nest.focus;
        for (i, k) in self.nest.kids.iter_mut().enumerate() {
            let ev = match ev {
                AppEvent::Tick { now_ms } => AppEvent::Tick { now_ms: *now_ms },
                AppEvent::Io => AppEvent::Io,
                AppEvent::Focus(on) if focus == Some(i) => AppEvent::Focus(*on),
                _ => continue,
            };
            redraw |= k.win.event(ev, cx);
        }
        redraw
    }

    /// Frame `frame` from `pid`, if a program this one holds runs as it: whether to redraw.
    pub(super) fn kids_frame(&mut self, pid: u32, frame: &[u8], cx: &mut Cx<'_>) -> bool {
        self.nest.kids.iter_mut().any(|k| k.win.frame(pid, frame, cx))
    }

    /// When the programs it holds want a frame (a size due: now).
    pub(super) fn kids_frame_in(&self, now_ms: f64) -> Option<u32> {
        let due = self.nest.due.then_some(0);
        self.nest.kids.iter().filter_map(|k| k.win.frame_in(now_ms)).chain(due).min()
    }

    /// The window wants text input for a held program with the keys.
    pub(super) fn kid_types(&self) -> Option<bool> {
        self.nest.focus.and_then(|k| self.nest.kids.get(k)).map(|k| k.win.wants_text_input())
    }

    /// The squares of board `id` if a program it holds drew it.
    pub(super) fn kid_squares(&self, id: u32) -> Option<u32> {
        self.nest.kids.iter().find_map(|k| k.win.squares(id))
    }

    /// The window is closing: so are the programs it holds.
    pub(super) fn kids_closing(&mut self, cx: &mut Cx<'_>) {
        self.nest.kids.iter_mut().for_each(|k| k.win.closing(cx));
    }

    /// Draws each held program in its rect as its holder's frame laid it out (a size changed:
    /// told at the next tick); one not drawn there (scrolled away) has none.
    pub(super) fn draw_kids(&mut self, ui: &mut Ui<'_>) {
        for k in &mut self.nest.kids {
            k.rect = self.view.embeds.iter().find(|e| e.0 == k.id).map(|e| e.1);
            let Some(r) = k.rect else { continue };
            if k.win.told != Some((r.w as u16, r.h as u16)) && r.w >= LEAST && r.h >= LEAST {
                self.nest.due = true;
            }
            ui.push_clip(r);
            ui.within(r, |ui| k.win.draw(ui));
            ui.pop_clip();
        }
    }
}

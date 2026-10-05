//! The smoke test: a program run as a person and a clock would, before anyone sees it. It
//! renders (an app that shows nothing at first faults: `SHOWS_NOTHING`), clicks every button it
//! shows once (each as it shows by then, by its text: an earlier click may have moved or hidden
//! it), then lets [`TICKS`] ticks pass (each the program's shortest interval, every [`SLOW`]th
//! its longest, so a slow `every` runs too): every 5th it presses the next key the program
//! handles, every 7th taps a square of a grid or a unit of a canvas that has a handler, every
//! 11th clicks a button, every 13th types "12", "hello" or "" in each input, re-rendering after
//! each event and taking each from what shows by then (a key that ends a game may hide the
//! buttons: that tick's click presses one still shown, if any). Then, if it saves state, it is
//! closed and opened again with what it saved, as a person comes back to it: it renders, each
//! button it shows is clicked once and [`AGAIN`] ticks pass. It stops at the first fault, or
//! after [`BUDGET`] steps in all, and says what it saw. Canvases that showed, none of whose
//! shapes ever reached inside one, are a fault at the end (`OFF_CANVAS`): a picture drawn in the
//! window's pixels, not the canvas's units. A text whose point is on its canvas and that is
//! wider than it is a fault of the render that drew it (`TEXT_TOO_WIDE`): the desktop keeps such
//! a text inside its canvas, but for one that cannot fit (one whose point is off the canvas it
//! leaves where it is).

use applang_syntax::ast::Widget;

use crate::{App, Diag, Draw, Event, Limits, Node, Program, Shape, Span, codes};

/// The ticks a smoke test lets pass, and the most steps it spends.
pub const TICKS: u32 = 300;
const BUDGET: u64 = 20_000_000;
/// Every this many ticks, one lets the program's longest interval pass.
const SLOW: u32 = 20;
/// The ticks that pass after it is opened again.
const AGAIN: u32 = 10;
/// The last events a fault's account keeps.
const BEFORE: usize = 5;
/// What the first render after it is opened again is called.
const REOPENED: &str =
    "the first render after closing and opening it again (only saved states keep their values)";

/// The first fault: its diagnostic, what was done when it came ("clicking \"Start\"", "tick
/// 12", "the render after key \"left\""), and the events before that, oldest first.
#[derive(Debug, Clone)]
pub struct Fault {
    pub diag: Diag,
    pub during: String,
    pub before: Vec<String>,
}

/// What a smoke test saw: the first fault, the most steps one event or render took (and
/// which), how many events ran, the steps they took in all, and what looked wrong without
/// faulting.
#[derive(Debug, Clone, Default)]
pub struct Smoke {
    pub fault: Option<Fault>,
    pub most: (u64, String),
    pub events: u32,
    pub spent: u64,
    pub warnings: Vec<String>,
}

/// Smoke-tests `program` with `random` seeded by `seed` (see the module docs).
pub fn smoke(program: Program, seed: u64) -> Smoke {
    smoke_from(program, seed, "")
}

/// [`smoke`], the program started as it will really start: from `saved` ([`App::saved`]'s lines,
/// what its file of saved states holds), whatever they show ("": none).
pub fn smoke_from(program: Program, seed: u64, saved: &str) -> Smoke {
    let keys: Vec<String> = program.keys().iter().map(|k| k.key.clone()).collect();
    let (timed, canvas) = (!program.everys().is_empty(), first_canvas(program.widgets()));
    let mut app = App::new(program, Limits::default(), seed);
    if !saved.is_empty() {
        app.take_back(saved);
    }
    let mut t = Tester { app: Some(app), at: canvas, ..Tester::default() };
    t.rng = seed | 1;
    let Some(first) = t.render("the first render") else { return t.out };
    if first.is_empty() {
        let msg = "the app shows nothing: it has no widgets, or none shows at first";
        t.fault(Diag::new_code(codes::SHOWS_NOTHING, msg), "the first render".into());
        return t.out;
    }
    if !t.click_each(&first, "") {
        return t.out;
    }
    let mut changed = false;
    for n in 1..=TICKS {
        let ms = t.ms(n % SLOW == 0);
        let before = t.last.clone();
        if !t.step(Event::Tick { ms }, format!("tick {n}"), true) {
            return t.out;
        }
        changed |= t.last != before;
        if t.events(n, &keys).is_none() {
            return t.out;
        }
        if t.out.spent > BUDGET {
            break;
        }
    }
    let saved = t.app().saved();
    if !saved.is_empty() && !t.reopen(&saved) {
        return t.out;
    }
    if let (Some((w, h)), false, Some(at)) = (t.canvas, t.inside, t.at) {
        let msg = format!(
            "nothing it drew reached inside its canvas of {w} x {h} units: draw at x from 0 to {} \
             and y from 0 to {}, in the canvas's own units, not the window's pixels",
            w - 1,
            h - 1
        );
        t.fault(Diag::at_code(codes::OFF_CANVAS, msg, at), "the whole smoke test".into());
        return t.out;
    }
    if timed && !changed {
        t.out.warnings.push(format!("{TICKS} ticks never changed what it shows"));
    }
    if t.out.most.0 > Limits::default().fuel / 2 {
        let (n, what) = &t.out.most;
        t.out.warnings.push(format!("{what} took {n} steps, over half of the 1,000,000 allowed"));
    }
    t.out
}

/// A smoke test under way: the app, what it showed last, the events so far, its own xorshift
/// for picks, the report, the first canvas shown (its size), whether a shape ever reached
/// inside a canvas, and where the program's first canvas is.
#[derive(Default)]
struct Tester {
    app: Option<App>,
    last: Vec<Node>,
    log: Vec<String>,
    rng: u64,
    out: Smoke,
    canvas: Option<(u16, u16)>,
    inside: bool,
    at: Option<Span>,
}

impl Tester {
    fn app(&mut self) -> &mut App {
        self.app.as_mut().unwrap_or_else(|| unreachable!("made first"))
    }

    /// The ms a tick lets pass: the program's shortest interval, or its longest (`slow`); 100
    /// when no timer runs.
    fn ms(&mut self, slow: bool) -> u32 {
        let all = self.app().intervals().into_iter();
        let ms = if slow { all.max() } else { all.min() };
        ms.unwrap_or(100)
    }

    /// Clicks each button of `first` once, each as it shows by then, by its text (an earlier
    /// click may have moved or hidden it), saying `after` it; whether the test goes on.
    fn click_each(&mut self, first: &[Node], after: &str) -> bool {
        for (_, text) in buttons(first) {
            let now = buttons(&self.last);
            let Some(&(id, _)) = now.iter().find(|b| b.1 == text) else { continue };
            if !self.step(Event::Click { id }, [&quoted("clicking ", &text), after].concat(), false)
            {
                return false;
            }
        }
        true
    }

    /// Tick `n`'s events, each taken from what shows after the one before (so none is for
    /// something gone, `E0213`): every 5th tick the next of `keys`, every 7th a tap, every 11th
    /// a click, every 13th a text typed in each input still shown; `None` once one faults.
    fn events(&mut self, n: u32, keys: &[String]) -> Option<()> {
        if n % 5 == 0 && !keys.is_empty() {
            let name = &keys[(n as usize / 5) % keys.len()];
            self.go(Event::Key { name: name.clone() }, quoted("key ", name))?;
        }
        if n % 7 == 0 {
            let boards = boards(&self.last);
            if let Some(&(id, len, w)) = boards.get(self.pick(boards.len())) {
                let cell = self.pick(len as usize) as u32;
                let what = match w {
                    0 => format!("tapping square {cell}"),
                    w => format!("tapping the canvas at x {}, y {}", cell % w, cell / w),
                };
                self.go(Event::Tap { id, cell }, what)?;
            }
        }
        if n % 11 == 0 {
            let all = buttons(&self.last);
            if let Some((id, text)) = all.get(self.pick(all.len())) {
                self.go(Event::Click { id: *id }, quoted("clicking ", text))?;
            }
        }
        if n % 13 == 0 {
            let text = ["12", "hello", ""][(n as usize / 13) % 3];
            for state in inputs(&self.last) {
                if inputs(&self.last).contains(&state) {
                    let what = [&quoted("typing ", text), " in ", &state].concat();
                    self.go(Event::Input { state, text: text.into() }, what)?;
                }
            }
        }
        Some(())
    }

    /// [`Tester::step`] for an event a person makes: `None` once it faults.
    fn go(&mut self, ev: Event, what: String) -> Option<()> {
        self.step(ev, what, false).then_some(())
    }

    /// Closes the app and opens it again with `saved`, its saved states (anything they could
    /// not bring back is a warning); whether the test goes on.
    fn reopen(&mut self, saved: &str) -> bool {
        let notes = self.app().reopen(saved);
        self.out.warnings.extend(notes);
        let Some(first) = self.render(REOPENED) else { return false };
        let again = " after opening it again";
        if !self.click_each(&first, again) {
            return false;
        }
        (1..=AGAIN).all(|n| {
            let ms = self.ms(false);
            self.step(Event::Tick { ms }, format!("tick {n}{again}"), true)
        })
    }

    /// A number below `n` (0 if `n` is 0).
    fn pick(&mut self, n: usize) -> usize {
        let x = &mut self.rng;
        *x ^= *x << 13;
        *x ^= *x >> 7;
        *x ^= *x << 17;
        (*x % n.max(1) as u64) as usize
    }

    /// Notes `steps` spent by `what`.
    fn spend(&mut self, what: &str) {
        let steps = self.app().steps();
        self.out.spent += steps;
        if steps > self.out.most.0 {
            self.out.most = (steps, what.to_string());
        }
    }

    fn fault(&mut self, diag: Diag, during: String) {
        let before = self.log[self.log.len().saturating_sub(BEFORE)..].to_vec();
        self.out.fault = Some(Fault { diag, during, before });
    }

    /// Renders after `what`: what it shows, or `None` on a fault (a text wider than its canvas
    /// is one: [`wide`]).
    fn render(&mut self, what: &str) -> Option<Vec<Node>> {
        let r = self.app().render();
        self.spend(what);
        let mut wider = None;
        walk(r.as_deref().unwrap_or_default(), &mut |n| {
            if let Node::Canvas { w, h, draws, .. } = n {
                self.canvas.get_or_insert((*w, *h));
                self.inside |= draws.iter().any(|d| reaches(d, *w, *h));
                if wider.is_none() {
                    wider = draws.iter().find_map(|d| wide(d, (*w, *h)));
                }
            }
        });
        let r = match (r, wider, self.at) {
            (Ok(_), Some(msg), Some(at)) => Err(Diag::at_code(codes::TEXT_TOO_WIDE, msg, at)),
            (r, ..) => r,
        };
        match r {
            Ok(nodes) => {
                self.last.clone_from(&nodes);
                Some(nodes)
            }
            Err(d) => {
                self.fault(d, what.into());
                None
            }
        }
    }

    /// Handles `ev` (`what` it is), then renders if a handler ran (or `always`); whether the
    /// test goes on.
    fn step(&mut self, ev: Event, what: String, quiet: bool) -> bool {
        self.out.events += 1;
        let r = self.app().handle(&ev);
        self.spend(&what);
        let ran = match r {
            Ok(ran) => ran,
            Err(d) => {
                self.fault(d, what);
                return false;
            }
        };
        if !quiet || ran {
            self.log.push(what.clone());
        }
        !ran || self.render(&["the render after ", &what].concat()).is_some()
    }
}

/// `what` and then `text` in quotes.
fn quoted(what: &str, text: &str) -> String {
    [what, "\"", text, "\""].concat()
}

/// Every (id, text) of `nodes`' buttons, in order.
fn buttons(nodes: &[Node]) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    walk(nodes, &mut |n| {
        if let Node::Button { text, id } = n {
            out.push((*id, text.clone()));
        }
    });
    out
}

/// Every (id, squares, a canvas's width or 0) of `nodes`' grids and canvases that have a handler.
fn boards(nodes: &[Node]) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    walk(nodes, &mut |n| match n {
        Node::Grid { id: Some(id), cells, .. } if !cells.is_empty() => {
            out.push((*id, cells.len() as u32, 0));
        }
        Node::Canvas { id: Some(id), w, h, .. } => {
            out.push((*id, u32::from(*w) * u32::from(*h), u32::from(*w)));
        }
        _ => {}
    });
    out
}

/// Whether `d` reaches inside a canvas `w` x `h` (some of it, as near as its shape is known: a
/// text's chars as wide as it is tall).
fn reaches(d: &Draw, w: u16, h: u16) -> bool {
    let [x, y, a, b, c] = d.at.map(i64::from);
    let (rows, cols) = (d.text.lines().count() as i64, d.text.lines().map(|l| l.chars().count()));
    let cols = cols.max().unwrap_or(0) as i64;
    let (x0, y0, x1, y1) = match d.shape {
        Shape::Rect => (x, y, x + a, y + b),
        Shape::Circle | Shape::Ring if a > 0 && (d.shape == Shape::Circle || b > 0) => {
            (x - a, y - a, x + a + 1, y + a + 1)
        }
        Shape::Line if c > 0 => {
            (x.min(a) - c / 2, y.min(b) - c / 2, x.max(a) + c / 2 + 1, y.max(b) + c / 2 + 1)
        }
        Shape::Text if a > 0 && cols > 0 => {
            (x - cols * a / 2, y - a / 2, x + cols * a / 2 + 1, y + a / 2 + 1)
        }
        Shape::Sprite if d.ink() > 1 => (x, y, x + cols * a, y + rows * a),
        Shape::Pixels if d.ink() > 1 => (x, y, x + a * b, y + d.text.len() as i64 / a.max(1) * b),
        _ => return false,
    };
    x0 < i64::from(w) && x1 > 0 && y0 < i64::from(h) && y1 > 0 && x0 < x1 && y0 < y1
}

/// What is wrong with `d` if it is a text the desktop keeps inside its canvas `w` x `h` units
/// (its point on it, the far edges too) that is wider than the canvas: inset a quarter of its
/// size each side, each character reckoned 2/5 of its size (the boot font's are about half; only
/// narrow ones, i and 1, less), so one said to be nearly always is.
fn wide(d: &Draw, (w, h): (u16, u16)) -> Option<String> {
    let (chars, size) = (d.text.chars().count() as i64, i64::from(d.at[2]));
    let on = |at: i16, end: u16| (0..=i64::from(end)).contains(&i64::from(at));
    let need = ((4 * chars + 5) * size + 9) / 10;
    (d.shape == Shape::Text && on(d.at[0], w) && on(d.at[1], h) && need > i64::from(w)).then(|| {
        format!(
            "the text \"{}\" needs about {need} units across ({chars} characters at size {size}, \
             each about 2/5 of it, and a quarter of it either side), more than its canvas's {w}: \
             draw it smaller, or shorter",
            d.text
        )
    })
}

/// Where the first canvas of `ws` is, if it has one.
fn first_canvas(ws: &[Widget]) -> Option<Span> {
    ws.iter().find_map(|w| match w {
        Widget::Canvas { span, .. } => Some(*span),
        Widget::Row { children, .. } | Widget::Col { children, .. } => first_canvas(children),
        Widget::For { body, .. } => first_canvas(body),
        Widget::If { arms, els, .. } => {
            arms.iter().find_map(|a| first_canvas(&a.1)).or_else(|| first_canvas(els))
        }
        _ => None,
    })
}

/// The states `nodes`' inputs edit.
fn inputs(nodes: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    walk(nodes, &mut |n| {
        if let Node::Input { state, .. } = n {
            out.push(state.clone());
        }
    });
    out
}

fn walk(nodes: &[Node], f: &mut dyn FnMut(&Node)) {
    for n in nodes {
        f(n);
        if let Node::Row { children } | Node::Col { children } = n {
            walk(children, f);
        }
    }
}

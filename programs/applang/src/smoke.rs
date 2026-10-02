//! The smoke test: a program run as a person and a clock would, before anyone sees it. It
//! renders, clicks every button it shows once, then lets [`TICKS`] ticks pass (each the
//! program's own interval): every 5th it presses the next key the program handles, every 7th
//! taps a square of a grid that has a handler, every 11th clicks a button, every 13th types
//! "12", "hello" or "" in each input, re-rendering after each event. It stops at the first
//! fault, or after [`BUDGET`] steps in all, and says what it saw.

use crate::{App, Diag, Event, Limits, Node, Program};

/// The ticks a smoke test lets pass, and the most steps it spends.
pub const TICKS: u32 = 300;
const BUDGET: u64 = 20_000_000;
/// The last events a fault's account keeps.
const BEFORE: usize = 5;

/// The first fault: its diagnostic, what was done when it came ("clicking \"Start\"", "tick
/// 12", "the render after key \"left\""), and the events before that, oldest first.
#[derive(Debug, Clone)]
pub struct Fault {
    pub diag: Diag,
    pub during: String,
    pub before: Vec<String>,
}

/// What a smoke test saw: the first fault, the most steps one event or render took (and
/// which), how many events ran, and what looked wrong without faulting.
#[derive(Debug, Clone, Default)]
pub struct Smoke {
    pub fault: Option<Fault>,
    pub most: (u64, String),
    pub events: u32,
    pub warnings: Vec<String>,
}

/// Smoke-tests `program` with `random` seeded by `seed` (see the module docs).
pub fn smoke(program: Program, seed: u64) -> Smoke {
    let keys: Vec<String> = program.keys().iter().map(|k| k.key.clone()).collect();
    let timed = !program.everys().is_empty();
    let app = Some(App::new(program, Limits::default(), seed));
    let mut t = Tester { app, ..Tester::default() };
    t.rng = seed | 1;
    let Some(first) = t.render("the first render") else { return t.out };
    if first.is_empty() {
        t.out.warnings.push("it shows nothing".into());
    }
    for (id, text) in buttons(&first) {
        if !t.step(Event::Click { id }, format!("clicking {text:?}"), false) {
            return t.out;
        }
    }
    let mut changed = false;
    for n in 1..=TICKS {
        let ms = match t.app().timer() {
            0 => 100,
            ms => ms,
        };
        let before = t.last.clone();
        if !t.step(Event::Tick { ms }, format!("tick {n}"), true) {
            return t.out;
        }
        changed |= t.last != before;
        let mut events = Vec::new();
        if n % 5 == 0 && !keys.is_empty() {
            let name = keys[(n as usize / 5) % keys.len()].clone();
            events.push((Event::Key { name: name.clone() }, format!("key {name:?}")));
        }
        if n % 7 == 0 {
            let grids = grids(&t.last);
            if let Some(&(id, len)) = grids.get(t.pick(grids.len())) {
                let cell = t.pick(len) as u32;
                events.push((Event::Tap { id, cell }, format!("tapping square {cell}")));
            }
        }
        if n % 11 == 0 {
            let all = buttons(&t.last);
            if let Some((id, text)) = all.get(t.pick(all.len())) {
                events.push((Event::Click { id: *id }, format!("clicking {text:?}")));
            }
        }
        if n % 13 == 0 {
            let text = ["12", "hello", ""][(n as usize / 13) % 3];
            for state in inputs(&t.last) {
                let what = format!("typing {text:?} in {state}");
                events.push((Event::Input { state, text: text.into() }, what));
            }
        }
        for (ev, what) in events {
            if !t.step(ev, what, false) {
                return t.out;
            }
        }
        if t.spent > BUDGET {
            break;
        }
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

/// A smoke test under way: the app, what it showed last, the events so far, the steps spent,
/// its own xorshift for picks, and the report.
#[derive(Default)]
struct Tester {
    app: Option<App>,
    last: Vec<Node>,
    log: Vec<String>,
    spent: u64,
    rng: u64,
    out: Smoke,
}

impl Tester {
    fn app(&mut self) -> &mut App {
        self.app.as_mut().unwrap_or_else(|| unreachable!("made first"))
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
        self.spent += steps;
        if steps > self.out.most.0 {
            self.out.most = (steps, what.to_string());
        }
    }

    fn fault(&mut self, diag: Diag, during: String) {
        let before = self.log[self.log.len().saturating_sub(BEFORE)..].to_vec();
        self.out.fault = Some(Fault { diag, during, before });
    }

    /// Renders after `what`: what it shows, or `None` on a fault.
    fn render(&mut self, what: &str) -> Option<Vec<Node>> {
        let r = self.app().render();
        self.spend(what);
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

/// Every (id, squares) of `nodes`' grids that have a handler.
fn grids(nodes: &[Node]) -> Vec<(u32, usize)> {
    let mut out = Vec::new();
    walk(nodes, &mut |n| {
        if let Node::Grid { id: Some(id), cells, .. } = n {
            out.extend((!cells.is_empty()).then_some((*id, cells.len())));
        }
    });
    out
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

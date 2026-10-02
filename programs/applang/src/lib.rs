//! # applang: a total UI-app language
//!
//! An app is state declarations, functions, handlers (`every N ms`, `on key`) and a widget tree
//! with event handlers, and its guarantees are mechanical: [`compile`] type-checks everything
//! nameable and is the only way to a [`Program`]; every render and event runs on a fresh fuel
//! tank and calls never recurse, so it halts; a faulting event rolls back atomically; strings,
//! lists, state and render text are bounded by [`Limits`]; the parser bounds nesting, so eval
//! stays within a bounded stack. No host calls: time comes in as [`Event::Tick`] data and
//! chance from a seed given to [`App::new`], so an app is a pure function of its program, seed,
//! saved state and events. [`REFERENCE`] is the language card a generator is prompted with and
//! [`SHOTS`] its example programs, tested here, so prompt and verifier cannot drift; [`smoke`]
//! runs a program as a person and a clock would. Codes are banded per stage (see [`codes`]);
//! assert on codes, not messages.
//!
//! Forked from litelite's applite 0.2.0 (commit 4f5e056), with its front end
//! split out into [`applang_syntax`] and re-exported here.
//!
//! ```
//! use applang::{App, Event, Limits, Node, compile};
//!
//! let src = "state n = 0; button \"+\" { n += 1; } label \"n = \" + n;";
//! let mut app = App::new(compile(src).unwrap(), Limits::default(), 1);
//! app.render().unwrap();
//! app.handle(&Event::Click { id: 0 }).unwrap();
//! assert_eq!(app.render().unwrap()[1], Node::Label { text: "n = 1".to_string() });
//!
//! // No handler can hang the page, and a fault leaves the state untouched.
//! let src = "state x = 0; button \"spin\" { repeat 100000000 { x = x + 1; } }";
//! let mut app = App::new(compile(src).unwrap(), Limits::default(), 1);
//! app.render().unwrap();
//! let err = app.handle(&Event::Click { id: 0 }).unwrap_err();
//! assert_eq!(err.code, Some(applang::codes::FUEL_EXHAUSTED));
//! ```

#![forbid(unsafe_code)]

mod eval;
mod smoke;
#[cfg(test)]
mod tests;

pub use applang_syntax::ast::Type;
pub use applang_syntax::{Class, Program, codes, compile, highlight};
pub use eval::Value;
pub use lang::{Diag, Span};
pub use smoke::{Fault, Smoke, TICKS, smoke, smoke_from};

use applang_syntax::ast::{Lit, Stmt};
use eval::{Render, Run, Shown, State};

/// Hard resource limits for one [`App`]: guarantees, not hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Steps per event: one per statement, expression node, widget, loop iteration and item a
    /// list operation moves or copies, and one per 64 bytes of text a value copies (so fuel
    /// bounds memory too).
    pub fuel: u64,
    /// Steps per render.
    pub render_fuel: u64,
    /// Bytes in any one string value; a longer concat faults.
    pub max_str_bytes: usize,
    /// Bytes of all state together (a scalar 8, a string its bytes), checked when an event
    /// commits.
    pub max_state_bytes: usize,
    /// Bytes of one render's text (labels, buttons, inputs' names and values, grids).
    pub max_render_bytes: usize,
    /// Items in any one list.
    pub max_items: usize,
}

impl Default for Limits {
    /// 1,000,000 steps an event and 200,000 a render, 4 KiB strings, 256 KiB of state and of
    /// render text, 4,096 items a list.
    fn default() -> Self {
        Limits {
            fuel: 1_000_000,
            render_fuel: 200_000,
            max_str_bytes: 4 * 1024,
            max_state_bytes: applang_syntax::ast::MAX_STATE_BYTES,
            max_render_bytes: 256 * 1024,
            max_items: applang_syntax::ast::MAX_ITEMS,
        }
    }
}

/// One rendered widget. A Button's `id` (and a Grid's, when it has a handler) is its place among
/// the handlers this render showed: what a click or tap reports. `Input::state` is what a text
/// change names. A grid's squares are 0 (empty) to 8, `cols` a row, its texts one per square
/// or none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Label { text: String },
    Button { text: String, id: u32 },
    Input { state: String, value: String },
    Row { children: Vec<Node> },
    Col { children: Vec<Node> },
    Grid { id: Option<u32>, cols: u16, cells: Vec<u8>, texts: Vec<String> },
}

/// One host event. A click or tap names a handler of the last render; input text is clipped to
/// `max_str_bytes` at a char boundary; a tick says how many ms passed; a key is one of
/// [`applang_syntax::KEYS`], a letter or a digit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Click { id: u32 },
    Input { state: String, text: String },
    Tick { ms: u32 },
    Key { name: String },
    Tap { id: u32, cell: u32 },
}

/// A live app: a compiled program, its current state, the handlers its last render showed,
/// how far each `every` is into its interval, and the steps the last render or event took.
#[derive(Debug)]
pub struct App {
    program: Program,
    state: State,
    limits: Limits,
    shown: Vec<Shown>,
    acc: Vec<u64>,
    steps: u64,
    seed: u64,
}

impl App {
    /// Starts `program` at its declared initial state; `seed` (recorded: [`App::seed`]) is
    /// where `random` starts.
    pub fn new(program: Program, limits: Limits, seed: u64) -> App {
        let vals = inits(&program);
        // xorshift never leaves 0, so 0 is a fixed other seed.
        let state = State { vals, seed: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed } };
        let acc = vec![0; program.everys().len()];
        App { program, state, limits, shown: Vec::new(), acc, steps: 0, seed }
    }

    pub fn program(&self) -> &Program {
        &self.program
    }

    /// The seed it started with.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The steps the last render or event took.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Renders the current state, fueled; it changes nothing but which handlers clicks and
    /// taps name (none after a fault).
    pub fn render(&mut self) -> Result<Vec<Node>, Diag> {
        let lim = self.limits;
        let run = Run::new(&self.program, &mut self.state, &lim, lim.render_fuel);
        let mut r = Render { run, left: lim.max_render_bytes, shown: Vec::new() };
        let mut nodes = Vec::new();
        let done = r.widgets(self.program.widgets(), &mut nodes);
        self.steps = lim.render_fuel - r.run.fuel.remaining();
        self.shown = if done.is_ok() { r.shown } else { Vec::new() };
        done.map(|()| nodes)
    }

    /// Handles one event atomically: on `Err` the state is exactly as it was, and so is how far
    /// each `every` is into its interval (a faulted tick's time comes again with the next).
    /// Whether a handler ran (a tick with no `every` due, a key no `on key` names: none).
    pub fn handle(&mut self, event: &Event) -> Result<bool, Diag> {
        let lim = self.limits;
        let (mut next, mut acc) = (self.state.clone(), self.acc.clone());
        let mut run = Run::new(&self.program, &mut next, &lim, lim.fuel);
        let ran = dispatch(&mut run, &self.shown, &mut acc, event);
        self.steps = lim.fuel - run.fuel.remaining();
        let ran = ran?;
        let total: usize = next.vals.iter().map(Value::bytes).sum();
        if total > lim.max_state_bytes {
            let msg = format!("state totals {total} bytes; the limit is {}", lim.max_state_bytes);
            return Err(Diag::new_code(codes::STATE_TOO_BIG, msg));
        }
        (self.state, self.acc) = (next, acc);
        Ok(ran)
    }

    /// The current state, in declaration order.
    pub fn state(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.program.states().iter().map(|s| s.name.as_str()).zip(&self.state.vals)
    }

    /// How often it wants a [`Event::Tick`]: its shortest `every` interval now, in ms (0: no
    /// timer runs); one that faults counts as 100, so its tick shows the fault.
    pub fn timer(&mut self) -> u32 {
        self.intervals().into_iter().min().unwrap_or(0)
    }

    /// The intervals of the `every` blocks that run now, in ms (one that faults: 100).
    pub(crate) fn intervals(&mut self) -> Vec<u32> {
        let lim = self.limits;
        let mut run = Run::new(&self.program, &mut self.state, &lim, lim.render_fuel);
        let every = self.program.everys().iter().filter_map(|e| match run.expr(&e.interval) {
            Ok(Value::Int(n)) if n > 0 => Some(u32::try_from(n).unwrap_or(u32::MAX)),
            Ok(_) => None,
            Err(_) => Some(100),
        });
        every.collect()
    }

    /// Whether it has `on key` handlers.
    pub fn keys(&self) -> bool {
        !self.program.keys().is_empty()
    }

    /// Its `saved` states as `name = literal;` lines ("" if it has none).
    pub fn saved(&self) -> String {
        let mut out = String::new();
        for (s, v) in self.program.states().iter().zip(&self.state.vals).filter(|p| p.0.saved) {
            let v = match v {
                Value::Str(t) => ["\"", &eval::escape(t), "\""].concat(),
                Value::List(items) if items.is_empty() => {
                    let empty = ["[0; 0]", "[false; 0]", "[\"\"; 0]"];
                    empty[(s.init.ty() as usize).saturating_sub(3).min(2)].into()
                }
                v => v.to_string(),
            };
            out += &[&s.name, " = ", &v, ";\n"].concat();
        }
        out
    }

    /// Takes back saved states from [`App::saved`]'s lines, line by line: a name it no longer
    /// saves is skipped; one whose type changed is dropped, and so is a fixed list (see
    /// [`applang_syntax::ast::StateDecl`]) whose length changed, and a line that does not read,
    /// each with a note saying so; all are, with a note, if they would pass the state limit, or
    /// if it faults when it shows them but not afresh (so no saved state leaves an app that
    /// cannot even show itself).
    pub fn restore(&mut self, text: &str) -> Vec<String> {
        let before = self.state.vals.clone();
        let mut notes = self.take_back(text);
        let shows = if self.state.vals == before { Ok(Vec::new()) } else { self.render() };
        if let Err(d) = shows {
            let kept = std::mem::replace(&mut self.state.vals, before);
            match self.render() {
                Ok(_) => notes.push(format!(
                    "the saved state faults when it shows ({}); it starts afresh",
                    said(&d)
                )),
                Err(_) => self.state.vals = kept,
            }
        }
        notes
    }

    /// [`App::restore`] but for its last check: the saved states back, whatever shows (so a
    /// maker can see whether a program runs from what its app kept).
    pub fn take_back(&mut self, text: &str) -> Vec<String> {
        let (mut notes, before) = (Vec::new(), self.state.vals.clone());
        for (n, line) in text.lines().enumerate() {
            let lines = match applang_syntax::literals(line) {
                Ok(lines) => lines,
                Err(d) => {
                    let n = n + 1;
                    notes.push(format!("dropped saved line {n}: it did not read ({})", said(&d)));
                    continue;
                }
            };
            for (name, lit) in lines {
                let states = self.program.states();
                let Some(i) = states.iter().position(|s| s.saved && s.name == name) else {
                    continue;
                };
                let (init, fixed) = (&states[i].init, states[i].fixed);
                let len = |l: &Lit| if let Lit::List(_, items) = l { items.len() } else { 0 };
                if lit.ty() != init.ty() {
                    notes.push(format!(
                        "dropped the saved `{name}`: it is {} now",
                        init.ty().name()
                    ));
                } else if fixed && len(&lit) != len(init) {
                    let n = len(init);
                    notes.push(format!("dropped the saved `{name}`: it holds {n} items now"));
                } else {
                    self.state.vals[i] = Value::of(&lit);
                }
            }
        }
        if self.state.vals.iter().map(Value::bytes).sum::<usize>() > self.limits.max_state_bytes {
            self.state.vals = before;
            notes.push("the saved state is past the state limit; it starts afresh".into());
        }
        notes
    }

    /// Closed and opened again: its states afresh and its `every` blocks at their start, then
    /// `saved` ([`App::saved`]'s lines) taken back; the notes [`App::take_back`] gives.
    pub(crate) fn reopen(&mut self, saved: &str) -> Vec<String> {
        self.state.vals = inits(&self.program);
        self.acc.iter_mut().for_each(|a| *a = 0);
        self.take_back(saved)
    }
}

/// The program's states as declared.
fn inits(p: &Program) -> Vec<Value> {
    p.states().iter().map(|s| Value::of(&s.init)).collect()
}

/// `d` for a note: its code and message (a note has no source to point into).
fn said(d: &Diag) -> String {
    format!("E{:04} {}", d.code.unwrap_or_default(), d.message)
}

/// Runs `event` on `run`'s state: a shown handler (with what it sees), the `on key` handlers
/// of a key, the `every` blocks a tick makes due (each at most once), or an input's text.
fn dispatch(
    run: &mut Run<'_>,
    shown: &[Shown],
    acc: &mut [u64],
    event: &Event,
) -> Result<bool, Diag> {
    let bad = |msg: String| Diag::new_code(codes::BAD_EVENT, msg);
    let p = run.p;
    let body = |run: &mut Run<'_>, stmts: &[Stmt]| run.block(stmts).map(drop);
    match event {
        Event::Click { id } | Event::Tap { id, .. } => {
            let s =
                shown.get(*id as usize).ok_or_else(|| bad(format!("nothing shown has id {id}")))?;
            let Some((stmts, _)) = eval::handler(p.widgets(), s.id) else {
                return Err(bad(format!("no handler {}", s.id)));
            };
            run.locals.clone_from(&s.captured);
            match (event, s.cells) {
                (Event::Click { .. }, None) => {}
                (Event::Tap { cell, .. }, Some(n)) if *cell < n => {
                    run.locals.push(Value::Int(i64::from(*cell)));
                }
                _ => {
                    let msg = format!("{id} is a button clicked or a grid tapped on a square");
                    return Err(bad(msg));
                }
            }
            body(run, stmts)?;
        }
        Event::Key { name } => {
            let keys = p.keys().iter().filter(|k| k.key == *name);
            let mut ran = false;
            for k in keys {
                body(run, &k.body)?;
                ran = true;
            }
            return Ok(ran);
        }
        Event::Tick { ms } => {
            let mut ran = false;
            for (e, acc) in p.everys().iter().zip(acc) {
                let Value::Int(n) = run.expr(&e.interval)? else { unreachable!("checked: int") };
                let Ok(n @ 1..) = u64::try_from(n) else {
                    *acc = 0;
                    continue;
                };
                *acc += u64::from(*ms);
                if *acc >= n {
                    *acc = (*acc - n) % n;
                    body(run, &e.body)?;
                    ran = true;
                }
            }
            return Ok(ran);
        }
        Event::Input { state: name, text } => {
            let i = p.states().iter().position(|s| s.name == *name);
            let i = i.ok_or_else(|| bad(format!("no state named `{name}`")))?;
            let Value::Str(slot) = &mut run.st.vals[i] else {
                return Err(bad(format!("state `{name}` is not a string")));
            };
            // Host text is hostile: clip to the string bound, never mid-char.
            let mut cut = text.len().min(run.lim.max_str_bytes);
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            *slot = text[..cut].to_string();
        }
    }
    Ok(true)
}

/// Example programs as a reply gives them, for a prompt to show (both are tested: they compile
/// and pass [`smoke`]): what was asked, then the program.
pub const SHOTS: [(&str, &str); 2] = [
    ("make snake", include_str!("../shots/snake.app")),
    ("make a todo list I can check off", include_str!("../shots/todo.app")),
];

/// The compact, prompt-embeddable language card: hand a generator this.
pub const REFERENCE: &str = "\
applang: a small TOTAL language for apps and games. Every handler halts; a fault rolls its
event back and the app shows the error. A program is its states first, then functions,
handlers and widgets in any order. Comments: // and /* */.
STATE (first; the initial value fixes the type: int (64-bit, checked), bool, string, or a
list of one of them):
  state n = 0;   state name = \"\";   state on = false;
  state board = [0; 200];   state words = [\"a\", \"b\"];   state todos = [\"\"; 0];
  saved state best = 0;     -- kept when the app is closed and opened again
FUNCTIONS (a function calls only functions defined ABOVE it: no recursion; handlers and
widgets call any; parameters and results are int, bool or string; a function with a result
ends in return):
  fn at(x: int, y: int) -> int { return y * 10 + x; }
  fn reset() { score = 0; }   -- no result: it may change state
HANDLERS (top level):
  every 500 { STMTS }   every speed { STMTS }   -- each N ms while it shows; N <= 0 pauses:
                                                   make N 0 while nothing moves (before Start,
                                                   paused, game over), so the app rests
  on key \"left\" { STMTS }   -- \"left\" \"right\" \"up\" \"down\" \"space\" \"enter\" \"escape\", \"a\"..\"z\", \"0\"..\"9\"
WIDGETS (drawn top to bottom; EVERY widget ends with ; or its { } block, also inside the
braces of if, row, col and for):
  label EXPR;   button EXPR { STMTS }   input name;   (name: a string state, edited in place)
  row { WIDGETS }   col { WIDGETS }
  if EXPR { WIDGETS } else if EXPR { WIDGETS } else { WIDGETS }
  for i in 0..len(todos) { WIDGETS }   -- i (0 to N-1) is seen by the widgets and their handlers
  grid 10, board;                  -- a list of ints as squares, 10 a row: 0 empty, then colors
                                      1 red 2 green 3 yellow 4 blue 5 purple 6 cyan 7 silver 8 gray
                                      (7 and 8 look alike on a light theme: tell things apart by 1-6)
  grid 10, board { STMTS }         -- a tap (or a drag over squares) runs STMTS; cell = its index
  grid 2, [0, 0], words { STMTS }  -- a list of strings: one written in each square
  Widgets only read state: they call only functions that change nothing (and never random).
STATEMENTS (each ends with ; or its { } block; let works in any block):
  let x = EXPR;   x = EXPR;   x += EXPR;   xs[i] = EXPR;   f(a, b);   return EXPR;   return;
  if EXPR { } else if EXPR { } else { }   for i in A..B { }   repeat N { }
  push(xs, v);   insert(xs, i, v);   remove(xs, i);   clear(xs);
EXPRESSIONS: 42  true  \"text\" (escapes \\\" \\\\ \\n)  names  xs[i]  f(a, b)  [1, 2]  [0; n]  ( )
  - !   * / %   + -   < <= > >=   == !=   &&  ||    (/ and % round toward zero)
  + with a string on either side joins text: \"score \" + n
  len(xs)  len(s)  min(a, b)  max(a, b)  abs(a)  random(n) (0 to n-1)  parse(s, d) (int, else d)
  No ?: operator: write a function, fn mark(on: bool) -> string { if on { return \"x\"; } return \"\"; }
FAULTS: overflow, divide by zero, an index outside its list, a list past 4,096 items, a grid
square past 8, past 1,000,000 steps in one event or 200,000 in one render. There is no while,
no recursion, no float, no clock to read: time comes only from every.";

/// The rule a diagnostic's code says was broken, in a line, for a model fixing its program:
/// every code a program can earn; "" for the host's own (a bad event).
pub fn rule(code: u16) -> &'static str {
    match code {
        codes::UNEXPECTED_CHAR => "only the card's symbols exist: no . ' ? or @.",
        codes::UNTERMINATED_COMMENT => "every /* comment ends with */.",
        codes::BAD_INT => "ints are whole numbers that fit in 64 bits.",
        codes::UNTERMINATED_STRING => {
            "a string ends with \" on its line; write a line break as \\n."
        }
        codes::BAD_ESCAPE => "the only escapes are \\\" \\\\ and \\n.",
        codes::UNEXPECTED_TOKEN => {
            "every widget, let, assignment, call and return ends with ;, also inside the braces \
             of if, row, col and for (if on { label \"a\"; } else { label \"b\"; }). States come \
             first; there is no while, ?:, ++, *= or class."
        }
        codes::TOO_DEEP => "nest less: split long expressions and deep if chains into functions.",
        codes::DIV_BY_ZERO => "guard every / and % so the divisor is never 0.",
        codes::OVERFLOW => "keep ints within 64 bits: take % before you multiply.",
        codes::NEGATIVE_REPEAT => "repeat counts and list lengths are never negative.",
        codes::FUEL_EXHAUSTED => {
            "do less in one event (1,000,000 steps) or render (200,000): keep tables in lists, \
             loop less, copy long text and lists less, never redo work each tick."
        }
        codes::STR_TOO_LONG => "keep each string under 4 KiB.",
        codes::STATE_TOO_BIG => "keep all state under 256 KiB.",
        codes::RENDER_TOO_BIG => "show under 256 KiB of text at once.",
        codes::INDEX_OUT_OF_RANGE => {
            "an index runs from 0 to len(xs) - 1: check it (and the list's length) first."
        }
        codes::LIST_FULL => "a list holds at most 4,096 items.",
        codes::BAD_RANDOM => "random(n) needs n of at least 1.",
        codes::NO_RETURN | codes::MISSING_RETURN => {
            "a function with -> TYPE ends with return VALUE; (or an if/else returning in every \
             arm)."
        }
        codes::BAD_GRID => {
            "a grid has 1 to 100 columns, squares 0 to 8, and texts (if any) one per square."
        }
        codes::CALLS_TOO_DEEP => "call functions at most 32 deep.",
        codes::SHOWS_NOTHING => {
            "after its states, functions and handlers an app has its widgets (label, button, \
             input, row, col, if, for, grid), and some show from the start."
        }
        codes::DUP_STATE => {
            "declare each state, function and parameter once; built-ins keep \
                             their names."
        }
        codes::UNKNOWN_NAME => {
            "use only declared states, the lets of enclosing blocks, parameters, loop variables, \
             cell in a grid's handler, the built-ins and functions defined above."
        }
        codes::TYPE_MISMATCH => {
            "types never convert: compare like with like, conditions are bool, input binds a \
             string state, + with a string joins text, lists are not shown or compared whole, \
             and only a function with -> TYPE returns a value."
        }
        codes::CALL_BELOW => {
            "define each function above every function that calls it; a function never calls \
             itself."
        }
        codes::IMPURE_RENDER => {
            "widgets and every intervals only read: change state (and call random) in handlers."
        }
        codes::BAD_KEY => {
            "on key names left, right, up, down, space, enter, escape, a to z or 0 to 9."
        }
        codes::ARITY => "call each function with exactly its parameters.",
        _ => "",
    }
}

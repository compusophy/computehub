//! # applang — a total UI-app language on the compusophyOS kit
//!
//! The language a code-generating model writes SAFELY: an app is state
//! declarations plus a widget tree with fuel-bounded event handlers, and the
//! guarantees are mechanical — no generated program can hang the page, blow
//! the stack, exhaust memory, corrupt its state, or touch anything outside
//! its own widgets. Consumer: compusophyOS tier 0 apps (generate → verify →
//! run, in a browser), and [`REFERENCE`] is the exact language card a
//! generator is prompted with — one artifact, so prompt and verifier cannot
//! drift.
//!
//! Forked from litelite's applite 0.2.0 (commit 4f5e056), with its front end
//! (lexer, parser, checker, [`codes`]) split out into [`applang_syntax`] and
//! re-exported here.
//!
//! ## The language
//!
//! ```text
//! state count = 0;                      // literals fix each state's type
//! state name  = "world";               //   (int, bool, or string)
//!
//! label "The counter demo";            // widgets render top to bottom
//! row {                                 // row/col group horizontally/vertically
//!   button "-" { count = count - 1; }  // a handler: runs on click
//!   label count;
//!   button "+" { count = count + 1; }
//! }
//! input name;                           // two-way binding to a string state
//! if name != "" {                       // conditional UI, re-checked per render
//!   label "Hello, " + name + "!";     // `+` with a string concatenates
//! } else { label "Type your name."; }
//! ```
//!
//! Handlers use `let`, assignment, `if`/`else if`/`else`, and `repeat n { }`
//! — the only loop, its count evaluated once. Arithmetic is CHECKED;
//! comparisons are int-only except `==`/`!=` (same-type). No functions, no
//! recursion, no `while`, no host calls: the widget tree is the app's whole
//! world.
//!
//! ## The guarantees (what smallness buys)
//!
//! - **Static resolution.** [`compile`] type-checks everything nameable and
//!   is the only way to get a [`Program`]; the only runtime faults left are
//!   arithmetic, fuel, and string bounds.
//! - **Every event AND render halts** — each runs on a fresh fuel tank.
//! - **Faults are ATOMIC.** A handler runs against a copy of the state; only
//!   a clean finish commits (the bar-atomicity rule of litelite's stratlite,
//!   applied to UI events).
//! - **Bounded memory.** Strings are capped per value ([`Limits::max_str_bytes`],
//!   a too-long concat is a fault, never a silent clip) and per app
//!   ([`Limits::max_state_bytes`]) — `s = s + s` in a loop is a diag, not an
//!   OOM. Host input text is clipped at a char boundary, never mid-char.
//! - **Bounded nesting.** The parser depth-guards widget nesting AND binary
//!   spines (litelite's prooflite lesson): whatever parses, eval and drop
//!   glue walk within a bounded stack.
//!
//! Codes are banded per stage — lex `E00xx`, parse `E01xx`, runtime `E02xx`,
//! static check `E03xx` (see [`codes`]) — assert on codes, not messages.
//!
//! ```
//! use applang::{App, Event, Limits, Node, compile};
//!
//! let program = compile(
//!     "state count = 0;
//!      button \"+\" { count = count + 1; }
//!      label \"count = \" + count;",
//! )
//! .unwrap();
//! let mut app = App::new(program, Limits::default());
//! app.handle(&Event::Click { id: 0 }).unwrap();
//! app.handle(&Event::Click { id: 0 }).unwrap();
//! let nodes = app.render().unwrap();
//! assert_eq!(nodes[1], Node::Label { text: "count = 2".to_string() });
//!
//! // The headline guarantee: NO handler can hang the page.
//! let program = compile("state x = 0; button \"spin\" { repeat 100000000 { x = x + 1; } }")
//!     .unwrap();
//! let mut app = App::new(program, Limits::default());
//! let err = app.handle(&Event::Click { id: 0 }).unwrap_err();
//! assert_eq!(err.code, Some(applang::codes::FUEL_EXHAUSTED));
//! assert_eq!(app.render().unwrap().len(), 1); // and the state is untouched
//! ```

mod eval;

pub use applang_syntax::{Program, TokKind, Token, Type, codes, compile, lex};
pub use eval::Value;
pub use lang::{Diag, Span};

/// Hard resource limits for one [`App`]. All three are guarantees, not
/// hints; each render and each event handler gets a FRESH fuel tank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Evaluation steps per render / per event (1 per statement, expression
    /// node, widget, and `repeat` iteration).
    pub fuel: u64,
    /// Byte cap on any single string VALUE (a longer concat is a fault).
    pub max_str_bytes: usize,
    /// Byte cap on all string STATE combined, checked at event commit.
    pub max_state_bytes: usize,
}

impl Default for Limits {
    /// 100_000 fuel, 4 KiB strings, 64 KiB total string state.
    fn default() -> Self {
        Limits {
            fuel: 100_000,
            max_str_bytes: 4 * 1024,
            max_state_bytes: 64 * 1024,
        }
    }
}

/// One rendered widget — what a shell draws. `Button::id` is what a click
/// reports; `Input::state` is what a text change names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Label { text: String },
    Button { text: String, id: u32 },
    Input { state: String, value: String },
    Row { children: Vec<Node> },
    Col { children: Vec<Node> },
}

/// One host event. `Click` on a button that exists but is currently hidden
/// still runs its handler (total either way; visibility races are the
/// shell's concern, safety is not). `Input` text is clipped to the string
/// bound at a char boundary before it touches state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Click { id: u32 },
    Input { state: String, text: String },
}

/// A live app: a compiled program plus its current state.
#[derive(Debug)]
pub struct App {
    program: Program,
    state: eval::State,
    limits: Limits,
}

impl App {
    /// Start `program` at its declared initial state. [`compile`] is the only
    /// way to get a [`Program`], so it has passed the checker:
    ///
    /// ```compile_fail,E0425
    /// use applang::{App, Limits};
    /// // An unchecked tree would reach the runtime's checked invariants.
    /// let _ = App::new(applang::parse("label x;").unwrap(), Limits::default());
    /// ```
    pub fn new(program: Program, limits: Limits) -> App {
        let state = eval::init_state(&program);
        App {
            program,
            state,
            limits,
        }
    }

    /// Render the current state through the widget tree. Pure and fueled: a
    /// render neither mutates state nor runs forever.
    pub fn render(&self) -> Result<Vec<Node>, Diag> {
        eval::render(&self.program, &self.state, &self.limits)
    }

    /// Handle one event ATOMICALLY: on `Ok` the state advanced, on `Err` it
    /// is exactly as it was (render again either way — nothing is stale).
    pub fn handle(&mut self, event: &Event) -> Result<(), Diag> {
        self.state = eval::handle(&self.program, &self.state, event, &self.limits)?;
        Ok(())
    }

    /// The current state, declaration-ordered — for shells that persist or
    /// inspect it.
    pub fn state(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.state.iter().map(|(n, v)| (n.as_str(), v))
    }
}

/// The compact, prompt-embeddable language card — hand a generator THIS.
/// It is the crate's single description of the surface; a shell serves it
/// verbatim (e.g. behind a "copy prompt" button), so prompt and verifier are
/// one artifact (the REFERENCE rule of litelite's stratlite).
pub const REFERENCE: &str = "\
applang: a tiny TOTAL language for small interactive apps. One program = state
declarations, then a widget tree. Every event handler halts; faults roll back.
STATE (first, before any widget; the literal fixes the type — int, bool, string):
  state count = 0;   state name = \"world\";   state on = false;
WIDGETS (render top to bottom):
  label EXPR;                  -- one line of text (any type, displayed)
  button \"text\" { STMTS }      -- runs its handler when clicked
  input name;                  -- text field bound two-way to a STRING state
  row { WIDGETS }  col { WIDGETS }   -- horizontal / vertical grouping
  if EXPR { WIDGETS } else if EXPR { WIDGETS } else { WIDGETS }   -- conditional UI
HANDLER STATEMENTS (each ends with ;):
  let x = EXPR;      -- local variable       x = EXPR;   -- assign local or state
  if EXPR { ... } else if EXPR { ... } else { ... }
  repeat EXPR { ... }          -- the ONLY loop; count evaluated once, up front
EXPRESSIONS: 42, true, \"text\" (escapes \\\" \\\\ \\n); state/local names;
  - !; * / %; + -; < <= > >=; == != (same type only); && || (short-circuit); ( ).
  `+` with any string operand CONCATENATES (\"n = \" + count). Arithmetic is
  CHECKED: overflow and divide-by-zero are errors. Comments: // and /* nested */.
NO functions, NO recursion, NO while, NO host calls: the widgets are the whole
world. Type-checked before running: every name must resolve, types must match.";

#[cfg(test)]
mod tests {
    use super::*;

    fn app(src: &str) -> App {
        App::new(compile(src).unwrap(), Limits::default())
    }

    fn inp(state: &str, text: &str) -> Event {
        Event::Input {
            state: state.to_string(),
            text: text.to_string(),
        }
    }

    fn vals(a: &App) -> Vec<Value> {
        a.state().map(|(_, v)| v.clone()).collect()
    }

    fn texts(nodes: &[Node]) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(ns: &[Node], out: &mut Vec<String>) {
            for n in ns {
                match n {
                    Node::Label { text } => out.push(text.clone()),
                    Node::Row { children } | Node::Col { children } => walk(children, out),
                    _ => {}
                }
            }
        }
        walk(nodes, &mut out);
        out
    }

    #[test]
    fn the_counter_demo_end_to_end() {
        let mut a = app("state count = 0;
             row {
               button \"-\" { count = count - 1; }
               label count;
               button \"+\" { count = count + 1; }
             }
             if count >= 3 { label \"high\"; } else { label \"low\"; }");
        assert_eq!(texts(&a.render().unwrap()), ["0", "low"]);
        for _ in 0..3 {
            a.handle(&Event::Click { id: 1 }).unwrap();
        }
        a.handle(&Event::Click { id: 0 }).unwrap();
        a.handle(&Event::Click { id: 1 }).unwrap();
        assert_eq!(texts(&a.render().unwrap()), ["3", "high"]);
    }

    #[test]
    fn input_binds_two_ways_and_clips_hostile_text() {
        let mut a = app("state name = \"\"; input name; label \"hi \" + name;");
        a.handle(&inp("name", "Ada")).unwrap();
        let nodes = a.render().unwrap();
        assert_eq!(
            nodes[0],
            Node::Input {
                state: "name".to_string(),
                value: "Ada".to_string()
            }
        );
        assert_eq!(texts(&nodes), ["hi Ada"]);
        // Hostile input: 5000 multi-byte chars clip at the cap, never mid-char.
        a.handle(&inp("name", &"é".repeat(5000))).unwrap();
        let [Value::Str(s)] = &vals(&a)[..] else {
            panic!("state gone");
        };
        assert!(s.len() <= Limits::default().max_str_bytes);
        assert!(s.chars().all(|c| c == 'é')); // no torn char at the cut
    }

    #[test]
    fn faulting_handlers_roll_back_atomically() {
        let mut a = app("state x = 0; state y = 0;
             button \"boom\" { x = 99; y = 1 / y; }");
        let e = a.handle(&Event::Click { id: 0 }).unwrap_err();
        assert_eq!(e.code, Some(codes::DIV_BY_ZERO));
        // x's write happened BEFORE the fault — and was still rolled back.
        assert_eq!(vals(&a), [Value::Int(0), Value::Int(0)]);
    }

    #[test]
    fn string_bombs_are_diags_not_ooms() {
        // Exponential growth by self-concat trips the per-value cap.
        let mut a = app("state s = \"aaaa\"; button \"grow\" { repeat 60 { s = s + s; } }");
        let e = a.handle(&Event::Click { id: 0 }).unwrap_err();
        assert_eq!(e.code, Some(codes::STR_TOO_LONG));
        // Many states each under the value cap trip the commit total instead.
        let mut src = String::new();
        for i in 0..40 {
            src.push_str(&format!("state s{i} = \"\";\n"));
        }
        src.push_str("button \"fill\" {");
        for i in 0..40 {
            src.push_str(&format!("s{i} = \"{}\";", "b".repeat(3000)));
        }
        src.push('}');
        let mut a = app(&src);
        let e = a.handle(&Event::Click { id: 0 }).unwrap_err();
        assert_eq!(e.code, Some(codes::STATE_TOO_BIG));
        // Rolled back: nothing grew.
        assert!(a.state().all(|(_, v)| *v == Value::Str(String::new())));
    }

    #[test]
    fn render_is_fueled_too() {
        let a = App::new(
            compile("state x = 1; label x + x + x + x;").unwrap(),
            Limits {
                fuel: 3,
                ..Limits::default()
            },
        );
        assert_eq!(a.render().unwrap_err().code, Some(codes::FUEL_EXHAUSTED));
    }

    #[test]
    fn bad_events_are_coded_and_harmless() {
        let mut a = app("state n = 0; button \"b\" { n = n + 1; }");
        for ev in [
            Event::Click { id: 7 },
            inp("missing", ""),
            inp("n", "not a string state"),
        ] {
            assert_eq!(a.handle(&ev).unwrap_err().code, Some(codes::BAD_EVENT));
        }
        assert_eq!(vals(&a), [Value::Int(0)]);
    }

    #[test]
    fn hidden_buttons_still_handle_totally() {
        // A click racing a re-render targets a now-hidden button: still runs.
        let mut a = app("state show = true; state n = 0;
             if show { button \"inc\" { n = n + 1; show = false; } }");
        a.handle(&Event::Click { id: 0 }).unwrap();
        a.handle(&Event::Click { id: 0 }).unwrap(); // hidden now — still total
        assert_eq!(vals(&a), [Value::Bool(false), Value::Int(2)]);
    }

    #[test]
    fn diags_render_with_carets_and_the_card_is_real() {
        let src = "state x = 1;\nlabel x + true;";
        let r = compile(src).unwrap_err().render(src);
        assert!(r.contains("E0303"), "{r}");
        assert!(r.contains("label x + true;"), "{r}");
        // The card's opening example is real applang — prompt/verifier unity.
        let src = "state count = 0; state name = \"world\"; state on = false;
                   label count; input name; if on { label 1; }";
        assert!(compile(src).is_ok());
        assert!(REFERENCE.contains("state count = 0"));
    }
}

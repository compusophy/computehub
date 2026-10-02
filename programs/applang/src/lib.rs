//! # applang: a total UI-app language
//!
//! An app is state declarations plus a widget tree with event handlers, and
//! its guarantees are mechanical: [`compile`] type-checks everything nameable
//! and is the only way to a [`Program`]; every render and event runs on a
//! fresh fuel tank, so it halts; a faulting handler rolls back atomically;
//! strings, state and render text are bounded by [`Limits`]; the parser
//! bounds nesting, so eval stays within a bounded stack. No host calls: the
//! widgets are the app's whole world. [`REFERENCE`] is the language card a
//! generator is prompted with, so prompt and verifier cannot drift. Codes
//! are banded per stage (see [`codes`]); assert on codes, not messages.
//!
//! Forked from litelite's applite 0.2.0 (commit 4f5e056), with its front end
//! split out into [`applang_syntax`] and re-exported here.
//!
//! ```
//! use applang::{App, Event, Limits, Node, compile};
//!
//! let src = "state n = 0; button \"+\" { n = n + 1; } label \"n = \" + n;";
//! let mut app = App::new(compile(src).unwrap(), Limits::default());
//! app.handle(&Event::Click { id: 0 }).unwrap();
//! assert_eq!(app.render().unwrap()[1], Node::Label { text: "n = 1".to_string() });
//!
//! // No handler can hang the page, and a fault leaves the state untouched.
//! let src = "state x = 0; button \"spin\" { repeat 100000000 { x = x + 1; } }";
//! let mut app = App::new(compile(src).unwrap(), Limits::default());
//! let err = app.handle(&Event::Click { id: 0 }).unwrap_err();
//! assert_eq!(err.code, Some(applang::codes::FUEL_EXHAUSTED));
//! ```

#![forbid(unsafe_code)]

mod eval;

pub use applang_syntax::{Class, Program, codes, compile, highlight};
pub use eval::Value;
pub use lang::{Diag, Span};

/// Hard resource limits for one [`App`]: guarantees, not hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Steps per render or event: one per statement, expression node,
    /// widget and `repeat` iteration.
    pub fuel: u64,
    /// Bytes in any one string value; a longer concat faults.
    pub max_str_bytes: usize,
    /// Bytes of all string state together, checked when an event commits.
    pub max_state_bytes: usize,
    /// Bytes of one render's text (labels, buttons, inputs' names and values).
    pub max_render_bytes: usize,
}

impl Default for Limits {
    /// 100,000 fuel, 4 KiB strings, 64 KiB of string state, 256 KiB per render.
    fn default() -> Self {
        Limits {
            fuel: 100_000,
            max_str_bytes: 4 * 1024,
            max_state_bytes: 64 * 1024,
            max_render_bytes: 256 * 1024,
        }
    }
}

/// One rendered widget. `Button::id` is what a click reports;
/// `Input::state` is what a text change names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Label { text: String },
    Button { text: String, id: u32 },
    Input { state: String, value: String },
    Row { children: Vec<Node> },
    Col { children: Vec<Node> },
}

/// One host event. A click on a hidden button still runs its handler; input
/// text is clipped to `max_str_bytes` at a char boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Click { id: u32 },
    Input { state: String, text: String },
}

/// A live app: a compiled program and its current state.
#[derive(Debug)]
pub struct App {
    program: Program,
    state: eval::State,
    limits: Limits,
}

impl App {
    /// Starts `program` at its declared initial state.
    pub fn new(program: Program, limits: Limits) -> App {
        let state = eval::init_state(&program);
        App { program, state, limits }
    }

    /// Renders the current state: pure and fueled.
    pub fn render(&self) -> Result<Vec<Node>, Diag> {
        eval::render(&self.program, &self.state, &self.limits)
    }

    /// Handles one event atomically: on `Err` the state is exactly as it was.
    pub fn handle(&mut self, event: &Event) -> Result<(), Diag> {
        self.state = eval::handle(&self.program, &self.state, event, &self.limits)?;
        Ok(())
    }

    /// The current state, in declaration order.
    pub fn state(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.state.iter().map(|(n, v)| (n.as_str(), v))
    }
}

/// The compact, prompt-embeddable language card: hand a generator this.
pub const REFERENCE: &str = "\
applang: a tiny TOTAL language for small interactive apps. One program = state
declarations, then a widget tree. Every event handler halts; faults roll back.
STATE (first, before any widget; the literal fixes the type — int (64-bit), bool, string):
  state count = 0;   state name = \"world\";   state on = false;
WIDGETS (render top to bottom; label and input end with ; everywhere, also inside the
braces of if, row and col; a closing } takes no ;):
  label EXPR;                  -- one line of text (any type, displayed)
  button \"text\" { STMTS }      -- runs its handler when clicked
  input name;                  -- text field bound two-way to a STRING state
  row { WIDGETS }  col { WIDGETS }   -- horizontal / vertical grouping
  if EXPR { WIDGETS } else if EXPR { WIDGETS } else { WIDGETS }   -- conditional UI
  e.g. if on { label \"yes\"; } else { label \"no\"; }
HANDLER STATEMENTS (each ends with ;):
  let x = EXPR;      -- a local, in any block    x = EXPR;   -- assign local or state
  if EXPR { ... } else if EXPR { ... } else { ... }
  repeat EXPR { ... }          -- the ONLY loop; count evaluated once, up front
EXPRESSIONS: 42, true, \"text\" (escapes \\\" \\\\ \\n); state/local names;
  - !; * / %; + -; < <= > >=; == != (same type only); && || (short-circuit); ( ).
  `+` with any string operand CONCATENATES (\"n = \" + count). Ints are 64-bit and
  CHECKED: overflow and divide-by-zero are errors. Comments: // and /* nested */.
NO functions, NO recursion, NO while, NO host calls: the widgets are the whole
world. Type-checked before running: every name must resolve, types must match.";

/// The rule a diagnostic's code says was broken, in a line, for a model fixing its program:
/// every code a program can earn when it compiles or first renders; "" for the others.
pub fn rule(code: u16) -> &'static str {
    match code {
        codes::UNEXPECTED_CHAR => "only the card's symbols exist: no [ ] . , : or ' quotes.",
        codes::UNTERMINATED_COMMENT => "every /* comment ends with */.",
        codes::BAD_INT => "ints are whole numbers that fit in 64 bits.",
        codes::UNTERMINATED_STRING => {
            "a string ends with \" on its line; write a line break as \\n."
        }
        codes::BAD_ESCAPE => "the only escapes are \\\" \\\\ and \\n.",
        codes::UNEXPECTED_TOKEN => {
            "label, input, let and assignments end with ;, also inside the braces of if, row \
             and col (if on { label \"a\"; } else { label \"b\"; }), and no ; follows a closing \
             }. Statements go only in button handlers; there is no fn, for or while."
        }
        codes::TOO_DEEP => "nest less: split long expressions and deep if chains.",
        codes::DIV_BY_ZERO => "guard every / and % so the divisor is never 0.",
        codes::OVERFLOW => "keep ints within 64 bits: take % before you multiply.",
        codes::FUEL_EXHAUSTED => "do less in one render or click: at most 100,000 steps.",
        codes::STR_TOO_LONG => "keep each string under 4 KiB.",
        codes::RENDER_TOO_BIG => "show under 256 KiB of text at once.",
        codes::DUP_STATE => "declare each state once.",
        codes::UNKNOWN_NAME => {
            "use only declared states and the lets of enclosing handler blocks; there are no \
             built-in functions."
        }
        codes::TYPE_MISMATCH => {
            "types never convert: compare like with like, conditions are bool, input binds a \
             string state, and + with a string joins text."
        }
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(src: &str) -> App {
        App::new(compile(src).unwrap(), Limits::default())
    }

    fn click(a: &mut App, id: u32) -> Option<u16> {
        a.handle(&Event::Click { id }).err().and_then(|e| e.code)
    }

    fn inp(state: &str, text: &str) -> Event {
        Event::Input { state: state.to_string(), text: text.to_string() }
    }

    fn vals(a: &App) -> Vec<Value> {
        a.state().map(|(_, v)| v.clone()).collect()
    }

    fn texts(nodes: &[Node]) -> Vec<String> {
        let each = |n: &Node| match n {
            Node::Label { text } => vec![text.clone()],
            Node::Row { children } | Node::Col { children } => texts(children),
            _ => Vec::new(),
        };
        nodes.iter().flat_map(each).collect()
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
        for id in [1, 1, 1, 0, 1] {
            assert_eq!(click(&mut a, id), None);
        }
        assert_eq!(texts(&a.render().unwrap()), ["3", "high"]);
    }

    #[test]
    fn expressions_short_circuit_and_check_arithmetic() {
        let mut a = app("state n = 0; state s = \"\";
             button \"b\" { let t = n; if false && 1 / n == 0 || true { s = \"a\" + t + true; } }
             button \"neg\" { n = -(0 - 9223372036854775807 - 1); }
             button \"rep\" { repeat n - 1 { } }
             button \"mod\" { n = 7 % n; }");
        assert_eq!(click(&mut a, 0), None);
        assert_eq!(vals(&a), [Value::Int(0), Value::Str("a0true".into())]);
        for (id, code) in
            [(1, codes::OVERFLOW), (2, codes::NEGATIVE_REPEAT), (3, codes::DIV_BY_ZERO)]
        {
            assert_eq!(click(&mut a, id), Some(code));
        }
    }

    #[test]
    fn input_binds_two_ways_and_clips_hostile_text() {
        let mut a = app("state name = \"\"; input name; label \"hi \" + name;");
        a.handle(&inp("name", "Ada")).unwrap();
        let nodes = a.render().unwrap();
        assert_eq!(nodes[0], Node::Input { state: "name".to_string(), value: "Ada".to_string() });
        assert_eq!(texts(&nodes), ["hi Ada"]);
        // 5000 multi-byte chars clip at the cap, never mid-char.
        a.handle(&inp("name", &"é".repeat(5000))).unwrap();
        let [Value::Str(s)] = &vals(&a)[..] else { panic!("state gone") };
        assert!(s.len() <= Limits::default().max_str_bytes && s.chars().all(|c| c == 'é'));
    }

    #[test]
    fn faults_roll_back_and_bad_events_are_coded() {
        let mut a = app("state x = 0; state y = 0; button \"boom\" { x = 99; y = 1 / y; }");
        // x's write happened before the fault, and was still rolled back.
        assert_eq!(click(&mut a, 0), Some(codes::DIV_BY_ZERO));
        assert_eq!(vals(&a), [Value::Int(0), Value::Int(0)]);
        for ev in [Event::Click { id: 7 }, inp("missing", ""), inp("x", "not a string state")] {
            assert_eq!(a.handle(&ev).unwrap_err().code, Some(codes::BAD_EVENT));
        }
        assert_eq!(vals(&a), [Value::Int(0), Value::Int(0)]);
        // A click racing a re-render targets a now-hidden button: still runs.
        let mut a = app("state show = true; state n = 0;
             if show { button \"inc\" { n = n + 1; show = false; } }");
        assert_eq!((click(&mut a, 0), click(&mut a, 0)), (None, None));
        assert_eq!(vals(&a), [Value::Bool(false), Value::Int(2)]);
    }

    #[test]
    fn strings_state_render_and_fuel_are_bounded() {
        // Self-concat trips the per-value cap.
        let mut a = app("state s = \"aaaa\"; button \"grow\" { repeat 60 { s = s + s; } }");
        assert_eq!(click(&mut a, 0), Some(codes::STR_TOO_LONG));
        // Many states each under the value cap trip the commit total instead.
        let states: String = (0..40).map(|i| format!("state s{i} = \"\";")).collect();
        let sets: String = (0..40).map(|i| format!("s{i} = \"{}\";", "b".repeat(3000))).collect();
        let mut a = app(&[states, "button \"fill\" {".into(), sets, "}".into()].concat());
        assert_eq!(click(&mut a, 0), Some(codes::STATE_TOO_BIG));
        assert!(a.state().all(|(_, v)| *v == Value::Str(String::new())));
        // Each label is under the string cap; 70 of them pass the render cap.
        let src = ["state s = \"", &"x".repeat(4000), "\";", &"label s;".repeat(70)].concat();
        let e = app(&src).render().unwrap_err();
        assert_eq!((e.code, e.span.is_some()), (Some(codes::RENDER_TOO_BIG), true));
        let fits = app(&src[..src.len() - 6 * 8]).render().unwrap();
        assert_eq!(texts(&fits).concat().len(), 64 * 4000);
        let src = "state x = 1; label x + x + x + x;";
        let a = App::new(compile(src).unwrap(), Limits { fuel: 3, ..Limits::default() });
        assert_eq!(a.render().unwrap_err().code, Some(codes::FUEL_EXHAUSTED));
    }

    #[test]
    fn diags_render_with_carets_and_the_card_is_real() {
        let src = "state x = 1;\nlabel x + true;";
        let r = compile(src).unwrap_err().render(src);
        assert!(r.contains("E0303") && r.contains("label x + true;"), "{r}");
        // The card's opening example is real applang.
        let src = "state count = 0; state name = \"world\"; state on = false;
                   label count; input name; if on { label 1; }";
        assert!(compile(src).is_ok() && REFERENCE.contains("state count = 0"));
    }

    #[test]
    fn the_card_says_what_models_got_wrong_and_each_code_has_a_rule() {
        // A model's one slip: a label without `;` inside `if` braces. The card shows the form.
        let good = "if on { label \"yes\"; } else { label \"no\"; }";
        let bad = good.replacen("\"yes\";", "\"yes\"", 1);
        assert_eq!(compile(&["state on = true; ", &bad].concat()).unwrap_err().code, Some(101));
        assert!(compile(&["state on = true; ", good].concat()).is_ok() && REFERENCE.contains(good));
        // Ints are 64-bit (a model shrank its constants, fearing 32), and a let fits any block.
        assert!(REFERENCE.contains("int (64-bit)") && REFERENCE.contains("a local, in any block"));
        let mut a = app("state n = 0; button \"b\" { if true { let x = 3000000000; repeat 1 { \
                         let y = x * x; n = y % 7; } } }");
        let want = Value::Int(9_000_000_000_000_000_000 % 7);
        assert_eq!((click(&mut a, 0), vals(&a)), (None, vec![want]));
        // Every code a program earns when it compiles or first renders has its rule, so a fix
        // turn always says what was broken.
        let [big, long] = [4000, 3000].map(|n| ["state s = \"", &"x".repeat(n), "\";"].concat());
        let deep = ["label ", &"(".repeat(999), "1;"].concat();
        let cases: [(&str, u16); 15] = [
            ("label 1 # 2;", codes::UNEXPECTED_CHAR),
            ("/* x", codes::UNTERMINATED_COMMENT),
            ("label 99999999999999999999;", codes::BAD_INT),
            ("label \"x;", codes::UNTERMINATED_STRING),
            ("label \"\\q\";", codes::BAD_ESCAPE),
            ("label 1", codes::UNEXPECTED_TOKEN),
            (&deep, codes::TOO_DEEP),
            ("state n = 0; label 1 / n;", codes::DIV_BY_ZERO),
            ("state n = 9223372036854775807; label n + 1;", codes::OVERFLOW),
            (&"label 1;".repeat(60_000), codes::FUEL_EXHAUSTED),
            (&[&long, " label s + s;"].concat(), codes::STR_TOO_LONG),
            (&[big, "label s;".repeat(70)].concat(), codes::RENDER_TOO_BIG),
            ("state a = 0; state a = 1;", codes::DUP_STATE),
            ("label nope;", codes::UNKNOWN_NAME),
            ("label 1 + true;", codes::TYPE_MISMATCH),
        ];
        for (src, code) in cases {
            let first = compile(src).and_then(|p| App::new(p, Limits::default()).render());
            assert_eq!(first.unwrap_err().code, Some(code), "{}", &src[..src.len().min(40)]);
            assert!(rule(code).ends_with('.'), "{code}");
        }
        assert_eq!(rule(codes::BAD_EVENT), "");
    }
}

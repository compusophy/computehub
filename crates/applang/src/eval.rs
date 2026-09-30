//! applang runtime: a fueled tree-walk over widgets (render) and handler
//! bodies (events). Rendering is PURE — it evaluates expressions against
//! state it cannot mutate. Event handling is ATOMIC — the handler runs
//! against a copy of the state, and only a clean finish commits; any fault
//! (fuel, arithmetic, string bounds) leaves the app exactly as it was.
//! Static checking (`check`, in `applang_syntax`) already resolved every
//! name and type, so the faults left here are the ones no static system
//! this small can remove: checked arithmetic, fuel, and string-size bounds.

use applang_syntax::ast::{BinOp, Expr, Lit, Stmt, UnOp, Widget};
use fuel::Fuel;
use lang::{Diag, Span};

use crate::{Event, Limits, Node, Program, codes};

/// An applang runtime value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Str(String),
}

impl Value {
    pub(crate) fn from_lit(l: &Lit) -> Value {
        match l {
            Lit::Int(v) => Value::Int(*v),
            Lit::Bool(b) => Value::Bool(*b),
            Lit::Str(s) => Value::Str(s.clone()),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => f.write_str(s),
        }
    }
}

/// The app's live state: declaration-ordered `(name, value)` pairs.
pub(crate) type State = Vec<(String, Value)>;

pub(crate) fn init_state(program: &Program) -> State {
    program
        .states()
        .iter()
        .map(|s| (s.name.clone(), Value::from_lit(&s.init)))
        .collect()
}

fn fuel_out(sp: Span) -> Diag {
    Diag::at_code(codes::FUEL_EXHAUSTED, "fuel exhausted", sp)
}

fn overflow(op: &str, sp: Span) -> Diag {
    Diag::at_code(
        codes::OVERFLOW,
        format!("`{op}` overflowed the 64-bit integer range"),
        sp,
    )
}

/// Expression evaluation shared by render and handlers. `locals` is empty at
/// render (labels and conditions see only state).
struct Eval<'s> {
    fuel: &'s mut Fuel,
    state: &'s State,
    locals: &'s [(String, Value)],
    max_str: usize,
}

impl Eval<'_> {
    fn get(&self, name: &str) -> Value {
        self.locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .or_else(|| self.state.iter().find(|(n, _)| n == name))
            .map(|(_, v)| v.clone())
            .expect("checked: every name resolves")
    }

    fn expr(&mut self, e: &Expr) -> Result<Value, Diag> {
        self.fuel.burn(1).map_err(|_| fuel_out(e.span()))?;
        match e {
            Expr::Int(v, _) => Ok(Value::Int(*v)),
            Expr::Bool(b, _) => Ok(Value::Bool(*b)),
            Expr::Str(s, _) => Ok(Value::Str(s.clone())),
            Expr::Var(name, _) => Ok(self.get(name)),
            Expr::Unary(op, inner, sp) => {
                let v = self.expr(inner)?;
                match (op, v) {
                    (UnOp::Neg, Value::Int(n)) => n
                        .checked_neg()
                        .map(Value::Int)
                        .ok_or_else(|| overflow("-", *sp)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    _ => unreachable!("checked: unary operand types"),
                }
            }
            Expr::Binary(op, l, r, sp) => {
                // Short-circuit rows first — the right side must not run.
                if let BinOp::And | BinOp::Or = op {
                    let Value::Bool(lv) = self.expr(l)? else {
                        unreachable!("checked: bool operands")
                    };
                    return match (op, lv) {
                        (BinOp::And, false) => Ok(Value::Bool(false)),
                        (BinOp::Or, true) => Ok(Value::Bool(true)),
                        _ => self.expr(r),
                    };
                }
                let lv = self.expr(l)?;
                let rv = self.expr(r)?;
                self.binary(*op, lv, rv, *sp)
            }
        }
    }

    fn binary(&mut self, op: BinOp, lv: Value, rv: Value, sp: Span) -> Result<Value, Diag> {
        use BinOp::*;
        // String `+`: concatenation, the non-string side displayed in. The
        // result is BOUNDED — past `max_str` bytes it is a fault, not a
        // clip (a silently truncated string is a wrong-but-clean result).
        if op == Add && (matches!(lv, Value::Str(_)) || matches!(rv, Value::Str(_))) {
            let (a, b) = (lv.to_string(), rv.to_string());
            if a.len() + b.len() > self.max_str {
                return Err(Diag::at_code(
                    codes::STR_TOO_LONG,
                    format!(
                        "string would be {} bytes; the limit is {}",
                        a.len() + b.len(),
                        self.max_str
                    ),
                    sp,
                ));
            }
            return Ok(Value::Str(a + &b));
        }
        if let (Value::Int(a), Value::Int(b)) = (&lv, &rv) {
            let (a, b) = (*a, *b);
            let int = |r: Option<i64>| r.map(Value::Int).ok_or_else(|| overflow(op.sym(), sp));
            return match op {
                Add => int(a.checked_add(b)),
                Sub => int(a.checked_sub(b)),
                Mul => int(a.checked_mul(b)),
                Div | Rem if b == 0 => Err(Diag::at_code(
                    codes::DIV_BY_ZERO,
                    "division or remainder by zero",
                    sp,
                )),
                Div => int(a.checked_div(b)),
                Rem => int(a.checked_rem(b)),
                Lt => Ok(Value::Bool(a < b)),
                Le => Ok(Value::Bool(a <= b)),
                Gt => Ok(Value::Bool(a > b)),
                Ge => Ok(Value::Bool(a >= b)),
                Eq => Ok(Value::Bool(a == b)),
                Ne => Ok(Value::Bool(a != b)),
                And | Or => unreachable!("handled before operand eval"),
            };
        }
        match op {
            Eq => Ok(Value::Bool(lv == rv)),
            Ne => Ok(Value::Bool(lv != rv)),
            _ => unreachable!("checked: binary operand types"),
        }
    }
}

/// Render `state` through the widget tree: a fresh fuel tank, a pure pass.
pub(crate) fn render(program: &Program, state: &State, limits: &Limits) -> Result<Vec<Node>, Diag> {
    let mut fuel = Fuel::new(limits.fuel);
    let mut nodes = Vec::new();
    for w in program.widgets() {
        widget(w, state, &mut fuel, limits, &mut nodes)?;
    }
    Ok(nodes)
}

fn widget(
    w: &Widget,
    state: &State,
    fuel: &mut Fuel,
    limits: &Limits,
    out: &mut Vec<Node>,
) -> Result<(), Diag> {
    fuel.burn(1).map_err(|_| fuel_out(w_span(w)))?;
    let mut ev = Eval {
        fuel,
        state,
        locals: &[],
        max_str: limits.max_str_bytes,
    };
    match w {
        Widget::Label { value, .. } => {
            let text = ev.expr(value)?.to_string();
            out.push(Node::Label { text });
            Ok(())
        }
        Widget::Button { text, id, .. } => {
            out.push(Node::Button {
                text: text.clone(),
                id: *id,
            });
            Ok(())
        }
        Widget::Input { state: name, .. } => {
            let Value::Str(value) = ev.get(name) else {
                unreachable!("checked: input binds a string state")
            };
            out.push(Node::Input {
                state: name.clone(),
                value,
            });
            Ok(())
        }
        Widget::Row { children, .. } | Widget::Col { children, .. } => {
            let mut inner = Vec::new();
            for c in children {
                widget(c, state, fuel, limits, &mut inner)?;
            }
            out.push(if matches!(w, Widget::Row { .. }) {
                Node::Row { children: inner }
            } else {
                Node::Col { children: inner }
            });
            Ok(())
        }
        Widget::If { arms, els, .. } => {
            for (cond, body) in arms {
                let mut ev = Eval {
                    fuel,
                    state,
                    locals: &[],
                    max_str: limits.max_str_bytes,
                };
                if ev.expr(cond)? == Value::Bool(true) {
                    for c in body {
                        widget(c, state, fuel, limits, out)?;
                    }
                    return Ok(());
                }
            }
            for c in els {
                widget(c, state, fuel, limits, out)?;
            }
            Ok(())
        }
    }
}

fn w_span(w: &Widget) -> Span {
    match w {
        Widget::Label { span, .. }
        | Widget::Button { span, .. }
        | Widget::Input { span, .. }
        | Widget::Row { span, .. }
        | Widget::Col { span, .. }
        | Widget::If { span, .. } => *span,
    }
}

/// Handle one event atomically: run against a COPY of the state, commit only
/// on a clean finish. `Err` means the state is exactly as it was.
pub(crate) fn handle(
    program: &Program,
    state: &State,
    event: &Event,
    limits: &Limits,
) -> Result<State, Diag> {
    let bad = |msg: String| Diag::new_code(codes::BAD_EVENT, msg);
    let mut next = state.clone();
    match event {
        Event::Click { id } => {
            let body = find_button(program.widgets(), *id)
                .ok_or_else(|| bad(format!("no button with id {id}")))?;
            let mut fuel = Fuel::new(limits.fuel);
            let mut h = Handler {
                fuel: &mut fuel,
                state: &mut next,
                locals: Vec::new(),
                frames: Vec::new(),
                limits,
            };
            h.block(body)?;
        }
        Event::Input { state: name, text } => {
            let slot = next
                .iter_mut()
                .find(|(n, _)| n == name)
                .ok_or_else(|| bad(format!("no state named `{name}`")))?;
            let Value::Str(_) = slot.1 else {
                return Err(bad(format!("state `{name}` is not a string")));
            };
            // Host text is hostile by default: clip to the string bound at a
            // char boundary (never mid-char — the parents' mojibake lesson).
            let mut cut = text.len().min(limits.max_str_bytes);
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            slot.1 = Value::Str(text[..cut].to_string());
        }
    }
    let total: usize = next
        .iter()
        .map(|(_, v)| match v {
            Value::Str(s) => s.len(),
            _ => 0,
        })
        .sum();
    if total > limits.max_state_bytes {
        return Err(Diag::new_code(
            codes::STATE_TOO_BIG,
            format!(
                "string state totals {total} bytes; the limit is {}",
                limits.max_state_bytes
            ),
        ));
    }
    Ok(next)
}

fn find_button(widgets: &[Widget], id: u32) -> Option<&Vec<Stmt>> {
    for w in widgets {
        match w {
            Widget::Button { id: bid, body, .. } if *bid == id => return Some(body),
            Widget::Row { children, .. } | Widget::Col { children, .. } => {
                if let Some(b) = find_button(children, id) {
                    return Some(b);
                }
            }
            Widget::If { arms, els, .. } => {
                for (_, body) in arms {
                    if let Some(b) = find_button(body, id) {
                        return Some(b);
                    }
                }
                if let Some(b) = find_button(els, id) {
                    return Some(b);
                }
            }
            _ => {}
        }
    }
    None
}

/// Handler-body execution: statements over locals + the state copy.
struct Handler<'s> {
    fuel: &'s mut Fuel,
    state: &'s mut State,
    locals: Vec<(String, Value)>,
    frames: Vec<usize>,
    limits: &'s Limits,
}

impl Handler<'_> {
    fn block(&mut self, stmts: &[Stmt]) -> Result<(), Diag> {
        self.frames.push(self.locals.len());
        let r = stmts.iter().try_for_each(|s| self.stmt(s));
        let mark = self.frames.pop().unwrap_or(0);
        self.locals.truncate(mark);
        r
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, Diag> {
        let mut ev = Eval {
            fuel: self.fuel,
            state: self.state,
            locals: &self.locals,
            max_str: self.limits.max_str_bytes,
        };
        ev.expr(e)
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), Diag> {
        self.fuel.burn(1).map_err(|_| fuel_out(s_span(s)))?;
        match s {
            Stmt::Let { name, value, .. } => {
                let v = self.eval(value)?;
                self.locals.push((name.clone(), v));
                Ok(())
            }
            Stmt::Assign { name, value, .. } => {
                let v = self.eval(value)?;
                let slot = self
                    .locals
                    .iter_mut()
                    .rev()
                    .find(|(n, _)| n == name)
                    .or_else(|| self.state.iter_mut().find(|(n, _)| n == name))
                    .expect("checked: assignment target resolves");
                slot.1 = v;
                Ok(())
            }
            Stmt::If { arms, els, .. } => {
                for (cond, body) in arms {
                    if self.eval(cond)? == Value::Bool(true) {
                        return self.block(body);
                    }
                }
                self.block(els)
            }
            Stmt::Repeat { count, body, span } => {
                let Value::Int(n) = self.eval(count)? else {
                    unreachable!("checked: repeat count is int")
                };
                if n < 0 {
                    return Err(Diag::at_code(
                        codes::NEGATIVE_REPEAT,
                        format!("repeat count is {n}"),
                        *span,
                    ));
                }
                for _ in 0..n {
                    self.fuel.burn(1).map_err(|_| fuel_out(*span))?;
                    self.block(body)?;
                }
                Ok(())
            }
        }
    }
}

fn s_span(s: &Stmt) -> Span {
    match s {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::Repeat { span, .. } => *span,
    }
}

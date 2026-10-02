//! The runtime: a fueled tree walk. Rendering is pure: it reads state it
//! cannot change. Handling is atomic: a handler runs on a copy of the state,
//! committed only on a clean finish. The checker already resolved every name
//! and type, so the faults left are checked arithmetic, fuel and string bounds.

use applang_syntax::ast::{BinOp, Expr, Lit, Stmt, UnOp, Widget};
use fuel::Fuel;
use lang::{Diag, Span};

use crate::{Event, Limits, Node, Program, codes};

/// An applang runtime value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Value { Int(i64), Bool(bool), Str(String) }

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => f.write_str(s),
        }
    }
}

/// The live state: `(name, value)` pairs in declaration order.
pub(crate) type State = Vec<(String, Value)>;

pub(crate) fn init_state(program: &Program) -> State {
    let value = |l: &Lit| match l {
        Lit::Int(v) => Value::Int(*v),
        Lit::Bool(b) => Value::Bool(*b),
        Lit::Str(s) => Value::Str(s.clone()),
    };
    program.states().iter().map(|s| (s.name.clone(), value(&s.init))).collect()
}

/// Burns one unit, or faults at `sp`.
fn burn(fuel: &mut Fuel, sp: Span) -> Result<(), Diag> {
    fuel.burn(1).map_err(|_| Diag::at_code(codes::FUEL_EXHAUSTED, "fuel exhausted", sp))
}

fn overflow(op: &str, sp: Span) -> Diag {
    Diag::at_code(codes::OVERFLOW, format!("`{op}` overflowed the 64-bit integer range"), sp)
}

/// Expression evaluation for render and handlers; at render `locals` is empty.
struct Eval<'s> {
    fuel: &'s mut Fuel,
    state: &'s State,
    locals: &'s [(String, Value)],
    max_str: usize,
}

impl Eval<'_> {
    fn get(&self, name: &str) -> Value {
        let local = self.locals.iter().rev().find(|(n, _)| n == name);
        let found = local.or_else(|| self.state.iter().find(|(n, _)| n == name));
        found.map(|(_, v)| v.clone()).expect("checked: every name resolves")
    }

    fn expr(&mut self, e: &Expr) -> Result<Value, Diag> {
        burn(self.fuel, e.span())?;
        match e {
            Expr::Int(v, _) => Ok(Value::Int(*v)),
            Expr::Bool(b, _) => Ok(Value::Bool(*b)),
            Expr::Str(s, _) => Ok(Value::Str(s.clone())),
            Expr::Var(name, _) => Ok(self.get(name)),
            Expr::Unary(op, inner, sp) => match (op, self.expr(inner)?) {
                (UnOp::Neg, Value::Int(n)) => {
                    n.checked_neg().map(Value::Int).ok_or_else(|| overflow("-", *sp))
                }
                (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                _ => unreachable!("checked: unary operand types"),
            },
            // `&&` and `||` short-circuit: the right side must not run.
            Expr::Binary(op @ (BinOp::And | BinOp::Or), l, r, _) => {
                let Value::Bool(lv) = self.expr(l)? else { unreachable!("checked: bool") };
                match (op, lv) {
                    (BinOp::And, false) | (BinOp::Or, true) => Ok(Value::Bool(lv)),
                    _ => self.expr(r),
                }
            }
            Expr::Binary(op, l, r, sp) => {
                let lv = self.expr(l)?;
                let rv = self.expr(r)?;
                self.binary(*op, lv, rv, *sp)
            }
        }
    }

    fn binary(&mut self, op: BinOp, lv: Value, rv: Value, sp: Span) -> Result<Value, Diag> {
        use BinOp::*;
        // String `+` concatenates, the other side displayed in. Past
        // `max_str` bytes it faults: a silent clip would be wrong-but-clean.
        if op == Add && (matches!(lv, Value::Str(_)) || matches!(rv, Value::Str(_))) {
            let (a, b) = (lv.to_string(), rv.to_string());
            let len = a.len() + b.len();
            if len > self.max_str {
                let msg = format!("string would be {len} bytes; the limit is {}", self.max_str);
                return Err(Diag::at_code(codes::STR_TOO_LONG, msg, sp));
            }
            return Ok(Value::Str(a + &b));
        }
        let (Value::Int(a), Value::Int(b)) = (&lv, &rv) else {
            return match op {
                Eq => Ok(Value::Bool(lv == rv)),
                Ne => Ok(Value::Bool(lv != rv)),
                _ => unreachable!("checked: binary operand types"),
            };
        };
        let (a, b) = (*a, *b);
        let int = |r: Option<i64>| r.map(Value::Int).ok_or_else(|| overflow(op.sym(), sp));
        match op {
            Add => int(a.checked_add(b)),
            Sub => int(a.checked_sub(b)),
            Mul => int(a.checked_mul(b)),
            Div | Rem if b == 0 => {
                Err(Diag::at_code(codes::DIV_BY_ZERO, "division or remainder by zero", sp))
            }
            Div => int(a.checked_div(b)),
            Rem => int(a.checked_rem(b)),
            Lt => Ok(Value::Bool(a < b)),
            Le => Ok(Value::Bool(a <= b)),
            Gt => Ok(Value::Bool(a > b)),
            Ge => Ok(Value::Bool(a >= b)),
            Eq => Ok(Value::Bool(a == b)),
            Ne => Ok(Value::Bool(a != b)),
            And | Or => unreachable!("short-circuited in expr"),
        }
    }
}

/// One render: a fresh fuel tank and at most `max_render_bytes` of text.
struct Render<'s> {
    fuel: Fuel,
    left: usize,
    state: &'s State,
    max_str: usize,
}

pub(crate) fn render(program: &Program, state: &State, limits: &Limits) -> Result<Vec<Node>, Diag> {
    let fuel = Fuel::new(limits.fuel);
    let mut r =
        Render { fuel, left: limits.max_render_bytes, state, max_str: limits.max_str_bytes };
    let mut nodes = Vec::new();
    r.widgets(program.widgets(), &mut nodes)?;
    Ok(nodes)
}

impl Render<'_> {
    fn eval(&mut self, e: &Expr) -> Result<Value, Diag> {
        let (state, max_str) = (self.state, self.max_str);
        Eval { fuel: &mut self.fuel, state, locals: &[], max_str }.expr(e)
    }

    /// Takes `n` bytes of text from those left, or faults at `w`.
    fn spend(&mut self, n: usize, w: &Widget) -> Result<(), Diag> {
        let msg = "render output exceeds the byte limit";
        let err = || Diag::at_code(codes::RENDER_TOO_BIG, msg, w_span(w));
        self.left = self.left.checked_sub(n).ok_or_else(err)?;
        Ok(())
    }

    fn widgets(&mut self, ws: &[Widget], out: &mut Vec<Node>) -> Result<(), Diag> {
        ws.iter().try_for_each(|w| self.widget(w, out))
    }

    fn widget(&mut self, w: &Widget, out: &mut Vec<Node>) -> Result<(), Diag> {
        burn(&mut self.fuel, w_span(w))?;
        match w {
            Widget::Label { value, .. } => {
                let text = self.eval(value)?.to_string();
                self.spend(text.len(), w)?;
                out.push(Node::Label { text });
            }
            Widget::Button { text, id, .. } => {
                self.spend(text.len(), w)?;
                out.push(Node::Button { text: text.clone(), id: *id });
            }
            Widget::Input { state: name, .. } => {
                let Some((_, Value::Str(value))) = self.state.iter().find(|(n, _)| n == name)
                else {
                    unreachable!("checked: input binds a string state")
                };
                self.spend(name.len() + value.len(), w)?;
                out.push(Node::Input { state: name.clone(), value: value.clone() });
            }
            Widget::Row { children, .. } | Widget::Col { children, .. } => {
                let mut inner = Vec::new();
                self.widgets(children, &mut inner)?;
                out.push(match w {
                    Widget::Row { .. } => Node::Row { children: inner },
                    _ => Node::Col { children: inner },
                });
            }
            Widget::If { arms, els, .. } => {
                for (cond, body) in arms {
                    if self.eval(cond)? == Value::Bool(true) {
                        return self.widgets(body, out);
                    }
                }
                self.widgets(els, out)?;
            }
        }
        Ok(())
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

/// Handles one event on a copy of `state`; `Err` leaves `state` as it was.
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
            let (fuel, max_str) = (Fuel::new(limits.fuel), limits.max_str_bytes);
            Handler { fuel, state: &mut next, locals: Vec::new(), max_str }.block(body)?;
        }
        Event::Input { state: name, text } => {
            let slot = next
                .iter_mut()
                .find(|(n, _)| n == name)
                .ok_or_else(|| bad(format!("no state named `{name}`")))?;
            let Value::Str(_) = slot.1 else {
                return Err(bad(format!("state `{name}` is not a string")));
            };
            // Host text is hostile: clip to the string bound, never mid-char.
            let mut cut = text.len().min(limits.max_str_bytes);
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            slot.1 = Value::Str(text[..cut].to_string());
        }
    }
    let str_len = |(_, v): &(String, Value)| if let Value::Str(s) = v { s.len() } else { 0 };
    let total: usize = next.iter().map(str_len).sum();
    if total > limits.max_state_bytes {
        let msg =
            format!("string state totals {total} bytes; the limit is {}", limits.max_state_bytes);
        return Err(Diag::new_code(codes::STATE_TOO_BIG, msg));
    }
    Ok(next)
}

/// The handler of button `id`, hidden or not.
fn find_button(widgets: &[Widget], id: u32) -> Option<&[Stmt]> {
    widgets.iter().find_map(|w| match w {
        Widget::Button { id: bid, body, .. } if *bid == id => Some(&body[..]),
        Widget::Row { children, .. } | Widget::Col { children, .. } => find_button(children, id),
        Widget::If { arms, els, .. } => {
            arms.iter().find_map(|(_, body)| find_button(body, id)).or_else(|| find_button(els, id))
        }
        _ => None,
    })
}

/// Runs handler statements over block-scoped locals and the state copy.
struct Handler<'s> {
    fuel: Fuel,
    state: &'s mut State,
    locals: Vec<(String, Value)>,
    max_str: usize,
}

impl Handler<'_> {
    fn block(&mut self, stmts: &[Stmt]) -> Result<(), Diag> {
        let mark = self.locals.len();
        let r = stmts.iter().try_for_each(|s| self.stmt(s));
        self.locals.truncate(mark);
        r
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, Diag> {
        let (state, locals, max_str) = (&*self.state, &self.locals[..], self.max_str);
        Eval { fuel: &mut self.fuel, state, locals, max_str }.expr(e)
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), Diag> {
        let (Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::Repeat { span, .. }) = s;
        burn(&mut self.fuel, *span)?;
        match s {
            Stmt::Let { name, value, .. } => {
                let v = self.eval(value)?;
                self.locals.push((name.clone(), v));
            }
            Stmt::Assign { name, value, .. } => {
                let v = self.eval(value)?;
                let local = self.locals.iter_mut().rev().find(|(n, _)| n == name);
                let slot = local.or_else(|| self.state.iter_mut().find(|(n, _)| n == name));
                slot.expect("checked: assignment target resolves").1 = v;
            }
            Stmt::If { arms, els, .. } => {
                for (cond, body) in arms {
                    if self.eval(cond)? == Value::Bool(true) {
                        return self.block(body);
                    }
                }
                self.block(els)?;
            }
            Stmt::Repeat { count, body, span } => {
                let Value::Int(n) = self.eval(count)? else { unreachable!("checked: int") };
                if n < 0 {
                    let msg = format!("repeat count is {n}");
                    return Err(Diag::at_code(codes::NEGATIVE_REPEAT, msg, *span));
                }
                for _ in 0..n {
                    burn(&mut self.fuel, *span)?;
                    self.block(body)?;
                }
            }
        }
        Ok(())
    }
}

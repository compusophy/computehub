//! The static checker: names resolve and types come from literals and
//! initializers. Unguarded recursion is safe: the parser bounds AST depth.

use lang::{Diag, Span};

use crate::codes;
use crate::parse::{BinOp, Expr, Lit, Program, Stmt, UnOp, Widget};

/// applang's static types; every expression has exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
enum Type { Int, Bool, Str }

impl Type {
    fn name(self) -> &'static str {
        ["int", "bool", "string"][self as usize]
    }
}

/// Typed scopes: block-scoped locals over the states.
#[rustfmt::skip]
struct Scopes<'p> { states: &'p [(String, Type)], locals: Vec<(String, Type)> }

impl Scopes<'_> {
    fn get(&self, name: &str) -> Option<Type> {
        let found = |v: &[(String, Type)]| v.iter().rev().find(|(n, _)| n == name).map(|p| p.1);
        found(&self.locals).or_else(|| found(self.states))
    }
}

fn unknown(name: &str, sp: Span) -> Diag {
    let msg = format!("`{name}` is not a declared state or local variable");
    Diag::at_code(codes::UNKNOWN_NAME, msg, sp)
}

fn mismatch(msg: String, sp: Span) -> Diag {
    Diag::at_code(codes::TYPE_MISMATCH, msg, sp)
}

/// Checks that every name resolves and every expression types.
pub(crate) fn check(program: &Program) -> Result<(), Diag> {
    let mut states: Vec<(String, Type)> = Vec::new();
    for s in &program.states {
        if states.iter().any(|(n, _)| *n == s.name) {
            let msg = format!("state `{}` is declared twice", s.name);
            return Err(Diag::at_code(codes::DUP_STATE, msg, s.name_span));
        }
        let ty = match s.init {
            Lit::Int(_) => Type::Int,
            Lit::Bool(_) => Type::Bool,
            Lit::Str(_) => Type::Str,
        };
        states.push((s.name.clone(), ty));
    }
    program.widgets.iter().try_for_each(|w| widget(w, &states))
}

fn widget(w: &Widget, states: &[(String, Type)]) -> Result<(), Diag> {
    let mut sc = Scopes { states, locals: Vec::new() };
    let all = |ws: &[Widget]| ws.iter().try_for_each(|c| widget(c, states));
    match w {
        // Labels display any type; the value only has to have one.
        Widget::Label { value, .. } => expr(value, &sc).map(drop),
        Widget::Button { body, .. } => block(body, &mut sc),
        Widget::Input { state, state_span, .. } => match sc.get(state) {
            Some(Type::Str) => Ok(()),
            Some(t) => {
                let msg = format!("`input` binds a string state; `{state}` is {}", t.name());
                Err(mismatch(msg, *state_span))
            }
            None => Err(unknown(state, *state_span)),
        },
        Widget::Row { children, .. } | Widget::Col { children, .. } => all(children),
        Widget::If { arms, els, .. } => {
            for (cond, body) in arms {
                expect_type(cond, Type::Bool, "an `if` condition", &sc)?;
                all(body)?;
            }
            all(els)
        }
    }
}

fn block(stmts: &[Stmt], sc: &mut Scopes<'_>) -> Result<(), Diag> {
    let mark = sc.locals.len();
    let r = stmts.iter().try_for_each(|s| stmt(s, sc));
    sc.locals.truncate(mark);
    r
}

fn stmt(s: &Stmt, sc: &mut Scopes<'_>) -> Result<(), Diag> {
    match s {
        Stmt::Let { name, value, .. } => {
            let t = expr(value, sc)?;
            sc.locals.push((name.clone(), t));
            Ok(())
        }
        Stmt::Assign { name, name_span, value, .. } => {
            let target = sc.get(name).ok_or_else(|| unknown(name, *name_span))?;
            let got = expr(value, sc)?;
            let (t, g) = (target.name(), got.name());
            let msg = || format!("`{name}` is {t}; cannot assign {g} to it");
            if got == target { Ok(()) } else { Err(mismatch(msg(), value.span())) }
        }
        Stmt::If { arms, els, .. } => {
            for (cond, body) in arms {
                expect_type(cond, Type::Bool, "an `if` condition", sc)?;
                block(body, sc)?;
            }
            block(els, sc)
        }
        Stmt::Repeat { count, body, .. } => {
            expect_type(count, Type::Int, "a `repeat` count", sc)?;
            block(body, sc)
        }
    }
}

fn expect_type(e: &Expr, want: Type, what: &str, sc: &Scopes<'_>) -> Result<(), Diag> {
    let got = expr(e, sc)?;
    let msg = || format!("{what} must be {}, got {}", want.name(), got.name());
    if got == want { Ok(()) } else { Err(mismatch(msg(), e.span())) }
}

fn expr(e: &Expr, sc: &Scopes<'_>) -> Result<Type, Diag> {
    use BinOp::*;
    use Type::*;
    match e {
        Expr::Int(..) => Ok(Int),
        Expr::Bool(..) => Ok(Bool),
        Expr::Str(..) => Ok(Str),
        Expr::Var(name, sp) => sc.get(name).ok_or_else(|| unknown(name, *sp)),
        Expr::Unary(op, inner, sp) => match (op, expr(inner, sc)?) {
            (UnOp::Neg, Int) => Ok(Int),
            (UnOp::Not, Bool) => Ok(Bool),
            (UnOp::Neg, t) => Err(mismatch(format!("`-` needs int, got {}", t.name()), *sp)),
            (UnOp::Not, t) => Err(mismatch(format!("`!` needs bool, got {}", t.name()), *sp)),
        },
        Expr::Binary(op, l, r, sp) => {
            let (lt, rt) = (expr(l, sc)?, expr(r, sc)?);
            match (op, lt, rt) {
                // `+` adds ints; with a string operand it concatenates the other's display.
                (Add, Int, Int) => Ok(Int),
                (Add, Str, _) | (Add, _, Str) => Ok(Str),
                (Sub | Mul | Div | Rem, Int, Int) => Ok(Int),
                (Lt | Le | Gt | Ge, Int, Int) => Ok(Bool),
                // Equality is same-type only: `1 == "1"` is a bug, not `false`.
                (Eq | Ne, a, b) if a == b => Ok(Bool),
                (And | Or, Bool, Bool) => Ok(Bool),
                _ => {
                    let (o, l, r) = (op.sym(), lt.name(), rt.name());
                    Err(mismatch(format!("`{o}` cannot combine {l} and {r}"), *sp))
                }
            }
        }
    }
}

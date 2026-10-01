//! applang static checker: name resolution and type checking, at compile
//! time. State declarations fix every state's type from its literal; locals
//! take their initializer's type; every expression then has exactly one type
//! or a coded, spanned diag. After `check`, the only faults left to runtime
//! are arithmetic (checked), fuel, and string-size bounds — everything
//! nameable is caught before the app ever runs.
//!
//! Recursion here walks the AST directly with no guard of its own — safe for
//! the same reason eval is: the parser bounds AST depth (nesting AND binary
//! spines), so whatever parses, this can walk within a bounded stack.

use lang::{Diag, Span};

use crate::codes;
use crate::parse::{BinOp, Expr, Lit, Program, Stmt, UnOp, Widget};

/// applang's static types. Every expression has exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
    Str,
}

impl Type {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Type::Int => "int",
            Type::Bool => "bool",
            Type::Str => "string",
        }
    }
}

pub(crate) fn type_of_lit(l: &Lit) -> Type {
    match l {
        Lit::Int(_) => Type::Int,
        Lit::Bool(_) => Type::Bool,
        Lit::Str(_) => Type::Str,
    }
}

/// Typed lexical scopes for handler bodies: a flat stack plus frame marks,
/// with the state block as the permanent outermost frame.
struct Scopes<'p> {
    states: &'p [(String, Type)],
    locals: Vec<(String, Type)>,
    frames: Vec<usize>,
}

impl Scopes<'_> {
    fn push(&mut self) {
        self.frames.push(self.locals.len());
    }
    fn pop(&mut self) {
        let mark = self.frames.pop().unwrap_or(0);
        self.locals.truncate(mark);
    }
    fn get(&self, name: &str) -> Option<Type> {
        self.locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, t)| *t)
            .or_else(|| self.states.iter().find(|(n, _)| n == name).map(|(_, t)| *t))
    }
}

fn unknown(name: &str, sp: Span) -> Diag {
    Diag::at_code(
        codes::UNKNOWN_NAME,
        format!("`{name}` is not a declared state or local variable"),
        sp,
    )
}

fn mismatch(msg: String, sp: Span) -> Diag {
    Diag::at_code(codes::TYPE_MISMATCH, msg, sp)
}

/// Check `program`: every name resolves, every expression types. Called by
/// [`crate::compile`]; a `Program` you hold has always passed it.
pub(crate) fn check(program: &Program) -> Result<(), Diag> {
    let mut states: Vec<(String, Type)> = Vec::new();
    for s in &program.states {
        if states.iter().any(|(n, _)| *n == s.name) {
            return Err(Diag::at_code(
                codes::DUP_STATE,
                format!("state `{}` is declared twice", s.name),
                s.name_span,
            ));
        }
        states.push((s.name.clone(), type_of_lit(&s.init)));
    }
    for w in &program.widgets {
        widget(w, &states)?;
    }
    Ok(())
}

fn widget(w: &Widget, states: &[(String, Type)]) -> Result<(), Diag> {
    let render_scope = Scopes { states, locals: Vec::new(), frames: Vec::new() };
    match w {
        Widget::Label { value, .. } => {
            // Labels display any type; it only has to BE one.
            expr(value, &render_scope)?;
            Ok(())
        }
        Widget::Button { body, .. } => {
            let mut sc = Scopes { states, locals: Vec::new(), frames: Vec::new() };
            block(body, &mut sc)
        }
        Widget::Input { state, state_span, .. } => match states.iter().find(|(n, _)| n == state) {
            Some((_, Type::Str)) => Ok(()),
            Some((_, t)) => Err(mismatch(
                format!("`input` binds a string state; `{state}` is {}", t.name()),
                *state_span,
            )),
            None => Err(unknown(state, *state_span)),
        },
        Widget::Row { children, .. } | Widget::Col { children, .. } => {
            children.iter().try_for_each(|c| widget(c, states))
        }
        Widget::If { arms, els, .. } => {
            for (cond, body) in arms {
                expect_type(cond, Type::Bool, "an `if` condition", &render_scope)?;
                body.iter().try_for_each(|c| widget(c, states))?;
            }
            els.iter().try_for_each(|c| widget(c, states))
        }
    }
}

fn block(stmts: &[Stmt], sc: &mut Scopes<'_>) -> Result<(), Diag> {
    sc.push();
    let r = stmts.iter().try_for_each(|s| stmt(s, sc));
    sc.pop();
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
            let Some(target) = sc.get(name) else {
                return Err(unknown(name, *name_span));
            };
            let got = expr(value, sc)?;
            if got != target {
                return Err(mismatch(
                    format!("`{name}` is {}; cannot assign {} to it", target.name(), got.name()),
                    value.span(),
                ));
            }
            Ok(())
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
    if got != want {
        return Err(mismatch(
            format!("{what} must be {}, got {}", want.name(), got.name()),
            e.span(),
        ));
    }
    Ok(())
}

fn expr(e: &Expr, sc: &Scopes<'_>) -> Result<Type, Diag> {
    match e {
        Expr::Int(..) => Ok(Type::Int),
        Expr::Bool(..) => Ok(Type::Bool),
        Expr::Str(..) => Ok(Type::Str),
        Expr::Var(name, sp) => sc.get(name).ok_or_else(|| unknown(name, *sp)),
        Expr::Unary(op, inner, sp) => {
            let t = expr(inner, sc)?;
            match (op, t) {
                (UnOp::Neg, Type::Int) => Ok(Type::Int),
                (UnOp::Not, Type::Bool) => Ok(Type::Bool),
                (UnOp::Neg, _) => Err(mismatch(format!("`-` needs int, got {}", t.name()), *sp)),
                (UnOp::Not, _) => Err(mismatch(format!("`!` needs bool, got {}", t.name()), *sp)),
            }
        }
        Expr::Binary(op, l, r, sp) => {
            let lt = expr(l, sc)?;
            let rt = expr(r, sc)?;
            binary(*op, lt, rt, *sp)
        }
    }
}

fn binary(op: BinOp, lt: Type, rt: Type, sp: Span) -> Result<Type, Diag> {
    use BinOp::*;
    use Type::*;
    let ok = match (op, lt, rt) {
        // `+` is addition on ints and, with ANY string operand, concatenation
        // — the other side is displayed into the string ("n = " + count).
        (Add, Int, Int) => Some(Int),
        (Add, Str, _) | (Add, _, Str) => Some(Str),
        (Sub | Mul | Div | Rem, Int, Int) => Some(Int),
        (Lt | Le | Gt | Ge, Int, Int) => Some(Bool),
        // Equality is same-type only — `1 == "1"` is a bug, not `false`.
        (Eq | Ne, a, b) if a == b => Some(Bool),
        (And | Or, Bool, Bool) => Some(Bool),
        _ => None,
    };
    ok.ok_or_else(|| {
        mismatch(format!("`{}` cannot combine {} and {}", op.sym(), lt.name(), rt.name()), sp)
    })
}

#[cfg(test)]
mod tests {
    use crate::codes;
    use crate::compile;

    fn code(src: &str) -> u16 {
        compile(src).unwrap_err().code.unwrap()
    }

    #[test]
    fn names_and_types_are_checked_before_running() {
        #[rustfmt::skip]
        let cases = [
            ("label nope;", codes::UNKNOWN_NAME),
            ("state x = 1; button \"b\" { y = 2; }", codes::UNKNOWN_NAME),
            ("input missing;", codes::UNKNOWN_NAME),
            ("state x = 1; state x = 2;", codes::DUP_STATE),
            ("state n = 0; input n;", codes::TYPE_MISMATCH),
            ("if 1 { label 1; }", codes::TYPE_MISMATCH),
            ("state x = 1; button \"b\" { x = \"s\"; }", codes::TYPE_MISMATCH),
            ("state x = 1; button \"b\" { repeat true { } }", codes::TYPE_MISMATCH),
            ("label 1 == \"1\";", codes::TYPE_MISMATCH),
            ("label true + true;", codes::TYPE_MISMATCH),
            ("label -true;", codes::TYPE_MISMATCH),
        ];
        for (src, want) in cases {
            assert_eq!(code(src), want, "{src}");
        }
    }

    #[test]
    fn concat_coerces_and_locals_scope() {
        // Any-side string `+` concatenates; int arithmetic stays int.
        assert!(compile("state n = 0; label \"n = \" + n;").is_ok());
        assert!(compile("state s = \"x\"; label 1 + 2;").is_ok());
        // A block-local disappears when its block ends.
        assert_eq!(
            code("state x = 1; button \"b\" { if true { let t = 1; } x = t; }"),
            codes::UNKNOWN_NAME
        );
        // Locals may shadow states with a DIFFERENT type; the state is intact
        // after the handler (types checked where each name is visible).
        assert!(compile("state x = 1; button \"b\" { let x = \"s\"; x = \"t\"; }").is_ok());
    }
}

//! applang parser: tokens → App AST on the `lang::parse` harness. Recursion
//! enters only through `guarded` — widget nesting (`row`/`col`/`if`) and
//! handler statements alike — and binary folds charge the guard per spine
//! node (a lesson from litelite's prooflite, not forked here: the guard
//! bounds parser recursion, NOT AST depth, unless folds are charged too).
//! Every failure is a coded, spanned `Diag`.

use lang::parse::{DEFAULT_MAX_DEPTH, TokCursor};
use lang::{Diag, Span};

use crate::codes;
use crate::lex::{TokKind, Token, lex, unescape};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub enum UnOp { Neg, Not }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub enum BinOp { Or, And, Eq, Ne, Lt, Le, Gt, Ge, Add, Sub, Mul, Div, Rem }

impl BinOp {
    /// Indexed by declaration order — the discriminant.
    pub fn sym(self) -> &'static str {
        [
            "||", "&&", "==", "!=", "<", "<=", ">", ">=", "+", "-", "*", "/", "%",
        ][self as usize]
    }
}

#[derive(Debug)]
pub enum Expr {
    Int(i64, Span),
    Bool(bool, Span),
    Str(String, Span),
    Var(String, Span),
    Unary(UnOp, Box<Expr>, Span),
    Binary(BinOp, Box<Expr>, Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, sp)
            | Expr::Bool(_, sp)
            | Expr::Str(_, sp)
            | Expr::Var(_, sp)
            | Expr::Unary(_, _, sp)
            | Expr::Binary(_, _, _, sp) => *sp,
        }
    }

    fn set_span(&mut self, sp: Span) {
        match self {
            Expr::Int(_, s)
            | Expr::Bool(_, s)
            | Expr::Str(_, s)
            | Expr::Var(_, s)
            | Expr::Unary(_, _, s)
            | Expr::Binary(_, _, _, s) => *s = sp,
        }
    }
}

#[derive(Debug)]
#[rustfmt::skip]
pub enum Stmt {
    Let { name: String, value: Expr, span: Span },
    Assign { name: String, name_span: Span, value: Expr, span: Span },
    If { arms: Vec<(Expr, Vec<Stmt>)>, els: Vec<Stmt>, span: Span },
    Repeat { count: Expr, body: Vec<Stmt>, span: Span },
}

/// A state declaration's initial value — a LITERAL, so initialization is
/// trivially total and fixes the state's static type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Lit { Int(i64), Bool(bool), Str(String) }

#[derive(Debug)]
#[rustfmt::skip]
pub struct StateDecl { pub name: String, pub name_span: Span, pub init: Lit }

/// The widget tree. `Button::id` is assigned in parse order and is stable
/// for the program's life, visible or not; `If` arms are flat like statement
/// `if`, re-evaluated per render.
#[derive(Debug)]
#[rustfmt::skip]
pub enum Widget {
    Label { value: Expr, span: Span },
    Button { text: String, id: u32, body: Vec<Stmt>, span: Span },
    Input { state: String, state_span: Span, span: Span },
    Row { children: Vec<Widget>, span: Span },
    Col { children: Vec<Widget>, span: Span },
    If { arms: Vec<(Expr, Vec<Widget>)>, els: Vec<Widget>, span: Span },
}

/// A compiled applang app — proof the source is well-formed AND type-checks:
/// [`crate::compile`] is the only way to get one. Run it via `applang::App`.
///
/// The fields stay private so no other crate can forge a tree the parser's
/// depth guard and the checker never saw:
///
/// ```compile_fail,E0451
/// let _ = applang_syntax::Program { states: Vec::new(), widgets: Vec::new() };
/// ```
#[derive(Debug)]
pub struct Program {
    pub(crate) states: Vec<StateDecl>,
    pub(crate) widgets: Vec<Widget>,
}

impl Program {
    /// The state declarations, in source order (read-only: a `Program` can
    /// only be built by [`crate::compile`]).
    pub fn states(&self) -> &[StateDecl] {
        &self.states
    }

    /// The top-level widgets, in source order.
    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }
}

/// Parse `src` (lexing first) into a [`Program`], or the first error as a
/// coded, spanned `Diag`. `state` declarations must precede all widgets —
/// state is the app's whole data model, declared up front. Crate-private:
/// [`crate::compile`] is the public entry, so no unchecked `Program` escapes.
pub(crate) fn parse(src: &str) -> Result<Program, Diag> {
    let toks = lex(src)?;
    let mut cur = TokCursor::new(&toks);
    let mut states = Vec::new();
    while cur.peek().kind == TokKind::State {
        states.push(state_decl(src, &mut cur).map_err(|e| e.0)?);
    }
    let mut widgets = Vec::new();
    let mut next_id = 0u32;
    while !cur.at_last() {
        if cur.peek().kind == TokKind::State {
            return Err(Diag::at_code(
                codes::UNEXPECTED_TOKEN,
                "`state` declarations must come before the first widget",
                cur.peek().span,
            ));
        }
        widgets.push(widget(src, &mut cur, &mut next_id).map_err(|e| e.0)?);
    }
    Ok(Program { states, widgets })
}

struct PErr(Diag);

impl From<Span> for PErr {
    fn from(sp: Span) -> Self {
        PErr(Diag::at_code(
            codes::TOO_DEEP,
            format!("source nests deeper than the parser allows (depth cap {DEFAULT_MAX_DEPTH})"),
            sp,
        ))
    }
}

type PResult<T> = Result<T, PErr>;
type Toks<'t> = TokCursor<'t, Token>;

fn state_decl(src: &str, t: &mut Toks<'_>) -> PResult<StateDecl> {
    expect(src, t, TokKind::State, "`state`")?;
    let (name, name_span) = ident(src, t, "a state name")?;
    expect(src, t, TokKind::Assign, "`=`")?;
    let neg = t.eat(|x| x.kind == TokKind::Minus).is_some();
    let tok = *t.peek();
    #[rustfmt::skip]
    let init = match (tok.kind, neg) {
        (TokKind::Int(v), _) => Lit::Int(if neg { -v } else { v }),
        (TokKind::True, false) => Lit::Bool(true),
        (TokKind::False, false) => Lit::Bool(false),
        (TokKind::Str, false) => Lit::Str(unescape(text(src, tok.span))),
        _ => return Err(unexpected(src, t, "a literal (int, bool, or string)")),
    };
    t.advance();
    expect(src, t, TokKind::Semi, "`;`")?;
    Ok(StateDecl {
        name,
        name_span,
        init,
    })
}

fn widget(src: &str, t: &mut Toks<'_>, next_id: &mut u32) -> PResult<Widget> {
    t.guarded(|t| {
        let tok = *t.peek();
        match tok.kind {
            TokKind::Label => {
                t.advance();
                let value = expr(src, t)?;
                let end = expect(src, t, TokKind::Semi, "`;`")?;
                Ok(Widget::Label {
                    value,
                    span: Span::new(tok.span.start, end.end),
                })
            }
            TokKind::Button => {
                t.advance();
                let text_tok = t
                    .eat(|x| x.kind == TokKind::Str)
                    .ok_or_else(|| unexpected(src, t, "a string button label"))?;
                let (body, end) = braced(src, t, &mut stmt)?;
                let id = *next_id;
                *next_id += 1;
                Ok(Widget::Button {
                    text: unescape(text(src, text_tok.span)),
                    id,
                    body,
                    span: Span::new(tok.span.start, end.end),
                })
            }
            TokKind::Input => {
                t.advance();
                let (state, state_span) = ident(src, t, "a state name")?;
                let end = expect(src, t, TokKind::Semi, "`;`")?;
                Ok(Widget::Input {
                    state,
                    state_span,
                    span: Span::new(tok.span.start, end.end),
                })
            }
            TokKind::Row | TokKind::Col => {
                t.advance();
                let (children, end) = braced(src, t, &mut |s, t| widget(s, t, next_id))?;
                let span = Span::new(tok.span.start, end.end);
                Ok(if tok.kind == TokKind::Row {
                    Widget::Row { children, span }
                } else {
                    Widget::Col { children, span }
                })
            }
            TokKind::If => {
                let (arms, els, end) = if_chain(src, t, &mut |s, t| widget(s, t, next_id))?;
                Ok(Widget::If {
                    arms,
                    els,
                    span: Span::new(tok.span.start, end),
                })
            }
            _ => Err(unexpected(
                src,
                t,
                "a widget (label, button, input, row, col, if)",
            )),
        }
    })
}

/// `{ ITEM* }` — widgets and statements share the block shape, and both
/// `if` forms share the chain shape below (ITERATIVE: flat in the source,
/// flat in the AST, flat in guard depth).
fn braced<T>(
    src: &str,
    t: &mut Toks<'_>,
    item: &mut dyn FnMut(&str, &mut Toks<'_>) -> PResult<T>,
) -> PResult<(Vec<T>, Span)> {
    expect(src, t, TokKind::LBrace, "`{`")?;
    let mut items = Vec::new();
    while t.peek().kind != TokKind::RBrace {
        if t.at_last() {
            return Err(unexpected(src, t, "`}`"));
        }
        items.push(item(src, t)?);
    }
    let end = expect(src, t, TokKind::RBrace, "`}`")?;
    Ok((items, end))
}

#[allow(clippy::type_complexity)]
fn if_chain<T>(
    src: &str,
    t: &mut Toks<'_>,
    item: &mut dyn FnMut(&str, &mut Toks<'_>) -> PResult<T>,
) -> PResult<(Vec<(Expr, Vec<T>)>, Vec<T>, usize)> {
    let mut arms = Vec::new();
    let mut els = Vec::new();
    let mut end;
    loop {
        t.advance(); // the `if`
        let cond = expr(src, t)?;
        let (body, bend) = braced(src, t, item)?;
        end = bend.end;
        arms.push((cond, body));
        if t.eat(|x| x.kind == TokKind::Else).is_none() {
            break;
        }
        if t.peek().kind == TokKind::If {
            continue;
        }
        let (body, bend) = braced(src, t, item)?;
        els = body;
        end = bend.end;
        break;
    }
    Ok((arms, els, end))
}

fn stmt(src: &str, t: &mut Toks<'_>) -> PResult<Stmt> {
    t.guarded(|t| {
        let tok = *t.peek();
        match tok.kind {
            TokKind::Let => {
                t.advance();
                let (name, _) = ident(src, t, "a variable name")?;
                expect(src, t, TokKind::Assign, "`=`")?;
                let value = expr(src, t)?;
                let end = expect(src, t, TokKind::Semi, "`;`")?;
                Ok(Stmt::Let {
                    name,
                    value,
                    span: Span::new(tok.span.start, end.end),
                })
            }
            TokKind::Ident => {
                let name = text(src, tok.span).to_string();
                t.advance();
                expect(src, t, TokKind::Assign, "`=`")?;
                let value = expr(src, t)?;
                let end = expect(src, t, TokKind::Semi, "`;`")?;
                Ok(Stmt::Assign {
                    name,
                    name_span: tok.span,
                    value,
                    span: Span::new(tok.span.start, end.end),
                })
            }
            TokKind::If => {
                let (arms, els, end) = if_chain(src, t, &mut stmt)?;
                Ok(Stmt::If {
                    arms,
                    els,
                    span: Span::new(tok.span.start, end),
                })
            }
            TokKind::Repeat => {
                t.advance();
                let count = expr(src, t)?;
                let (body, end) = braced(src, t, &mut stmt)?;
                Ok(Stmt::Repeat {
                    count,
                    body,
                    span: Span::new(tok.span.start, end.end),
                })
            }
            _ => Err(unexpected(src, t, "a statement")),
        }
    })
}

const LADDER: &[&[(TokKind, BinOp)]] = &[
    &[(TokKind::OrOr, BinOp::Or)],
    &[(TokKind::AndAnd, BinOp::And)],
    &[(TokKind::EqEq, BinOp::Eq), (TokKind::BangEq, BinOp::Ne)],
    &[
        (TokKind::Lt, BinOp::Lt),
        (TokKind::LtEq, BinOp::Le),
        (TokKind::Gt, BinOp::Gt),
        (TokKind::GtEq, BinOp::Ge),
    ],
    &[(TokKind::Plus, BinOp::Add), (TokKind::Minus, BinOp::Sub)],
    &[
        (TokKind::Star, BinOp::Mul),
        (TokKind::Slash, BinOp::Div),
        (TokKind::Percent, BinOp::Rem),
    ],
];

fn expr(src: &str, t: &mut Toks<'_>) -> PResult<Expr> {
    t.guarded(|t| binary(src, t, 0))
}

fn binary(src: &str, t: &mut Toks<'_>, level: usize) -> PResult<Expr> {
    if level == LADDER.len() {
        return unary(src, t);
    }
    // Each fold deepens the AST's left spine by one node the evaluator (and
    // drop glue) later recurse through — so every fold charges one guard
    // entry, released together when this level completes (litelite's
    // prooflite lesson).
    let mut entered = 0usize;
    let r = fold_level(src, t, level, &mut entered);
    for _ in 0..entered {
        t.leave();
    }
    r
}

fn fold_level(src: &str, t: &mut Toks<'_>, level: usize, entered: &mut usize) -> PResult<Expr> {
    let mut lhs = binary(src, t, level + 1)?;
    'level: loop {
        for &(kind, op) in LADDER[level] {
            if t.peek().kind == kind {
                *entered += 1;
                t.enter()?;
                t.advance();
                let rhs = binary(src, t, level + 1)?;
                let span = Span::new(lhs.span().start, rhs.span().end);
                lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs), span);
                continue 'level;
            }
        }
        return Ok(lhs);
    }
}

fn unary(src: &str, t: &mut Toks<'_>) -> PResult<Expr> {
    t.guarded(|t| {
        let tok = *t.peek();
        let op = match tok.kind {
            TokKind::Minus => UnOp::Neg,
            TokKind::Bang => UnOp::Not,
            _ => return primary(src, t),
        };
        t.advance();
        let operand = unary(src, t)?;
        let span = Span::new(tok.span.start, operand.span().end);
        Ok(Expr::Unary(op, Box::new(operand), span))
    })
}

fn primary(src: &str, t: &mut Toks<'_>) -> PResult<Expr> {
    let tok = *t.peek();
    match tok.kind {
        TokKind::Int(v) => {
            t.advance();
            Ok(Expr::Int(v, tok.span))
        }
        TokKind::True => {
            t.advance();
            Ok(Expr::Bool(true, tok.span))
        }
        TokKind::False => {
            t.advance();
            Ok(Expr::Bool(false, tok.span))
        }
        TokKind::Str => {
            t.advance();
            Ok(Expr::Str(unescape(text(src, tok.span)), tok.span))
        }
        TokKind::Ident => {
            t.advance();
            Ok(Expr::Var(text(src, tok.span).to_string(), tok.span))
        }
        TokKind::LParen => {
            t.advance();
            let mut inner = expr(src, t)?;
            let rparen = expect(src, t, TokKind::RParen, "`)`")?;
            inner.set_span(Span::new(tok.span.start, rparen.end));
            Ok(inner)
        }
        _ => Err(unexpected(src, t, "an expression")),
    }
}

fn ident(src: &str, t: &mut Toks<'_>, what: &str) -> PResult<(String, Span)> {
    match t.eat(|x| x.kind == TokKind::Ident) {
        Some(tok) => Ok((text(src, tok.span).to_string(), tok.span)),
        None => Err(unexpected(src, t, what)),
    }
}

fn expect(src: &str, t: &mut Toks<'_>, kind: TokKind, what: &str) -> PResult<Span> {
    match t.eat(|x| x.kind == kind) {
        Some(tok) => Ok(tok.span),
        None => Err(unexpected(src, t, what)),
    }
}

fn unexpected(src: &str, t: &Toks<'_>, what: &str) -> PErr {
    let tok = t.peek();
    PErr(Diag::at_code(
        codes::UNEXPECTED_TOKEN,
        format!("expected {what}, found {}", describe(src, tok)),
        tok.span,
    ))
}

fn describe(src: &str, tok: &Token) -> String {
    match tok.kind {
        TokKind::Eof => "end of input".to_string(),
        _ => format!("`{}`", text(src, tok.span)),
    }
}

fn text(src: &str, sp: Span) -> &str {
    &src[sp.start..sp.end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_app_parses_with_stable_button_ids() {
        let p = parse(
            "state count = 0;
             state name = \"world\";
             label \"Counter\";
             row {
               button \"-\" { count = count - 1; }
               label count;
               button \"+\" { count = count + 1; }
             }
             input name;
             if count > 10 { label \"big!\"; button \"reset\" { count = 0; } }",
        )
        .unwrap();
        assert_eq!(p.states.len(), 2);
        assert_eq!(p.states[1].init, Lit::Str("world".to_string()));
        assert_eq!(p.widgets.len(), 4);
        // Parse-ordered button ids (0,1 in the row, 2 in the if) are pinned
        // end-to-end by the lib tests that click them.
    }

    #[test]
    fn state_after_widgets_and_bad_shapes_are_coded() {
        let e = parse("label 1; state x = 0;").unwrap_err();
        assert_eq!(e.code, Some(codes::UNEXPECTED_TOKEN));
        assert!(e.message.contains("before the first widget"), "{e}");
        for src in [
            "state x = y;",     // init must be a literal
            "state x = -true;", // negation is for ints only
            "button { }",       // button needs its label
            "label 1",          // missing `;`
            "row label 1; }",   // missing `{`
            "widget",           // not a widget
        ] {
            assert_eq!(
                parse(src).unwrap_err().code,
                Some(codes::UNEXPECTED_TOKEN),
                "{src}"
            );
        }
        // Negative int literals in state inits DO parse.
        assert!(matches!(
            parse("state x = -5;").unwrap().states[0].init,
            Lit::Int(-5)
        ));
    }

    #[test]
    fn nesting_and_operator_chains_charge_the_guard() {
        // Deep widget nesting trips the cap — never a stack overflow.
        let deep = format!("{}label 1;{}", "row {".repeat(200), "}".repeat(200));
        assert_eq!(parse(&deep).unwrap_err().code, Some(codes::TOO_DEEP));
        // Long flat operator chains charge the guard per fold (litelite's
        // prooflite lesson — the AST spine is what the evaluator must walk).
        let chain = format!("label {}0;", "1+".repeat(500));
        assert_eq!(parse(&chain).unwrap_err().code, Some(codes::TOO_DEEP));
        let ok = format!("label {}0;", "1+".repeat(40));
        assert!(parse(&ok).is_ok());
    }
}

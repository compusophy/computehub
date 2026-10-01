//! The applang parser: tokens to the app AST on the `lang::parse` cursor.
//! Widgets, statements, expressions and every binary fold enter the depth
//! guard, so it bounds AST depth (nesting and operator spines), not just
//! parser recursion. Every failure is a coded, spanned `Diag`.

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
    pub fn sym(self) -> &'static str {
        ["||", "&&", "==", "!=", "<", "<=", ">", ">=", "+", "-", "*", "/", "%"][self as usize]
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
            Expr::Int(_, sp) | Expr::Bool(_, sp) | Expr::Str(_, sp) | Expr::Var(_, sp) => *sp,
            Expr::Unary(_, _, sp) | Expr::Binary(_, _, _, sp) => *sp,
        }
    }

    fn span_mut(&mut self) -> &mut Span {
        match self {
            Expr::Int(_, sp) | Expr::Bool(_, sp) | Expr::Str(_, sp) | Expr::Var(_, sp) => sp,
            Expr::Unary(_, _, sp) | Expr::Binary(_, _, _, sp) => sp,
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

/// A state's initial value: a literal, which fixes the state's type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Lit { Int(i64), Bool(bool), Str(String) }

#[derive(Debug)]
#[rustfmt::skip]
pub struct StateDecl { pub name: String, pub name_span: Span, pub init: Lit }

/// The widget tree. Button ids count from 0 in parse order, visible or not.
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

/// A compiled app: proof that the source parses and type-checks, since only
/// [`crate::compile`] makes one. Its fields stay private, so no other crate
/// can forge a tree the depth guard and the checker never saw:
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
    /// The state declarations, in source order.
    pub fn states(&self) -> &[StateDecl] {
        &self.states
    }

    /// The top-level widgets, in source order.
    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }
}

/// Lexes and parses `src`. All `state` declarations come before the first
/// widget. Crate-private: only checked programs leave the crate.
pub(crate) fn parse(src: &str) -> Result<Program, Diag> {
    let toks = lex(src)?;
    let mut cur = TokCursor::new(&toks);
    let mut states = Vec::new();
    while cur.peek().kind == TokKind::State {
        states.push(state_decl(src, &mut cur).map_err(|e| e.0)?);
    }
    let (mut widgets, mut next_id) = (Vec::new(), 0u32);
    while !cur.at_last() {
        if cur.peek().kind == TokKind::State {
            let msg = "`state` declarations must come before the first widget";
            return Err(Diag::at_code(codes::UNEXPECTED_TOKEN, msg, cur.peek().span));
        }
        widgets.push(widget(src, &mut cur, &mut next_id).map_err(|e| e.0)?);
    }
    Ok(Program { states, widgets })
}

struct PErr(Diag);

impl From<Span> for PErr {
    fn from(sp: Span) -> Self {
        let msg =
            format!("source nests deeper than the parser allows (depth cap {DEFAULT_MAX_DEPTH})");
        PErr(Diag::at_code(codes::TOO_DEEP, msg, sp))
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
    Ok(StateDecl { name, name_span, init })
}

fn widget(src: &str, t: &mut Toks<'_>, next_id: &mut u32) -> PResult<Widget> {
    t.guarded(|t| {
        let tok = *t.peek();
        let span = |end: usize| Span::new(tok.span.start, end);
        match tok.kind {
            TokKind::Label => {
                t.advance();
                let value = expr(src, t)?;
                let end = expect(src, t, TokKind::Semi, "`;`")?;
                Ok(Widget::Label { value, span: span(end.end) })
            }
            TokKind::Button => {
                t.advance();
                let text_tok = t
                    .eat(|x| x.kind == TokKind::Str)
                    .ok_or_else(|| unexpected(src, t, "a string button label"))?;
                let (body, end) = braced(src, t, &mut stmt)?;
                let (text, id) = (unescape(text(src, text_tok.span)), *next_id);
                *next_id += 1;
                Ok(Widget::Button { text, id, body, span: span(end.end) })
            }
            TokKind::Input => {
                t.advance();
                let (state, state_span) = ident(src, t, "a state name")?;
                let end = expect(src, t, TokKind::Semi, "`;`")?;
                Ok(Widget::Input { state, state_span, span: span(end.end) })
            }
            TokKind::Row | TokKind::Col => {
                t.advance();
                let (children, end) = braced(src, t, &mut |s, t| widget(s, t, next_id))?;
                let span = span(end.end);
                Ok(if tok.kind == TokKind::Row {
                    Widget::Row { children, span }
                } else {
                    Widget::Col { children, span }
                })
            }
            TokKind::If => {
                let (arms, els, end) = if_chain(src, t, &mut |s, t| widget(s, t, next_id))?;
                Ok(Widget::If { arms, els, span: span(end) })
            }
            _ => Err(unexpected(src, t, "a widget (label, button, input, row, col, if)")),
        }
    })
}

type Item<'a, T> = &'a mut dyn FnMut(&str, &mut Toks<'_>) -> PResult<T>;

/// `{ ITEM* }`, for widgets and statements alike.
fn braced<T>(src: &str, t: &mut Toks<'_>, item: Item<'_, T>) -> PResult<(Vec<T>, Span)> {
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

/// `if C { } else if C { } else { }`, iteratively: flat in the AST and in
/// guard depth. Returns the arms, the else body and the end offset.
#[allow(clippy::type_complexity)]
fn if_chain<T>(
    src: &str,
    t: &mut Toks<'_>,
    item: Item<'_, T>,
) -> PResult<(Vec<(Expr, Vec<T>)>, Vec<T>, usize)> {
    let mut arms = Vec::new();
    loop {
        t.advance(); // the `if`
        let cond = expr(src, t)?;
        let (body, end) = braced(src, t, item)?;
        arms.push((cond, body));
        if t.eat(|x| x.kind == TokKind::Else).is_none() {
            return Ok((arms, Vec::new(), end.end));
        }
        if t.peek().kind != TokKind::If {
            let (els, end) = braced(src, t, item)?;
            return Ok((arms, els, end.end));
        }
    }
}

fn stmt(src: &str, t: &mut Toks<'_>) -> PResult<Stmt> {
    t.guarded(|t| {
        let tok = *t.peek();
        let span = |end: usize| Span::new(tok.span.start, end);
        match tok.kind {
            // `let x = e;` is `let` and then an assignment's shape.
            TokKind::Let | TokKind::Ident => {
                let is_let = t.eat(|x| x.kind == TokKind::Let).is_some();
                let (name, name_span) = ident(src, t, "a variable name")?;
                expect(src, t, TokKind::Assign, "`=`")?;
                let value = expr(src, t)?;
                let span = span(expect(src, t, TokKind::Semi, "`;`")?.end);
                Ok(if is_let {
                    Stmt::Let { name, value, span }
                } else {
                    Stmt::Assign { name, name_span, value, span }
                })
            }
            TokKind::If => {
                let (arms, els, end) = if_chain(src, t, &mut stmt)?;
                Ok(Stmt::If { arms, els, span: span(end) })
            }
            TokKind::Repeat => {
                t.advance();
                let count = expr(src, t)?;
                let (body, end) = braced(src, t, &mut stmt)?;
                Ok(Stmt::Repeat { count, body, span: span(end.end) })
            }
            _ => Err(unexpected(src, t, "a statement")),
        }
    })
}

/// Binary operators by precedence, loosest first.
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
    &[(TokKind::Star, BinOp::Mul), (TokKind::Slash, BinOp::Div), (TokKind::Percent, BinOp::Rem)],
];

fn expr(src: &str, t: &mut Toks<'_>) -> PResult<Expr> {
    t.guarded(|t| binary(src, t, 0))
}

/// One precedence level. Each fold deepens the AST's left spine, which eval
/// and drop glue recurse through, so each charges one guard entry, all
/// released when the level completes.
fn binary(src: &str, t: &mut Toks<'_>, level: usize) -> PResult<Expr> {
    if level == LADDER.len() {
        return unary(src, t);
    }
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
    let tok = *t.advance();
    let sp = tok.span;
    Ok(match tok.kind {
        TokKind::Int(v) => Expr::Int(v, sp),
        TokKind::True | TokKind::False => Expr::Bool(tok.kind == TokKind::True, sp),
        TokKind::Str => Expr::Str(unescape(text(src, sp)), sp),
        TokKind::Ident => Expr::Var(text(src, sp).to_string(), sp),
        TokKind::LParen => {
            // The parens join the inner span, so diagnostics cover them too.
            let mut inner = expr(src, t)?;
            let end = expect(src, t, TokKind::RParen, "`)`")?.end;
            *inner.span_mut() = Span::new(sp.start, end);
            inner
        }
        _ => return Err(unexpected_tok(src, &tok, "an expression")),
    })
}

fn ident(src: &str, t: &mut Toks<'_>, what: &str) -> PResult<(String, Span)> {
    let sp = expect(src, t, TokKind::Ident, what)?;
    Ok((text(src, sp).to_string(), sp))
}

fn expect(src: &str, t: &mut Toks<'_>, kind: TokKind, what: &str) -> PResult<Span> {
    match t.eat(|x| x.kind == kind) {
        Some(tok) => Ok(tok.span),
        None => Err(unexpected(src, t, what)),
    }
}

fn unexpected(src: &str, t: &Toks<'_>, what: &str) -> PErr {
    unexpected_tok(src, t.peek(), what)
}

fn unexpected_tok(src: &str, tok: &Token, what: &str) -> PErr {
    let found = match tok.kind {
        TokKind::Eof => "end of input".to_string(),
        _ => format!("`{}`", text(src, tok.span)),
    };
    let msg = format!("expected {what}, found {found}");
    PErr(Diag::at_code(codes::UNEXPECTED_TOKEN, msg, tok.span))
}

fn text(src: &str, sp: Span) -> &str {
    &src[sp.start..sp.end]
}

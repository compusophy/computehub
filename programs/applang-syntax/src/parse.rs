//! The applang parser: tokens to the app AST on the `lang::parse` cursor. Each
//! widget, statement, expression and binary fold enters the depth guard, so
//! it bounds AST depth (nesting and operator spines), not just recursion.
//! Names stay unresolved ([`Slot::Local`] `u32::MAX`, [`Target::Unresolved`]) until the
//! checker fills them in.

use lang::parse::{DEFAULT_MAX_DEPTH, TokCursor};
use lang::{Diag, Span};

use crate::codes;
use crate::lex::{TokKind, Token, lex, unescape};

/// The most items a list holds.
pub const MAX_ITEMS: usize = 4096;

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

/// applang's static types: three scalars and a list of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Type { Int, Bool, Str, Ints, Bools, Strs }

impl Type {
    pub fn name(self) -> &'static str {
        ["int", "bool", "string", "a list of int", "a list of bool", "a list of string"]
            [self as usize]
    }

    /// The list of this scalar, or the scalar of this list.
    pub fn list(self) -> Option<Type> {
        [Some(Type::Ints), Some(Type::Bools), Some(Type::Strs), None, None, None][self as usize]
    }

    pub fn elem(self) -> Option<Type> {
        [None, None, None, Some(Type::Int), Some(Type::Bool), Some(Type::Str)][self as usize]
    }
}

/// Where a name lives: a state by declaration order, or a local by its place in the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    State(u32),
    Local(u32),
}

/// A name in an expression or as a target.
#[derive(Debug, Clone)]
pub struct Var {
    pub name: String,
    pub span: Span,
    pub slot: Slot,
}

/// The built-in functions: values, then the list changes (statements only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Builtin { Len, Min, Max, Abs, Random, Parse, Push, Insert, Remove, Clear }

pub const BUILTINS: [&str; 10] =
    ["len", "min", "max", "abs", "random", "parse", "push", "insert", "remove", "clear"];

/// What a call calls, once checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Unresolved,
    Builtin(Builtin),
    Fn(u32),
}

#[derive(Debug, Clone)]
pub struct Call {
    pub name: String,
    pub args: Vec<Expr>,
    pub span: Span,
    pub target: Target,
}

#[derive(Debug, Clone)]
#[rustfmt::skip]
pub enum Expr {
    Int(i64, Span), Bool(bool, Span), Str(String, Span), Var(Var),
    Index(Var, Box<Expr>, Span), Call(Call),
    /// `[a, b, c]`, and `[value; count]`.
    List(Vec<Expr>, Span), Fill(Box<Expr>, Box<Expr>, Span),
    Unary(UnOp, Box<Expr>, Span), Binary(BinOp, Box<Expr>, Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, sp) | Expr::Bool(_, sp) | Expr::Str(_, sp) => *sp,
            Expr::Var(v) => v.span,
            Expr::Call(c) => c.span,
            Expr::Index(_, _, sp) | Expr::List(_, sp) | Expr::Fill(_, _, sp) => *sp,
            Expr::Unary(_, _, sp) | Expr::Binary(_, _, _, sp) => *sp,
        }
    }

    fn span_mut(&mut self) -> &mut Span {
        match self {
            Expr::Int(_, sp) | Expr::Bool(_, sp) | Expr::Str(_, sp) => sp,
            Expr::Var(v) => &mut v.span,
            Expr::Call(c) => &mut c.span,
            Expr::Index(_, _, sp) | Expr::List(_, sp) | Expr::Fill(_, _, sp) => sp,
            Expr::Unary(_, _, sp) | Expr::Binary(_, _, _, sp) => sp,
        }
    }
}

#[derive(Debug, Clone)]
#[rustfmt::skip]
pub enum Stmt {
    Let { name: String, value: Expr, span: Span },
    Assign { target: Var, value: Expr, span: Span },
    SetIndex { target: Var, index: Expr, value: Expr, span: Span },
    If { arms: Vec<(Expr, Vec<Stmt>)>, els: Vec<Stmt>, span: Span },
    Repeat { count: Expr, body: Vec<Stmt>, span: Span },
    For { var: String, from: Expr, to: Expr, body: Vec<Stmt>, span: Span },
    Return { value: Option<Expr>, span: Span },
    Call(Call),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::SetIndex { span, .. }
            | Stmt::If { span, .. }
            | Stmt::Repeat { span, .. }
            | Stmt::For { span, .. }
            | Stmt::Return { span, .. } => *span,
            Stmt::Call(c) => c.span,
        }
    }
}

/// A literal: a state's initial value (which fixes its type) or a saved value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Lit { Int(i64), Bool(bool), Str(String), List(Type, Vec<Lit>) }

impl Lit {
    pub fn ty(&self) -> Type {
        match self {
            Lit::Int(_) => Type::Int,
            Lit::Bool(_) => Type::Bool,
            Lit::Str(_) => Type::Str,
            Lit::List(t, _) => *t,
        }
    }
}

#[derive(Debug)]
#[rustfmt::skip]
pub struct StateDecl { pub name: String, pub name_span: Span, pub init: Lit, pub saved: bool }

/// A function: no result type is a procedure (it may change state); `pure` once checked says
/// it changes nothing, so a render may call it.
#[derive(Debug)]
pub struct FnDecl {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<(String, Type)>,
    pub ret: Option<Type>,
    pub body: Vec<Stmt>,
    pub span: Span,
    pub pure: bool,
}

/// `every N { }` and `on key "k" { }`.
#[derive(Debug)]
pub struct Every {
    pub interval: Expr,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug)]
pub struct OnKey {
    pub key: String,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// A button's or grid's handler: its id (parse order, from 0) and statements.
#[derive(Debug)]
pub struct Handler {
    pub id: u32,
    pub body: Vec<Stmt>,
}

/// The widget tree.
#[derive(Debug)]
#[rustfmt::skip]
pub enum Widget {
    Label { value: Expr, span: Span },
    Button { text: Expr, handler: Handler, span: Span },
    Input { state: Var, span: Span },
    Row { children: Vec<Widget>, span: Span },
    Col { children: Vec<Widget>, span: Span },
    If { arms: Vec<(Expr, Vec<Widget>)>, els: Vec<Widget>, span: Span },
    For { var: String, from: Expr, to: Expr, body: Vec<Widget>, span: Span },
    Grid { cols: Expr, cells: Expr, texts: Option<Expr>, handler: Option<Handler>, span: Span },
}

impl Widget {
    pub fn span(&self) -> Span {
        match self {
            Widget::Label { span, .. }
            | Widget::Button { span, .. }
            | Widget::Input { span, .. }
            | Widget::Row { span, .. }
            | Widget::Col { span, .. }
            | Widget::If { span, .. }
            | Widget::For { span, .. }
            | Widget::Grid { span, .. } => *span,
        }
    }
}

/// A compiled app: only [`crate::compile`] makes one, and its fields stay
/// private, so no crate can forge a tree the guard and checker never saw:
///
/// ```compile_fail,E0451
/// let _ = applang_syntax::Program { states: Vec::new(), widgets: Vec::new() };
/// ```
#[derive(Debug)]
pub struct Program {
    pub(crate) states: Vec<StateDecl>,
    pub(crate) fns: Vec<FnDecl>,
    pub(crate) everys: Vec<Every>,
    pub(crate) keys: Vec<OnKey>,
    pub(crate) widgets: Vec<Widget>,
}

impl Program {
    /// The state declarations, in source order.
    pub fn states(&self) -> &[StateDecl] {
        &self.states
    }

    /// The functions, in source order.
    pub fn fns(&self) -> &[FnDecl] {
        &self.fns
    }

    /// The `every` blocks, then the `on key` handlers, in source order.
    pub fn everys(&self) -> &[Every] {
        &self.everys
    }

    pub fn keys(&self) -> &[OnKey] {
        &self.keys
    }

    /// The top-level widgets, in source order.
    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }
}

/// Lexes and parses `src` (states first); private: only checked programs leave.
pub(crate) fn parse(src: &str) -> Result<Program, Diag> {
    let toks = lex(src)?;
    let mut t = TokCursor::new(&toks);
    let mut p = Program {
        states: Vec::new(),
        fns: Vec::new(),
        everys: Vec::new(),
        keys: Vec::new(),
        widgets: Vec::new(),
    };
    let (mut ids, mut first) = (0u32, true);
    while !t.at_last() {
        // A stray `;` (after a closing `}`, or doubled) is nothing.
        if t.eat(|x| x.kind == TokKind::Semi).is_some() {
            continue;
        }
        first &= t.peek().kind == TokKind::State || word(src, t.peek(), "saved");
        item(src, &mut t, &mut p, (&mut ids, first)).map_err(|e| e.0)?;
    }
    Ok(p)
}

/// One top-level item into `p`: a state (only among the `first`), a function, a handler or a
/// widget (its handler ids from `ids`).
fn item(
    src: &str,
    t: &mut Toks<'_>,
    p: &mut Program,
    (ids, first): (&mut u32, bool),
) -> PResult<()> {
    let tok = *t.peek();
    let span = |end: Span| Span::new(tok.span.start, end.end);
    if tok.kind == TokKind::State || word(src, &tok, "saved") {
        if !first {
            let msg = "`state` declarations must come first, before any widget, fn or handler";
            return Err(PErr(Diag::at_code(codes::UNEXPECTED_TOKEN, msg, tok.span)));
        }
        p.states.push(state_decl(src, t)?);
    } else if tok.kind == TokKind::Fn {
        p.fns.push(fn_decl(src, t)?);
    } else if word(src, &tok, "every") {
        t.advance();
        let interval = expr(src, t)?;
        let (body, end) = braced(src, t, &mut stmt)?;
        p.everys.push(Every { interval, body, span: span(end) });
    } else if word(src, &tok, "on") {
        t.advance();
        if !word(src, t.peek(), "key") {
            return Err(unexpected(src, t.peek(), "`key` (as in on key \"left\" { })"));
        }
        t.advance();
        let k = expect(src, t, TokKind::Str, "a key name in quotes")?;
        let key = unescape(text(src, k));
        let (body, end) = braced(src, t, &mut stmt)?;
        p.keys.push(OnKey { key, body, span: span(end) });
    } else {
        p.widgets.push(widget(src, t, ids)?);
    }
    Ok(())
}

/// `name = literal;` lines (a saved state's file), in order.
pub fn literals(src: &str) -> Result<Vec<(String, Lit)>, Diag> {
    let toks = lex(src)?;
    let mut t = TokCursor::new(&toks);
    let mut out = Vec::new();
    while !t.at_last() {
        let one = |t: &mut Toks<'_>| -> PResult<(String, Lit)> {
            let (name, _) = ident(src, t, "a state name")?;
            expect(src, t, TokKind::Assign, "`=`")?;
            let lit = literal(src, t, true)?;
            expect(src, t, TokKind::Semi, "`;`")?;
            Ok((name, lit))
        };
        out.push(one(&mut t).map_err(|e| e.0)?);
    }
    Ok(out)
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

/// Whether `tok` is the name `w`: a word only where a name could not be.
fn word(src: &str, tok: &Token, w: &str) -> bool {
    tok.kind == TokKind::Ident && text(src, tok.span) == w
}

fn state_decl(src: &str, t: &mut Toks<'_>) -> PResult<StateDecl> {
    let saved = word(src, t.peek(), "saved");
    if saved {
        t.advance();
    }
    expect(src, t, TokKind::State, "`state`")?;
    let (name, name_span) = ident(src, t, "a state name")?;
    expect(src, t, TokKind::Assign, "`=`")?;
    let init = literal(src, t, true)?;
    expect(src, t, TokKind::Semi, "`;`")?;
    Ok(StateDecl { name, name_span, init, saved })
}

/// An int (perhaps negative), bool or string literal; with `list`, also a list of one of them:
/// `[1, 2]` or `[0; 200]`.
fn literal(src: &str, t: &mut Toks<'_>, list: bool) -> PResult<Lit> {
    let open = *t.peek();
    if list && t.eat(|x| x.kind == TokKind::LBracket).is_some() {
        let first = literal(src, t, false)?;
        let mut items = vec![first.clone()];
        if t.eat(|x| x.kind == TokKind::Semi).is_some() {
            let n = t.peek();
            let TokKind::Int(count) = n.kind else {
                return Err(unexpected(src, n, "a whole number of items"));
            };
            let count = usize::try_from(count).unwrap_or(usize::MAX);
            if count > MAX_ITEMS {
                let msg = format!("a list holds at most {MAX_ITEMS} items");
                return Err(PErr(Diag::at_code(codes::LIST_FULL, msg, n.span)));
            }
            t.advance();
            items = vec![first.clone(); count];
        } else {
            while t.eat(|x| x.kind == TokKind::Comma).is_some() {
                if t.peek().kind == TokKind::RBracket {
                    break;
                }
                let tok = *t.peek();
                let item = literal(src, t, false)?;
                let (code, msg) = match items.len() {
                    MAX_ITEMS.. => (codes::LIST_FULL, "a list holds at most 4096 items"),
                    _ if item.ty() != first.ty() => {
                        (codes::TYPE_MISMATCH, "a list's items all have one type")
                    }
                    _ => {
                        items.push(item);
                        continue;
                    }
                };
                return Err(PErr(Diag::at_code(code, msg, tok.span)));
            }
        }
        expect(src, t, TokKind::RBracket, "`]`")?;
        let ty = first.ty().list().ok_or_else(|| unexpected(src, &open, "a list of literals"))?;
        return Ok(Lit::List(ty, items));
    }
    let neg = t.eat(|x| x.kind == TokKind::Minus).is_some();
    let tok = *t.peek();
    #[rustfmt::skip]
    let lit = match (tok.kind, neg) {
        (TokKind::Int(v), _) => Lit::Int(if neg { -v } else { v }),
        (TokKind::True, false) => Lit::Bool(true),
        (TokKind::False, false) => Lit::Bool(false),
        (TokKind::Str, false) => Lit::Str(unescape(text(src, tok.span))),
        _ => return Err(unexpected(src, &tok, "a literal (int, bool, string or list)")),
    };
    t.advance();
    Ok(lit)
}

fn ty(src: &str, t: &mut Toks<'_>) -> PResult<Type> {
    let tok = *t.peek();
    let found = match (tok.kind == TokKind::Ident).then(|| text(src, tok.span)) {
        Some("int") => Type::Int,
        Some("bool") => Type::Bool,
        Some("string") => Type::Str,
        _ => return Err(unexpected(src, &tok, "a type (int, bool or string)")),
    };
    t.advance();
    Ok(found)
}

fn fn_decl(src: &str, t: &mut Toks<'_>) -> PResult<FnDecl> {
    let start = t.advance().span.start;
    let (name, name_span) = ident(src, t, "a function name")?;
    expect(src, t, TokKind::LParen, "`(`")?;
    let mut params = Vec::new();
    while t.eat(|x| x.kind == TokKind::RParen).is_none() {
        if !params.is_empty() {
            expect(src, t, TokKind::Comma, "`,` or `)`")?;
        }
        let (p, _) = ident(src, t, "a parameter name")?;
        expect(src, t, TokKind::Colon, "`:` and the parameter's type")?;
        params.push((p, ty(src, t)?));
    }
    let ret = match t.eat(|x| x.kind == TokKind::Arrow) {
        Some(_) => Some(ty(src, t)?),
        None => None,
    };
    let (body, end) = braced(src, t, &mut stmt)?;
    let span = Span::new(start, end.end);
    Ok(FnDecl { name, name_span, params, ret, body, span, pure: false })
}

/// A handler's block, its id the next of `ids`.
fn handler(src: &str, t: &mut Toks<'_>, ids: &mut u32) -> PResult<(Handler, Span)> {
    let (body, end) = braced(src, t, &mut stmt)?;
    *ids += 1;
    Ok((Handler { id: *ids - 1, body }, end))
}

fn widget(src: &str, t: &mut Toks<'_>, ids: &mut u32) -> PResult<Widget> {
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
                let text = expr(src, t)?;
                let (handler, end) = handler(src, t, ids)?;
                Ok(Widget::Button { text, handler, span: span(end.end) })
            }
            TokKind::Input => {
                t.advance();
                let (name, sp) = ident(src, t, "a state name")?;
                let end = expect(src, t, TokKind::Semi, "`;`")?;
                let state = Var { name, span: sp, slot: Slot::Local(u32::MAX) };
                Ok(Widget::Input { state, span: span(end.end) })
            }
            TokKind::Row | TokKind::Col => {
                t.advance();
                let (children, end) = braced(src, t, &mut |s, t| widget(s, t, ids))?;
                let span = span(end.end);
                Ok(if tok.kind == TokKind::Row {
                    Widget::Row { children, span }
                } else {
                    Widget::Col { children, span }
                })
            }
            TokKind::If => {
                let (arms, els, end) = if_chain(src, t, &mut |s, t| widget(s, t, ids))?;
                Ok(Widget::If { arms, els, span: span(end) })
            }
            TokKind::For => {
                let (var, from, to) = for_head(src, t)?;
                let (body, end) = braced(src, t, &mut |s, t| widget(s, t, ids))?;
                Ok(Widget::For { var, from, to, body, span: span(end.end) })
            }
            _ if word(src, &tok, "grid") => {
                t.advance();
                let cols = expr(src, t)?;
                expect(src, t, TokKind::Comma, "`,` and the grid's list")?;
                let cells = expr(src, t)?;
                let texts = match t.eat(|x| x.kind == TokKind::Comma) {
                    Some(_) => Some(expr(src, t)?),
                    None => None,
                };
                let (handler, end) = match t.eat(|x| x.kind == TokKind::Semi) {
                    Some(semi) => (None, semi.span),
                    None => handler(src, t, ids).map(|(h, end)| (Some(h), end))?,
                };
                Ok(Widget::Grid { cols, cells, texts, handler, span: span(end.end) })
            }
            _ => Err(unexpected(
                src,
                t.peek(),
                "a widget (label, button, input, row, col, if, for, grid), fn, every or on key",
            )),
        }
    })
}

/// `for NAME in FROM..TO`, up to its block.
fn for_head(src: &str, t: &mut Toks<'_>) -> PResult<(String, Expr, Expr)> {
    t.advance();
    let (var, _) = ident(src, t, "a loop variable")?;
    expect(src, t, TokKind::In, "`in`")?;
    let from = expr(src, t)?;
    expect(src, t, TokKind::DotDot, "`..` (as in 0..10)")?;
    Ok((var, from, expr(src, t)?))
}

type Item<'a, T> = &'a mut dyn FnMut(&str, &mut Toks<'_>) -> PResult<T>;

/// `{ ITEM* }`, for widgets and statements alike.
fn braced<T>(src: &str, t: &mut Toks<'_>, item: Item<'_, T>) -> PResult<(Vec<T>, Span)> {
    expect(src, t, TokKind::LBrace, "`{`")?;
    let mut items = Vec::new();
    while t.peek().kind != TokKind::RBrace {
        if t.at_last() {
            return Err(unexpected(src, t.peek(), "`}`"));
        }
        if t.eat(|x| x.kind == TokKind::Semi).is_none() {
            items.push(item(src, t)?);
        }
    }
    let end = expect(src, t, TokKind::RBrace, "`}`")?;
    Ok((items, end))
}

/// An if chain's arms, its else body and its end offset.
type Chain<T> = (Vec<(Expr, Vec<T>)>, Vec<T>, usize);

/// `if C { } else if C { } else { }`, iteratively: flat in the AST and in guard depth.
fn if_chain<T>(src: &str, t: &mut Toks<'_>, item: Item<'_, T>) -> PResult<Chain<T>> {
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
        let semi = |t: &mut Toks<'_>| expect(src, t, TokKind::Semi, "`;`").map(|s| span(s.end));
        match tok.kind {
            TokKind::Let => {
                t.advance();
                let (name, _) = ident(src, t, "a variable name")?;
                expect(src, t, TokKind::Assign, "`=`")?;
                let value = expr(src, t)?;
                Ok(Stmt::Let { name, value, span: semi(t)? })
            }
            TokKind::Ident => {
                t.advance();
                let target = Var { name: text(src, tok.span).into(), span: tok.span, slot: NONE };
                match t.peek().kind {
                    TokKind::LParen => {
                        let call = call(src, t, target)?;
                        semi(t)?;
                        Ok(Stmt::Call(call))
                    }
                    TokKind::LBracket => {
                        t.advance();
                        let index = expr(src, t)?;
                        let end = expect(src, t, TokKind::RBracket, "`]`")?.end;
                        let read =
                            || Expr::Index(target.clone(), Box::new(index.clone()), span(end));
                        let value = assigned(src, t, read)?;
                        Ok(Stmt::SetIndex { target, index, value, span: semi(t)? })
                    }
                    _ => {
                        let value = assigned(src, t, || Expr::Var(target.clone()))?;
                        Ok(Stmt::Assign { target, value, span: semi(t)? })
                    }
                }
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
            TokKind::For => {
                let (var, from, to) = for_head(src, t)?;
                let (body, end) = braced(src, t, &mut stmt)?;
                Ok(Stmt::For { var, from, to, body, span: span(end.end) })
            }
            TokKind::Return => {
                t.advance();
                let value = match t.peek().kind {
                    TokKind::Semi => None,
                    _ => Some(expr(src, t)?),
                };
                Ok(Stmt::Return { value, span: semi(t)? })
            }
            _ => Err(unexpected(src, t.peek(), "a statement")),
        }
    })
}

/// An unresolved slot.
const NONE: Slot = Slot::Local(u32::MAX);

/// What an assignment stores: `= v`, or `+= v` / `-= v` as the target `read` plus or minus `v`.
fn assigned(src: &str, t: &mut Toks<'_>, read: impl Fn() -> Expr) -> PResult<Expr> {
    let op = match t.peek().kind {
        TokKind::Assign => None,
        TokKind::PlusEq => Some(BinOp::Add),
        TokKind::MinusEq => Some(BinOp::Sub),
        _ => return Err(unexpected(src, t.peek(), "`=` (or `+=`, `-=`)")),
    };
    t.advance();
    let value = expr(src, t)?;
    Ok(match op {
        None => value,
        Some(op) => {
            let lhs = read();
            let span = Span::new(lhs.span().start, value.span().end);
            Expr::Binary(op, Box::new(lhs), Box::new(value), span)
        }
    })
}

/// Binary operators by precedence, loosest first.
#[rustfmt::skip]
const LADDER: &[&[(TokKind, BinOp)]] = &[
    &[(TokKind::OrOr, BinOp::Or)],
    &[(TokKind::AndAnd, BinOp::And)],
    &[(TokKind::EqEq, BinOp::Eq), (TokKind::BangEq, BinOp::Ne)],
    &[(TokKind::Lt, BinOp::Lt), (TokKind::LtEq, BinOp::Le),
      (TokKind::Gt, BinOp::Gt), (TokKind::GtEq, BinOp::Ge)],
    &[(TokKind::Plus, BinOp::Add), (TokKind::Minus, BinOp::Sub)],
    &[(TokKind::Star, BinOp::Mul), (TokKind::Slash, BinOp::Div), (TokKind::Percent, BinOp::Rem)],
];

fn expr(src: &str, t: &mut Toks<'_>) -> PResult<Expr> {
    t.guarded(|t| binary(src, t, 0))
}

/// One precedence level. Each fold deepens the left spine eval and drop glue
/// recurse through, so it charges a guard entry until the level completes.
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
        TokKind::Ident => {
            let var = Var { name: text(src, sp).to_string(), span: sp, slot: NONE };
            match t.peek().kind {
                TokKind::LParen => Expr::Call(call(src, t, var)?),
                TokKind::LBracket => {
                    t.advance();
                    let index = expr(src, t)?;
                    let end = expect(src, t, TokKind::RBracket, "`]`")?.end;
                    Expr::Index(var, Box::new(index), Span::new(sp.start, end))
                }
                _ => Expr::Var(var),
            }
        }
        TokKind::LBracket => {
            // `[]` has no type: the checker says so.
            if let Some(close) = t.eat(|x| x.kind == TokKind::RBracket) {
                return Ok(Expr::List(Vec::new(), Span::new(sp.start, close.span.end)));
            }
            let first = expr(src, t)?;
            if t.eat(|x| x.kind == TokKind::Semi).is_some() {
                let count = expr(src, t)?;
                let end = expect(src, t, TokKind::RBracket, "`]`")?.end;
                return Ok(Expr::Fill(Box::new(first), Box::new(count), Span::new(sp.start, end)));
            }
            let mut items = vec![first];
            while t.eat(|x| x.kind == TokKind::Comma).is_some() {
                if t.peek().kind == TokKind::RBracket {
                    break;
                }
                items.push(expr(src, t)?);
            }
            let end = expect(src, t, TokKind::RBracket, "`]`")?.end;
            Expr::List(items, Span::new(sp.start, end))
        }
        TokKind::LParen => {
            // The parens join the inner span, so diagnostics cover them too.
            let mut inner = expr(src, t)?;
            let end = expect(src, t, TokKind::RParen, "`)`")?.end;
            *inner.span_mut() = Span::new(sp.start, end);
            inner
        }
        _ => return Err(unexpected(src, &tok, "an expression")),
    })
}

/// `name(args)`, the `(` next.
fn call(src: &str, t: &mut Toks<'_>, name: Var) -> PResult<Call> {
    t.advance();
    let mut args = Vec::new();
    let end = loop {
        if let Some(close) = t.eat(|x| x.kind == TokKind::RParen) {
            break close.span.end;
        }
        if !args.is_empty() {
            expect(src, t, TokKind::Comma, "`,` or `)`")?;
        }
        args.push(expr(src, t)?);
    };
    let span = Span::new(name.span.start, end);
    Ok(Call { name: name.name, args, span, target: Target::Unresolved })
}

fn ident(src: &str, t: &mut Toks<'_>, what: &str) -> PResult<(String, Span)> {
    let sp = expect(src, t, TokKind::Ident, what)?;
    Ok((text(src, sp).to_string(), sp))
}

fn expect(src: &str, t: &mut Toks<'_>, kind: TokKind, what: &str) -> PResult<Span> {
    match t.eat(|x| x.kind == kind) {
        Some(tok) => Ok(tok.span),
        None => Err(unexpected(src, t.peek(), what)),
    }
}

fn unexpected(src: &str, tok: &Token, what: &str) -> PErr {
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

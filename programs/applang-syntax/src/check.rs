//! The static checker: the declared states fit in the state limit, every name resolves (to a
//! state's or a local's slot, written into the tree), every expression has one type, a function
//! calls only functions above it (so calls never recurse; handlers and widgets call any), a
//! function with a result ends in `return`, what draws changes nothing, and which state lists
//! keep their length. Unguarded recursion is safe: the parser bounds AST depth.

use lang::{Diag, Span};

use crate::codes;
use crate::parse::{BUILTINS, BinOp, Builtin, Call, Expr, Lit, MAX_STATE_BYTES, Program, Slot};
use crate::parse::{Stmt, Target, Type, UnOp, Var, Widget};

/// The keys an `on key` handler may name, besides a letter or digit.
pub const KEYS: [&str; 7] = ["left", "right", "up", "down", "space", "enter", "escape"];

#[rustfmt::skip]
const BUILTIN: [Builtin; 10] = [
    Builtin::Len, Builtin::Min, Builtin::Max, Builtin::Abs, Builtin::Random, Builtin::Parse,
    Builtin::Push, Builtin::Insert, Builtin::Remove, Builtin::Clear,
];

/// A checked function as calls see it: name, parameter types, result, whether it changes
/// nothing.
struct Sig {
    name: String,
    params: Vec<Type>,
    ret: Option<Type>,
    pure: bool,
}

/// What code is checked against: the states (and their declared lengths, 0 for a scalar), the
/// functions it may call (in a function, those above it), every function's name, the frame's
/// locals, the result a `return` gives (in a function), whether it may change nothing (a render
/// or an interval), whether it changed state, and which states' lengths it may change.
struct Ck<'a> {
    states: &'a [(String, Type)],
    lens: &'a [usize],
    sigs: &'a [Sig],
    names: &'a [String],
    locals: Vec<(String, Type)>,
    ret: Option<Option<Type>>,
    pure: bool,
    changes: bool,
    resized: Vec<bool>,
}

fn mismatch(msg: String, sp: Span) -> Diag {
    Diag::at_code(codes::TYPE_MISMATCH, msg, sp)
}

fn dup(name: &str, sp: Span) -> Diag {
    Diag::at_code(codes::DUP_STATE, format!("`{name}` is declared twice"), sp)
}

/// Checks `p`, resolving its names in place.
pub(crate) fn check(p: &mut Program) -> Result<(), Diag> {
    let (mut states, mut lens, mut total) = (Vec::new(), Vec::new(), 0);
    for s in &p.states {
        if states.iter().any(|(n, _)| *n == s.name) {
            return Err(dup(&s.name, s.name_span));
        }
        total += s.init.bytes();
        if total > MAX_STATE_BYTES {
            let msg = format!(
                "the states take {total} bytes by here; all state holds at most {MAX_STATE_BYTES}"
            );
            return Err(Diag::at_code(codes::STATE_TOO_BIG, msg, s.name_span));
        }
        states.push((s.name.clone(), s.init.ty()));
        lens.push(if let Lit::List(_, items) = &s.init { items.len() } else { 0 });
    }
    let names: Vec<String> = p.fns.iter().map(|f| f.name.clone()).collect();
    let (mut sigs, mut resized) = (Vec::new(), vec![false; states.len()]);
    for f in &mut p.fns {
        if sigs.iter().any(|s: &Sig| s.name == f.name) || BUILTINS.contains(&f.name.as_str()) {
            let msg = format!("`{}` is declared twice (or is built in)", f.name);
            return Err(Diag::at_code(codes::DUP_STATE, msg, f.name_span));
        }
        for (i, (n, _)) in f.params.iter().enumerate() {
            if f.params[..i].iter().any(|(m, _)| m == n) {
                return Err(dup(n, f.name_span));
            }
        }
        // Only the functions above it: so no call recurses.
        let (sigs_now, locals) = (&sigs[..], f.params.clone());
        let mut ck = Ck {
            states: &states,
            lens: &lens,
            sigs: sigs_now,
            names: &names,
            locals,
            ret: None,
            pure: false,
            changes: false,
            resized,
        };
        ck.ret = Some(f.ret);
        ck.block(&mut f.body)?;
        if f.ret.is_some() && !ends(&f.body) {
            let msg = format!("`{}` must end in `return` (on every path)", f.name);
            return Err(Diag::at_code(codes::MISSING_RETURN, msg, f.name_span));
        }
        (f.pure, resized) = (!ck.changes, ck.resized);
        let params = f.params.iter().map(|p| p.1).collect();
        sigs.push(Sig { name: f.name.clone(), params, ret: f.ret, pure: f.pure });
    }
    let mut ck = Ck {
        states: &states,
        lens: &lens,
        sigs: &sigs,
        names: &names,
        locals: Vec::new(),
        ret: None,
        pure: true,
        changes: false,
        resized,
    };
    for e in &mut p.everys {
        ck.pure = true;
        ck.expect(&mut e.interval, Type::Int, "an `every` interval")?;
        ck.handler(&mut e.body, false)?;
    }
    for k in &mut p.keys {
        let one =
            k.key.len() == 1 && k.key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if !one && !KEYS.contains(&k.key.as_str()) {
            let msg = format!(
                "no key is called \"{}\": use left, right, up, down, space, enter, escape, a to z \
                 or 0 to 9",
                k.key
            );
            return Err(Diag::at_code(codes::BAD_KEY, msg, k.span));
        }
        ck.handler(&mut k.body, false)?;
    }
    p.widgets.iter_mut().try_for_each(|w| ck.widget(w))?;
    for (s, resized) in p.states.iter_mut().zip(ck.resized) {
        s.fixed = !resized;
    }
    Ok(())
}

/// The length a list expression surely has: a literal's, or a fill's by a literal count.
fn length(e: &Expr) -> Option<usize> {
    match e {
        Expr::List(items, _) => Some(items.len()),
        Expr::Fill(_, count, _) => match **count {
            Expr::Int(n, _) => usize::try_from(n).ok(),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `stmts` ends in `return` on every path: a return, or an if chain with an else whose
/// every arm does.
fn ends(stmts: &[Stmt]) -> bool {
    match stmts.last() {
        Some(Stmt::Return { .. }) => true,
        Some(Stmt::If { arms, els, .. }) => {
            !els.is_empty() && arms.iter().all(|(_, b)| ends(b)) && ends(els)
        }
        _ => false,
    }
}

impl Ck<'_> {
    /// A widget, drawn: it may change nothing; its handlers may.
    fn widget(&mut self, w: &mut Widget) -> Result<(), Diag> {
        self.pure = true;
        match w {
            Widget::Label { value, .. } => self.scalar(value, "a label").map(drop),
            Widget::Button { text, handler, .. } => {
                self.scalar(text, "a button's text")?;
                self.handler(&mut handler.body, false)
            }
            Widget::Input { state, .. } => {
                let i = self.states.iter().position(|(n, _)| *n == state.name);
                match i.map(|i| (i, self.states[i].1)) {
                    Some((i, Type::Str)) => {
                        state.slot = Slot::State(i as u32);
                        Ok(())
                    }
                    Some((_, t)) => {
                        let msg = format!(
                            "`input` binds a string state; `{}` is {}",
                            state.name,
                            t.name()
                        );
                        Err(mismatch(msg, state.span))
                    }
                    None => Err(unknown(&state.name, state.span)),
                }
            }
            Widget::Row { children, .. } | Widget::Col { children, .. } => {
                children.iter_mut().try_for_each(|c| self.widget(c))
            }
            Widget::If { arms, els, .. } => {
                for (cond, body) in arms {
                    self.pure = true;
                    self.expect(cond, Type::Bool, "an `if` condition")?;
                    body.iter_mut().try_for_each(|c| self.widget(c))?;
                }
                els.iter_mut().try_for_each(|c| self.widget(c))
            }
            Widget::For { var, from, to, body, .. } => {
                self.expect(from, Type::Int, "a `for` start")?;
                self.expect(to, Type::Int, "a `for` end")?;
                self.locals.push((var.clone(), Type::Int));
                let r = body.iter_mut().try_for_each(|c| self.widget(c));
                self.locals.pop();
                r
            }
            Widget::Grid { cols, cells, texts, handler, .. } => {
                self.expect(cols, Type::Int, "a grid's column count")?;
                self.expect(cells, Type::Ints, "a grid's squares")?;
                if let Some(texts) = texts {
                    self.expect(texts, Type::Strs, "a grid's texts")?;
                }
                handler.as_mut().map_or(Ok(()), |h| self.handler(&mut h.body, true))
            }
        }
    }

    /// A handler's statements, seeing the loop variables around it (and `cell` in a grid's).
    fn handler(&mut self, body: &mut [Stmt], cell: bool) -> Result<(), Diag> {
        let (mark, pure, ret) = (self.locals.len(), self.pure, self.ret);
        (self.pure, self.ret) = (false, None);
        if cell {
            self.locals.push(("cell".into(), Type::Int));
        }
        let r = self.block(body);
        (self.pure, self.ret) = (pure, ret);
        self.locals.truncate(mark);
        r
    }

    fn block(&mut self, stmts: &mut [Stmt]) -> Result<(), Diag> {
        let mark = self.locals.len();
        let r = stmts.iter_mut().try_for_each(|s| self.stmt(s));
        self.locals.truncate(mark);
        r
    }

    fn stmt(&mut self, s: &mut Stmt) -> Result<(), Diag> {
        match s {
            Stmt::Let { name, value, .. } => {
                let t = self.expr(value)?;
                self.locals.push((name.clone(), t));
            }
            Stmt::Assign { target, value, .. } => {
                let want = self.place(target)?;
                self.expect(value, want, &format!("`{}`", target.name))?;
                // A state list given a list of another (or no sure) length changes its length.
                if let (Slot::State(i), Some(_)) = (target.slot, want.elem()) {
                    let i = i as usize;
                    self.resized[i] |= length(value) != Some(self.lens[i]);
                }
            }
            Stmt::SetIndex { target, index, op, value, span } => {
                let elem = self.list(target)?;
                self.place(target)?;
                self.expect(index, Type::Int, "an index")?;
                let what = format!("an item of `{}`", target.name);
                match op {
                    None => self.expect(value, elem, &what)?,
                    // `xs[i] += v` stores `xs[i] + v`.
                    Some(op) => {
                        let got = combine(*op, elem, self.expr(value)?, *span)?;
                        if got != elem {
                            let msg = format!("{what} must be {}, got {}", elem.name(), got.name());
                            return Err(mismatch(msg, *span));
                        }
                    }
                }
            }
            Stmt::If { arms, els, .. } => {
                for (cond, body) in arms {
                    self.expect(cond, Type::Bool, "an `if` condition")?;
                    self.block(body)?;
                }
                self.block(els)?;
            }
            Stmt::Repeat { count, body, .. } => {
                self.expect(count, Type::Int, "a `repeat` count")?;
                self.block(body)?;
            }
            Stmt::For { var, from, to, body, .. } => {
                self.expect(from, Type::Int, "a `for` start")?;
                self.expect(to, Type::Int, "a `for` end")?;
                self.locals.push((var.clone(), Type::Int));
                let r = self.block(body);
                self.locals.pop();
                r?;
            }
            Stmt::Return { value, span } => match (self.ret, value) {
                (Some(Some(t)), Some(v)) => self.expect(v, t, "the result")?,
                (Some(None) | None, None) => {}
                (Some(Some(t)), None) => {
                    let msg = format!("this function gives {}: write `return VALUE;`", t.name());
                    return Err(mismatch(msg, *span));
                }
                (_, Some(_)) => {
                    let msg = "only a function with `-> TYPE` returns a value".to_string();
                    return Err(mismatch(msg, *span));
                }
            },
            Stmt::Call(c) => _ = self.call(c, false)?,
        }
        Ok(())
    }

    /// The type of `v`'s slot, resolving it.
    fn resolve(&self, v: &mut Var) -> Result<Type, Diag> {
        if let Some(i) = self.locals.iter().rposition(|(n, _)| *n == v.name) {
            v.slot = Slot::Local(i as u32);
            return Ok(self.locals[i].1);
        }
        let i = self.states.iter().position(|(n, _)| *n == v.name);
        let i = i.ok_or_else(|| unknown(&v.name, v.span))?;
        v.slot = Slot::State(i as u32);
        Ok(self.states[i].1)
    }

    /// `v` as a target: resolved, its type; a state's change noted.
    fn place(&mut self, v: &mut Var) -> Result<Type, Diag> {
        let t = self.resolve(v)?;
        if matches!(v.slot, Slot::State(_)) {
            self.changes = true;
        }
        Ok(t)
    }

    /// The item type of list `v`.
    fn list(&self, v: &mut Var) -> Result<Type, Diag> {
        let t = self.resolve(v)?;
        let msg = || format!("`{}` is {}, not a list", v.name, t.name());
        t.elem().ok_or_else(|| mismatch(msg(), v.span))
    }

    fn expect(&mut self, e: &mut Expr, want: Type, what: &str) -> Result<(), Diag> {
        let got = self.expr(e)?;
        let msg = || format!("{what} must be {}, got {}", want.name(), got.name());
        if got == want { Ok(()) } else { Err(mismatch(msg(), e.span())) }
    }

    /// An int, bool or string, for `what` to show.
    fn scalar(&mut self, e: &mut Expr, what: &str) -> Result<Type, Diag> {
        let t = self.expr(e)?;
        let msg = || format!("{what} shows an int, bool or string, not {}", t.name());
        if t.elem().is_none() { Ok(t) } else { Err(mismatch(msg(), e.span())) }
    }

    /// Something that changes state at `sp`: an error where nothing may change.
    fn impure(&mut self, what: &str, sp: Span) -> Result<(), Diag> {
        self.changes = true;
        if self.pure {
            let msg = format!(
                "{what} changes state, so a widget or an `every` interval cannot use it: \
                 call it from a handler"
            );
            return Err(Diag::at_code(codes::IMPURE_RENDER, msg, sp));
        }
        Ok(())
    }

    fn expr(&mut self, e: &mut Expr) -> Result<Type, Diag> {
        use Type::*;
        match e {
            Expr::Int(..) => Ok(Int),
            Expr::Bool(..) => Ok(Bool),
            Expr::Str(..) => Ok(Str),
            Expr::Var(v) => self.resolve(v),
            Expr::Index(v, index, _) => {
                let elem = self.list(v)?;
                self.expect(index, Int, "an index")?;
                Ok(elem)
            }
            Expr::Call(c) => self.call(c, true).map(|t| t.unwrap_or(Int)),
            Expr::List(items, sp) => {
                let Some((first, rest)) = items.split_first_mut() else {
                    let msg = "an empty list has no type: write [0; 0], [false; 0] or [\"\"; 0]";
                    return Err(mismatch(msg.into(), *sp));
                };
                let t = self.scalar(first, "a list")?;
                for item in rest {
                    self.expect(item, t, "every item of a list")?;
                }
                Ok(t.list().unwrap_or(Ints))
            }
            Expr::Fill(item, count, _) => {
                let t = self.scalar(item, "a list")?;
                self.expect(count, Int, "a list's length")?;
                Ok(t.list().unwrap_or(Ints))
            }
            Expr::Unary(op, inner, sp) => match (*op, self.expr(inner)?) {
                (UnOp::Neg, Int) => Ok(Int),
                (UnOp::Not, Bool) => Ok(Bool),
                (UnOp::Neg, t) => Err(mismatch(format!("`-` needs int, got {}", t.name()), *sp)),
                (UnOp::Not, t) => Err(mismatch(format!("`!` needs bool, got {}", t.name()), *sp)),
            },
            Expr::Binary(op, l, r, sp) => {
                let (lt, rt) = (self.expr(l)?, self.expr(r)?);
                combine(*op, lt, rt, *sp)
            }
        }
    }

    /// A call: its result type (`None`: none), which `value` requires.
    fn call(&mut self, c: &mut Call, value: bool) -> Result<Option<Type>, Diag> {
        let ret = match BUILTINS.iter().position(|n| *n == c.name) {
            Some(i) => {
                c.target = Target::Builtin(BUILTIN[i]);
                self.builtin(BUILTIN[i], c)?
            }
            None => {
                // In a function, `sigs` holds only the functions above it.
                let found = self.sigs.iter().enumerate().find(|(_, s)| s.name == c.name);
                let Some((i, sig)) = found else {
                    if self.names.contains(&c.name) {
                        let msg = format!(
                            "`{}` is not defined above this function: a function calls only \
                             functions defined above it (no recursion)",
                            c.name
                        );
                        return Err(Diag::at_code(codes::CALL_BELOW, msg, c.span));
                    }
                    let msg = format!("there is no function `{}`", c.name);
                    return Err(Diag::at_code(codes::UNKNOWN_NAME, msg, c.span));
                };
                self.args(c, &sig.params)?;
                if !sig.pure {
                    self.impure(&format!("`{}`", c.name), c.span)?;
                }
                c.target = Target::Fn(i as u32);
                sig.ret
            }
        };
        if value && ret.is_none() {
            let msg = format!("`{}` gives no value: call it as a statement", c.name);
            return Err(mismatch(msg, c.span));
        }
        Ok(ret)
    }

    /// Checks `c`'s arguments against `params`.
    fn args(&mut self, c: &mut Call, params: &[Type]) -> Result<(), Diag> {
        if c.args.len() != params.len() {
            let msg =
                format!("`{}` takes {} arguments, not {}", c.name, params.len(), c.args.len());
            return Err(Diag::at_code(codes::ARITY, msg, c.span));
        }
        for (a, t) in c.args.iter_mut().zip(params) {
            self.expect(a, *t, &format!("an argument of `{}`", c.name))?;
        }
        Ok(())
    }

    fn builtin(&mut self, b: Builtin, c: &mut Call) -> Result<Option<Type>, Diag> {
        use Type::*;
        let ints = |n| vec![Int; n];
        match b {
            Builtin::Len => {
                self.args_n(c, 1)?;
                let t = self.expr(&mut c.args[0])?;
                if t != Str && t.elem().is_none() {
                    let msg = format!("`len` takes a list or a string, not {}", t.name());
                    return Err(mismatch(msg, c.args[0].span()));
                }
            }
            Builtin::Min | Builtin::Max => self.args(c, &ints(2))?,
            Builtin::Abs => self.args(c, &ints(1))?,
            Builtin::Random => {
                self.args(c, &ints(1))?;
                self.impure("`random`", c.span)?;
            }
            Builtin::Parse => self.args(c, &[Str, Int])?,
            Builtin::Push | Builtin::Insert | Builtin::Remove | Builtin::Clear => {
                // The arguments after the list: push(xs, v), insert(xs, i, v), remove(xs, i).
                let after = |elem| [vec![elem], vec![Int, elem], vec![Int], vec![]];
                self.args_n(c, after(Int)[b as usize - 6].len() + 1)?;
                let Some((Expr::Var(v), rest)) = c.args.split_first_mut() else {
                    let msg = format!("`{}` takes a list's name first", c.name);
                    return Err(mismatch(msg, c.span));
                };
                let elem = self.list(v)?;
                if let Slot::State(i) = v.slot {
                    self.impure(&format!("`{}` of a state", c.name), c.span)?;
                    self.resized[i as usize] = true;
                }
                for (a, t) in rest.iter_mut().zip(&after(elem)[b as usize - 6]) {
                    self.expect(a, *t, &format!("an argument of `{}`", c.name))?;
                }
                return Ok(None);
            }
        }
        Ok(Some(Int))
    }

    fn args_n(&self, c: &Call, n: usize) -> Result<(), Diag> {
        if c.args.len() == n {
            return Ok(());
        }
        let msg = format!("`{}` takes {n} arguments, not {}", c.name, c.args.len());
        Err(Diag::at_code(codes::ARITY, msg, c.span))
    }
}

/// The type `op` gives operands of types `lt` and `rt`, or a mismatch at `sp`.
fn combine(op: BinOp, lt: Type, rt: Type, sp: Span) -> Result<Type, Diag> {
    use BinOp::*;
    use Type::*;
    let lists = lt.elem().is_some() || rt.elem().is_some();
    match (op, lt, rt) {
        _ if lists => {}
        // `+` adds ints; with a string operand it concatenates the other's display.
        (Add, Int, Int) => return Ok(Int),
        (Add, Str, _) | (Add, _, Str) => return Ok(Str),
        (Sub | Mul | Div | Rem, Int, Int) => return Ok(Int),
        (Lt | Le | Gt | Ge, Int, Int) => return Ok(Bool),
        // Equality is same-type only: `1 == "1"` is a bug, not `false`.
        (Eq | Ne, a, b) if a == b => return Ok(Bool),
        (And | Or, Bool, Bool) => return Ok(Bool),
        _ => {}
    }
    let (o, l, r) = (op.sym(), lt.name(), rt.name());
    Err(mismatch(format!("`{o}` cannot combine {l} and {r}"), sp))
}

fn unknown(name: &str, sp: Span) -> Diag {
    let msg = format!("`{name}` is not a declared state or local variable");
    Diag::at_code(codes::UNKNOWN_NAME, msg, sp)
}

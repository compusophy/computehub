//! The static checker: the declared states fit in the state limit, every name resolves (to a
//! state's or a local's slot, written into the tree), every expression has one type, functions
//! call one another in any order but never recurse (each is checked after those it calls), a
//! function with a result ends in `return`, `break` and `continue` sit in loops, what renders
//! changes nothing, shapes are drawn only where a canvas draws (in the function it calls, and the
//! functions those call: a function that draws changes nothing), no state list is cleared for
//! good, and which state lists keep their length. Unguarded recursion is safe: the parser bounds
//! AST depth.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use lang::{Diag, Span};

use crate::codes;
use crate::parse::{BUILTINS, BinOp, Builtin, Call, DRAWN, Expr, FnDecl, Lit, MAX_STATE_BYTES};
use crate::parse::{Program, SHAPES, Slot};
use crate::parse::{Stmt, Target, Type, UnOp, Var, Widget};

/// The keys an `on key` handler may name, besides a letter or digit.
pub const KEYS: [&str; 7] = ["left", "right", "up", "down", "space", "enter", "escape"];

#[rustfmt::skip]
const BUILTIN: [Builtin; 19] = [
    Builtin::Len, Builtin::Min, Builtin::Max, Builtin::Abs, Builtin::Random, Builtin::Parse,
    Builtin::Push, Builtin::Insert, Builtin::Remove, Builtin::Clear, Builtin::Rect,
    Builtin::Circle, Builtin::Ring, Builtin::Line, Builtin::Text, Builtin::Sprite,
    Builtin::Pixels, Builtin::Sin, Builtin::Cos,
];

/// A checked function as calls see it: parameter types, result, whether it changes nothing,
/// whether it draws.
struct Sig {
    params: Vec<Type>,
    ret: Option<Type>,
    pure: bool,
    draws: bool,
}

/// What the code does to each state's length: changes it, grows it (a push, an insert or a whole
/// list assigned) and where it first clears it.
struct Sizes {
    resized: Vec<bool>,
    grown: Vec<bool>,
    cleared: Vec<Option<Span>>,
}

/// What code is checked against: the states (and their declared lengths, 0 for a scalar), every
/// function's signature once checked (a function is checked after those it calls), every
/// function's name, the frame's locals, the result a `return` gives (in a function), whether it
/// may change nothing (a render or an interval), whether it changed state, what it does to the
/// states' lengths, whether the program has a canvas, whether it may draw (a function, or a
/// canvas's call), whether it drew, and how many loops enclose it in its function or handler.
struct Ck<'a> {
    states: &'a [(String, Type)],
    lens: &'a [usize],
    sigs: &'a [Option<Sig>],
    names: &'a [String],
    locals: Vec<(String, Type)>,
    ret: Option<Option<Type>>,
    pure: bool,
    changes: bool,
    sizes: Sizes,
    canvas: bool,
    draw: bool,
    drew: bool,
    loops: u32,
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
    // The names that came with the canvas are built in only beside one: an older program may
    // have its own `line`; and `pixels` came later still, so it may have its own beside one.
    let canvas = p.widgets.iter().any(has_canvas);
    for (k, f) in p.fns.iter().enumerate() {
        let built = BUILTINS.iter().position(|b| *b == f.name);
        let beside = |i| canvas && i != Builtin::Pixels as usize;
        if names[..k].contains(&f.name) || built.is_some_and(|i| i < DRAWN || beside(i)) {
            let msg = match built {
                Some(DRAWN..) => format!(
                    "`{}` is built in beside a canvas (rect, circle, ring, line, text and sprite \
                     draw; sin and cos give values): name your function otherwise",
                    f.name
                ),
                _ => format!("`{}` is declared twice (or is built in)", f.name),
            };
            return Err(Diag::at_code(codes::DUP_STATE, msg, f.name_span));
        }
        for (i, (n, _)) in f.params.iter().enumerate() {
            if f.params[..i].iter().any(|(m, _)| m == n) {
                return Err(dup(n, f.name_span));
            }
        }
    }
    let n = states.len();
    let mut sizes =
        Sizes { resized: vec![false; n], grown: vec![false; n], cleared: vec![None; n] };
    let mut sigs: Vec<Option<Sig>> = p.fns.iter().map(|_| None).collect();
    for i in order(&p.fns)? {
        let f = &mut p.fns[i];
        let mut ck = Ck {
            states: &states,
            lens: &lens,
            sigs: &sigs,
            names: &names,
            locals: f.params.clone(),
            ret: Some(f.ret),
            pure: false,
            changes: false,
            sizes,
            canvas,
            draw: true,
            drew: false,
            loops: 0,
        };
        ck.block(&mut f.body)?;
        if f.ret.is_some() && !ends(&f.body) {
            let msg = format!("`{}` must end in `return` (on every path)", f.name);
            return Err(Diag::at_code(codes::MISSING_RETURN, msg, f.name_span));
        }
        if ck.drew && ck.changes {
            let msg = format!(
                "`{}` draws, so it changes nothing (no state set, no list changed, no random): \
                 change state in handlers, and the canvas shows it",
                f.name
            );
            return Err(Diag::at_code(codes::IMPURE_RENDER, msg, f.name_span));
        }
        let (pure, draws) = (!ck.changes, ck.drew);
        (f.pure, sizes) = (pure, ck.sizes);
        let params = f.params.iter().map(|p| p.1).collect();
        sigs[i] = Some(Sig { params, ret: f.ret, pure, draws });
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
        sizes,
        canvas,
        draw: false,
        drew: false,
        loops: 0,
    };
    for e in &mut p.everys {
        ck.pure = true;
        ck.expect(&mut e.interval, Type::Int, "an `every` interval")?;
        ck.handler(&mut e.body, &[])?;
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
        ck.handler(&mut k.body, &[])?;
    }
    p.widgets.iter_mut().try_for_each(|w| ck.widget(w))?;
    // A list a clear empties and nothing grows again holds nothing from then on: a reset meant as
    // `board = [0; 9];`, whose every index would fault.
    for (i, s) in p.states.iter().enumerate() {
        let (Some(sp), false, Lit::List(t, items)) =
            (ck.sizes.cleared[i], ck.sizes.grown[i], &s.init)
        else {
            continue;
        };
        if items.is_empty() {
            continue;
        }
        let zero = match t {
            Type::Bools => "false",
            Type::Strs => "\"\"",
            _ => "0",
        };
        let msg = format!(
            "`clear({0})` empties {0} for good: nothing pushes to, inserts into or assigns {0}, \
             so after it {0} has no items. To reset it, assign a fresh list: {0} = [{zero}; {1}];",
            s.name,
            items.len()
        );
        return Err(Diag::at_code(codes::CLEARED_FOR_GOOD, msg, sp));
    }
    for (s, resized) in p.states.iter_mut().zip(ck.sizes.resized) {
        s.fixed = !resized;
    }
    Ok(())
}

/// The functions' indexes, each after every function it calls (so a call meets a checked
/// callee), the earliest in the source first where the order is free; or, if a function calls
/// itself (directly or through others), an error at that call.
fn order(fns: &[FnDecl]) -> Result<Vec<usize>, Diag> {
    let n = fns.len();
    // Each function's calls of functions: the callee and the call's span, in body order.
    let calls: Vec<Vec<(usize, Span)>> = fns
        .iter()
        .map(|f| {
            let mut out = Vec::new();
            each_call(&f.body, &mut |c| {
                if let Some(g) = fns.iter().position(|g| g.name == c.name) {
                    out.push((g, c.span));
                }
            });
            out
        })
        .collect();
    let mut waiting: Vec<usize> = calls.iter().map(Vec::len).collect();
    let mut callers = vec![Vec::new(); n];
    for (f, cs) in calls.iter().enumerate() {
        for &(g, _) in cs {
            callers[g].push(f);
        }
    }
    let mut ready: BinaryHeap<Reverse<usize>> =
        (0..n).filter(|&f| waiting[f] == 0).map(Reverse).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(Reverse(f)) = ready.pop() {
        order.push(f);
        for &c in &callers[f] {
            waiting[c] -= 1;
            if waiting[c] == 0 {
                ready.push(Reverse(c));
            }
        }
    }
    if order.len() == n {
        return Ok(order);
    }
    // Every function left calls one left: walk them until one comes round again.
    let mut at = (0..n).find(|&f| waiting[f] > 0).unwrap_or(0);
    let mut path: Vec<(usize, Span)> = Vec::new();
    loop {
        if let Some(k) = path.iter().position(|&(g, _)| g == at) {
            return Err(recursion(fns, &path[k..]));
        }
        let Some(&(next, sp)) = calls[at].iter().find(|&&(g, _)| waiting[g] > 0) else {
            let msg = format!("`{}` calls itself", fns[at].name);
            return Err(Diag::at_code(codes::RECURSES, msg, fns[at].name_span));
        };
        path.push((at, sp));
        at = next;
    }
}

/// The error for `cycle`, each function with its call of the next (the last's of the first): at
/// the call in the cycle's earliest function.
fn recursion(fns: &[FnDecl], cycle: &[(usize, Span)]) -> Diag {
    let Some(k) = (0..cycle.len()).min_by_key(|&k| cycle[k].0) else {
        return Diag::at_code(codes::RECURSES, "a function calls itself", Span::new(0, 0));
    };
    let (f, sp) = cycle[k];
    let others: Vec<String> = cycle[k + 1..]
        .iter()
        .chain(&cycle[..k])
        .map(|&(g, _)| format!("`{}`", fns[g].name))
        .collect();
    let how = match others.len() {
        0 => String::new(),
        _ => format!(" through {}", others.join(", ")),
    };
    let msg = format!(
        "`{}` calls itself{how}: applang has no recursion. Loop instead (for, repeat, while), \
         keeping what is left to do in a list",
        fns[f].name
    );
    Diag::at_code(codes::RECURSES, msg, sp)
}

/// Runs `f` on every call in `stmts`, the calls in arguments too.
fn each_call(stmts: &[Stmt], f: &mut dyn FnMut(&Call)) {
    for s in stmts {
        match s {
            Stmt::Let { value, .. } | Stmt::Assign { value, .. } => expr_calls(value, f),
            Stmt::SetIndex { index, value, .. } => {
                expr_calls(index, f);
                expr_calls(value, f);
            }
            Stmt::If { arms, els, .. } => {
                for (cond, body) in arms {
                    expr_calls(cond, f);
                    each_call(body, f);
                }
                each_call(els, f);
            }
            Stmt::Repeat { count, body, .. } => {
                expr_calls(count, f);
                each_call(body, f);
            }
            Stmt::For { from, to, body, .. } => {
                expr_calls(from, f);
                expr_calls(to, f);
                each_call(body, f);
            }
            Stmt::While { cond, body, .. } => {
                expr_calls(cond, f);
                each_call(body, f);
            }
            Stmt::Return { value, .. } => value.iter().for_each(|v| expr_calls(v, f)),
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Call(c) => call_calls(c, f),
        }
    }
}

fn expr_calls(e: &Expr, f: &mut dyn FnMut(&Call)) {
    match e {
        Expr::Int(..) | Expr::Bool(..) | Expr::Str(..) | Expr::Var(_) => {}
        Expr::Index(_, inner, _) | Expr::Unary(_, inner, _) => expr_calls(inner, f),
        Expr::Call(c) => call_calls(c, f),
        Expr::List(items, _) => items.iter().for_each(|x| expr_calls(x, f)),
        Expr::Fill(a, b, _) | Expr::Binary(_, a, b, _) | Expr::At(a, b, _) => {
            expr_calls(a, f);
            expr_calls(b, f);
        }
        Expr::Cond(c, a, b, _) => {
            expr_calls(c, f);
            expr_calls(a, f);
            expr_calls(b, f);
        }
    }
}

fn call_calls(c: &Call, f: &mut dyn FnMut(&Call)) {
    f(c);
    c.args.iter().for_each(|a| expr_calls(a, f));
}

/// Whether `w` is a canvas or holds one.
fn has_canvas(w: &Widget) -> bool {
    match w {
        Widget::Canvas { .. } => true,
        Widget::Row { children, .. } | Widget::Col { children, .. } => {
            children.iter().any(has_canvas)
        }
        Widget::For { body, .. } => body.iter().any(has_canvas),
        Widget::If { arms, els, .. } => arms.iter().flat_map(|a| &a.1).chain(els).any(has_canvas),
        _ => false,
    }
}

/// The length a list expression surely has: a literal's, or a fill's by a literal count.
fn length(e: &Expr) -> Option<usize> {
    match e {
        Expr::List(items, _) => Some(items.len()),
        Expr::Fill(_, count, _) => match **count {
            Expr::Int(n, _) => usize::try_from(n).ok(),
            _ => None,
        },
        Expr::Cond(_, a, b, _) => length(a).filter(|&n| length(b) == Some(n)),
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
                self.handler(&mut handler.body, &[])
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
                handler.as_mut().map_or(Ok(()), |h| self.handler(&mut h.body, &["cell"]))
            }
            Widget::Canvas { w, h, scene, handler, span } => {
                self.expect(w, Type::Int, "a canvas's width")?;
                self.expect(h, Type::Int, "a canvas's height")?;
                // Its call draws it, and may draw only there.
                self.draw = true;
                let drawn = self.call(scene, false);
                self.draw = false;
                drawn?;
                let Some(handler) = handler else { return Ok(()) };
                // A state or a loop's variable of the name would be hidden in the handler.
                let mut names = self.states.iter().chain(&self.locals).map(|(n, _)| n.as_str());
                if let Some(n) = names.find(|n| *n == "x" || *n == "y") {
                    let msg = format!(
                        "`{n}` is taken here: a canvas's handler names the tap's place x and y, \
                         so name your state or loop variable otherwise"
                    );
                    return Err(Diag::at_code(codes::DUP_STATE, msg, *span));
                }
                self.handler(&mut handler.body, &["x", "y"])
            }
        }
    }

    /// A handler's statements, seeing the loop variables around it and `names`, ints (`cell` in
    /// a grid's, `x` and `y` in a canvas's).
    fn handler(&mut self, body: &mut [Stmt], names: &[&str]) -> Result<(), Diag> {
        let (mark, pure, ret, draw) = (self.locals.len(), self.pure, self.ret, self.draw);
        let loops = self.loops;
        (self.pure, self.ret, self.draw, self.loops) = (false, None, false, 0);
        self.locals.extend(names.iter().map(|n| (n.to_string(), Type::Int)));
        let r = self.block(body);
        (self.pure, self.ret, self.draw, self.loops) = (pure, ret, draw, loops);
        self.locals.truncate(mark);
        r
    }

    /// A loop's body: `break` and `continue` may stand in it.
    fn looped(&mut self, body: &mut [Stmt]) -> Result<(), Diag> {
        self.loops += 1;
        let r = self.block(body);
        self.loops -= 1;
        r
    }

    /// A shape or a function that draws, called `name` at `sp`: drawn here, or where nothing
    /// may draw, an error.
    fn drawing(&mut self, name: &str, sp: Span) -> Result<(), Diag> {
        let msg = match (self.canvas, self.draw) {
            (true, true) => {
                self.drew = true;
                return Ok(());
            }
            (false, _) => format!(
                "`{name}` draws only on a canvas: add canvas W, H, scene(); and draw in scene"
            ),
            (true, false) => format!(
                "`{name}` draws, so only a canvas, or a function a canvas calls, may call it: \
                 handlers change state, and the canvas shows it"
            ),
        };
        Err(Diag::at_code(codes::DRAW_OUTSIDE, msg, sp))
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
                    self.sizes.resized[i] |= length(value) != Some(self.lens[i]);
                    self.sizes.grown[i] = true;
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
                self.looped(body)?;
            }
            Stmt::For { var, from, to, body, .. } => {
                self.expect(from, Type::Int, "a `for` start")?;
                self.expect(to, Type::Int, "a `for` end")?;
                self.locals.push((var.clone(), Type::Int));
                let r = self.looped(body);
                self.locals.pop();
                r?;
            }
            Stmt::While { cond, body, .. } => {
                self.expect(cond, Type::Bool, "a `while` condition")?;
                self.looped(body)?;
            }
            Stmt::Break(sp) => self.in_loop("break", *sp)?,
            Stmt::Continue(sp) => self.in_loop("continue", *sp)?,
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

    /// `word` at `sp`, which only a loop may hold.
    fn in_loop(&self, word: &str, sp: Span) -> Result<(), Diag> {
        if self.loops > 0 {
            return Ok(());
        }
        let msg = format!(
            "`{word}` stands only inside a loop (for, repeat or while) of its own function or \
             handler"
        );
        Err(Diag::at_code(codes::OUTSIDE_LOOP, msg, sp))
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
                // A string's item is a letter, itself a string.
                let elem = if self.resolve(v)? == Str { Str } else { self.list(v)? };
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
            Expr::Cond(cond, yes, no, sp) => {
                self.expect(cond, Bool, "a `?:` condition")?;
                let (a, b) = (self.expr(yes)?, self.expr(no)?);
                if a != b {
                    let (a, b) = (a.name(), b.name());
                    let msg = format!("the two sides of `?:` give one type, not {a} and {b}");
                    return Err(mismatch(msg, *sp));
                }
                Ok(a)
            }
            Expr::At(list, index, _) => {
                let t = self.expr(list)?;
                let Some(elem) = t.elem().or((t == Str).then_some(Str)) else {
                    let msg = format!("only a list or a string has items: this is {}", t.name());
                    return Err(mismatch(msg, list.span()));
                };
                self.expect(index, Int, "an index")?;
                Ok(elem)
            }
        }
    }

    /// A call: its result type (`None`: none), which `value` requires.
    fn call(&mut self, c: &mut Call, value: bool) -> Result<Option<Type>, Diag> {
        // A name that came with the canvas is the program's own function, if it has one.
        let built = BUILTINS.iter().position(|n| *n == c.name);
        let ret = match built.filter(|&i| i < DRAWN || !self.names.contains(&c.name)) {
            Some(i) => {
                let b = BUILTIN[i];
                c.target = Target::Builtin(b);
                if b.draws() {
                    self.drawing(&c.name, c.span)?;
                }
                self.builtin(b, c)?
            }
            None => {
                let Some(i) = self.names.iter().position(|n| *n == c.name) else {
                    let msg = format!("there is no function `{}`", c.name);
                    return Err(Diag::at_code(codes::UNKNOWN_NAME, msg, c.span));
                };
                // A function is checked after those it calls (`order`), so this one was.
                let Some(sig) = self.sigs.get(i).and_then(Option::as_ref) else {
                    let msg = format!("`{}` calls itself", c.name);
                    return Err(Diag::at_code(codes::RECURSES, msg, c.span));
                };
                self.args(c, &sig.params)?;
                if !sig.pure {
                    self.impure(&format!("`{}`", c.name), c.span)?;
                }
                if sig.draws {
                    self.drawing(&c.name, c.span)?;
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
            Builtin::Abs | Builtin::Sin | Builtin::Cos => self.args(c, &ints(1))?,
            // The shapes: the thing first (a text's value, a sprite's rows, pixels' cells),
            // then ints.
            Builtin::Rect
            | Builtin::Circle
            | Builtin::Ring
            | Builtin::Line
            | Builtin::Text
            | Builtin::Sprite
            | Builtin::Pixels => {
                let k = b as usize - Builtin::Rect as usize;
                let n = SHAPES[k].split(", ").count();
                if c.args.len() != n {
                    let msg = format!(
                        "`{}` takes {n} arguments ({}), not {}",
                        c.name,
                        SHAPES[k],
                        c.args.len()
                    );
                    return Err(Diag::at_code(codes::ARITY, msg, c.span));
                }
                let first = match b {
                    Builtin::Text => {
                        self.scalar(&mut c.args[0], "a text")?;
                        1
                    }
                    Builtin::Sprite => {
                        self.expect(&mut c.args[0], Strs, "a sprite's rows")?;
                        1
                    }
                    Builtin::Pixels => {
                        self.expect(&mut c.args[0], Ints, "pixels' cells")?;
                        1
                    }
                    _ => 0,
                };
                for a in &mut c.args[first..] {
                    self.expect(a, Int, &format!("an argument of `{}`", c.name))?;
                }
                return Ok(None);
            }
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
                    let i = i as usize;
                    self.impure(&format!("`{}` of a state", c.name), c.span)?;
                    self.sizes.resized[i] = true;
                    match b {
                        Builtin::Push | Builtin::Insert => self.sizes.grown[i] = true,
                        Builtin::Clear => _ = self.sizes.cleared[i].get_or_insert(c.span),
                        _ => {}
                    }
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

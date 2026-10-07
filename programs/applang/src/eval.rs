//! The runtime: a fueled tree walk over a checked tree whose names are slots. Rendering changes
//! nothing (the checker allows it no assignment, list change, `random` or call to a function
//! that does; only a canvas's call draws, into [`Run::draws`]). Handling is atomic: a handler
//! runs on a copy of the state, committed only on a clean finish. The faults left are checked
//! arithmetic, indexes, list and string bounds, grids, canvases and their shapes, `random`'s
//! bound, call depth and fuel. Fuel pays for bytes too: every value a run
//! makes or copies costs a step per item and per [`BYTES_A_STEP`] bytes of text, paid before it
//! is made, so a run's memory and time are bounded by its fuel.

use std::mem;

use applang_syntax::ast::{BinOp, Builtin, Call, Expr, Lit, SHAPES, Slot, Stmt, Target, UnOp};
use applang_syntax::ast::{Var, Widget};
use fuel::Fuel;
use lang::{Diag, Span};

use crate::{Draw, Limits, Node, Program, Shape, codes};

/// The deepest calls nest; with the parser's depth cap, a bounded stack.
const MAX_CALLS: u32 = 32;
/// The most columns a grid has.
pub(crate) const MAX_COLS: i64 = 100;
/// The bytes of text a step copies: an event's fuel makes at most 64 MB, a render's 12.8 MB.
pub(crate) const BYTES_A_STEP: usize = 64;
/// The most units a canvas has a side.
pub(crate) const MAX_SIDE: i64 = 1024;
/// The most cells pixels have a row and a column.
pub(crate) const PIXELS_SIDE: usize = 64;
/// The char of a pixel of each color, as uiwire's Pixels hold them; `.` is none (-1).
const PAINT: &[u8; 12] = b"0123456789ab";
/// A thousand times the sine of each degree from 0 to 90, rounded.
#[rustfmt::skip]
const SINE: [u16; 91] = [
    0, 17, 35, 52, 70, 87, 105, 122, 139, 156, 174, 191, 208, 225, 242, 259, 276, 292, 309, 326,
    342, 358, 375, 391, 407, 423, 438, 454, 469, 485, 500, 515, 530, 545, 559, 574, 588, 602, 616,
    629, 643, 656, 669, 682, 695, 707, 719, 731, 743, 755, 766, 777, 788, 799, 809, 819, 829, 839,
    848, 857, 866, 875, 883, 891, 899, 906, 914, 921, 927, 934, 940, 946, 951, 956, 961, 966, 970,
    974, 978, 982, 985, 988, 990, 993, 995, 996, 998, 999, 999, 1000, 1000,
];

/// A thousand times the sine of `deg` degrees, rounded: total, any int is an angle.
fn sine(deg: i64) -> i64 {
    let d = deg.rem_euclid(360);
    let (q, sign) = match d {
        0..=90 => (d, 1),
        91..=180 => (180 - d, 1),
        181..=270 => (d - 180, -1),
        _ => (360 - d, -1),
    };
    sign * i64::from(SINE[q as usize])
}

/// An applang runtime value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Value { Int(i64), Bool(bool), Str(String), List(Vec<Value>) }

impl std::fmt::Display for Value {
    /// A scalar as a label shows it; a list as a literal of its items.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => f.write_str(s),
            Value::List(items) => {
                f.write_str("[")?;
                for (i, v) in items.iter().enumerate() {
                    f.write_str(if i == 0 { "" } else { ", " })?;
                    match v {
                        Value::Str(s) => write!(f, "\"{}\"", escape(s))?,
                        v => write!(f, "{v}")?,
                    }
                }
                f.write_str("]")
            }
        }
    }
}

/// `s` with `"`, `\` and line breaks escaped, as a string literal holds it.
pub(crate) fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

impl Value {
    /// The bytes it counts for in the state: 8 a scalar, a string its own.
    pub(crate) fn bytes(&self) -> usize {
        match self {
            Value::Str(s) => s.len().max(8),
            Value::List(items) => 8 + items.iter().map(Value::bytes).sum::<usize>(),
            _ => 8,
        }
    }

    /// The steps a copy of it costs: one per item of a list, and one per [`BYTES_A_STEP`] bytes
    /// of its text.
    pub(crate) fn cost(&self) -> u64 {
        match self {
            Value::Str(s) => text(s.len()),
            Value::List(items) => items.iter().map(|v| 1 + v.cost()).sum(),
            _ => 0,
        }
    }

    pub(crate) fn of(lit: &Lit) -> Value {
        match lit {
            Lit::Int(v) => Value::Int(*v),
            Lit::Bool(b) => Value::Bool(*b),
            Lit::Str(s) => Value::Str(s.clone()),
            Lit::List(_, items) => Value::List(items.iter().map(Value::of).collect()),
        }
    }
}

/// The live state: each state's value in declaration order, and `random`'s seed.
#[derive(Debug, Clone)]
pub(crate) struct State {
    pub vals: Vec<Value>,
    pub seed: u64,
}

/// A handler a render showed: its id, the squares of its grid or the units of its canvas (none:
/// a button's), a canvas's width (0: none) and the loop variables it sees.
#[derive(Debug, Clone)]
pub(crate) struct Shown {
    pub id: u32,
    pub cells: Option<u32>,
    pub cols: u32,
    pub captured: Vec<Value>,
}

/// What a block did: ran to its end, returned (with a value), or left its loop or turn of it.
pub(crate) enum Flow {
    Next,
    Ret(Option<Value>),
    Break,
    Continue,
}

fn int(v: Value) -> i64 {
    let Value::Int(n) = v else { unreachable!("checked: an int") };
    n
}

/// Letter `i` of `s`, a string of one; or how many letters it has.
fn letter(s: &str, i: i64) -> Result<Value, usize> {
    let c = usize::try_from(i).ok().and_then(|i| s.chars().nth(i));
    c.map(|c| Value::Str(c.to_string())).ok_or_else(|| s.chars().count())
}

/// The steps `n` bytes of text cost.
fn text(n: usize) -> u64 {
    (n / BYTES_A_STEP) as u64
}

fn fault(code: u16, msg: impl Into<String>, sp: Span) -> Diag {
    Diag::at_code(code, msg, sp)
}

/// One run of code: the program, a tank of fuel, the state, the locals (the current call's
/// from `base`), how deep calls nest, the shapes a canvas's call drew, and the ink and the
/// pixels' cells left.
pub(crate) struct Run<'a> {
    pub p: &'a Program,
    pub fuel: Fuel,
    pub st: &'a mut State,
    pub locals: Vec<Value>,
    base: usize,
    depth: u32,
    pub lim: &'a Limits,
    pub draws: Vec<Draw>,
    pub ink: usize,
    cells: usize,
}

impl<'a> Run<'a> {
    pub fn new(p: &'a Program, st: &'a mut State, lim: &'a Limits, fuel: u64) -> Run<'a> {
        let (locals, draws, ink, cells) = (Vec::new(), Vec::new(), lim.max_ink, lim.max_pixels);
        let fuel = Fuel::new(fuel);
        Run { p, fuel, st, locals, base: 0, depth: 0, lim, draws, ink, cells }
    }

    /// Burns `n` units, or faults at `sp`.
    pub fn burn(&mut self, n: u64, sp: Span) -> Result<(), Diag> {
        self.fuel.burn(n).map_err(|_| fault(codes::FUEL_EXHAUSTED, "fuel exhausted", sp))
    }

    fn slot(&self, v: &Var) -> &Value {
        match v.slot {
            Slot::State(i) => &self.st.vals[i as usize],
            Slot::Local(i) => &self.locals[self.base + i as usize],
        }
    }

    fn slot_mut(&mut self, v: &Var) -> &mut Value {
        match v.slot {
            Slot::State(i) => &mut self.st.vals[i as usize],
            Slot::Local(i) => &mut self.locals[self.base + i as usize],
        }
    }

    /// A copy of `v`'s value, paid for first.
    fn read(&mut self, v: &Var) -> Result<Value, Diag> {
        let cost = self.slot(v).cost();
        self.burn(cost, v.span)?;
        Ok(self.slot(v).clone())
    }

    /// List `v`'s items.
    fn items(&mut self, v: &Var) -> &mut Vec<Value> {
        let Value::List(items) = self.slot_mut(v) else { unreachable!("checked: a list") };
        items
    }

    /// A copy of item `i` of list `v`, paid for first, or a fault at `sp`.
    fn item(&mut self, v: &Var, i: i64, sp: Span) -> Result<Value, Diag> {
        let i = self.index(v, i, false, sp)?;
        let cost = self.items(v)[i].cost();
        self.burn(cost, sp)?;
        Ok(self.items(v)[i].clone())
    }

    /// The index `i` of list `v` (`end`: one past the last too), or a fault at `sp`.
    fn index(&mut self, v: &Var, i: i64, end: bool, sp: Span) -> Result<usize, Diag> {
        let n = self.items(v).len();
        match usize::try_from(i).ok().filter(|&i| i < n + usize::from(end)) {
            Some(i) => Ok(i),
            None => {
                let msg = format!("index {i} is outside `{}`, which has {n} items", v.name);
                Err(fault(codes::INDEX_OUT_OF_RANGE, msg, sp))
            }
        }
    }

    /// A list of `n` items is no longer than allowed, or a fault at `sp`.
    fn fits(&self, n: usize, sp: Span) -> Result<(), Diag> {
        if n > self.lim.max_items {
            let msg = format!("a list would hold {n} items; the limit is {}", self.lim.max_items);
            return Err(fault(codes::LIST_FULL, msg, sp));
        }
        Ok(())
    }

    pub fn expr(&mut self, e: &Expr) -> Result<Value, Diag> {
        self.burn(1, e.span())?;
        match e {
            Expr::Int(v, _) => Ok(Value::Int(*v)),
            Expr::Bool(b, _) => Ok(Value::Bool(*b)),
            Expr::Str(s, sp) => {
                self.burn(text(s.len()), *sp)?;
                Ok(Value::Str(s.clone()))
            }
            Expr::Var(v) => self.read(v),
            Expr::Index(v, i, sp) => {
                let i = int(self.expr(i)?);
                let read = match self.slot(v) {
                    Value::Str(s) => Some((text(s.len()), letter(s, i))),
                    _ => None,
                };
                let Some((cost, got)) = read else { return self.item(v, i, *sp) };
                self.burn(cost, *sp)?;
                got.map_err(|n| {
                    let msg = format!("index {i} is outside `{}`, which has {n} letters", v.name);
                    fault(codes::INDEX_OUT_OF_RANGE, msg, *sp)
                })
            }
            Expr::Call(c) => Ok(self.call(c)?.unwrap_or_else(|| unreachable!("checked: a value"))),
            Expr::List(items, sp) => {
                self.fits(items.len(), *sp)?;
                Ok(Value::List(items.iter().map(|e| self.expr(e)).collect::<Result<_, _>>()?))
            }
            Expr::Fill(item, count, sp) => {
                let item = self.expr(item)?;
                let n = int(self.expr(count)?);
                if n < 0 {
                    let msg = format!("a list's length is {n}");
                    return Err(fault(codes::NEGATIVE_REPEAT, msg, *sp));
                }
                self.fits(usize::try_from(n).unwrap_or(usize::MAX), *sp)?;
                // Every item a copy, paid for before the list is made.
                self.burn((n as u64).saturating_mul(1 + item.cost()), *sp)?;
                Ok(Value::List(vec![item; n as usize]))
            }
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
            Expr::Cond(cond, yes, no, _) => match self.expr(cond)? {
                Value::Bool(true) => self.expr(yes),
                _ => self.expr(no),
            },
            Expr::At(list, i, sp) => {
                let whole = self.expr(list)?;
                let i = int(self.expr(i)?);
                let got = match whole {
                    Value::List(mut items) => {
                        let n = items.len();
                        match usize::try_from(i).ok().filter(|&k| k < n) {
                            Some(k) => Ok(items.swap_remove(k)),
                            None => Err(format!("a list of {n} items")),
                        }
                    }
                    Value::Str(s) => letter(&s, i).map_err(|n| format!("a string of {n} letters")),
                    _ => unreachable!("checked: a list or a string"),
                };
                let outside = |what| format!("index {i} is outside {what}");
                got.map_err(|what| fault(codes::INDEX_OUT_OF_RANGE, outside(what), *sp))
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
            let max = self.lim.max_str_bytes;
            if len > max {
                let msg = format!("string would be {len} bytes; the limit is {max}");
                return Err(fault(codes::STR_TOO_LONG, msg, sp));
            }
            self.burn(text(len), sp)?;
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
                Err(fault(codes::DIV_BY_ZERO, "division or remainder by zero", sp))
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

    /// A call: its value, if it has one.
    fn call(&mut self, c: &Call) -> Result<Option<Value>, Diag> {
        let i = match c.target {
            Target::Builtin(b) => return self.builtin(b, c),
            Target::Fn(i) => i as usize,
            Target::Unresolved => unreachable!("checked: every call resolves"),
        };
        let p = self.p;
        let f = &p.fns()[i];
        let args = c.args.iter().map(|a| self.expr(a)).collect::<Result<Vec<_>, _>>()?;
        if self.depth >= MAX_CALLS {
            let msg = format!("calls nest deeper than {MAX_CALLS}");
            return Err(fault(codes::CALLS_TOO_DEEP, msg, c.span));
        }
        let base = mem::replace(&mut self.base, self.locals.len());
        self.locals.extend(args);
        self.depth += 1;
        let flow = self.block(&f.body);
        self.depth -= 1;
        self.locals.truncate(self.base);
        self.base = base;
        match flow? {
            Flow::Ret(v) => Ok(v),
            // `break` and `continue` stand only in loops (the checker sees to it).
            _ if f.ret.is_none() => Ok(None),
            _ => {
                let msg = format!("`{}` finished without a result", f.name);
                Err(fault(codes::NO_RETURN, msg, c.span))
            }
        }
    }

    fn builtin(&mut self, b: Builtin, c: &Call) -> Result<Option<Value>, Diag> {
        let sp = c.span;
        let list = || match &c.args[0] {
            Expr::Var(v) => v,
            _ => unreachable!("checked: a list's name"),
        };
        let arg = |r: &mut Self, k: usize| r.expr(&c.args[k]);
        let v = match b {
            Builtin::Len => match &c.args[0] {
                // Counted in place: a list's length is known, a string's chars are read.
                Expr::Var(v) => {
                    let (n, cost) = match self.slot(v) {
                        Value::List(items) => (items.len(), 0),
                        Value::Str(s) => (s.chars().count(), text(s.len())),
                        _ => unreachable!("checked: a list or string"),
                    };
                    self.burn(cost, sp)?;
                    n as i64
                }
                e => match self.expr(e)? {
                    Value::List(items) => items.len() as i64,
                    v => v.to_string().chars().count() as i64,
                },
            },
            Builtin::Min => int(arg(self, 0)?).min(int(arg(self, 1)?)),
            Builtin::Max => int(arg(self, 0)?).max(int(arg(self, 1)?)),
            Builtin::Abs => int(arg(self, 0)?).checked_abs().ok_or_else(|| overflow("abs", sp))?,
            Builtin::Random => {
                let n = int(arg(self, 0)?);
                if n < 1 {
                    let msg = format!("random({n}): the bound must be at least 1");
                    return Err(fault(codes::BAD_RANDOM, msg, sp));
                }
                // xorshift64: the seed is state, so a fault rolls it back too.
                let mut x = self.st.seed;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.st.seed = x;
                (x % n as u64) as i64
            }
            Builtin::Parse => {
                let (s, d) = (arg(self, 0)?.to_string(), int(arg(self, 1)?));
                s.trim().parse().unwrap_or(d)
            }
            Builtin::Push | Builtin::Insert => {
                let at = match b {
                    Builtin::Insert => Some(int(arg(self, 1)?)),
                    _ => None,
                };
                let v = arg(self, c.args.len() - 1)?;
                let xs = list();
                let n = self.items(xs).len();
                self.fits(n + 1, sp)?;
                let i = match at {
                    Some(i) => self.index(xs, i, true, sp)?,
                    None => n,
                };
                self.burn((n - i) as u64, sp)?;
                self.items(xs).insert(i, v);
                return Ok(None);
            }
            Builtin::Remove => {
                let i = int(arg(self, 1)?);
                let xs = list();
                let i = self.index(xs, i, false, sp)?;
                let moved = self.items(xs).len() - i;
                self.burn(moved as u64, sp)?;
                self.items(xs).remove(i);
                return Ok(None);
            }
            Builtin::Clear => {
                self.items(list()).clear();
                return Ok(None);
            }
            Builtin::Sin => sine(int(arg(self, 0)?)),
            Builtin::Cos => sine(int(arg(self, 0)?).rem_euclid(360) + 90),
            Builtin::Rect
            | Builtin::Circle
            | Builtin::Ring
            | Builtin::Line
            | Builtin::Text
            | Builtin::Sprite
            | Builtin::Pixels => {
                self.draw(b, c)?;
                return Ok(None);
            }
        };
        Ok(Some(Value::Int(v)))
    }

    /// Draws shape `b` as call `c` gives it (the checker lets only a canvas's call draw): each
    /// number within an i16 and each size never negative, its color 0 to 11, a text one line,
    /// pixels whole rows of cells (see [`Run::pixels`]), and its ink within what the render has
    /// left; else a fault at the call.
    fn draw(&mut self, b: Builtin, c: &Call) -> Result<(), Diag> {
        let (k, sp) = (b as usize - Builtin::Rect as usize, c.span);
        let bad = |msg: String| Err(fault(codes::BAD_DRAW, msg, sp));
        let vals = c.args.iter().map(|a| self.expr(a)).collect::<Result<Vec<_>, _>>()?;
        // The thing first (a text's value, a sprite's rows, pixels' cells), then the numbers
        // and the color.
        let (mut words, first) = match &vals[0] {
            _ if b == Builtin::Text => (vals[0].to_string(), 1),
            Value::List(rows) if b == Builtin::Sprite => {
                let rows: Vec<String> = rows.iter().map(Value::to_string).collect();
                if rows.iter().any(|r| r.contains('\n')) {
                    return bad("a sprite's row is one line: it has a line break".into());
                }
                (rows.join("\n"), 1)
            }
            Value::List(_) if b == Builtin::Pixels => (String::new(), 1),
            _ => (String::new(), 0),
        };
        if b == Builtin::Text && words.contains('\n') {
            return bad("`text` draws one line: its value has a line break".into());
        }
        let names = SHAPES[k].split(", ").skip(first);
        let (mut at, mut color) = ([0i16; 5], 0);
        for (i, (v, name)) in vals[first..].iter().map(|v| int(v.clone())).zip(names).enumerate() {
            if name == "color" {
                if !(0..=11).contains(&v) {
                    return bad(format!("`{}`'s color is {v}: colors are 0 to 11", c.name));
                }
                color = v as u8;
                continue;
            }
            let size = matches!(name, "w" | "h" | "r" | "width" | "size" | "side");
            match i16::try_from(v) {
                Ok(n) if !size || n >= 0 => at[i] = n,
                Ok(_) => {
                    return bad(format!("`{}`'s {name} is {v}: a size is never negative", c.name));
                }
                Err(_) => {
                    return bad(format!(
                        "`{}`'s {name} is {v}: a drawing's numbers are -32,768 to 32,767 (drop \
                         what flies far off)",
                        c.name
                    ));
                }
            }
        }
        if let (Builtin::Pixels, Value::List(cells)) = (b, &vals[0]) {
            words = self.pixels(cells, at[2], sp)?;
        }
        use Shape::*;
        let shape = [Rect, Circle, Ring, Line, Text, Sprite, Pixels][k];
        let d = Draw { shape, color, at, text: words };
        let Some(left) = self.ink.checked_sub(d.ink()) else {
            let msg = format!(
                "one render draws at most {} ink (a shape is 1, a text 1 more a character, a \
                 sprite 1 more a square, pixels 1 more a run of one color in a row): draw less \
                 at once",
                self.lim.max_ink
            );
            return bad(msg);
        };
        self.burn(text(d.text.len()), sp)?;
        self.ink = left;
        self.draws.push(d);
        Ok(())
    }

    /// Pixels' `cells`, `w` a row, as uiwire holds them (a char a cell, [`PAINT`] or `.`): 1 to
    /// [`PIXELS_SIDE`] cells a row, as many whole rows at most, each cell -1 (none) or a color 0
    /// to 11, and no more cells than the render has left; else a fault at `sp`.
    fn pixels(&mut self, cells: &[Value], w: i16, sp: Span) -> Result<String, Diag> {
        let bad = |msg: String| Err(fault(codes::BAD_PIXELS, msg, sp));
        let (n, most) = (cells.len(), PIXELS_SIDE);
        let w = usize::try_from(w).unwrap_or(0);
        if !(1..=most).contains(&w) {
            return bad(format!("pixels are 1 to {most} cells a row, not {w}"));
        }
        if n % w != 0 {
            return bad(format!("{n} cells are no whole rows of {w}: {} left over", n % w));
        }
        if n / w > most {
            return bad(format!("{n} cells, {w} a row, are {} rows: {most} at most", n / w));
        }
        let Some(left) = self.cells.checked_sub(n) else {
            let most = self.lim.max_pixels;
            return bad(format!("one render draws at most {most} cells of pixels: draw fewer"));
        };
        let mut text = String::with_capacity(n);
        for (i, v) in cells.iter().enumerate() {
            match *v {
                Value::Int(-1) => text.push('.'),
                Value::Int(k @ 0..=11) => text.push(char::from(PAINT[k as usize])),
                Value::Int(k) => {
                    return bad(format!("cell {i} is {k}: a cell is -1 (none) or a color 0 to 11"));
                }
                _ => unreachable!("checked: a list of ints"),
            }
        }
        self.cells = left;
        Ok(text)
    }

    pub fn block(&mut self, stmts: &[Stmt]) -> Result<Flow, Diag> {
        let mark = self.locals.len();
        let mut flow = Ok(Flow::Next);
        for s in stmts {
            flow = self.stmt(s);
            if !matches!(flow, Ok(Flow::Next)) {
                break;
            }
        }
        self.locals.truncate(mark);
        flow
    }

    fn stmt(&mut self, s: &Stmt) -> Result<Flow, Diag> {
        self.burn(1, s.span())?;
        match s {
            Stmt::Let { value, .. } => {
                let v = self.expr(value)?;
                self.locals.push(v);
            }
            Stmt::Assign { target, value, .. } => {
                let v = self.expr(value)?;
                *self.slot_mut(target) = v;
            }
            Stmt::SetIndex { target, index, op, value, span } => {
                let i = int(self.expr(index)?);
                let v = match op {
                    None => self.expr(value)?,
                    // The index ran once; the item is read before `value` runs, as `x += v`
                    // reads x first.
                    Some(op) => {
                        let old = self.item(target, i, *span)?;
                        let v = self.expr(value)?;
                        self.binary(*op, old, v, *span)?
                    }
                };
                // `value` may have changed the list's length: check the index again.
                let i = self.index(target, i, false, *span)?;
                self.items(target)[i] = v;
            }
            Stmt::If { arms, els, .. } => {
                for (cond, body) in arms {
                    if self.expr(cond)? == Value::Bool(true) {
                        return self.block(body);
                    }
                }
                return self.block(els);
            }
            Stmt::Repeat { count, body, span } => {
                let n = int(self.expr(count)?);
                if n < 0 {
                    let msg = format!("repeat count is {n}");
                    return Err(fault(codes::NEGATIVE_REPEAT, msg, *span));
                }
                for _ in 0..n {
                    self.burn(1, *span)?;
                    match self.block(body)? {
                        Flow::Ret(v) => return Ok(Flow::Ret(v)),
                        Flow::Break => break,
                        Flow::Next | Flow::Continue => {}
                    }
                }
            }
            Stmt::For { from, to, body, span, .. } => {
                let (from, to) = (int(self.expr(from)?), int(self.expr(to)?));
                for k in from..to {
                    self.burn(1, *span)?;
                    self.locals.push(Value::Int(k));
                    let flow = self.block(body);
                    self.locals.pop();
                    match flow? {
                        Flow::Ret(v) => return Ok(Flow::Ret(v)),
                        Flow::Break => break,
                        Flow::Next | Flow::Continue => {}
                    }
                }
            }
            // As long as its condition holds, and its fuel lasts: a loop that never ends faults.
            Stmt::While { cond, body, span } => loop {
                self.burn(1, *span)?;
                if self.expr(cond)? != Value::Bool(true) {
                    break;
                }
                match self.block(body)? {
                    Flow::Ret(v) => return Ok(Flow::Ret(v)),
                    Flow::Break => break,
                    Flow::Next | Flow::Continue => {}
                }
            },
            Stmt::Break(_) => return Ok(Flow::Break),
            Stmt::Continue(_) => return Ok(Flow::Continue),
            Stmt::Return { value, .. } => {
                let v = value.as_ref().map(|e| self.expr(e)).transpose()?;
                return Ok(Flow::Ret(v));
            }
            Stmt::Call(c) => _ = self.call(c)?,
        }
        Ok(Flow::Next)
    }
}

fn overflow(op: &str, sp: Span) -> Diag {
    fault(codes::OVERFLOW, format!("`{op}` overflowed the 64-bit integer range"), sp)
}

/// One render: a run, the bytes of text left, and the handlers shown so far.
pub(crate) struct Render<'a> {
    pub run: Run<'a>,
    pub left: usize,
    pub shown: Vec<Shown>,
}

impl Render<'_> {
    /// Takes `n` bytes of text from those left, or faults at `sp`.
    fn spend(&mut self, n: usize, sp: Span) -> Result<(), Diag> {
        let msg = "render output exceeds the byte limit";
        let err = || fault(codes::RENDER_TOO_BIG, msg, sp);
        self.left = self.left.checked_sub(n).ok_or_else(err)?;
        Ok(())
    }

    /// Notes handler `id` shown with what it sees (the `cells` of a grid or canvas, a canvas's
    /// `cols`); its instance number.
    fn show(&mut self, id: u32, cells: Option<u32>, cols: u32) -> u32 {
        let captured = self.run.locals.clone();
        self.shown.push(Shown { id, cells, cols, captured });
        self.shown.len() as u32 - 1
    }

    pub fn widgets(&mut self, ws: &[Widget], out: &mut Vec<Node>) -> Result<(), Diag> {
        ws.iter().try_for_each(|w| self.widget(w, out))
    }

    fn widget(&mut self, w: &Widget, out: &mut Vec<Node>) -> Result<(), Diag> {
        let sp = w.span();
        self.run.burn(1, sp)?;
        match w {
            Widget::Label { value, .. } => {
                let text = self.run.expr(value)?.to_string();
                self.spend(text.len(), sp)?;
                out.push(Node::Label { text });
            }
            Widget::Button { text, handler, .. } => {
                let text = self.run.expr(text)?.to_string();
                self.spend(text.len(), sp)?;
                out.push(Node::Button { text, id: self.show(handler.id, None, 0) });
            }
            Widget::Input { state, .. } => {
                let Value::Str(value) = self.run.slot(state).clone() else {
                    unreachable!("checked: input binds a string state")
                };
                self.spend(state.name.len() + value.len(), sp)?;
                out.push(Node::Input { state: state.name.clone(), value });
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
                    if self.run.expr(cond)? == Value::Bool(true) {
                        return self.widgets(body, out);
                    }
                }
                self.widgets(els, out)?;
            }
            Widget::For { from, to, body, .. } => {
                let (from, to) = (int(self.run.expr(from)?), int(self.run.expr(to)?));
                for k in from..to {
                    self.run.burn(1, sp)?;
                    self.run.locals.push(Value::Int(k));
                    let r = self.widgets(body, out);
                    self.run.locals.pop();
                    r?;
                }
            }
            Widget::Grid { cols, cells, texts, handler, .. } => {
                let node = self.grid(cols, cells, texts.as_ref(), sp)?;
                let (cols, cells, texts) = node;
                let n = cells.len() as u32;
                let id = handler.as_ref().map(|h| self.show(h.id, Some(n), 0));
                out.push(Node::Grid { id, cols, cells, texts });
            }
            Widget::Canvas { w, h, scene, handler, .. } => {
                let (w, h) = (int(self.run.expr(w)?), int(self.run.expr(h)?));
                if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
                    let msg = format!("a canvas is 1 to 1,024 units a side, not {w} x {h}");
                    return Err(fault(codes::BAD_DRAW, msg, sp));
                }
                self.run.draws.clear();
                self.run.call(scene)?;
                let draws = mem::take(&mut self.run.draws);
                // A draw's 12 bytes and its text.
                self.spend(draws.iter().map(|d| 12 + d.text.len()).sum(), sp)?;
                let (cells, cols) = ((w * h) as u32, w as u32);
                let id = handler.as_ref().map(|h| self.show(h.id, Some(cells), cols));
                out.push(Node::Canvas { id, w: w as u16, h: h as u16, draws });
            }
        }
        Ok(())
    }

    /// A grid's columns, squares and texts, checked: 1 to 100 columns, squares 0 to 8, a text
    /// per square (or none).
    fn grid(
        &mut self,
        cols: &Expr,
        cells: &Expr,
        texts: Option<&Expr>,
        sp: Span,
    ) -> Result<(u16, Vec<u8>, Vec<String>), Diag> {
        let bad = |msg: String| Err(fault(codes::BAD_GRID, msg, sp));
        let n = int(self.run.expr(cols)?);
        if !(1..=MAX_COLS).contains(&n) {
            return bad(format!("a grid has 1 to {MAX_COLS} columns, not {n}"));
        }
        let Value::List(items) = self.run.expr(cells)? else { unreachable!("checked: a list") };
        let mut squares = Vec::with_capacity(items.len());
        for (i, v) in items.iter().enumerate() {
            match int(v.clone()) {
                v @ 0..=8 => squares.push(v as u8),
                v => return bad(format!("square {i} is {v}; a grid's squares are 0 to 8")),
            }
        }
        let texts = match texts.map(|t| self.run.expr(t)).transpose()? {
            Some(Value::List(t)) if t.len() == squares.len() => {
                t.into_iter().map(|v| v.to_string()).collect()
            }
            Some(Value::List(t)) => {
                let (a, b) = (t.len(), squares.len());
                return bad(format!("a grid has {b} squares but {a} texts: give one per square"));
            }
            _ => Vec::new(),
        };
        self.spend(squares.len() + texts.iter().map(String::len).sum::<usize>(), sp)?;
        Ok((n as u16, squares, texts))
    }
}

/// The handler with id `id`: its statements, and whether a grid's or a canvas's (it sees
/// where it was tapped).
pub(crate) fn handler(ws: &[Widget], id: u32) -> Option<(&[Stmt], bool)> {
    ws.iter().find_map(|w| match w {
        Widget::Button { handler: h, .. } if h.id == id => Some((&h.body[..], false)),
        Widget::Grid { handler: Some(h), .. } | Widget::Canvas { handler: Some(h), .. }
            if h.id == id =>
        {
            Some((&h.body[..], true))
        }
        Widget::Row { children, .. }
        | Widget::Col { children, .. }
        | Widget::For { body: children, .. } => handler(children, id),
        Widget::If { arms, els, .. } => {
            arms.iter().find_map(|(_, body)| handler(body, id)).or_else(|| handler(els, id))
        }
        _ => None,
    })
}

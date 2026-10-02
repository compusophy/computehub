//! # applang-syntax: the applang front end
//!
//! The lexer, parser and static checker behind [`compile`], the verify step
//! of generate, verify, run. The runtime and the language card live in the
//! `applang` crate, which re-exports [`compile`], [`Program`] and [`codes`].
//! [`ast`] is the read-only tree the runtime walks, its names resolved to slots; only
//! [`compile`] builds a [`Program`], so every one has been type-checked. [`codes`] is the one
//! diagnostic table for both crates. [`highlight`] classes every token and comment for an
//! editor, and never fails. [`literals`] reads `name = literal;` lines (a saved state).
//!
//! Forked from litelite's applite 0.2.0 (commit 4f5e056), whose lexer,
//! parser, checker and diagnostic codes this crate holds.

#![forbid(unsafe_code)]

mod check;
mod lex;
mod parse;
#[cfg(test)]
mod tests;

pub use check::KEYS;
pub use lang::{Diag, Span};
pub use lex::{Class, highlight};
pub use parse::{Program, literals};

/// The parsed tree, read-only, for the `applang` runtime.
pub mod ast {
    pub use crate::parse::{BUILTINS, MAX_ITEMS, MAX_STATE_BYTES};
    pub use crate::parse::{BinOp, Builtin, Call, Every, Expr, FnDecl, Handler, Lit, OnKey};
    pub use crate::parse::{Slot, StateDecl, Stmt, Target, Type, UnOp, Var, Widget};
}

/// Stable diagnostic codes: lex `E00xx`, parse `E01xx`, runtime `E02xx`, check
/// `E03xx`. `BAD_EVENT`, and `STATE_TOO_BIG` at commit, have no span (no source text).
pub mod codes {
    pub const UNEXPECTED_CHAR: u16 = 1;
    pub const UNTERMINATED_COMMENT: u16 = 2;
    /// A malformed or out-of-range integer literal.
    pub const BAD_INT: u16 = 3;
    /// A string with no closing `"` on its line.
    pub const UNTERMINATED_STRING: u16 = 4;
    /// An escape other than `\"`, `\\` and `\n`.
    pub const BAD_ESCAPE: u16 = 5;
    pub const UNEXPECTED_TOKEN: u16 = 101;
    /// Nesting past the `lang::parse` depth cap.
    pub const TOO_DEEP: u16 = 102;
    pub const DIV_BY_ZERO: u16 = 203;
    /// Arithmetic left the 64-bit integer range.
    pub const OVERFLOW: u16 = 204;
    /// A negative `repeat` count or list length.
    pub const NEGATIVE_REPEAT: u16 = 205;
    /// A render or handler ran out of fuel and was stopped.
    pub const FUEL_EXHAUSTED: u16 = 206;
    /// A string value past `Limits::max_str_bytes`.
    pub const STR_TOO_LONG: u16 = 211;
    /// State past `Limits::max_state_bytes` at commit, or declared past
    /// [`crate::ast::MAX_STATE_BYTES`].
    pub const STATE_TOO_BIG: u16 = 212;
    /// A host event that no widget or state matches.
    pub const BAD_EVENT: u16 = 213;
    /// One render's text past `Limits::max_render_bytes`.
    pub const RENDER_TOO_BIG: u16 = 214;
    /// An index outside its list.
    pub const INDEX_OUT_OF_RANGE: u16 = 215;
    /// A list past [`crate::ast::MAX_ITEMS`] items.
    pub const LIST_FULL: u16 = 216;
    /// `random(n)` with `n` under 1.
    pub const BAD_RANDOM: u16 = 217;
    /// A function with a result finished without one (the checker rules this out).
    pub const NO_RETURN: u16 = 218;
    /// A grid with no columns or too many, a square past 8, or texts not one per square.
    pub const BAD_GRID: u16 = 219;
    /// Calls nested past the runtime's depth cap.
    pub const CALLS_TOO_DEEP: u16 = 220;
    /// An app whose first render shows nothing (the smoke test's; no span).
    pub const SHOWS_NOTHING: u16 = 221;
    /// A name declared twice (a state, function or parameter).
    pub const DUP_STATE: u16 = 301;
    /// A name that is no declared state or visible local.
    pub const UNKNOWN_NAME: u16 = 302;
    pub const TYPE_MISMATCH: u16 = 303;
    /// A function's call to a function defined below it (or to itself).
    pub const CALL_BELOW: u16 = 304;
    /// A function with a result that does not end in `return`.
    pub const MISSING_RETURN: u16 = 305;
    /// A widget or interval that would change state.
    pub const IMPURE_RENDER: u16 = 306;
    /// An `on key` naming no key.
    pub const BAD_KEY: u16 = 307;
    /// A call with the wrong number of arguments.
    pub const ARITY: u16 = 308;
}

/// Parses and statically checks `src`: the verify step. Running the [`Program`] can then only
/// fault on arithmetic, indexes, list and string bounds, grids, `random`'s bound or fuel.
pub fn compile(src: &str) -> Result<Program, Diag> {
    let mut program = parse::parse(src)?;
    check::check(&mut program)?;
    Ok(program)
}

//! # applang-syntax — the applang front end
//!
//! The lexer ([`lex`]), parser and static checker behind [`compile`], the
//! verify step of generate → verify → run. The runtime (`App`, `Event`,
//! `Limits`, `Node`, `Value`) and the language card (`REFERENCE`) live in
//! the `applang` crate, which re-exports this crate's public items (all but
//! [`ast`]); most callers want `applang`.
//!
//! [`codes`] is the one diagnostic table for both crates, runtime codes
//! included. [`ast`] exposes the parsed tree read-only so the runtime can
//! walk it; a [`Program`] can only be built by [`compile`], so every one
//! has been type-checked (the parser alone is not public: the runtime's
//! invariants assume the checker ran).
//!
//! Forked from litelite's applite 0.2.0 (commit 4f5e056), whose lexer,
//! parser, checker and diagnostic codes this crate holds.

#![forbid(unsafe_code)]

mod check;
mod lex;
mod parse;

pub use check::Type;
pub use lang::{Diag, Span};
pub use lex::{TokKind, Token, lex};
pub use parse::Program;

/// The parsed tree, read-only: what [`Program::states`] and
/// [`Program::widgets`] hand out. Public so the `applang` runtime can walk
/// it; only [`compile`] builds a [`Program`], and it type-checks what it
/// builds.
pub mod ast {
    pub use crate::parse::{BinOp, Expr, Lit, StateDecl, Stmt, UnOp, Widget};
}

/// Stable diagnostic codes, banded by stage: lex `E00xx`, parse `E01xx`,
/// runtime `E02xx`, static check `E03xx`. `BAD_EVENT` and `STATE_TOO_BIG`
/// are spanless — they locate a host event or a commit, not source text.
pub mod codes {
    /// A character that starts no applang token.
    pub const UNEXPECTED_CHAR: u16 = 1;
    /// `/*` without its matching `*/`.
    pub const UNTERMINATED_COMMENT: u16 = 2;
    /// Malformed or out-of-range integer literal.
    pub const BAD_INT: u16 = 3;
    /// `"` without its closing `"` on the same line.
    pub const UNTERMINATED_STRING: u16 = 4;
    /// A `\` escape that is not `\"`, `\\`, or `\n`.
    pub const BAD_ESCAPE: u16 = 5;
    /// The parser needed a different token (the message names both sides).
    pub const UNEXPECTED_TOKEN: u16 = 101;
    /// Source nests deeper than the `lang::parse` depth cap.
    pub const TOO_DEEP: u16 = 102;
    /// `/` or `%` with a zero divisor.
    pub const DIV_BY_ZERO: u16 = 203;
    /// Arithmetic left the 64-bit integer range.
    pub const OVERFLOW: u16 = 204;
    /// `repeat` with a negative count.
    pub const NEGATIVE_REPEAT: u16 = 205;
    /// The fuel tank ran dry — the render or handler was stopped, as promised.
    pub const FUEL_EXHAUSTED: u16 = 206;
    /// A string value would exceed `applang::Limits::max_str_bytes`.
    pub const STR_TOO_LONG: u16 = 211;
    /// Committed string state would exceed `applang::Limits::max_state_bytes`.
    pub const STATE_TOO_BIG: u16 = 212;
    /// The host sent an event no widget or state matches.
    pub const BAD_EVENT: u16 = 213;
    /// One render's text would exceed `applang::Limits::max_render_bytes`.
    pub const RENDER_TOO_BIG: u16 = 214;
    /// The same state name declared twice.
    pub const DUP_STATE: u16 = 301;
    /// A name that is no declared state or visible local.
    pub const UNKNOWN_NAME: u16 = 302;
    /// An operator, condition, binding, or assignment got the wrong type.
    pub const TYPE_MISMATCH: u16 = 303;
}

/// Parse AND statically check `src` — the verify step. A [`Program`] you
/// hold has passed both; running it can only fault on arithmetic, fuel, or
/// string bounds, and those roll back.
///
/// ```compile_fail,E0423
/// // There is no public parse-only route to a `Program`.
/// let _ = applang_syntax::parse("label x;");
/// ```
pub fn compile(src: &str) -> Result<Program, Diag> {
    let program = parse::parse(src)?;
    check::check(&program)?;
    Ok(program)
}

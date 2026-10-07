//! # applang-syntax: the applang front end
//!
//! The parser and static checker behind [`compile`] (on `applang-lex`'s tokens), the verify step
//! of generate, verify, run. The runtime and the language card live in the
//! `applang` crate, which re-exports [`compile`], [`Program`] and [`codes`].
//! [`ast`] is the read-only tree the runtime walks, its names resolved to slots; only
//! [`compile`] builds a [`Program`], so every one has been type-checked. [`codes`] is the one
//! diagnostic table for both crates. [`highlight`] classes every token and comment for an
//! editor, and never fails. [`literals`] reads `name = literal;` lines (a saved state).
//!
//! Forked from litelite's applite 0.2.0 (commit 4f5e056), whose parser and
//! checker this crate holds (its lexer and codes: `applang-lex`).

#![forbid(unsafe_code)]

mod check;
mod parse;
#[cfg(test)]
mod tests;

pub use applang_lex::{Class, codes, highlight};
pub use check::KEYS;
pub use lang::{Diag, Span};
pub use parse::{Program, literals};

/// The parsed tree, read-only, for the `applang` runtime.
pub mod ast {
    pub use crate::parse::{BUILTINS, DRAWN, MAX_ITEMS, MAX_STATE_BYTES, SHAPES};
    pub use crate::parse::{BinOp, Builtin, Call, Every, Expr, FnDecl, Handler, Lit, OnKey};
    pub use crate::parse::{Slot, StateDecl, Stmt, Target, Type, UnOp, Var, Widget};
}

/// Parses and statically checks `src`: the verify step. Running the [`Program`] can then only
/// fault on arithmetic, indexes, list and string bounds, grids, canvases' draws (pixels too),
/// `random`'s bound or fuel.
pub fn compile(src: &str) -> Result<Program, Diag> {
    let mut program = parse::parse(src)?;
    check::check(&mut program)?;
    Ok(program)
}

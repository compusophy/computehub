//! Language front-end kit for compusophyOS: spanned, coded diagnostics
//! ([`diag`]), a UTF-8-safe byte-cursor lexer ([`lex`]), and a depth-guarded
//! recursive-descent harness ([`parse`]). Zero dependencies, native + wasm32.
//!
//! [`Diag`] and [`Span`] are re-exported here; `lex` and `parse` also
//! re-export [`Span`], so each module alone is enough to name one.
//!
//! Forked from litelite's diaglite, lexlite and parselite 0.2.0
//! (commit 4f5e056), merged into one crate.

pub mod diag;
pub mod lex;
pub mod parse;

pub use diag::{Diag, Span};

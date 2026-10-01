//! Language front-end kit for compusophyOS: spanned, coded diagnostics
//! ([`diag`]), a UTF-8-safe byte-cursor lexer ([`lex`]) and a depth-guarded
//! recursive-descent token cursor ([`parse`]). Zero dependencies, native
//! and wasm32. Invariants: no primitive panics on a span inside a multi-byte
//! char, and recursion enters only through the depth guard.
//!
//! Forked from litelite's diaglite, lexlite and parselite 0.2.0
//! (commit 4f5e056), merged into one crate.

#![forbid(unsafe_code)]

pub mod diag;
pub mod lex;
pub mod parse;

pub use diag::{Diag, Span};

//! # applang-syntax: the applang front end
//!
//! The lexer, parser and static checker behind [`compile`], the verify step
//! of generate, verify, run. The runtime and the language card live in the
//! `applang` crate, which re-exports [`compile`], [`Program`] and [`codes`].
//! [`ast`] is the read-only tree the runtime walks; only [`compile`] builds a
//! [`Program`], so every one has been type-checked. [`codes`] is the one
//! diagnostic table for both crates. [`highlight`] classes every token and
//! comment for an editor, and never fails.
//!
//! Forked from litelite's applite 0.2.0 (commit 4f5e056), whose lexer,
//! parser, checker and diagnostic codes this crate holds.

#![forbid(unsafe_code)]

mod check;
mod lex;
mod parse;

pub use lang::{Diag, Span};
pub use lex::{Class, highlight};
pub use parse::Program;

/// The parsed tree, read-only, for the `applang` runtime.
pub mod ast {
    pub use crate::parse::{BinOp, Expr, Lit, StateDecl, Stmt, UnOp, Widget};
}

/// Stable diagnostic codes: lex `E00xx`, parse `E01xx`, runtime `E02xx`, check
/// `E03xx`. `STATE_TOO_BIG` and `BAD_EVENT` have no span (no source text).
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
    pub const NEGATIVE_REPEAT: u16 = 205;
    /// A render or handler ran out of fuel and was stopped.
    pub const FUEL_EXHAUSTED: u16 = 206;
    /// A string value past `Limits::max_str_bytes`.
    pub const STR_TOO_LONG: u16 = 211;
    /// String state past `Limits::max_state_bytes` at commit.
    pub const STATE_TOO_BIG: u16 = 212;
    /// A host event that no widget or state matches.
    pub const BAD_EVENT: u16 = 213;
    /// One render's text past `Limits::max_render_bytes`.
    pub const RENDER_TOO_BIG: u16 = 214;
    pub const DUP_STATE: u16 = 301;
    /// A name that is no declared state or visible local.
    pub const UNKNOWN_NAME: u16 = 302;
    pub const TYPE_MISMATCH: u16 = 303;
}

/// Parses and statically checks `src`: the verify step. Running the
/// [`Program`] can then only fault on arithmetic, fuel or string bounds.
pub fn compile(src: &str) -> Result<Program, Diag> {
    let program = parse::parse(src)?;
    check::check(&program)?;
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::ast::{Lit, Widget};
    use super::codes::*;
    use super::*;

    #[test]
    fn every_failure_is_coded_and_a_whole_app_compiles() {
        // Deep widget nesting trips the depth cap, never a stack overflow,
        // and so do long flat operator chains (the AST spine eval walks).
        let deep = format!("{}label 1;{}", "row {".repeat(200), "}".repeat(200));
        let chain = format!("label {}0;", "1+".repeat(500));
        #[rustfmt::skip]
        let cases = [
            (&deep[..], TOO_DEEP), (&chain[..], TOO_DEEP),
            ("\"open", UNTERMINATED_STRING), ("\"line\nbreak\"", UNTERMINATED_STRING),
            ("\"bad \\q escape\"", BAD_ESCAPE), ("123abc", BAD_INT),
            ("label 99999999999999999999;", BAD_INT), ("@", UNEXPECTED_CHAR),
            ("/* open", UNTERMINATED_COMMENT),
            ("label 1; state x = 0;", UNEXPECTED_TOKEN), ("state x = y;", UNEXPECTED_TOKEN),
            ("state x = -true;", UNEXPECTED_TOKEN), ("button { }", UNEXPECTED_TOKEN),
            ("label 1", UNEXPECTED_TOKEN), ("row label 1; }", UNEXPECTED_TOKEN),
            ("widget", UNEXPECTED_TOKEN),
            ("label nope;", UNKNOWN_NAME), ("state x = 1; button \"b\" { y = 2; }", UNKNOWN_NAME),
            ("input missing;", UNKNOWN_NAME), ("state x = 1; state x = 2;", DUP_STATE),
            ("state n = 0; input n;", TYPE_MISMATCH), ("if 1 { label 1; }", TYPE_MISMATCH),
            ("state x = 1; button \"b\" { x = \"s\"; }", TYPE_MISMATCH),
            ("state x = 1; button \"b\" { repeat true { } }", TYPE_MISMATCH),
            ("label 1 == \"1\";", TYPE_MISMATCH), ("label true + true;", TYPE_MISMATCH),
            ("label -true;", TYPE_MISMATCH),
            // A block-local disappears when its block ends.
            ("state x = 1; button \"b\" { if true { let t = 1; } x = t; }", UNKNOWN_NAME),
        ];
        for (src, want) in cases {
            assert_eq!(compile(src).unwrap_err().code, Some(want), "{src}");
        }
        let e = compile("label 1; state x = 0;").unwrap_err();
        assert!(e.message.contains("before the first widget"), "{e}");
        // A whole app compiles.
        let p = compile(
            "state count = 0; state name = \"w\\\"o\\\\r\\nld\"; state neg = -1_000;
             label \"Counter\"; /* a /* nested */ comment */
             row { button \"-\" { count = count - 1; } label count; }
             input name; // a line comment
             if count > 1000 { label \"big \" + count; } else if (count == 0) { } else { }",
        )
        .unwrap();
        let inits: Vec<_> = p.states().iter().map(|s| s.init.clone()).collect();
        assert_eq!(inits, [Lit::Int(0), Lit::Str("w\"o\\r\nld".into()), Lit::Int(-1000)]);
        assert_eq!(p.widgets().len(), 4);
        assert!(matches!(&p.widgets()[3], Widget::If { arms, .. } if arms.len() == 2));
        // `+` with a string concatenates; locals may shadow a state with another type.
        assert!(compile("state s = \"x\"; label 1 + 2; label \"n = \" + s;").is_ok());
        assert!(compile("state x = 1; button \"b\" { let x = \"s\"; x = \"t\"; }").is_ok());
        // Spans count bytes and cover multi-byte chars whole.
        let span = |src| compile(src).map(drop).unwrap_err().span;
        assert_eq!(span("label \"é\" é;"), Some(Span::new(11, 13)));
        assert_eq!(span("label !\"é\";"), Some(Span::new(6, 11)));
        // A short operator chain is within the depth cap.
        assert!(compile(&format!("label {}0;", "1+".repeat(40))).is_ok());
    }

    #[test]
    fn highlight_classes_every_token_and_goes_on_past_errors() {
        use Class::*;
        let src =
            "state s = \"é\\\"\"; // hi\nlabel s+12 @ é /* a */ true 1x \"a\\q\";\n\"open\n/* to";
        let got: Vec<_> =
            highlight(src).into_iter().map(|(s, c)| (&src[s.start..s.end], c)).collect();
        #[rustfmt::skip]
        let want = [
            ("state", Keyword), ("s", Name), ("=", Punct), ("\"é\\\"\"", Str), (";", Punct),
            ("// hi", Comment), ("label", Keyword), ("s", Name), ("+", Punct), ("12", Number),
            ("@", Error), ("é", Error), ("/* a */", Comment), ("true", Keyword), ("1x", Error),
            ("\"a\\q", Error), ("\";", Error), ("\"open", Error), ("/* to", Comment),
        ];
        assert_eq!(got, want);
        // On any input: no panic; ranges in order, apart, non-empty, on char boundaries.
        let alphabet = ["a", "1", " ", "\n", "\"", "\\", "/", "*", "é", "😀", "=", ";", "_", "n"];
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) as usize % n
        };
        for _ in 0..3000 {
            let src: String = (0..next(24)).map(|_| alphabet[next(alphabet.len())]).collect();
            let mut end = 0;
            for (span, _) in highlight(&src) {
                assert!(end <= span.start && span.start < span.end && span.end <= src.len());
                assert!(src.is_char_boundary(span.start) && src.is_char_boundary(span.end));
                end = span.end;
            }
        }
    }
}

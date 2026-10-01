//! The applang lexer: source text to spanned tokens on the `lang::lex`
//! cursor. Tokens are `Copy`: `Ident` and `Str` text is re-sliced from the
//! source by span, and strings are unescaped at parse.

use lang::lex::{Cursor, ident_cont, ident_start};
use lang::{Diag, Span};

use crate::codes;

/// A token kind; `Int` carries its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub(crate) enum TokKind {
    Int(i64), Ident, Str, True, False,
    State, Label, Button, Input, Row, Col, Let, If, Else, Repeat,
    Plus, Minus, Star, Slash, Percent, Bang, BangEq, Assign, EqEq,
    Lt, LtEq, Gt, GtEq, AndAnd, OrOr, LParen, RParen, LBrace, RBrace, Semi, Eof,
}
use TokKind::*;

#[rustfmt::skip]
const KEYWORDS: [(&str, TokKind); 12] = [
    ("state", State), ("label", Label), ("button", Button), ("input", Input), ("row", Row),
    ("col", Col), ("let", Let), ("if", If), ("else", Else), ("repeat", Repeat),
    ("true", True), ("false", False),
];

/// Two-char operators first, so the longest match wins.
#[rustfmt::skip]
const PUNCT: [(&str, TokKind); 20] = [
    ("&&", AndAnd), ("||", OrOr), ("==", EqEq), ("!=", BangEq), ("<=", LtEq), (">=", GtEq),
    ("+", Plus), ("-", Minus), ("*", Star), ("/", Slash), ("%", Percent), ("!", Bang),
    ("=", Assign), ("<", Lt), (">", Gt), ("(", LParen), (")", RParen), ("{", LBrace),
    ("}", RBrace), (";", Semi),
];

/// A spanned token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token {
    pub kind: TokKind,
    pub span: Span,
}

impl lang::parse::Tok for Token {
    fn span(&self) -> Span {
        self.span
    }
}

/// Lexes `src` into tokens ending with an `Eof` sentinel. Trivia:
/// whitespace, `//` line comments and nested `/* */` block comments.
pub(crate) fn lex(src: &str) -> Result<Vec<Token>, Diag> {
    let mut cur = Cursor::new(src);
    let mut toks = Vec::new();
    loop {
        cur.skip_ws();
        if cur.skip_line_comment("//") {
            continue;
        }
        match cur.skip_block_comment("/*", "*/") {
            Ok(true) => continue,
            Ok(false) => {}
            Err(sp) => {
                let msg = "unterminated block comment";
                return Err(Diag::at_code(codes::UNTERMINATED_COMMENT, msg, sp));
            }
        }
        if cur.at_eof() {
            toks.push(Token { kind: Eof, span: Span::new(src.len(), src.len()) });
            return Ok(toks);
        }
        let start = cur.pos();
        let kind = next_kind(&mut cur, start)?;
        toks.push(Token { kind, span: cur.span_from(start) });
    }
}

fn next_kind(cur: &mut Cursor<'_>, start: usize) -> Result<TokKind, Diag> {
    if let Some(sp) = cur.eat_ident() {
        let word = cur.text(sp);
        return Ok(KEYWORDS.iter().find(|(k, _)| *k == word).map_or(Ident, |&(_, kind)| kind));
    }
    if let Some(digits) = cur.eat_decimal() {
        return int_literal(cur, start, digits);
    }
    if cur.peek() == Some(b'"') {
        return str_literal(cur, start);
    }
    for (text, kind) in PUNCT {
        if cur.eat_str(text) {
            return Ok(kind);
        }
    }
    // Span the whole char, never a byte inside one; `lex` saw it is not EOF.
    let c = cur.next_char().unwrap_or_default();
    let msg = format!("unexpected character `{c}`");
    Err(Diag::at_code(codes::UNEXPECTED_CHAR, msg, cur.span_from(start)))
}

/// A single-line `"…"` literal; its span includes the quotes. Only `\"`,
/// `\\` and `\n` escapes exist; [`unescape`] decodes them at parse.
fn str_literal(cur: &mut Cursor<'_>, start: usize) -> Result<TokKind, Diag> {
    cur.bump();
    loop {
        match cur.peek() {
            None | Some(b'\n') => {
                let (msg, sp) = ("unterminated string literal", Span::new(start, start + 1));
                return Err(Diag::at_code(codes::UNTERMINATED_STRING, msg, sp));
            }
            Some(b'"') => {
                cur.bump();
                return Ok(Str);
            }
            Some(b'\\') => {
                cur.bump();
                if !matches!(cur.peek(), Some(b'"' | b'\\' | b'n')) {
                    let esc = cur.pos() - 1;
                    cur.next_char();
                    let msg = "unknown escape (only \\\" \\\\ \\n exist)";
                    return Err(Diag::at_code(codes::BAD_ESCAPE, msg, cur.span_from(esc)));
                }
                cur.bump();
            }
            Some(_) => _ = cur.next_char(),
        }
    }
}

/// Decodes a `Str` token's source (quotes included); the lexer already
/// rejected every other escape.
pub(crate) fn unescape(quoted: &str) -> String {
    let mut out = String::with_capacity(quoted.len());
    let mut chars = quoted[1..quoted.len() - 1].chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
        } else if let Some(e) = chars.next() {
            out.push(if e == 'n' { '\n' } else { e });
        }
    }
    out
}

/// Decimal with `_` separators. A digit run running into ident chars
/// (`123abc`) is one malformed literal, never two tokens.
fn int_literal(cur: &mut Cursor<'_>, start: usize, digits: Span) -> Result<TokKind, Diag> {
    if cur.peek().is_some_and(ident_start) {
        cur.eat_while(ident_cont);
        let msg = "malformed integer literal";
        return Err(Diag::at_code(codes::BAD_INT, msg, cur.span_from(start)));
    }
    let text: String = cur.text(digits).chars().filter(|&c| c != '_').collect();
    text.parse().map(Int).map_err(|_| {
        let msg = format!("integer literal out of range (max {})", i64::MAX);
        Diag::at_code(codes::BAD_INT, msg, cur.span_from(start))
    })
}

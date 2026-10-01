//! applang lexer: source text → spanned tokens, on the `lang::lex` cursor.
//! The lexer of litelite's prooflite (not forked here) plus ONE addition:
//! string literals (`"…"` with `\"`, `\\`, and `\n` escapes, single-line). The token stays `Copy` — `Str`
//! carries no text; the parser re-slices the span and unescapes there.

use lang::lex::{Cursor, ident_cont, ident_start};
use lang::{Diag, Span};

use crate::codes;

/// A token kind. `Int` carries its parsed value; `Ident` and `Str` text is
/// recovered by slicing the source with the token's span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    Int(i64),
    Ident,
    Str,
    True,
    False,
    State,
    Label,
    Button,
    Input,
    Row,
    Col,
    Let,
    If,
    Else,
    Repeat,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    BangEq,
    Assign,
    EqEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Semi,
    Eof,
}

/// A spanned token — the slice a [`lang::parse::TokCursor`] rides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

impl lang::parse::Tok for Token {
    fn span(&self) -> Span {
        self.span
    }
}

/// Lex `src` into tokens, ending with a [`TokKind::Eof`] sentinel. Trivia:
/// whitespace, `//` line comments, nested `/* */` block comments.
pub fn lex(src: &str) -> Result<Vec<Token>, Diag> {
    let mut cur = Cursor::new(src);
    let mut toks = Vec::new();
    loop {
        skip_trivia(&mut cur)?;
        if cur.at_eof() {
            toks.push(Token { kind: TokKind::Eof, span: Span::new(src.len(), src.len()) });
            return Ok(toks);
        }
        let start = cur.pos();
        let kind = next_kind(&mut cur)?;
        toks.push(Token { kind, span: cur.span_from(start) });
    }
}

fn skip_trivia(cur: &mut Cursor<'_>) -> Result<(), Diag> {
    loop {
        cur.skip_ws();
        if cur.skip_line_comment("//") {
            continue;
        }
        match cur.skip_block_comment("/*", "*/", true) {
            Ok(true) => continue,
            Ok(false) => return Ok(()),
            Err(sp) => {
                return Err(Diag::at_code(
                    codes::UNTERMINATED_COMMENT,
                    "unterminated block comment",
                    sp,
                ));
            }
        }
    }
}

fn keyword(text: &str) -> Option<TokKind> {
    Some(match text {
        "state" => TokKind::State,
        "label" => TokKind::Label,
        "button" => TokKind::Button,
        "input" => TokKind::Input,
        "row" => TokKind::Row,
        "col" => TokKind::Col,
        "let" => TokKind::Let,
        "if" => TokKind::If,
        "else" => TokKind::Else,
        "repeat" => TokKind::Repeat,
        "true" => TokKind::True,
        "false" => TokKind::False,
        _ => return None,
    })
}

fn next_kind(cur: &mut Cursor<'_>) -> Result<TokKind, Diag> {
    if let Some(sp) = cur.eat_ident(ident_start, ident_cont) {
        return Ok(keyword(cur.text(sp)).unwrap_or(TokKind::Ident));
    }
    if cur.peek().is_some_and(|b| b.is_ascii_digit()) {
        return int_literal(cur);
    }
    if cur.peek() == Some(b'"') {
        return str_literal(cur);
    }
    for (text, kind) in [
        ("&&", TokKind::AndAnd),
        ("||", TokKind::OrOr),
        ("==", TokKind::EqEq),
        ("!=", TokKind::BangEq),
        ("<=", TokKind::LtEq),
        (">=", TokKind::GtEq),
        ("+", TokKind::Plus),
        ("-", TokKind::Minus),
        ("*", TokKind::Star),
        ("/", TokKind::Slash),
        ("%", TokKind::Percent),
        ("!", TokKind::Bang),
        ("=", TokKind::Assign),
        ("<", TokKind::Lt),
        (">", TokKind::Gt),
        ("(", TokKind::LParen),
        (")", TokKind::RParen),
        ("{", TokKind::LBrace),
        ("}", TokKind::RBrace),
        (";", TokKind::Semi),
    ] {
        if cur.eat_str(text) {
            return Ok(kind);
        }
    }
    // Not an applang byte: consume ONE FULL CHAR so the diag spans it whole
    // (a byte-wide span inside a multi-byte char would render as mojibake).
    let start = cur.pos();
    match cur.next_char() {
        Ok(Some(c)) => Err(Diag::at_code(
            codes::UNEXPECTED_CHAR,
            format!("unexpected character `{c}`"),
            cur.span_from(start),
        )),
        _ => Err(Diag::at_code(
            codes::UNEXPECTED_CHAR,
            "unexpected byte",
            Span::new(start, start + 1),
        )),
    }
}

/// A `"…"` literal. The token's span INCLUDES the quotes; escapes are only
/// validated here and decoded by [`unescape`] at parse. Strings are
/// single-line: a raw newline (or EOF) before the closing quote is an error
/// pinned at the opener, and only `\"`, `\\`, `\n` escapes exist.
fn str_literal(cur: &mut Cursor<'_>) -> Result<TokKind, Diag> {
    let start = cur.pos();
    cur.bump(); // opening quote
    loop {
        match cur.peek() {
            None | Some(b'\n') => {
                return Err(Diag::at_code(
                    codes::UNTERMINATED_STRING,
                    "unterminated string literal",
                    Span::new(start, start + 1),
                ));
            }
            Some(b'"') => {
                cur.bump();
                return Ok(TokKind::Str);
            }
            Some(b'\\') => {
                cur.bump();
                match cur.peek() {
                    Some(b'"') | Some(b'\\') | Some(b'n') => {
                        cur.bump();
                    }
                    _ => {
                        let esc = cur.pos() - 1;
                        let _ = cur.next_char(); // span the escaped char whole
                        return Err(Diag::at_code(
                            codes::BAD_ESCAPE,
                            "unknown escape (only \\\" \\\\ \\n exist)",
                            cur.span_from(esc),
                        ));
                    }
                }
            }
            Some(_) => {
                // Multi-byte chars are fine inside strings — consume whole.
                let _ = cur.next_char();
            }
        }
    }
}

/// Decode a `Str` token's source slice (quotes included) to its value.
/// Total by construction: the lexer already rejected every other shape.
pub(crate) fn unescape(quoted: &str) -> String {
    let inner = &quoted[1..quoted.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(e) => out.push(e), // `"` or `\` — the lexer allowed no other
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Decimal only (underscore separators allowed) — an app language needs no
/// hex. A digit run flowing into ident chars (`123abc`) is ONE malformed
/// literal, never two silently-misparsed tokens.
fn int_literal(cur: &mut Cursor<'_>) -> Result<TokKind, Diag> {
    let start = cur.pos();
    let digits = cur.eat_decimal(true).expect("caller saw a digit");
    if cur.peek().is_some_and(ident_start) {
        cur.eat_while(ident_cont);
        return Err(Diag::at_code(
            codes::BAD_INT,
            "malformed integer literal",
            cur.span_from(start),
        ));
    }
    let text: String = cur.text(digits).chars().filter(|&c| c != '_').collect();
    text.parse().map(TokKind::Int).map_err(|_| {
        Diag::at_code(
            codes::BAD_INT,
            format!("integer literal out of range (max {})", i64::MAX),
            cur.span_from(start),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokKind> {
        lex(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn keywords_operators_and_literals() {
        assert_eq!(
            kinds("state x = 1; button row col input label"),
            [
                TokKind::State,
                TokKind::Ident,
                TokKind::Assign,
                TokKind::Int(1),
                TokKind::Semi,
                TokKind::Button,
                TokKind::Row,
                TokKind::Col,
                TokKind::Input,
                TokKind::Label,
                TokKind::Eof
            ]
        );
        assert_eq!(kinds("1_000")[0], TokKind::Int(1000));
    }

    #[test]
    fn strings_escapes_and_unicode() {
        let toks = lex("\"héllo\" \"a\\\"b\\\\c\\nd\"").unwrap();
        assert_eq!(toks[0].kind, TokKind::Str);
        assert_eq!(unescape("\"héllo\""), "héllo");
        assert_eq!(unescape("\"a\\\"b\\\\c\\nd\""), "a\"b\\c\nd");
        // The span includes both quotes and the multi-byte char whole.
        assert_eq!(toks[0].span.end, "\"héllo\"".len());
    }

    #[test]
    fn bad_strings_are_coded() {
        assert_eq!(lex("\"open").unwrap_err().code, Some(codes::UNTERMINATED_STRING));
        assert_eq!(lex("\"line\nbreak\"").unwrap_err().code, Some(codes::UNTERMINATED_STRING));
        let e = lex("\"bad \\q escape\"").unwrap_err();
        assert_eq!(e.code, Some(codes::BAD_ESCAPE));
        assert_eq!(lex("123abc").unwrap_err().code, Some(codes::BAD_INT));
        assert_eq!(lex("@").unwrap_err().code, Some(codes::UNEXPECTED_CHAR));
    }
}

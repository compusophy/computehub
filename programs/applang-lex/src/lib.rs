//! # applang-lex: applang's tokens
//!
//! The lexer under `applang-syntax`'s parser ([`lex`]: source text to `Copy` spanned tokens on
//! the `lang::lex` cursor; `Ident` and `Str` text is re-sliced by span, unescaped at parse by
//! [`unescape`]), [`highlight`] (every token and comment classed for an editor, never failing)
//! and [`codes`], the one diagnostic table of the applang crates.
//!
//! Forked from litelite's applite 0.2.0 (commit 4f5e056), whose lexer and codes these were.

#![forbid(unsafe_code)]

use lang::lex::{Cursor, ident_cont, ident_start};
use lang::{Diag, Span};

/// A token kind; `Int` carries its value, at most 2^63 (the least int's magnitude).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub enum TokKind {
    Int(u64), Ident, Str, True, False,
    State, Label, Button, Input, Let, If, Else, Repeat, While, Break, Continue, Fn, For, In,
    Return, Plus, Minus, Star, Slash, Percent, Bang, BangEq, Assign, EqEq, PlusEq, MinusEq,
    StarEq, SlashEq, PercentEq,
    Lt, LtEq, Gt, GtEq, AndAnd, OrOr, LParen, RParen, LBrace, RBrace, Semi,
    LBracket, RBracket, Comma, Colon, Question, Arrow, DotDot, Eof,
}
use TokKind::*;

/// The reserved words. `saved`, `every`, `on`, `key`, `row`, `col`, `grid`, `canvas` and `cell`
/// are words only where a name could not be (so a state may still be called `row` or `on`).
#[rustfmt::skip]
const KEYWORDS: [(&str, TokKind); 17] = [
    ("state", State), ("label", Label), ("button", Button), ("input", Input), ("let", Let),
    ("if", If), ("else", Else), ("repeat", Repeat), ("while", While), ("break", Break),
    ("continue", Continue), ("true", True), ("false", False), ("fn", Fn), ("for", For),
    ("in", In), ("return", Return),
];

/// Two-char operators first, so the longest match wins.
#[rustfmt::skip]
const PUNCT: [(&str, TokKind); 32] = [
    ("&&", AndAnd), ("||", OrOr), ("==", EqEq), ("!=", BangEq), ("<=", LtEq), (">=", GtEq),
    ("+=", PlusEq), ("-=", MinusEq), ("*=", StarEq), ("/=", SlashEq), ("%=", PercentEq),
    ("->", Arrow), ("..", DotDot),
    ("+", Plus), ("-", Minus), ("*", Star), ("/", Slash), ("%", Percent), ("!", Bang),
    ("=", Assign), ("<", Lt), (">", Gt), ("(", LParen), (")", RParen), ("{", LBrace),
    ("}", RBrace), (";", Semi), ("[", LBracket), ("]", RBracket), (",", Comma), (":", Colon),
    ("?", Question),
];

/// A spanned token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub struct Token { pub kind: TokKind, pub span: Span }

impl lang::parse::Tok for Token {
    fn span(&self) -> Span {
        self.span
    }
}

/// Lexes `src` into tokens ending in `Eof`; whitespace, `//` and nested `/* */` are trivia.
pub fn lex(src: &str) -> Result<Vec<Token>, Diag> {
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

/// What a range of source is, for an editor's colors. Keywords include `true`
/// and `false`; an unterminated comment runs to the end; Error is a bad token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Class { Keyword, Str, Number, Comment, Name, Punct, Error }

/// Every token and comment of `src` as classed byte ranges on char boundaries,
/// in order. Never fails: a bad token is one [`Class::Error`] range.
pub fn highlight(src: &str) -> Vec<(Span, Class)> {
    let mut cur = Cursor::new(src);
    let mut out = Vec::new();
    loop {
        cur.skip_ws();
        let start = cur.pos();
        let class =
            if cur.skip_line_comment("//") || cur.skip_block_comment("/*", "*/") != Ok(false) {
                Class::Comment
            } else if cur.at_eof() {
                return out;
            } else {
                match next_kind(&mut cur, start) {
                    Ok(Int(_)) => Class::Number,
                    Ok(Str) => Class::Str,
                    Ok(Ident) => Class::Name,
                    Ok(k) if KEYWORDS.iter().any(|&(_, kw)| kw == k) => Class::Keyword,
                    Ok(_) => Class::Punct,
                    Err(_) => Class::Error,
                }
            };
        // Every arm consumed at least one char; this is only a backstop.
        if cur.pos() == start {
            cur.next_char();
        }
        out.push((cur.span_from(start), class));
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

/// Decodes a `Str` token's source (quotes included); the lexer rejected bad escapes.
pub fn unescape(quoted: &str) -> String {
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

/// Decimal with `_` separators, up to 2^63: the parser takes that only after a `-`. A digit
/// run running into ident chars (`123abc`) is one malformed literal, never two tokens.
fn int_literal(cur: &mut Cursor<'_>, start: usize, digits: Span) -> Result<TokKind, Diag> {
    if cur.peek().is_some_and(ident_start) {
        cur.eat_while(ident_cont);
        let msg = "malformed integer literal";
        return Err(Diag::at_code(codes::BAD_INT, msg, cur.span_from(start)));
    }
    let text: String = cur.text(digits).chars().filter(|&c| c != '_').collect();
    text.parse().ok().filter(|&v: &u64| v <= 1 << 63).map(Int).ok_or_else(|| {
        let msg = format!("integer literal out of range (max {})", i64::MAX);
        Diag::at_code(codes::BAD_INT, msg, cur.span_from(start))
    })
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
    /// `applang_syntax::ast::MAX_STATE_BYTES`.
    pub const STATE_TOO_BIG: u16 = 212;
    /// A host event that no widget or state matches.
    pub const BAD_EVENT: u16 = 213;
    /// One render's text past `Limits::max_render_bytes`.
    pub const RENDER_TOO_BIG: u16 = 214;
    /// An index outside its list.
    pub const INDEX_OUT_OF_RANGE: u16 = 215;
    /// A list past `applang_syntax::ast::MAX_ITEMS` items.
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
    /// A canvas or a shape it cannot draw: a side past 1 to 1,024, a color past 0 to 11, a
    /// negative size, a number past -32,768 to 32,767, a text (or a sprite's row) of two lines,
    /// or more ink in one render than a canvas holds.
    pub const BAD_DRAW: u16 = 222;
    /// Canvases showed, but nothing they drew in a whole smoke test reached inside one (the
    /// smoke test's; at the program's first canvas).
    pub const OFF_CANVAS: u16 = 223;
    /// Pixels it cannot draw: a row past 1 to 64 cells, more than 64 rows or a part of one, a
    /// cell past -1 to 11, or more cells in one render than its canvases hold.
    pub const BAD_PIXELS: u16 = 224;
    /// A canvas's text, its point on the canvas, wider than the canvas, as near as its characters
    /// and size say (the smoke test's; at the program's first canvas).
    pub const TEXT_TOO_WIDE: u16 = 225;
    /// A name declared twice (a state, function or parameter).
    pub const DUP_STATE: u16 = 301;
    /// A name that is no declared state or visible local.
    pub const UNKNOWN_NAME: u16 = 302;
    pub const TYPE_MISMATCH: u16 = 303;
    /// A function that calls itself, directly or through others: applang has no recursion.
    pub const RECURSES: u16 = 304;
    /// A function with a result that does not end in `return`.
    pub const MISSING_RETURN: u16 = 305;
    /// A widget or interval that would change state.
    pub const IMPURE_RENDER: u16 = 306;
    /// An `on key` naming no key.
    pub const BAD_KEY: u16 = 307;
    /// A call with the wrong number of arguments.
    pub const ARITY: u16 = 308;
    /// A shape, or a function that draws, called where no canvas draws: a handler, an `every`
    /// or `on key`, a widget's expression, or a program with no canvas.
    pub const DRAW_OUTSIDE: u16 = 309;
    /// A `break` or `continue` outside a loop of its own function or handler.
    pub const OUTSIDE_LOOP: u16 = 310;
    /// A state list that a `clear` empties and nothing grows again: from then on it has no items.
    pub const CLEARED_FOR_GOOD: u16 = 311;
}

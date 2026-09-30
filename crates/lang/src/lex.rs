//! Byte-cursor lexer kit. The `struct { src, pos }` scaffold that rustlite,
//! soliditylite, and bashlite each hand-rolled — written once, with the
//! invariants that bit them baked in: UTF-8-safe char consumption (the mojibake
//! bug was fixed twice, differently), explicit nested-vs-flat block comments
//! (the two compilers silently diverged), and span-preserving trivia skipping.
//!
//! Zero dependencies beyond `lang::diag`. Native + wasm32.

// Every primitive here hands back a Span, so `lang::lex` alone must
// be enough to name one.
pub use crate::diag::Span;

/// A byte cursor over source text. The kit's primitives return [`Span`]s so
/// every token a language builds is location-pinned by construction.
pub struct Cursor<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    /// Current byte offset.
    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn at_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// Byte at the cursor, if any.
    pub fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    /// Byte at `cursor + n`.
    pub fn peek_at(&self, n: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + n).copied()
    }

    /// Advance one byte and return it. Only safe for ASCII decisions — use
    /// [`next_char`](Self::next_char) when the byte may start a multi-byte char.
    pub fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    /// Consume `b` if it is next.
    pub fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Consume `prefix` if the source continues with it.
    pub fn eat_str(&mut self, prefix: &str) -> bool {
        if self.src[self.pos..].starts_with(prefix) {
            self.pos += prefix.len();
            true
        } else {
            false
        }
    }

    /// Consume bytes while `pred` holds; the consumed span (possibly empty).
    pub fn eat_while(&mut self, pred: impl Fn(u8) -> bool) -> Span {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if !pred(b) {
                break;
            }
            self.pos += 1;
        }
        Span::new(start, self.pos)
    }

    /// Span from `start` to the cursor.
    pub fn span_from(&self, start: usize) -> Span {
        Span::new(start, self.pos)
    }

    /// Source text of `span` (clamped to char boundaries; never panics).
    pub fn text(&self, span: Span) -> &'a str {
        let start = floor_boundary(self.src, span.start);
        let end = floor_boundary(self.src, span.end.min(self.src.len()));
        &self.src[start..end.max(start)]
    }

    /// Decode the char at the cursor and advance past it. `Err(span)` on a
    /// byte sequence that is not valid UTF-8 — never split a char in half:
    /// byte-wise `push(b as char)` is the mojibake bug this kit exists to kill.
    pub fn next_char(&mut self) -> Result<Option<char>, Span> {
        if self.at_eof() {
            return Ok(None);
        }
        match self.src[self.pos..].chars().next() {
            Some(c) => {
                self.pos += c.len_utf8();
                Ok(Some(c))
            }
            // &str is always valid UTF-8, so chars() at a boundary never fails;
            // the reachable error is a cursor parked mid-char by byte ops.
            None => Err(Span::new(self.pos, self.pos + 1)),
        }
    }

    /// Skip spaces/tabs/newlines/CR.
    pub fn skip_ws(&mut self) {
        self.eat_while(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
    }

    /// Skip a line comment if `prefix` is next; true when one was skipped.
    pub fn skip_line_comment(&mut self, prefix: &str) -> bool {
        if !self.src[self.pos..].starts_with(prefix) {
            return false;
        }
        self.eat_while(|b| b != b'\n');
        true
    }

    /// Skip a block comment if `open` is next. `nested` says whether `open`
    /// inside the comment nests (rustlite) or not (soliditylite) — an explicit
    /// flag because the two parents silently diverged here.
    /// `Err(span-of-open)` when unterminated.
    pub fn skip_block_comment(
        &mut self,
        open: &str,
        close: &str,
        nested: bool,
    ) -> Result<bool, Span> {
        let start = self.pos;
        if !self.eat_str(open) {
            return Ok(false);
        }
        let mut depth = 1usize;
        while depth > 0 {
            if self.at_eof() {
                return Err(Span::new(start, start + open.len()));
            }
            if self.eat_str(close) {
                depth -= 1;
            } else if nested && self.eat_str(open) {
                depth += 1;
            } else {
                // Advance one full char, not one byte (multi-byte safe).
                self.next_char()?;
            }
        }
        Ok(true)
    }

    /// Consume an identifier: one `is_start` byte then `is_cont` bytes.
    pub fn eat_ident(
        &mut self,
        is_start: impl Fn(u8) -> bool,
        is_cont: impl Fn(u8) -> bool,
    ) -> Option<Span> {
        let start = self.pos;
        match self.peek() {
            Some(b) if is_start(b) => self.pos += 1,
            _ => return None,
        }
        self.eat_while(is_cont);
        Some(self.span_from(start))
    }

    /// Consume decimal digits (optionally with `_` separators). The span
    /// includes separators; strip them when parsing the value.
    pub fn eat_decimal(&mut self, allow_underscore: bool) -> Option<Span> {
        self.eat_digits(|b| b.is_ascii_digit(), allow_underscore)
    }

    /// Consume hex digits (optionally with `_` separators), e.g. after `0x`.
    pub fn eat_hex(&mut self, allow_underscore: bool) -> Option<Span> {
        self.eat_digits(|b| b.is_ascii_hexdigit(), allow_underscore)
    }

    fn eat_digits(&mut self, is_digit: impl Fn(u8) -> bool, sep: bool) -> Option<Span> {
        let start = self.pos;
        match self.peek() {
            Some(b) if is_digit(b) => self.pos += 1,
            _ => return None,
        }
        self.eat_while(|b| is_digit(b) || (sep && b == b'_'));
        Some(self.span_from(start))
    }
}

/// Standard identifier-start: `[A-Za-z_]`.
pub fn ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Standard identifier-continue: `[A-Za-z0-9_]`.
pub fn ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idents_and_spans() {
        let mut c = Cursor::new("foo_1 +");
        let s = c.eat_ident(ident_start, ident_cont).unwrap();
        assert_eq!(c.text(s), "foo_1");
        c.skip_ws();
        assert!(c.eat(b'+'));
        assert!(c.eat_ident(ident_start, ident_cont).is_none());
        assert!(c.at_eof());
    }

    #[test]
    fn digits_respect_the_underscore_flag() {
        let mut c = Cursor::new("1_000");
        let s = c.eat_decimal(true).unwrap();
        assert_eq!(c.text(s), "1_000");
        let mut c = Cursor::new("1_000");
        let s = c.eat_decimal(false).unwrap();
        assert_eq!(c.text(s), "1"); // stops at `_`
        let mut c = Cursor::new("dead_beef");
        let s = c.eat_hex(true).unwrap();
        assert_eq!(c.text(s), "dead_beef");
    }

    #[test]
    fn block_comments_nested_vs_flat() {
        // Nested (rustlite semantics): inner /* */ nests.
        let mut c = Cursor::new("/* a /* b */ c */x");
        assert!(c.skip_block_comment("/*", "*/", true).unwrap());
        assert_eq!(c.peek(), Some(b'x'));
        // Flat (soliditylite semantics): first */ closes.
        let mut c = Cursor::new("/* a /* b */ c */x");
        assert!(c.skip_block_comment("/*", "*/", false).unwrap());
        assert_eq!(c.pos(), 12);
        // Unterminated → Err pinned at the opener.
        let mut c = Cursor::new("abc /* nope");
        c.eat_while(|b| b != b'/');
        let e = c.skip_block_comment("/*", "*/", true).unwrap_err();
        assert_eq!((e.start, e.end), (4, 6));
    }

    #[test]
    fn line_comments_stop_at_newline() {
        let mut c = Cursor::new("# hi\nx");
        assert!(c.skip_line_comment("#"));
        assert_eq!(c.peek(), Some(b'\n'));
        assert!(!c.skip_line_comment("#"));
    }

    #[test]
    fn next_char_is_multibyte_safe() {
        let mut c = Cursor::new("é—😀");
        assert_eq!(c.next_char().unwrap(), Some('é'));
        assert_eq!(c.next_char().unwrap(), Some('—'));
        assert_eq!(c.next_char().unwrap(), Some('😀'));
        assert_eq!(c.next_char().unwrap(), None);
    }

    #[test]
    fn block_comment_skips_multibyte_content() {
        let mut c = Cursor::new("/* — em-dash 😀 */x");
        assert!(c.skip_block_comment("/*", "*/", true).unwrap());
        assert_eq!(c.peek(), Some(b'x'));
    }

    #[test]
    fn text_clamps_to_char_boundaries() {
        let c = Cursor::new("a—b");
        // Span ends mid-em-dash: clamp, don't panic.
        assert_eq!(c.text(Span::new(0, 2)), "a");
        assert_eq!(c.text(Span::new(0, 99)), "a—b");
    }
}

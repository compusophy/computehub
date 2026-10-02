//! A byte cursor for lexers: UTF-8-safe char consumption, nested block
//! comments and span-returning primitives, so every token is location-pinned.

pub use crate::diag::Span;
use crate::diag::floor_boundary;

/// A byte cursor over source text.
pub struct Cursor<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    /// The current byte offset.
    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn at_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// The byte at the cursor, if any.
    pub fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    /// Advances one byte and returns it; for ASCII only (see [`Self::next_char`]).
    pub fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    /// Consumes `prefix` if the source continues with it.
    pub fn eat_str(&mut self, prefix: &str) -> bool {
        let found = self.src[self.pos..].starts_with(prefix);
        if found {
            self.pos += prefix.len();
        }
        found
    }

    /// Consumes bytes while `pred` holds; the consumed span (possibly empty).
    pub fn eat_while(&mut self, pred: impl Fn(u8) -> bool) -> Span {
        let start = self.pos;
        while self.peek().is_some_and(&pred) {
            self.pos += 1;
        }
        Span::new(start, self.pos)
    }

    /// The span from `start` to the cursor.
    pub fn span_from(&self, start: usize) -> Span {
        Span::new(start, self.pos)
    }

    /// The source text of `span`, clamped to char boundaries (never panics).
    pub fn text(&self, span: Span) -> &'a str {
        let start = floor_boundary(self.src, span.start);
        &self.src[start..floor_boundary(self.src, span.end).max(start)]
    }

    /// Decodes the char at the cursor and advances past it whole.
    pub fn next_char(&mut self) -> Option<char> {
        let c = self.src[self.pos..].chars().next()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Skips spaces, tabs, newlines and CRs.
    pub fn skip_ws(&mut self) {
        self.eat_while(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
    }

    /// Skips a line comment if `prefix` is next; whether one was skipped.
    pub fn skip_line_comment(&mut self, prefix: &str) -> bool {
        let found = self.src[self.pos..].starts_with(prefix);
        if found {
            self.eat_while(|b| b != b'\n');
        }
        found
    }

    /// Skips a nested block comment if `open` is next; whether one was
    /// skipped, or `Err(span of the opener)` when it is unterminated.
    pub fn skip_block_comment(&mut self, open: &str, close: &str) -> Result<bool, Span> {
        let start = self.pos;
        if !self.eat_str(open) {
            return Ok(false);
        }
        let mut depth = 1usize;
        while depth > 0 {
            if self.eat_str(close) {
                depth -= 1;
            } else if self.eat_str(open) {
                depth += 1;
            } else if self.next_char().is_none() {
                return Err(Span::new(start, start + open.len()));
            }
        }
        Ok(true)
    }

    /// Consumes an identifier: one [`ident_start`] byte, then [`ident_cont`] bytes.
    pub fn eat_ident(&mut self) -> Option<Span> {
        self.eat_run(ident_start, ident_cont)
    }

    /// Consumes decimal digits and `_` separators (the span keeps the separators).
    pub fn eat_decimal(&mut self) -> Option<Span> {
        self.eat_run(|b| b.is_ascii_digit(), |b| b.is_ascii_digit() || b == b'_')
    }

    fn eat_run(&mut self, first: fn(u8) -> bool, rest: fn(u8) -> bool) -> Option<Span> {
        let start = self.pos;
        if !self.peek().is_some_and(first) {
            return None;
        }
        self.pos += 1;
        self.eat_while(rest);
        Some(self.span_from(start))
    }
}

/// `[A-Za-z_]`.
pub fn ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// `[A-Za-z0-9_]`.
pub fn ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idents_digits_and_spans() {
        let mut c = Cursor::new("foo_1 +");
        let s = c.eat_ident().unwrap();
        assert_eq!(c.text(s), "foo_1");
        c.skip_ws();
        assert!(c.eat_str("+") && c.eat_ident().is_none() && c.at_eof());
        let mut c = Cursor::new("1_000x");
        let s = c.eat_decimal().unwrap();
        assert_eq!((c.text(s), c.peek()), ("1_000", Some(b'x')));
        assert_eq!((c.eat_decimal(), c.pos()), (None, 5));
    }

    #[test]
    fn comments_nest_skip_whole_chars_and_pin_the_unterminated() {
        let mut c = Cursor::new("/* a /* b */ c — 😀 */x");
        assert_eq!(c.skip_block_comment("/*", "*/"), Ok(true));
        assert_eq!(c.peek(), Some(b'x'));
        assert_eq!(c.skip_block_comment("/*", "*/"), Ok(false));
        let mut c = Cursor::new("abc /* nope");
        c.eat_while(|b| b != b'/');
        assert_eq!(c.skip_block_comment("/*", "*/"), Err(Span::new(4, 6)));
        let mut c = Cursor::new("# hi\nx");
        assert!(c.skip_line_comment("#"));
        assert_eq!(c.peek(), Some(b'\n'));
        assert!(!c.skip_line_comment("#"));
    }

    #[test]
    fn chars_and_text_are_multibyte_safe() {
        let mut c = Cursor::new("é—😀");
        let got: Vec<_> = std::iter::from_fn(|| c.next_char()).collect();
        assert_eq!(got, ['é', '—', '😀']);
        let c = Cursor::new("a—b");
        // A span ending mid-em-dash clamps instead of panicking.
        assert_eq!((c.text(Span::new(0, 2)), c.text(Span::new(0, 99))), ("a", "a—b"));
    }
}

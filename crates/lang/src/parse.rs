//! A token cursor for recursive descent with a built-in depth guard: a
//! parser on it recurses only through [`TokCursor::guarded`] (or
//! [`TokCursor::enter`]), so deeply nested input is a diagnostic, never a
//! stack overflow (on wasm an uncatchable abort).

pub use crate::diag::Span;

/// The nesting cap, in guard entries: one paren level costs about two
/// entries and ten real frames, well inside a browser wasm stack.
pub const DEFAULT_MAX_DEPTH: usize = 96;

/// Anything with a [`Span`] can ride the cursor.
pub trait Tok {
    fn span(&self) -> Span;
}

/// A token cursor over a non-empty slice ending in an EOF sentinel;
/// [`Self::advance`] clamps there, so peeking past the end keeps
/// returning EOF.
pub struct TokCursor<'a, T: Tok> {
    toks: &'a [T],
    pos: usize,
    depth: usize,
}

impl<'a, T: Tok> TokCursor<'a, T> {
    /// Panics if `toks` is empty: append the EOF sentinel first.
    pub fn new(toks: &'a [T]) -> Self {
        assert!(!toks.is_empty(), "TokCursor requires an EOF-terminated, non-empty token slice");
        Self { toks, pos: 0, depth: 0 }
    }

    /// The current token (the sentinel once input is exhausted).
    pub fn peek(&self) -> &'a T {
        &self.toks[self.pos]
    }

    /// Whether the cursor sits on the last (sentinel) token.
    pub fn at_last(&self) -> bool {
        self.pos == self.toks.len() - 1
    }

    /// Returns the current token and advances, clamping at the sentinel.
    pub fn advance(&mut self) -> &'a T {
        let tok = &self.toks[self.pos];
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        tok
    }

    /// Advances past the current token if `pred` accepts it.
    pub fn eat(&mut self, pred: impl Fn(&T) -> bool) -> Option<&'a T> {
        if pred(self.peek()) { Some(self.advance()) } else { None }
    }

    /// Enters one level; past the cap, `Err(current span)`. Every `enter`,
    /// failed ones included, needs a [`Self::leave`].
    pub fn enter(&mut self) -> Result<(), Span> {
        self.depth += 1;
        if self.depth > DEFAULT_MAX_DEPTH {
            return Err(self.peek().span());
        }
        Ok(())
    }

    pub fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Runs `f` inside one guard entry, leaving on every path.
    pub fn guarded<R, E: From<Span>>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<R, E>,
    ) -> Result<R, E> {
        let r = match self.enter() {
            Ok(()) => f(self),
            Err(sp) => Err(E::from(sp)),
        };
        self.leave();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct T(u8, Span);
    impl Tok for T {
        fn span(&self) -> Span {
            self.1
        }
    }

    fn toks(kinds: &[u8]) -> Vec<T> {
        kinds.iter().enumerate().map(|(i, &k)| T(k, Span::new(i, i + 1))).collect()
    }

    #[test]
    fn advance_eat_and_the_sentinel() {
        let ts = toks(b"(1)$");
        let mut c = TokCursor::new(&ts);
        assert!(c.eat(|t| t.0 == b'(').is_some() && c.eat(|t| t.0 == b'(').is_none());
        for _ in 0..5 {
            c.advance();
        }
        assert!(c.at_last());
        assert_eq!((c.peek().0, c.peek().span()), (b'$', Span::new(3, 4)));
    }

    #[test]
    fn the_guard_trips_past_the_cap_and_guarded_always_leaves() {
        let ts = toks(b"$");
        let mut c = TokCursor::new(&ts);
        for _ in 0..DEFAULT_MAX_DEPTH {
            assert!(c.enter().is_ok());
        }
        assert_eq!(c.enter(), Err(Span::new(0, 1)));
        for _ in 0..DEFAULT_MAX_DEPTH + 5 {
            c.leave(); // saturates, never underflows
        }
        assert_eq!(c.depth, 0);
        // Errors deep inside, and the guard's own trip, both unwind to 0.
        fn descend(c: &mut TokCursor<'_, T>, n: usize) -> Result<(), Span> {
            c.guarded(|c| if n == 0 { Err(c.peek().span()) } else { descend(c, n - 1) })
        }
        for n in [5, 2 * DEFAULT_MAX_DEPTH] {
            assert!(descend(&mut c, n).is_err());
            assert_eq!(c.depth, 0);
        }
    }
}

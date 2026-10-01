//! Recursive-descent parser harness: token cursor + the recursion depth guard,
//! defined ONCE. rustlite and soliditylite each carried a verbatim copy of this
//! ~120-LOC scaffold (including a duplicated `MAX_RECURSION_DEPTH = 96`);
//! bashlite had NO guard — nested `if`-in-`if` recursed unbounded. A language
//! built on this harness cannot forget the guard, because `enter()` is the only
//! way in.
//!
//! Zero dependencies beyond `lang::diag`. Native + wasm32.

// `Tok::span`, `enter`, and `guarded`'s error all speak Span, so
// `lang::parse` alone must be enough to name one.
pub use crate::diag::Span;

/// Hard cap on recursive-descent nesting depth. Purpose-sized languages run on
/// agent/LLM-authored source, often inside a browser wasm stack; without a cap,
/// deeply nested input (`((((…))))`, `{{{…}}}`) recurses one frame per token
/// and overflows the stack — on wasm an UNCATCHABLE abort that kills the whole
/// tab instead of returning a diagnostic.
///
/// The cap is in "guard entries", not source nesting levels: one paren level
/// typically costs ~2 entries (expr + unary) and ~10 real stack frames (the
/// precedence ladder), so the cap must stay well under stack_size /
/// frames_per_entry. 96 entries ≈ 48 paren levels ≈ a few hundred frames —
/// comfortably inside a browser wasm stack, far beyond real programs.
pub const DEFAULT_MAX_DEPTH: usize = 96;

/// Anything with a [`Span`] can ride the cursor.
pub trait Tok {
    fn span(&self) -> Span;
}

/// A clamping token cursor with a built-in depth guard.
///
/// Convention: the token slice is non-empty and ends with the language's
/// EOF-sentinel token; [`advance`](Self::advance) clamps at the last token, so
/// `peek()` past the end keeps returning EOF instead of panicking.
pub struct TokCursor<'a, T: Tok> {
    toks: &'a [T],
    pos: usize,
    depth: usize,
    max_depth: usize,
}

impl<'a, T: Tok> TokCursor<'a, T> {
    /// Panics if `toks` is empty — append your EOF sentinel first.
    pub fn new(toks: &'a [T]) -> Self {
        Self::with_max_depth(toks, DEFAULT_MAX_DEPTH)
    }

    pub fn with_max_depth(toks: &'a [T], max_depth: usize) -> Self {
        assert!(!toks.is_empty(), "TokCursor requires an EOF-terminated, non-empty token slice");
        Self { toks, pos: 0, depth: 0, max_depth }
    }

    /// Current token (the EOF sentinel once input is exhausted).
    pub fn peek(&self) -> &'a T {
        &self.toks[self.pos]
    }

    /// Token at `cursor + n`, clamped to the sentinel.
    pub fn peek_at(&self, n: usize) -> &'a T {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)]
    }

    /// Span of the current token.
    pub fn span(&self) -> Span {
        self.peek().span()
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// True once the cursor sits on the last (sentinel) token.
    pub fn at_last(&self) -> bool {
        self.pos == self.toks.len() - 1
    }

    /// Return the current token and advance (clamping at the sentinel).
    pub fn advance(&mut self) -> &'a T {
        let tok = &self.toks[self.pos];
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        tok
    }

    /// Advance past the current token if `pred` accepts it.
    pub fn eat(&mut self, pred: impl Fn(&T) -> bool) -> Option<&'a T> {
        if pred(self.peek()) { Some(self.advance()) } else { None }
    }

    /// Enter one recursion level; `Err(current span)` past the cap — map it to
    /// your language's "nesting too deep" diagnostic. Pair with
    /// [`leave`](Self::leave) on every return path (or use
    /// [`guarded`](Self::guarded), which pairs them for you).
    pub fn enter(&mut self) -> Result<(), Span> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(self.span());
        }
        Ok(())
    }

    pub fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Run `f` inside one guard entry, leaving on ALL paths — the pattern the
    /// hand-rolled parsers occasionally got wrong on early returns. `enter()`
    /// increments even when it trips, so the trip path must leave too (a leak
    /// this harness's own first test run caught — the invariant earns its keep).
    pub fn guarded<R, E: From<Span>>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<R, E>,
    ) -> Result<R, E> {
        if let Err(sp) = self.enter() {
            self.leave();
            return Err(E::from(sp));
        }
        let r = f(self);
        self.leave();
        r
    }

    pub fn depth(&self) -> usize {
        self.depth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum K {
        Num,
        Open,
        Close,
        Eof,
    }
    struct T(K, Span);
    impl Tok for T {
        fn span(&self) -> Span {
            self.1
        }
    }
    fn toks(kinds: &[K]) -> Vec<T> {
        kinds
            .iter()
            .enumerate()
            .map(|(i, k)| {
                T(
                    match k {
                        K::Num => K::Num,
                        K::Open => K::Open,
                        K::Close => K::Close,
                        K::Eof => K::Eof,
                    },
                    Span::new(i, i + 1),
                )
            })
            .collect()
    }

    #[test]
    fn advance_clamps_at_the_sentinel() {
        let ts = toks(&[K::Num, K::Eof]);
        let mut c = TokCursor::new(&ts);
        c.advance();
        assert!(c.at_last());
        c.advance();
        c.advance();
        assert_eq!(c.peek().0, K::Eof); // still EOF, no panic
        assert_eq!(c.span(), Span::new(1, 2));
    }

    #[test]
    fn depth_guard_trips_past_the_cap() {
        let ts = toks(&[K::Eof]);
        let mut c = TokCursor::with_max_depth(&ts, 3);
        assert!(c.enter().is_ok());
        assert!(c.enter().is_ok());
        assert!(c.enter().is_ok());
        let e = c.enter().unwrap_err();
        assert_eq!(e, Span::new(0, 1)); // pinned at the current token
        c.leave();
        c.leave();
        c.leave();
        c.leave();
        c.leave(); // saturates, never underflows
        assert_eq!(c.depth(), 0);
        assert!(c.enter().is_ok());
    }

    #[test]
    fn guarded_leaves_on_error_paths_too() {
        let ts = toks(&[K::Eof]);
        let mut c = TokCursor::with_max_depth(&ts, 8);
        // A recursive descent that errors deep inside: depth must return to 0.
        fn descend(c: &mut TokCursor<'_, T>, n: usize) -> Result<(), Span> {
            c.guarded(|c| if n == 0 { Err(c.span()) } else { descend(c, n - 1) })
        }
        assert!(descend(&mut c, 5).is_err());
        assert_eq!(c.depth(), 0);
        // Deeper than the cap → the guard itself errors.
        assert!(descend(&mut c, 20).is_err());
        assert_eq!(c.depth(), 0);
    }

    #[test]
    fn eat_and_peek_at() {
        let ts = toks(&[K::Open, K::Num, K::Close, K::Eof]);
        let mut c = TokCursor::new(&ts);
        assert!(c.eat(|t| t.0 == K::Open).is_some());
        assert!(c.eat(|t| t.0 == K::Open).is_none());
        assert_eq!(c.peek_at(1).0, K::Close);
        assert_eq!(c.peek_at(99).0, K::Eof); // clamps
    }
}

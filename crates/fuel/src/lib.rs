//! Fuel + byte-budget primitives, defined once. The parents implemented this
//! idea three unrelated ways (bashlite's per-statement fuel + 256 KiB output
//! caps, soliditylite's interpreter STEP_BUDGET + memory cap, rustlite's
//! parse-depth cap + out-of-band JS watchdog).
//!
//! Fuel is the guarantee engine: an evaluator that burns fuel on every step is
//! MECHANICALLY total — "this program halts within N units" is a property you
//! get by construction, not by review. The composition rule that matters (and
//! that bashlite got right): sub-programs spend from the SAME budget, so
//! fractal composition terminates cleanly too.
//!
//! Zero dependencies. Native + wasm32.
//!
//! Forked from litelite's fuellite 0.2.0 (commit 4f5e056).

#![forbid(unsafe_code)]

/// The budget is spent. Map to your language's "fuel exhausted" diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exhausted;

impl std::fmt::Display for Exhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fuel exhausted")
    }
}

impl std::error::Error for Exhausted {}

/// A draining execution budget. Pass `&mut Fuel` down into every sub-evaluation
/// (substitution, sourced script, child call) — never fork a fresh budget for a
/// child, or composition re-opens the runaway hole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fuel {
    remaining: u64,
}

impl Fuel {
    pub fn new(units: u64) -> Self {
        Self { remaining: units }
    }

    /// Spend `cost` units; `Err(Exhausted)` if the tank runs dry. The failing
    /// call leaves the tank at 0 (no partial spend below zero).
    pub fn burn(&mut self, cost: u64) -> Result<(), Exhausted> {
        if cost > self.remaining {
            self.remaining = 0;
            return Err(Exhausted);
        }
        self.remaining -= cost;
        Ok(())
    }

    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    pub fn is_exhausted(&self) -> bool {
        self.remaining == 0
    }
}

/// A capped output sink counter. Appends clamp instead of erroring, so bounded
/// producers keep going and the caller learns the output was clipped —
/// bashlite's MAX_OUTPUT_BYTES pattern (stdout AND stderr each get one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteBudget {
    cap: usize,
    used: usize,
}

impl ByteBudget {
    pub fn new(cap: usize) -> Self {
        Self { cap, used: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.cap - self.used
    }

    pub fn used(&self) -> usize {
        self.used
    }

    /// Reserve up to `want` bytes; returns the granted amount.
    pub fn grant(&mut self, want: usize) -> usize {
        let g = want.min(self.remaining());
        self.used += g;
        g
    }

    /// Append `s` to `dst` within budget, truncating at a char boundary.
    /// Returns `false` when anything was clipped.
    pub fn push_str(&mut self, dst: &mut String, s: &str) -> bool {
        let granted = self.grant(s.len());
        if granted == s.len() {
            dst.push_str(s);
            return true;
        }
        // Clamp the cut to a char boundary; refund the bytes we can't use.
        let mut cut = granted;
        while cut > 0 && !s.is_char_boundary(cut) {
            cut -= 1;
        }
        self.used -= granted - cut;
        dst.push_str(&s[..cut]);
        false
    }

    /// Append `bytes` to `dst` within budget. `false` when clipped.
    pub fn push_bytes(&mut self, dst: &mut Vec<u8>, bytes: &[u8]) -> bool {
        let granted = self.grant(bytes.len());
        dst.extend_from_slice(&bytes[..granted]);
        granted == bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuel_burns_down_and_exhausts() {
        let mut f = Fuel::new(10);
        assert!(f.burn(4).is_ok());
        assert!(f.burn(6).is_ok());
        assert!(f.is_exhausted());
        assert_eq!(f.burn(1), Err(Exhausted));
        // Over-burn zeroes the tank rather than partially spending.
        let mut f = Fuel::new(5);
        assert_eq!(f.burn(9), Err(Exhausted));
        assert_eq!(f.remaining(), 0);
    }

    #[test]
    fn one_tank_bounds_a_whole_composition() {
        // An adversarial program that never returns on its own: parent and
        // recursive children spend from ONE &mut Fuel, so it halts by
        // exhaustion regardless of shape — the bashlite fractal invariant.
        fn spin(f: &mut Fuel, depth: usize) -> Result<(), Exhausted> {
            loop {
                f.burn(1)?;
                if depth > 0 {
                    spin(f, depth - 1)?;
                }
            }
        }
        let mut f = Fuel::new(10_000);
        assert_eq!(spin(&mut f, 5), Err(Exhausted));
        assert_eq!(f.remaining(), 0);
        // A finite program within budget completes with fuel to spare.
        let mut f = Fuel::new(100);
        for _ in 0..40 {
            f.burn(2).unwrap();
        }
        assert_eq!(f.remaining(), 20);
    }

    #[test]
    fn byte_budget_grants_and_clips() {
        let mut b = ByteBudget::new(5);
        let mut out = String::new();
        assert!(b.push_str(&mut out, "abc"));
        assert!(!b.push_str(&mut out, "defgh")); // clipped
        assert_eq!(out, "abcde");
        assert_eq!(b.remaining(), 0);
        assert!(!b.push_str(&mut out, "x"));
        assert_eq!(out, "abcde");
    }

    #[test]
    fn byte_budget_never_splits_a_char() {
        let mut b = ByteBudget::new(4);
        let mut out = String::new();
        // "a😀" is 5 bytes; only "a" fits without splitting the emoji.
        assert!(!b.push_str(&mut out, "a😀"));
        assert_eq!(out, "a");
        assert_eq!(b.used(), 1); // unusable grant refunded
        let mut v = Vec::new();
        let mut b2 = ByteBudget::new(2);
        assert!(!b2.push_bytes(&mut v, &[1, 2, 3]));
        assert_eq!(v, vec![1, 2]);
    }
}
